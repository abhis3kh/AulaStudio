use crate::protocol::{self, Lighting, Transport};
use hidapi::{HidApi, HidDevice};
use std::{
    collections::BTreeMap,
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
pub struct Keyboard {
    _lease: crate::connection_lease::ConnectionLease,
    device: HidDevice,
    pub transport: Transport,
    pub product: String,
    experimental_receiver_writes: bool,
}
#[derive(Debug, Clone)]
pub struct DeviceStatus {
    pub transport: Transport,
    pub product: String,
    pub lighting: Lighting,
    pub sleep_seconds: Option<u16>,
}
impl Keyboard {
    pub fn connect() -> Result<Self> {
        Self::connect_via(Some(Transport::Usb))
    }
    pub fn connect_via(preferred: Option<Transport>) -> Result<Self> {
        let api = HidApi::new().map_err(|e| e.to_string())?;
        let mut candidates = api
            .device_list()
            .filter(|d| {
                ((d.vendor_id() == 0x258a && d.product_id() == 0x010c && d.usage_page() >= 0xff00)
                    || (d.vendor_id() == 0x3554
                        && d.product_id() == 0xfa09
                        && d.usage_page() == 0xff02))
                    && preferred.is_none_or(|transport| match transport {
                        Transport::Usb => d.vendor_id() == 0x258a,
                        Transport::Receiver => d.vendor_id() == 0x3554,
                        Transport::Bluetooth => false,
                    })
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|d| (d.vendor_id() != 0x258a, d.interface_number() != 1));
        let mut errors = Vec::new();
        for info in candidates {
            let lease = crate::connection_lease::ConnectionLease::acquire()?;
            let device = match info.open_device(&api) {
                Ok(d) => d,
                Err(e) => {
                    errors.push(e.to_string());
                    continue;
                }
            };
            let mut keyboard = Self {
                _lease: lease,
                device,
                transport: if info.vendor_id() == 0x258a {
                    Transport::Usb
                } else {
                    Transport::Receiver
                },
                product: info.product_string().unwrap_or("AULA F75").to_string(),
                experimental_receiver_writes: false,
            };
            match keyboard.settings() {
                Ok(_) => return Ok(keyboard),
                Err(e) => errors.push(e),
            }
        }
        if errors.is_empty() {
            Err(match preferred {
                Some(Transport::Usb) => "F75 USB connection not found. Connect the USB cable and switch the keyboard to wired mode.".into(),
                Some(Transport::Receiver) => "F75 receiver connection not found.".into(),
                _ => "F75 not found. Connect the USB cable; wireless configuration is unsupported in this version.".into(),
            })
        } else {
            Err(format!(
                "No configurable F75 interface responded: {}",
                errors.join("; ")
            ))
        }
    }
    pub fn read(&mut self, command: u8, sub: u8, length: usize) -> Result<Vec<u8>> {
        match self.transport {
            Transport::Usb => {
                let requests = protocol::usb_reports(command | 0x80, sub, length, None)
                    .map_err(|e| e.to_string())?;
                let mut output = Vec::with_capacity(length);
                for (index, request) in requests.iter().enumerate() {
                    self.device
                        .send_feature_report(request)
                        .map_err(|e| e.to_string())?;
                    thread::sleep(Duration::from_millis(20));
                    let mut reply = [0u8; 520];
                    reply[0] = 6;
                    let received = self
                        .device
                        .get_feature_report(&mut reply)
                        .map_err(|e| e.to_string())?;
                    let size = u16::from_le_bytes([reply[6], reply[7]]) as usize;
                    if received < 8
                        || reply[0] != 6
                        || reply[1] & 0x7f != command
                        || reply[2] != sub
                        || reply[5] != index as u8
                        || size > 512
                        || received < 8 + size
                    {
                        return Err("Invalid USB response framing".into());
                    }
                    output.extend_from_slice(&reply[8..8 + size]);
                }
                if output.len() != length {
                    return Err(format!(
                        "Expected {length} bytes, received {}",
                        output.len()
                    ));
                }
                Ok(output)
            }
            Transport::Receiver => {
                let command = command | 0x40;
                let request = protocol::receiver_read(command, sub).map_err(|e| e.to_string())?;
                let expected = length.div_ceil(14);
                let mut chunks = BTreeMap::<usize, Vec<u8>>::new();
                // Retry replies can outlive a completed read. Remove queued reports
                // before starting a different request, especially a different Fn layer.
                for _ in 0..256 {
                    let mut pending = [0u8; 64];
                    if self
                        .device
                        .read_timeout(&mut pending, 0)
                        .map_err(|e| e.to_string())?
                        == 0
                    {
                        break;
                    }
                }
                for _ in 0..3 {
                    self.device.write(&request).map_err(|e| e.to_string())?;
                    let deadline = Instant::now() + Duration::from_millis(1500);
                    while Instant::now() < deadline {
                        let mut reply = [0u8; 64];
                        let n = self
                            .device
                            .read_timeout(&mut reply, 100)
                            .map_err(|e| e.to_string())?;
                        if n < 20 || reply[0] != 0x13 || reply[1] & 0x7f != command {
                            continue;
                        }
                        if reply[1] & 0x80 != 0 || reply[2] & 0x80 != 0 {
                            return Err("Keyboard rejected packet checksum".into());
                        }
                        let size = (reply[4] & 15) as usize;
                        let index = reply[3] as usize;
                        if size > 14 || index >= expected || (reply[2] & 0x7f) as usize != expected
                        {
                            return Err(format!(
                                "Invalid receiver response framing: packet {index}, payload {size} bytes, declared {} packets, expected {expected}",
                                reply[2] & 0x7f
                            ));
                        }
                        chunks.insert(index, reply[5..5 + size].to_vec());
                        if chunks.len() == expected {
                            let data = chunks.values().flatten().copied().collect::<Vec<_>>();
                            if data.len() != length {
                                return Err("Incomplete receiver response payload".into());
                            }
                            return Ok(data);
                        }
                    }
                }
                Err("Keyboard response timed out. Wake the keyboard and retry.".into())
            }
            Transport::Bluetooth => Err("Bluetooth writes and reads are disabled".into()),
        }
    }
    pub fn write(&mut self, command: u8, sub: u8, payload: &[u8]) -> Result<()> {
        // Hardware retests showed partial transfers and failed restoration.
        // Keep receiver reads available, but require the verified USB path for writes.
        if self.transport == Transport::Receiver && !self.experimental_receiver_writes {
            return Err("Wireless writes are disabled after hardware verification failed. Connect USB to apply changes.".into());
        }
        match self.transport {
            Transport::Usb => {
                for report in protocol::usb_reports(command, sub, payload.len(), Some(payload))
                    .map_err(|e| e.to_string())?
                {
                    thread::sleep(Duration::from_millis(20));
                    let mut result = Err("USB write failed".into());
                    for _ in 0..3 {
                        match self.device.send_feature_report(&report) {
                            Ok(_) => {
                                result = Ok(());
                                break;
                            }
                            Err(e) => {
                                result = Err(e.to_string());
                                thread::sleep(Duration::from_millis(70));
                            }
                        }
                    }
                    result?;
                }
                Ok(())
            }
            Transport::Receiver => {
                for report in
                    protocol::receiver_write(command, sub, payload).map_err(|e| e.to_string())?
                {
                    let mut acknowledged = false;
                    for _ in 0..6 {
                        thread::sleep(Duration::from_millis(15));
                        self.device.write(&report).map_err(|e| e.to_string())?;
                        if command & 0x80 != 0 {
                            acknowledged = true;
                            break;
                        }
                        // This installed OEM build waits 2 or 7 seconds for ACKs.
                        // Use its upper bound in explicit diagnostics, not a one-second resend.
                        let deadline = Instant::now() + Duration::from_millis(7000);
                        let mut checksum_rejected = false;
                        while Instant::now() < deadline {
                            let mut reply = [0u8; 64];
                            let n = self
                                .device
                                .read_timeout(&mut reply, 100)
                                .map_err(|e| e.to_string())?;
                            if n >= 4 && matches!(reply[0], 0x13 | 0xff) {
                                eprintln!(
                                    "Receiver reply: length={n}, id=0x{:02x}, command=0x{:02x}, count={}, index={}",
                                    reply[0], reply[1], reply[2], reply[3]
                                );
                            } else if n > 0 {
                                // Do not log bytes that could belong to typing reports.
                                eprintln!(
                                    "Unrelated receiver report: length={n}, id=0x{:02x}",
                                    reply[0]
                                );
                            }
                            match protocol::receiver_ack(&reply[..n], command, report[3]) {
                                protocol::ReceiverAck::Accepted => { acknowledged = true; break; }
                                protocol::ReceiverAck::ChecksumRejected => { checksum_rejected = true; break; }
                                protocol::ReceiverAck::DeviceRejected => return Err("Receiver explicitly rejected the transfer; stop and recover over USB.".into()),
                                protocol::ReceiverAck::Unrelated => {}
                            }
                        }
                        if acknowledged {
                            break;
                        }
                        // A timeout does not establish rejection. Resending could
                        // duplicate an accepted packet or disrupt a partial transfer.
                        if !checksum_rejected {
                            break;
                        }
                    }
                    if !acknowledged {
                        return Err(format!(
                            "Receiver did not acknowledge command 0x{command:02x}, sub {sub}, packet {} of {}; state is uncertain. Refresh before retrying.",
                            report[3] + 1,
                            report[2]
                        ));
                    }
                }
                Ok(())
            }
            Transport::Bluetooth => Err("Bluetooth writes are disabled".into()),
        }
    }
    #[cfg(feature = "wireless-diagnostics")]
    pub fn enable_receiver_write_diagnostics(&mut self) -> Result<()> {
        if self.transport != Transport::Receiver {
            return Err("Wireless diagnostics require a receiver connection".into());
        }
        self.experimental_receiver_writes = true;
        Ok(())
    }
    pub fn settings(&mut self) -> Result<Vec<u8>> {
        let block = self.read(4, 0, 128)?;
        protocol::validate_settings(&block).map_err(|e| e.to_string())?;
        Ok(block)
    }
    pub fn status(&mut self) -> Result<DeviceStatus> {
        let block = self.settings()?;
        self.status_from_settings(&block)
    }
    fn status_from_settings(&self, block: &[u8]) -> Result<DeviceStatus> {
        Ok(DeviceStatus {
            transport: self.transport,
            product: self.product.clone(),
            lighting: Lighting::read(block).map_err(|e| e.to_string())?,
            sleep_seconds: protocol::SLEEP
                .iter()
                .find(|(_, v)| *v == block[24])
                .map(|(s, _)| *s),
        })
    }
    pub fn apply_lighting(&mut self, lighting: Lighting) -> Result<DeviceStatus> {
        let source = self.settings()?;
        let updated = lighting.apply(&source).map_err(|e| e.to_string())?;
        self.write(4, 0, &updated)?;
        let actual = self.settings()?;
        verify_settings_readback(&updated, &actual)?;
        self.status_from_settings(&actual)
    }
    pub fn apply_sleep(&mut self, seconds: u16) -> Result<DeviceStatus> {
        let source = self.settings()?;
        let updated = protocol::sleep_settings(&source, seconds).map_err(|e| e.to_string())?;
        self.write(4, 0, &updated)?;
        let actual = self.settings()?;
        verify_settings_readback(&updated, &actual)?;
        self.status_from_settings(&actual)
    }
    pub fn paint(&mut self, colors: &[[u8; 3]; 126]) -> Result<DeviceStatus> {
        let block = self.settings()?;
        let plane_bytes =
            protocol::color_transfer(colors, self.transport).map_err(|error| error.to_string())?;
        self.write(
            if self.transport == Transport::Usb {
                6
            } else {
                2
            },
            0,
            &plane_bytes,
        )?;
        if self.read_colors()? != *colors {
            return Err(
                "Per-key colors did not match readback; lighting mode was not changed".into(),
            );
        }
        let previous = Lighting::read(&block).map_err(|e| e.to_string())?;
        self.apply_lighting(Lighting {
            effect: 21,
            ..previous
        })
    }
    /// Send a live frame without changing saved lighting settings or color data.
    pub fn stream_colors(&self, colors: &[[u8; 3]; 126]) -> Result<()> {
        if self.transport != Transport::Usb {
            return Err("Music sync requires the wired USB connection".into());
        }
        let report = protocol::usb_live_rgb_report(colors);
        // This API returns HidResult<()> and checks HidD_SetFeature success.
        // Do not retry an uncertain frame or route it through persistent writes.
        self.device
            .send_feature_report(&report)
            .map_err(|error| format!("Live RGB frame transfer failed: {error}"))
    }
    pub fn read_colors(&mut self) -> Result<[[u8; 3]; 126]> {
        let buffer = self.read(
            if self.transport == Transport::Usb {
                6
            } else {
                2
            },
            0,
            if self.transport == Transport::Receiver {
                378
            } else {
                384
            },
        )?;
        let mut colors = [[0; 3]; 126];
        for (index, color) in colors.iter_mut().enumerate() {
            for channel in 0..3 {
                color[channel] = buffer[channel * 126 + index]
            }
        }
        Ok(colors)
    }
    pub fn keymap(&mut self, layer: u8) -> Result<Vec<u8>> {
        if layer > 1 {
            return Err("Only base and Fn layers are supported".into());
        }
        let data = self.read(
            if self.transport == Transport::Usb {
                3
            } else {
                1
            },
            layer,
            if self.transport == Transport::Receiver {
                504
            } else {
                512
            },
        )?;
        protocol::normalize_keymap(&data, self.transport).map_err(|error| error.to_string())
    }
    pub fn remap(&mut self, layer: u8, matrix: usize, code: [u8; 4]) -> Result<Vec<u8>> {
        let old = self.keymap(layer)?;
        let updated = protocol::remap(&old, matrix, code).map_err(|e| e.to_string())?;
        self.write(
            if self.transport == Transport::Usb {
                3
            } else {
                1
            },
            layer,
            &updated,
        )?;
        let result = self.keymap(layer)?;
        if result != updated {
            return Err("Keymap write did not match readback".into());
        }
        Ok(result)
    }
    pub fn apply_macros(
        &mut self,
        bindings: &[crate::macros::MacroBinding],
    ) -> Result<[Vec<u8>; 2]> {
        if self.transport != Transport::Usb {
            return Err("On-board macros require the USB cable".into());
        }
        let local = std::env::var_os("LOCALAPPDATA")
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or("Cannot save macro recovery checkpoint: LOCALAPPDATA is unavailable")?;
        replace_macros(self, bindings, &local.join("AulaStudio").join("recovery"))
    }
    pub fn clear_macro_assignments(&mut self) -> Result<[Vec<u8>; 2]> {
        let mut result = [self.keymap(0)?, self.keymap(1)?];
        for (layer, map) in result.iter_mut().enumerate() {
            let old = map.clone();
            *map = protocol::without_macros(&old).map_err(|error| error.to_string())?;
            if *map != old {
                self.write(
                    if self.transport == Transport::Usb {
                        3
                    } else {
                        1
                    },
                    layer as u8,
                    map,
                )?;
                if self.keymap(layer as u8)? != *map {
                    return Err("Clearing macro assignments did not match readback; some changes may have applied".into());
                }
            }
        }
        Ok(result)
    }
}

fn verify_settings_readback(expected: &[u8], actual: &[u8]) -> Result<()> {
    if actual != expected {
        return Err(
            "Settings write did not match the complete readback. Refresh before retrying.".into(),
        );
    }
    Ok(())
}

/// The transaction boundary is independent of HID so failures after partial
/// transfers can be tested without touching a physical keyboard.
trait MacroIo {
    fn settings(&mut self) -> Result<Vec<u8>>;
    fn colors(&mut self) -> Result<[[u8; 3]; 126]>;
    fn keymap(&mut self, layer: u8) -> Result<Vec<u8>>;
    fn bank(&mut self) -> Result<Vec<u8>>;
    fn write_keymap(&mut self, layer: u8, map: &[u8]) -> Result<()>;
    fn write_bank(&mut self, bank: &[u8]) -> Result<()>;
}

impl MacroIo for Keyboard {
    fn settings(&mut self) -> Result<Vec<u8>> {
        Keyboard::settings(self)
    }
    fn colors(&mut self) -> Result<[[u8; 3]; 126]> {
        self.read_colors()
    }
    fn keymap(&mut self, layer: u8) -> Result<Vec<u8>> {
        Keyboard::keymap(self, layer)
    }
    fn bank(&mut self) -> Result<Vec<u8>> {
        self.read(5, 0, 4096)
    }
    fn write_keymap(&mut self, layer: u8, map: &[u8]) -> Result<()> {
        self.write(3, layer, map)
    }
    fn write_bank(&mut self, bank: &[u8]) -> Result<()> {
        self.write(5, 0, bank)
    }
}

#[derive(serde::Serialize)]
struct MacroRecovery {
    version: u8,
    settings: Vec<u8>,
    colors: Vec<[u8; 3]>,
    keymaps: [Vec<u8>; 2],
    macro_bank: Vec<u8>,
}

impl MacroRecovery {
    fn capture(board: &mut impl MacroIo) -> Result<Self> {
        let original = Self {
            version: 2,
            settings: board.settings()?,
            colors: board.colors()?.to_vec(),
            keymaps: [board.keymap(0)?, board.keymap(1)?],
            macro_bank: board.bank()?,
        };
        protocol::validate_settings(&original.settings).map_err(|error| error.to_string())?;
        for map in &original.keymaps {
            protocol::normalize_keymap(map, Transport::Usb).map_err(|error| error.to_string())?;
        }
        if original.macro_bank.len() != 4096 {
            return Err("Cannot save an incomplete hardware macro bank".into());
        }
        Ok(original)
    }

    fn save(&self, directory: &std::path::Path) -> Result<std::path::PathBuf> {
        use std::io::Write;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("Cannot timestamp macro checkpoint: {error}"))?
            .as_nanos();
        let path = directory.join(format!("macros-{}-{timestamp}.json", std::process::id()));
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("Cannot create macro recovery folder: {error}"))?;
        let data = serde_json::to_vec(self)
            .map_err(|error| format!("Cannot serialize macro checkpoint: {error}"))?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                format!("Cannot create macro checkpoint {}: {error}", path.display())
            })?;
        file.write_all(&data)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                format!(
                    "Cannot durably save macro checkpoint {}: {error}",
                    path.display()
                )
            })?;
        Ok(path)
    }

    fn restore(&self, board: &mut impl MacroIo) -> Result<()> {
        // Do not replace bank contents while any key might still reference a
        // partially installed bank. A failed cleanup stops recovery here.
        for (layer, original) in self.keymaps.iter().enumerate() {
            let clean = protocol::without_macros(original).map_err(|error| error.to_string())?;
            write_macro_keymap(board, layer as u8, &clean)?;
        }
        board.write_bank(&self.macro_bank)?;
        if board.bank()? != self.macro_bank {
            return Err("Recovered macro bank did not match readback".into());
        }
        for (layer, original) in self.keymaps.iter().enumerate() {
            write_macro_keymap(board, layer as u8, original)?;
        }
        if board.settings()? != self.settings || board.colors()?.as_slice() != self.colors {
            return Err("Settings or colors changed during macro recovery".into());
        }
        Ok(())
    }
}

