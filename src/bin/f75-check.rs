use aula_studio::{
    device::Keyboard,
    protocol::{Lighting, Transport},
};
const MAX_RECOVERY_BYTES: usize = 64_000;

trait RecoveryIo {
    fn transport(&self) -> Transport;
    fn read(&mut self, command: u8, sub: u8, length: usize) -> Result<Vec<u8>, String>;
    fn write(&mut self, command: u8, sub: u8, data: &[u8]) -> Result<(), String>;
}

impl RecoveryIo for Keyboard {
    fn transport(&self) -> Transport {
        self.transport
    }
    fn read(&mut self, command: u8, sub: u8, length: usize) -> Result<Vec<u8>, String> {
        Keyboard::read(self, command, sub, length)
    }
    fn write(&mut self, command: u8, sub: u8, data: &[u8]) -> Result<(), String> {
        Keyboard::write(self, command, sub, data)
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
struct Recovery {
    version: u8,
    settings: Vec<u8>,
    colors: Vec<[u8; 3]>,
    keymaps: Vec<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    macro_bank: Option<Vec<u8>>,
}
impl Recovery {
    fn validate(&self) -> Result<(), String> {
        if !matches!(self.version, 1 | 2) || self.colors.len() != 126 || self.keymaps.len() != 2 {
            return Err("Invalid recovery checkpoint shape".into());
        }
        if (self.version == 2
            && self
                .macro_bank
                .as_ref()
                .is_none_or(|bank| bank.len() != 4096))
            || (self.version == 1 && self.macro_bank.is_some())
        {
            return Err("Invalid recovery macro bank".into());
        }
        aula_studio::protocol::validate_settings(&self.settings).map_err(|e| e.to_string())?;
        for map in &self.keymaps {
            aula_studio::protocol::normalize_keymap(map, Transport::Usb)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn restore(&self, keyboard: &mut impl RecoveryIo) -> Result<(), String> {
        self.validate()?;
        if keyboard.transport() != Transport::Usb {
            return Err("Recovery requires USB".into());
        }
        if let Some(bank) = &self.macro_bank {
            // Existing assignments may reference a different or partially written
            // bank. Disable them on both layers before replacing any bank bytes.
            for layer in 0..=1 {
                let current = keyboard.read(3, layer, 512)?;
                let disabled = aula_studio::protocol::without_macros(&current)
                    .map_err(|error| error.to_string())?;
                if disabled != current {
                    keyboard.write(3, layer, &disabled)?;
                    if keyboard.read(3, layer, 512)? != disabled {
                        return Err(
                            "Could not verify disabled macro assignments; macro bank was not changed"
                                .into(),
                        );
                    }
                }
            }
            keyboard.write(5, 0, bank)?;
            if keyboard.read(5, 0, 4096)? != *bank {
                return Err("Recovered macro bank readback mismatch".into());
            }
        }
        let colors: [[u8; 3]; 126] = self
            .colors
            .as_slice()
            .try_into()
            .map_err(|_| "Invalid recovery colors")?;
        let planes = aula_studio::protocol::color_planes(&colors);
        keyboard.write(6, 0, &planes)?;
        for (layer, map) in self.keymaps.iter().enumerate() {
            keyboard.write(3, layer as u8, map)?;
            if keyboard.read(3, layer as u8, 512)? != *map {
                return Err("Recovered keymap readback mismatch".into());
            }
        }
        keyboard.write(4, 0, &self.settings)?;
        if keyboard.read(4, 0, 128)? != self.settings || keyboard.read(6, 0, 384)? != planes {
            return Err("Recovered lighting/color readback mismatch".into());
        }
        Ok(())
    }
}

fn load_checkpoint(reader: impl std::io::Read) -> Result<Recovery, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .take(MAX_RECOVERY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read recovery checkpoint: {error}"))?;
    if bytes.len() > MAX_RECOVERY_BYTES {
        return Err("Recovery checkpoint exceeds size limit".into());
    }
    let data: Recovery = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid recovery JSON: {error}"))?;
    data.validate()?;
    Ok(data)
}

fn save_checkpoint(snapshot: &Recovery) -> Result<std::path::PathBuf, String> {
    snapshot.validate()?;
    let path = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("verification")
        .join(format!(
            "recovery-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        ));
    std::fs::create_dir_all(path.parent().ok_or("Missing recovery directory")?)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    let data = serde_json::to_vec(snapshot).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(&data)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(path)
}
fn receiver_settings_test_requested(arguments: &[String]) -> Result<bool, String> {
    let requested = arguments
        .iter()
        .any(|argument| argument == "--receiver-settings-test");
    if requested
        && arguments
            .iter()
            .skip(1)
            .any(|argument| argument != "--receiver-settings-test")
    {
        return Err("Receiver settings testing cannot be combined with other operations".into());
    }
    Ok(requested)
}
fn main() -> Result<(), String> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let receiver_settings_test = receiver_settings_test_requested(&arguments)?;
    #[cfg(not(feature = "wireless-diagnostics"))]
    if receiver_settings_test {
        return Err("Receiver write testing requires the explicit wireless-diagnostics build feature. The normal app keeps wireless writes disabled.".into());
    }
    let macro_test = if let Some(index) = arguments.iter().position(|a| a == "--macro-test") {
        if arguments.iter().any(|a| {
            matches!(
                a.as_str(),
                "--receiver" | "--recover" | "--roundtrip" | "--online" | "--inventory"
            )
        }) {
            return Err("Macro testing cannot be combined with other operations".into());
        }
        let matrix: usize = arguments
            .get(index + 1)
            .ok_or("Supply the approved physical key matrix after --macro-test")?
            .parse()
            .map_err(|_| "Invalid key matrix")?;
        let layout: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../assets/keyboard-layout.json"))
                .map_err(|e| e.to_string())?;
        if !layout.iter().any(|key| {
            key["matrix"].as_u64() == Some(matrix as u64)
                && key["width"].as_f64().is_some_and(|w| w > 0.0)
                && key["height"].as_f64().is_some_and(|h| h > 0.0)
        }) {
            return Err("Choose a visible physical key from the keyboard layout".into());
        }
        Some(matrix)
    } else {
        None
    };
    if arguments.iter().any(|a| a == "--online") {
        let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
        let info = api
            .device_list()
            .find(|d| {
                d.vendor_id() == 0x3554 && d.product_id() == 0xfa09 && d.usage_page() == 0xff02
            })
            .ok_or("Receiver control interface not found")?;
        let device = info.open_device(&api).map_err(|e| e.to_string())?;
        device
            .write(&aula_studio::protocol::receiver_read(7, 0).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        for _ in 0..20 {
            let mut reply = [0u8; 64];
            let n = device
                .read_timeout(&mut reply, 100)
                .map_err(|e| e.to_string())?;
            if n >= 20 && reply[0] == 0x13 && reply[1] & 0x7f == 7 {
                println!("Receiver online flag: {}", reply[5] != 0);
                return Ok(());
            }
        }
        return Err("Receiver online query timed out".into());
    }
    if arguments.iter().any(|a| a == "--inventory") {
        let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
        for info in api.device_list().filter(|d| {
            matches!(
                (d.vendor_id(), d.product_id()),
                (0x258a, 0x010c) | (0x3554, 0xfa09)
            )
        }) {
            println!(
                "{:04x}:{:04x} page=0x{:04x} usage=0x{:04x} interface={} product={}",
                info.vendor_id(),
                info.product_id(),
                info.usage_page(),
                info.usage(),
                info.interface_number(),
                info.product_string().unwrap_or("Unknown")
            );
        }
        return Ok(());
    }
    let recovery = if let Some(index) = arguments.iter().position(|a| a == "--recover") {
        let path = arguments
            .get(index + 1)
            .ok_or("Supply the retained checkpoint path after --recover")?;
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        Some((load_checkpoint(file)?, path))
    } else {
        None
    };
    let mut keyboard = Keyboard::connect_via(if recovery.is_some() || macro_test.is_some() {
        Some(Transport::Usb)
    } else if receiver_settings_test || std::env::args().any(|a| a == "--receiver") {
        Some(Transport::Receiver)
    } else {
        Some(Transport::Usb)
    })?;
    let status = keyboard.status()?;
    println!("Connected: {} via {:?}", status.product, status.transport);
    println!(
        "Lighting: {:?}; sleep: {:?}",
        status.lighting, status.sleep_seconds
    );
    if let Some((recovery, path)) = recovery {
        recovery.restore(&mut keyboard).map_err(|error| {
            format!("Recovery could not be verified: {error}. Checkpoint retained at {path}")
        })?;
        println!(
            "USB checkpoint restoration passed with exact readback. Checkpoint file retained."
        );
        return Ok(());
    }
    let colors = keyboard.read_colors()?;
    let maps = [keyboard.keymap(0)?, keyboard.keymap(1)?];
    #[cfg(feature = "wireless-diagnostics")]
    if receiver_settings_test {
        let snapshot = Recovery {
            version: 1,
            settings: keyboard.settings()?,
            colors: colors.to_vec(),
            keymaps: maps.to_vec(),
            macro_bank: None,
        };
        let checkpoint = save_checkpoint(&snapshot)?;
        println!(
            "Receiver recovery snapshot saved at {}",
            checkpoint.display()
        );
        keyboard.enable_receiver_write_diagnostics()?;
        if let Err(error) = keyboard.write(4, 0, &snapshot.settings) {
            return Err(format!(
                "Unchanged-settings transfer failed: {error}. No further wireless writes attempted. Recover over USB using --recover {}",
                checkpoint.display()
            ));
        }
        let verified = (|| {
            if keyboard.settings()? != snapshot.settings
                || keyboard.read_colors()? != colors
                || keyboard.keymap(0)? != maps[0]
                || keyboard.keymap(1)? != maps[1]
            {
                return Err("Receiver state differs from the pre-test snapshot".to_string());
            }
            Ok(())
        })();
        verified.map_err(|error| format!("Receiver readback could not establish unchanged state: {error}. No further wireless writes attempted. Recover over USB using --recover {}", checkpoint.display()))?;
        println!(
            "Unchanged receiver settings transfer passed. Settings, colors, and both keymaps match the saved snapshot. Checkpoint retained."
        );
        return Ok(());
    }
    if let Some(matrix) = macro_test {
        let snapshot = Recovery {
            version: 2,
            settings: keyboard.settings()?,
            colors: colors.to_vec(),
            keymaps: maps.to_vec(),
            macro_bank: Some(keyboard.read(5, 0, 4096)?),
        };
        let checkpoint = save_checkpoint(&snapshot)?;
        println!(
            "Full macro recovery snapshot saved at {}",
            checkpoint.display()
        );
        let bindings = [aula_studio::macros::MacroBinding {
            name: "Playback test".into(),
            matrix,
            events: aula_studio::macros::text_events("aula", 40)?,
        }];
        if let Err(error) = keyboard.apply_macros(&bindings) {
            return match snapshot.restore(&mut keyboard) {
                Ok(()) => Err(format!(
                    "Macro upload failed; original data restored and verified: {error}. Snapshot retained at {}",
                    checkpoint.display()
                )),
                Err(restore) => Err(format!(
                    "Macro upload failed: {error}. Restoration failed: {restore}. Snapshot retained at {}",
                    checkpoint.display()
                )),
            };
        }
        println!(
            "Test macro uploaded and assignments verified. Press the approved key in an empty test field. Actual playback remains unverified until observed. Restore with --recover {}",
            checkpoint.display()
        );
        return Ok(());
    }
    println!("Read 126 LED colors and both 512-byte keymap layers successfully.");
    println!(
        "Physical layout markers: ISO extra key assigned={}, ISO slash position assigned={}, ANSI slash position assigned={}",
        maps[0][76 * 4..76 * 4 + 4] != [0; 4],
        maps[0][75 * 4..75 * 4 + 4] != [0; 4],
        maps[0][80 * 4..80 * 4 + 4] != [0; 4]
    );
    if std::env::args().any(|a| a == "--macros") {
        if keyboard.transport != Transport::Usb {
            return Err("Macro bank inspection is USB-only".into());
        }
        let bank = keyboard.read(5, 0, 4096)?;
        println!(
            "Macro bank read succeeded: {} bytes, first payload offset {}. Macro contents are not printed.",
            bank.len(),
            u16::from_le_bytes([bank[0], bank[1]])
        );
    }
    if std::env::args().any(|a| a == "--roundtrip") {
        let original = keyboard.settings()?;
        // Retain the exact original data if a device disconnect prevents restoration.
        // Earlier in-memory-only snapshots disappeared when a failed check exited.
        let recovery = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join("verification")
            .join(format!(
                "recovery-{}-{}.json",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_nanos()
            ));
        std::fs::create_dir_all(recovery.parent().ok_or("Missing recovery directory")?)
            .map_err(|e| e.to_string())?;
        let checkpoint = serde_json::json!({"version":1,"transport":format!("{:?}",keyboard.transport),"settings":original.clone(),"colors":colors.to_vec(),"keymaps":maps.clone()});
        let data = serde_json::to_vec(&checkpoint).map_err(|e| e.to_string())?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&recovery)
            .map_err(|e| e.to_string())?;
        file.write_all(&data)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        println!("Recovery checkpoint retained at {}", recovery.display());
        let result = (|| {
            let desired = Lighting {
                effect: 1,
                brightness: (status.lighting.brightness + 1) % 10,
                speed: 2,
                color: 6,
            };
            keyboard.apply_lighting(desired)?;
            println!("Lighting write and readback matched.");
            keyboard.apply_sleep(0)?;
            println!("Sleep setting write and readback matched.");
            keyboard.paint(&colors)?;
            println!("Per-key color write and readback matched.");
            let code: [u8; 4] = maps[0][..4].try_into().map_err(|_| "Missing Esc mapping")?;
            keyboard.remap(0, 0, code)?;
            println!("Unchanged key assignment write and readback matched.");
            Ok::<(), String>(())
        })();
        let restored = keyboard.write(4, 0, &original).and_then(|_| {
            if keyboard.settings()? != original {
                return Err("Restored lighting differs from original".into());
            }
            if keyboard.read_colors()? != colors
                || keyboard.keymap(0)? != maps[0]
                || keyboard.keymap(1)? != maps[1]
            {
                return Err("Hardware state differs from initial snapshot".into());
            }
            Ok(())
        });
        match (result, restored) {
            (Ok(()), Ok(())) => {
                std::fs::remove_file(&recovery).map_err(|e| e.to_string())?;
                println!(
                    "Roundtrip passed. Original lighting, colors, and keymaps restored and verified."
                )
            }
            (Err(e), Ok(())) => {
                return Err(format!("Check failed; original settings restored: {e}"));
            }
            (test_result, Err(e)) => {
                return Err(format!(
                    "RESTORATION FAILED. Original test result: {test_result:?}. Inspect the keyboard before continuing: {e}. Recovery snapshot: {}",
                    recovery.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receiver_diagnostics_reject_combined_operations_before_hardware_access() {
        let args = |items: &[&str]| {
            items
                .iter()
                .map(|item| (*item).to_string())
                .collect::<Vec<_>>()
        };
        assert!(
            receiver_settings_test_requested(&args(&["f75-check", "--receiver-settings-test"]))
                .unwrap()
        );
        assert!(!receiver_settings_test_requested(&args(&["f75-check", "--receiver"])).unwrap());
        for other in [
            "--macro-test",
            "--recover",
            "--roundtrip",
            "--inventory",
            "--online",
            "--receiver",
            "--macros",
            "unexpected",
        ] {
            assert!(
                receiver_settings_test_requested(&args(&[
                    "f75-check",
                    "--receiver-settings-test",
                    other
                ]))
                .is_err()
            );
        }
    }
    fn fixture() -> Recovery {
        let mut settings = vec![0; 128];
        settings[126..].copy_from_slice(&[0x5a, 0xa5]);
        let mut map = vec![0; 512];
        map[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
        Recovery {
            version: 1,
            settings,
            colors: vec![[1, 2, 3]; 126],
            keymaps: vec![map.clone(), map],
            macro_bank: None,
        }
    }
    #[test]
    fn earlier_checkpoints_remain_readable_without_macro_data() {
        let source = serde_json::to_vec(&fixture()).unwrap();
        let decoded: Recovery = serde_json::from_slice(&source).unwrap();
        decoded.validate().unwrap();
        assert!(decoded.macro_bank.is_none());
    }
    #[test]
    fn macro_checkpoint_preserves_full_bank_and_both_maps() {
        let mut snapshot = fixture();
        snapshot.version = 2;
        snapshot.macro_bank = Some((0..4096).map(|index| (index % 256) as u8).collect());
        let source = serde_json::to_vec(&snapshot).unwrap();
        let decoded: Recovery = serde_json::from_slice(&source).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.macro_bank, snapshot.macro_bank);
        assert_eq!(decoded.keymaps, snapshot.keymaps);
    }
    #[test]
    fn incomplete_or_ambiguous_macro_checkpoints_are_rejected() {
        let mut snapshot = fixture();
        snapshot.version = 2;
        assert!(snapshot.validate().is_err());
        snapshot.macro_bank = Some(vec![0; 4095]);
        assert!(snapshot.validate().is_err());
        snapshot.macro_bank = Some(vec![0; 4096]);
        snapshot.validate().unwrap();
        snapshot.version = 1;
        assert!(snapshot.validate().is_err());
        snapshot.version = 2;
        snapshot.keymaps.pop();
        assert!(snapshot.validate().is_err());
    }

    #[derive(Clone, Copy, PartialEq)]
    enum RecoveryFault {
        FnCleanupMismatch,
        BankWrite,
    }
    struct FakeRecoveryDevice {
        transport: Transport,
        maps: Vec<Vec<u8>>,
        bank: Vec<u8>,
        settings: Vec<u8>,
        colors: Vec<u8>,
        writes: Vec<(u8, u8)>,
        reads: usize,
        fault: Option<RecoveryFault>,
    }
    impl FakeRecoveryDevice {
        fn new() -> Self {
            let initial = fixture();
            let mut maps = initial.keymaps;
            maps[0][12 * 4..12 * 4 + 4].copy_from_slice(&[3, 1, 1, 3]);
            maps[1][13 * 4..13 * 4 + 4].copy_from_slice(&[3, 1, 1, 2]);
            Self {
                transport: Transport::Usb,
                maps,
                bank: vec![77; 4096],
                settings: initial.settings,
                colors: vec![6; 384],
                writes: Vec::new(),
                reads: 0,
                fault: None,
            }
        }
        fn assert_no_macro_references(&self) {
            assert!(self.maps.iter().all(|map| {
                map[..504]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|code| code[0] != 3)
            }));
        }
    }
    impl RecoveryIo for FakeRecoveryDevice {
        fn transport(&self) -> Transport {
            self.transport
        }
        fn read(&mut self, command: u8, sub: u8, length: usize) -> Result<Vec<u8>, String> {
            self.reads += 1;
            let mut data = match command {
                3 => self.maps[sub as usize].clone(),
                4 => self.settings.clone(),
                5 => self.bank.clone(),
                6 => self.colors.clone(),
                _ => panic!("Unexpected command"),
            };
            if command == 3
                && sub == 1
                && self.writes.contains(&(3, 1))
                && self.fault == Some(RecoveryFault::FnCleanupMismatch)
            {
                self.fault = None;
                data[0] ^= 1;
            }
            assert_eq!(data.len(), length);
            Ok(data)
        }
        fn write(&mut self, command: u8, sub: u8, data: &[u8]) -> Result<(), String> {
            self.writes.push((command, sub));
            match command {
                3 => self.maps[sub as usize] = data.to_vec(),
                4 => self.settings = data.to_vec(),
                5 => {
                    self.assert_no_macro_references();
                    self.bank[..2048].copy_from_slice(&data[..2048]);
                    if self.fault == Some(RecoveryFault::BankWrite) {
                        self.fault = None;
                        return Err("Injected partially accepted bank write".into());
                    }
                    self.bank = data.to_vec();
                }
                6 => self.colors = data.to_vec(),
                _ => panic!("Unexpected command"),
            }
            Ok(())
        }
    }

    fn macro_fixture() -> Recovery {
        let mut snapshot = fixture();
        snapshot.version = 2;
        snapshot.macro_bank = Some(vec![99; 4096]);
        snapshot.keymaps[0][78 * 4..78 * 4 + 4].copy_from_slice(&[3, 1, 1, 0]);
        snapshot
    }

    #[test]
    fn macro_recovery_disables_current_references_before_restoring_the_bank() {
        let snapshot = macro_fixture();
        let mut device = FakeRecoveryDevice::new();
        snapshot.restore(&mut device).unwrap();
        assert_eq!(device.maps, snapshot.keymaps);
        assert_eq!(device.bank, snapshot.macro_bank.unwrap());
        assert_eq!(device.settings, snapshot.settings);
        assert_eq!(
            device.colors,
            aula_studio::protocol::color_planes(&[[1, 2, 3]; 126])
        );
        assert_eq!(&device.writes[..3], &[(3, 0), (3, 1), (5, 0)]);
    }

    #[test]
    fn macro_recovery_never_changes_the_bank_if_cleanup_cannot_be_verified() {
        let mut device = FakeRecoveryDevice::new();
        let original_bank = device.bank.clone();
        device.fault = Some(RecoveryFault::FnCleanupMismatch);
        let error = macro_fixture().restore(&mut device).unwrap_err();
        assert!(error.contains("macro bank was not changed"), "{error}");
        assert_eq!(device.bank, original_bank);
        assert!(device.writes.iter().all(|(command, _)| *command == 3));
    }

    #[test]
    fn failed_macro_bank_recovery_leaves_all_macro_assignments_disabled() {
        let mut device = FakeRecoveryDevice::new();
        device.fault = Some(RecoveryFault::BankWrite);
        assert!(macro_fixture().restore(&mut device).is_err());
        device.assert_no_macro_references();
        assert_eq!(device.writes, [(3, 0), (3, 1), (5, 0)]);
    }

    #[test]
    fn legacy_recovery_does_not_replace_macro_bank_data() {
        let mut device = FakeRecoveryDevice::new();
        let original_bank = device.bank.clone();
        fixture().restore(&mut device).unwrap();
        assert_eq!(device.bank, original_bank);
        assert!(device.writes.iter().all(|(command, _)| *command != 5));
    }

    #[test]
    fn invalid_checkpoint_or_non_usb_transport_prevents_all_device_access() {
        let mut device = FakeRecoveryDevice::new();
        device.transport = Transport::Receiver;
        assert!(macro_fixture().restore(&mut device).is_err());
        assert_eq!(device.reads, 0);
        assert!(device.writes.is_empty());
        device.transport = Transport::Usb;
        let mut invalid = macro_fixture();
        invalid.macro_bank.as_mut().unwrap().pop();
        assert!(invalid.restore(&mut device).is_err());
        assert_eq!(device.reads, 0);
        assert!(device.writes.is_empty());
    }

    #[test]
    fn checkpoint_size_boundary_is_enforced_before_reading_an_entire_file() {
        let mut payload = serde_json::to_vec(&macro_fixture()).unwrap();
        payload.resize(MAX_RECOVERY_BYTES, b' ');
        assert!(load_checkpoint(payload.as_slice()).is_ok());
        payload.resize(MAX_RECOVERY_BYTES * 2, b' ');
        let mut reader = std::io::Cursor::new(payload);
        assert!(
            load_checkpoint(&mut reader)
                .err()
                .unwrap()
                .contains("size limit")
        );
        assert_eq!(reader.position(), MAX_RECOVERY_BYTES as u64 + 1);
    }
}