fn write_macro_keymap(board: &mut impl MacroIo, layer: u8, map: &[u8]) -> Result<()> {
    board.write_keymap(layer, map)?;
    if board.keymap(layer)? != map {
        return Err(format!("Macro keymap layer {layer} did not match readback"));
    }
    Ok(())
}

fn replace_macros(
    board: &mut impl MacroIo,
    bindings: &[crate::macros::MacroBinding],
    directory: &std::path::Path,
) -> Result<[Vec<u8>; 2]> {
    let encoded = crate::macros::encode(bindings)?;
    let original = MacroRecovery::capture(board)?;
    let clean = [
        protocol::without_macros(&original.keymaps[0]).map_err(|error| error.to_string())?,
        protocol::without_macros(&original.keymaps[1]).map_err(|error| error.to_string())?,
    ];
    let mut assigned = clean[0].clone();
    for (index, binding) in bindings.iter().enumerate() {
        assigned = protocol::remap(&assigned, binding.matrix, [3, 1, 1, index as u8])
            .map_err(|error| error.to_string())?;
    }
    let mut new_bank = vec![0; 4096];
    new_bank[..encoded.len()].copy_from_slice(&encoded);
    // No writes are allowed until this complete version-2 checkpoint is durable.
    let checkpoint = original.save(directory)?;
    let applied: Result<[Vec<u8>; 2]> = (|| {
        for (layer, map) in clean.iter().enumerate() {
            if *map != original.keymaps[layer] {
                write_macro_keymap(board, layer as u8, map)?;
            }
        }
        board.write_bank(&new_bank)?;
        if board.bank()? != new_bank {
            return Err("Uploaded macro bank did not match readback".into());
        }
        board.write_keymap(0, &assigned)?;
        let base = board.keymap(0)?;
        let function = board.keymap(1)?;
        if base != assigned || function != clean[1] {
            return Err("Installed macro assignments did not match readback".into());
        }
        Ok([base, function])
    })();
    match applied {
        Ok(maps) => Ok(maps),
        Err(error) => match original.restore(board) {
            Ok(()) => Err(format!(
                "Macro update failed: {error}. Original macro bank and both keymaps were restored and verified. Checkpoint retained: {}",
                checkpoint.display()
            )),
            Err(recovery) => Err(format!(
                "Macro update failed: {error}. Recovery could not be verified: {recovery}. Checkpoint retained: {}. Recover over USB with f75-check --recover \"{}\"",
                checkpoint.display(),
                checkpoint.display()
            )),
        },
    }
}

#[cfg(test)]
mod macro_transaction_tests {
    use super::*;
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "aula-macro-transaction-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            for entry in std::fs::read_dir(&self.0).unwrap() {
                std::fs::remove_file(entry.unwrap().path()).unwrap();
            }
            std::fs::remove_dir(&self.0).unwrap();
        }
    }

    #[derive(Clone, Copy)]
    enum Fault {
        Error,
        Mismatch,
    }
    struct FakeKeyboard {
        settings: Vec<u8>,
        colors: [[u8; 3]; 126],
        maps: [Vec<u8>; 2],
        bank: Vec<u8>,
        operation: usize,
        faults: BTreeMap<usize, Fault>,
        writes: Vec<u8>,
        checkpoint_folder: PathBuf,
    }
    impl FakeKeyboard {
        fn new(directory: &Path) -> Self {
            let mut settings = vec![0; 128];
            settings[126..].copy_from_slice(&[0x5a, 0xa5]);
            let mut map = vec![0; 512];
            map[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
            map[20 * 4..20 * 4 + 4].copy_from_slice(&[0, 0, 0, 4]);
            let mut maps = [map.clone(), map];
            maps[0][12 * 4..12 * 4 + 4].copy_from_slice(&[3, 1, 1, 4]);
            maps[1][13 * 4..13 * 4 + 4].copy_from_slice(&[3, 1, 1, 2]);
            Self {
                settings,
                colors: [[20, 40, 60]; 126],
                maps,
                bank: (0..4096).map(|index| (index % 251) as u8).collect(),
                operation: 0,
                faults: BTreeMap::new(),
                writes: Vec::new(),
                checkpoint_folder: directory.into(),
            }
        }
        fn step(&mut self) -> Result<bool> {
            self.operation += 1;
            match self.faults.remove(&self.operation) {
                Some(Fault::Error) => Err(format!("Injected transport failure {}", self.operation)),
                Some(Fault::Mismatch) => Ok(true),
                None => Ok(false),
            }
        }
        fn before_write(&mut self, command: u8) {
            let checkpoint = std::fs::read_dir(&self.checkpoint_folder)
                .unwrap()
                .next()
                .expect("Every write must have a durable recovery checkpoint")
                .unwrap()
                .path();
            let data: serde_json::Value =
                serde_json::from_slice(&std::fs::read(checkpoint).unwrap()).unwrap();
            assert_eq!(data["version"], 2);
            assert_eq!(data["macro_bank"].as_array().unwrap().len(), 4096);
            self.writes.push(command);
        }
        fn assert_bank_has_no_active_references(&self) {
            assert!(
                self.maps.iter().all(|map| map[..504]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|code| code[0] != 3)),
                "A bank replacement must not run with old macro assignments active"
            );
        }
    }
    impl MacroIo for FakeKeyboard {
        fn settings(&mut self) -> Result<Vec<u8>> {
            let mismatch = self.step()?;
            let mut value = self.settings.clone();
            if mismatch {
                value[0] ^= 1;
            }
            Ok(value)
        }
        fn colors(&mut self) -> Result<[[u8; 3]; 126]> {
            let mismatch = self.step()?;
            let mut value = self.colors;
            if mismatch {
                value[0][0] ^= 1;
            }
            Ok(value)
        }
        fn keymap(&mut self, layer: u8) -> Result<Vec<u8>> {
            let mismatch = self.step()?;
            let mut value = self.maps[layer as usize].clone();
            if mismatch {
                value[0] ^= 1;
            }
            Ok(value)
        }
        fn bank(&mut self) -> Result<Vec<u8>> {
            let mismatch = self.step()?;
            let mut value = self.bank.clone();
            if mismatch {
                value[0] ^= 1;
            }
            Ok(value)
        }
        fn write_keymap(&mut self, layer: u8, map: &[u8]) -> Result<()> {
            self.before_write(3);
            // A failed USB call may already have changed part of the hardware.
            self.maps[layer as usize][..256].copy_from_slice(&map[..256]);
            let mismatch = self.step()?;
            self.maps[layer as usize] = map.to_vec();
            if mismatch {
                self.maps[layer as usize][0] ^= 1;
            }
            Ok(())
        }
        fn write_bank(&mut self, bank: &[u8]) -> Result<()> {
            self.before_write(5);
            self.assert_bank_has_no_active_references();
            self.bank[..2048].copy_from_slice(&bank[..2048]);
            let mismatch = self.step()?;
            self.bank = bank.to_vec();
            if mismatch {
                self.bank[0] ^= 1;
            }
            Ok(())
        }
    }

    fn bindings() -> Vec<crate::macros::MacroBinding> {
        vec![crate::macros::MacroBinding {
            name: "Example".into(),
            matrix: 78,
            events: crate::macros::text_events("a", 20).unwrap(),
        }]
    }

    #[test]
    fn replacement_disables_old_references_and_checkpoints_the_complete_original() {
        let directory = TestDirectory::new();
        let mut board = FakeKeyboard::new(&directory.0);
        let original_maps = board.maps.clone();
        let original_bank = board.bank.clone();
        let macros = bindings();
        let result = replace_macros(&mut board, &macros, &directory.0).unwrap();
        assert_eq!(result, board.maps);
        assert_eq!(&result[0][78 * 4..78 * 4 + 4], &[3, 1, 1, 0]);
        assert_eq!(&result[0][12 * 4..12 * 4 + 4], &[0; 4]);
        assert_eq!(&result[1][13 * 4..13 * 4 + 4], &[0; 4]);
        assert_eq!(&result[0][20 * 4..20 * 4 + 4], &[0, 0, 0, 4]);
        let encoded = crate::macros::encode(&macros).unwrap();
        assert_eq!(&board.bank[..encoded.len()], encoded);
        assert!(board.bank[encoded.len()..].iter().all(|byte| *byte == 0));
        assert_eq!(board.operation, 14);
        let checkpoint = std::fs::read_dir(&directory.0)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let data: serde_json::Value =
            serde_json::from_slice(&std::fs::read(checkpoint).unwrap()).unwrap();
        assert_eq!(data["version"], 2);
        assert_eq!(data["settings"], serde_json::json!(board.settings));
        assert_eq!(data["colors"], serde_json::json!(board.colors.as_slice()));
        assert_eq!(data["keymaps"], serde_json::json!(original_maps));
        assert_eq!(data["macro_bank"], serde_json::json!(original_bank));
    }

    #[test]
    fn every_transient_transport_failure_restores_or_prevents_writes() {
        for failure in 1..=14 {
            let directory = TestDirectory::new();
            let mut board = FakeKeyboard::new(&directory.0);
            let original_maps = board.maps.clone();
            let original_bank = board.bank.clone();
            board.faults.insert(failure, Fault::Error);
            let error = replace_macros(&mut board, &bindings(), &directory.0).unwrap_err();
            assert_eq!(board.maps, original_maps, "Failure at operation {failure}");
            assert_eq!(board.bank, original_bank, "Failure at operation {failure}");
            if failure <= 5 {
                assert!(board.writes.is_empty());
            } else {
                assert!(error.contains("restored and verified"), "{error}");
                assert!(error.contains("Checkpoint retained"), "{error}");
            }
        }
    }

    #[test]
    fn mismatched_bank_and_keymap_readback_trigger_verified_rollback() {
        for failure in [7, 9, 11, 13, 14] {
            let directory = TestDirectory::new();
            let mut board = FakeKeyboard::new(&directory.0);
            let original_maps = board.maps.clone();
            let original_bank = board.bank.clone();
            board.faults.insert(failure, Fault::Mismatch);
            let error = replace_macros(&mut board, &bindings(), &directory.0).unwrap_err();
            assert!(error.contains("restored and verified"), "{error}");
            assert_eq!(board.maps, original_maps);
            assert_eq!(board.bank, original_bank);
        }
    }

    #[test]
    fn failed_bank_recovery_keeps_keys_disabled_and_reports_the_checkpoint() {
        let directory = TestDirectory::new();
        let mut board = FakeKeyboard::new(&directory.0);
        board.faults.insert(11, Fault::Error); // New bank was written, readback failed.
        board.faults.insert(16, Fault::Error); // Restoring the bank partially failed.
        let error = replace_macros(&mut board, &bindings(), &directory.0).unwrap_err();
        assert!(error.contains("Recovery could not be verified"), "{error}");
        assert!(error.contains("f75-check --recover"), "{error}");
        assert!(
            error.contains(&directory.0.display().to_string()),
            "{error}"
        );
        board.assert_bank_has_no_active_references();
        assert_eq!(board.operation, 16);
    }

    #[test]
    fn failed_cleanup_during_recovery_prevents_further_bank_writes() {
        let directory = TestDirectory::new();
        let mut board = FakeKeyboard::new(&directory.0);
        board.faults.insert(10, Fault::Error); // New bank write partially succeeded.
        board.faults.insert(13, Fault::Error); // Recovery cannot clear Fn reliably.
        let error = replace_macros(&mut board, &bindings(), &directory.0).unwrap_err();
        assert!(error.contains("Recovery could not be verified"), "{error}");
        assert_eq!(
            board.writes.iter().filter(|command| **command == 5).count(),
            1
        );
        assert_eq!(board.operation, 13);
    }

    #[test]
    fn checkpoint_failure_and_invalid_macro_input_never_write_hardware() {
        let directory = TestDirectory::new();
        let mut board = FakeKeyboard::new(&directory.0);
        let blocked = directory.0.join("blocked");
        std::fs::write(&blocked, b"existing file").unwrap();
        assert!(replace_macros(&mut board, &bindings(), &blocked).is_err());
        assert!(board.writes.is_empty());
        assert_eq!(board.operation, 5);

        let directory = TestDirectory::new();
        let mut board = FakeKeyboard::new(&directory.0);
        assert!(replace_macros(&mut board, &[], &directory.0).is_err());
        assert_eq!(board.operation, 0);
        assert!(board.writes.is_empty());
    }

    #[test]
    fn settings_readback_detects_corruption_outside_the_requested_fields() {
        let mut original = vec![0; 128];
        original[126..].copy_from_slice(&[0x5a, 0xa5]);
        let lighting = Lighting {
            effect: 1,
            brightness: 9,
            speed: 2,
            color: 0,
        };
        let lit = lighting.apply(&original).unwrap();
        let expected = protocol::sleep_settings(&lit, 300).unwrap();
        let mut corrupted = expected.clone();
        corrupted[3] ^= 1;
        // The old partial checks accepted this block despite unrelated data loss.
        assert_eq!(Lighting::read(&corrupted).unwrap(), lighting);
        assert_eq!(corrupted[24], expected[24]);
        assert!(verify_settings_readback(&expected, &corrupted).is_err());
        assert!(verify_settings_readback(&expected, &expected).is_ok());
    }
}
