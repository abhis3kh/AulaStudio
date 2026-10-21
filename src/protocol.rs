//! F75 transport framing. No hardware I/O occurs in this module.
//! Protocol facts: installed KB.ini and the linked Linux project's PROTOCOL.md.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Receiver,
    Bluetooth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError(pub &'static str);
impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ProtocolError {}
type Result<T> = std::result::Result<T, ProtocolError>;

pub fn receiver_read(command: u8, sub: u8) -> Result<[u8; 20]> {
    if sub > 15 {
        return Err(ProtocolError("Subcommand exceeds 4 bits"));
    }
    let mut report = [0; 20];
    report[..5].copy_from_slice(&[0x13, command, 1, 0, sub << 4]);
    report[19] = report[..19].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    Ok(report)
}

pub fn receiver_write(command: u8, sub: u8, data: &[u8]) -> Result<Vec<[u8; 20]>> {
    if sub > 15 || data.is_empty() || data.len() > 1778 {
        return Err(ProtocolError("Invalid receiver packet size or subcommand"));
    }
    let count = data.len().div_ceil(14) as u8;
    Ok(data
        .chunks(14)
        .enumerate()
        .map(|(index, chunk)| {
            let mut report = [0; 20];
            report[..5].copy_from_slice(&[
                0x13,
                command,
                count,
                index as u8,
                (sub << 4) | chunk.len() as u8,
            ]);
            report[5..5 + chunk.len()].copy_from_slice(chunk);
            report[19] = report[..19].iter().fold(0u8, |a, b| a.wrapping_add(*b));
            report
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverAck {
    Unrelated,
    Accepted,
    ChecksumRejected,
    DeviceRejected,
}
pub fn receiver_ack(reply: &[u8], command: u8, index: u8) -> ReceiverAck {
    if reply.first() == Some(&0xff) {
        return ReceiverAck::DeviceRejected;
    }
    if reply.len() < 20 || reply[0] != 0x13 || reply[1] != command {
        return ReceiverAck::Unrelated;
    }
    if reply[2] & 0x80 != 0 {
        return ReceiverAck::ChecksumRejected;
    }
    if reply[3] != index {
        return ReceiverAck::Unrelated;
    }
    ReceiverAck::Accepted
}

pub fn usb_reports(
    command: u8,
    sub: u8,
    length: usize,
    payload: Option<&[u8]>,
) -> Result<Vec<[u8; 520]>> {
    if length == 0 || length > 512 * 255 || payload.is_some_and(|p| p.len() != length) {
        return Err(ProtocolError("Invalid USB transfer size"));
    }
    let count = length.div_ceil(512);
    Ok((0..count)
        .map(|index| {
            let size = (length - index * 512).min(512);
            let mut report = [0; 520];
            report[..8].copy_from_slice(&[
                6,
                command,
                sub,
                0,
                count as u8,
                index as u8,
                size as u8,
                (size >> 8) as u8,
            ]);
            if let Some(data) = payload {
                report[8..8 + size].copy_from_slice(&data[index * 512..index * 512 + size]);
            }
            report
        })
        .collect())
}

/// Live USB display data uses interleaved RGB, unlike the saved per-key planes.
/// One complete frame fits in a single feature report, command 0x08, sub 0.
pub fn usb_live_rgb_report(colors: &[[u8; 3]; 126]) -> [u8; 520] {
    let mut report = [0; 520];
    report[..8].copy_from_slice(&[0x06, 0x08, 0, 0, 1, 0, 0x7a, 0x01]);
    for (index, color) in colors.iter().enumerate() {
        let offset = 8 + index * 3;
        report[offset..offset + 3].copy_from_slice(color);
    }
    report
}

pub fn validate_settings(block: &[u8]) -> Result<()> {
    if block.len() != 128 || block[126..] != [0x5a, 0xa5] {
        return Err(ProtocolError("Unsupported settings block; refusing write"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lighting {
    pub effect: u8,
    pub brightness: u8,
    pub speed: u8,
    pub color: u8,
}
fn effect_slot(effect: u8) -> Result<usize> {
    match effect {
        1..=18 => Ok((effect - 1) as usize),
        21 => Ok(18),
        0 => Ok(19),
        _ => Err(ProtocolError("Unsupported effect")),
    }
}
impl Lighting {
    pub fn read(block: &[u8]) -> Result<Self> {
        validate_settings(block)?;
        let slot = effect_slot(block[10])?;
        let offset = 0x3a + slot * 2;
        let settings = Self {
            effect: block[10],
            brightness: block[offset],
            speed: block[offset + 1] >> 4,
            color: block[offset + 1] & 15,
        };
        settings.validate()?;
        Ok(settings)
    }
    pub fn validate(self) -> Result<()> {
        effect_slot(self.effect)?;
        if self.brightness > 9 || self.speed > 4 || self.color > 7 {
            return Err(ProtocolError("Invalid lighting values"));
        }
        Ok(())
    }
    pub fn apply(self, source: &[u8]) -> Result<Vec<u8>> {
        validate_settings(source)?;
        self.validate()?;
        let mut output = source.to_vec();
        output[9] = u8::from(self.effect == 21);
        output[10] = self.effect;
        let offset = 0x3a + effect_slot(self.effect)? * 2;
        output[offset] = self.brightness;
        output[offset + 1] = (self.speed << 4) | self.color;
        Ok(output)
    }
}
pub const SLEEP: [(u16, u8); 10] = [
    (0, 0),
    (60, 2),
    (90, 3),
    (120, 4),
    (180, 6),
    (240, 8),
    (300, 10),
    (600, 20),
    (900, 30),
    (1200, 40),
];
pub fn sleep_settings(source: &[u8], seconds: u16) -> Result<Vec<u8>> {
    validate_settings(source)?;
    let value = SLEEP
        .iter()
        .find(|(s, _)| *s == seconds)
        .ok_or(ProtocolError("Unsupported sleep timeout"))?
        .1;
    let mut output = source.to_vec();
    output[24] = value;
    Ok(output)
}
pub fn color_planes(colors: &[[u8; 3]; 126]) -> [u8; 384] {
    let mut output = [0; 384];
    for (index, rgb) in colors.iter().enumerate() {
        for channel in 0..3 {
            output[channel * 126 + index] = rgb[channel];
        }
    }
    output
}
pub fn remap(source: &[u8], matrix: usize, code: [u8; 4]) -> Result<Vec<u8>> {
    if source.len() != 512 || source[508..] != [0, 0, 0x5a, 0xa5] || matrix >= 126 {
        return Err(ProtocolError("Unsupported keymap or matrix index"));
    }
    if !matches!(code[0], 0 | 2 | 3) {
        return Err(ProtocolError("Unsupported key action"));
    }
    let mut output = source.to_vec();
    output[matrix * 4..matrix * 4 + 4].copy_from_slice(&code);
    Ok(output)
}

pub fn without_macros(source: &[u8]) -> Result<Vec<u8>> {
    if source.len() != 512 || source[508..] != [0, 0, 0x5a, 0xa5] {
        return Err(ProtocolError("Unsupported keymap"));
    }
    let mut result = source.to_vec();
    for code in result[..504].as_chunks_mut::<4>().0 {
        if code[0] == 3 {
            code.fill(0)
        }
    }
    Ok(result)
}

pub fn color_transfer(colors: &[[u8; 3]; 126], transport: Transport) -> Result<Vec<u8>> {
    let data = color_planes(colors);
    match transport {
        Transport::Usb => Ok(data.to_vec()),
        Transport::Receiver => Ok(data[..378].to_vec()),
        Transport::Bluetooth => Err(ProtocolError(
            "Bluetooth color configuration is unsupported",
        )),
    }
}

pub fn normalize_keymap(source: &[u8], transport: Transport) -> Result<Vec<u8>> {
    match transport {
        Transport::Usb if source.len() == 512 && source[508..] == [0, 0, 0x5a, 0xa5] => {
            Ok(source.to_vec())
        }
        Transport::Receiver if source.len() == 504 => {
            // Receiver reads expose 126 entries only. The last two reserved entries
            // belong to SET framing: zeros followed by KB.ini CRC's 5A A5 marker.
            // These bytes are derived write framing, not observed receiver settings.
            let mut map = source.to_vec();
            map.extend([0, 0, 0, 0, 0, 0, 0x5a, 0xa5]);
            Ok(map)
        }
        _ => Err(ProtocolError(
            "Unsupported keymap response size or signature",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usb_live_frame_uses_live_command_and_interleaved_rgb() {
        let colors = std::array::from_fn(|index| [index as u8, 255 - index as u8, 37]);
        let report = usb_live_rgb_report(&colors);
        assert_eq!(&report[..8], &[0x06, 0x08, 0, 0, 1, 0, 0x7a, 0x01]);
        for (index, color) in colors.iter().enumerate() {
            assert_eq!(&report[8 + index * 3..11 + index * 3], color);
        }
        assert!(report[386..].iter().all(|byte| *byte == 0));

        // Saved colors use command 0x06 and R/G/B planes, not this live format.
        let saved = usb_reports(0x06, 0, 384, Some(&color_planes(&colors))).unwrap();
        assert_ne!(report, saved[0]);
    }

    #[test]
    fn usb_live_silence_keeps_the_complete_frame_header() {
        let report = usb_live_rgb_report(&[[0; 3]; 126]);
        assert_eq!(&report[..8], &[0x06, 0x08, 0, 0, 1, 0, 0x7a, 0x01]);
        assert!(report[8..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn receiver_ack_distinguishes_rejection_from_unrelated_reports() {
        let mut reply = [0u8; 20];
        reply[..4].copy_from_slice(&[0x13, 4, 10, 1]);
        assert_eq!(receiver_ack(&reply, 4, 1), ReceiverAck::Accepted);
        assert_eq!(receiver_ack(&reply, 4, 2), ReceiverAck::Unrelated);
        assert_eq!(receiver_ack(&reply, 2, 1), ReceiverAck::Unrelated);
        assert_eq!(receiver_ack(&reply[..19], 4, 1), ReceiverAck::Unrelated);
        reply[2] |= 0x80;
        assert_eq!(receiver_ack(&reply, 4, 1), ReceiverAck::ChecksumRejected);
        assert_eq!(receiver_ack(&[0xff], 4, 1), ReceiverAck::DeviceRejected);
    }
    fn block() -> Vec<u8> {
        let mut b = vec![0; 128];
        b[126..].copy_from_slice(&[0x5a, 0xa5]);
        b
    }
    #[test]
    fn receiver_rgb_transfer_omits_usb_reserved_padding() {
        let colors = [[1, 2, 3]; 126];
        let data = color_transfer(&colors, Transport::Receiver).unwrap();
        assert_eq!(data.len(), 378);
        assert_eq!(receiver_write(2, 0, &data).unwrap().len(), 27);
        assert_eq!(color_transfer(&colors, Transport::Usb).unwrap().len(), 384);
        assert!(color_transfer(&colors, Transport::Bluetooth).is_err());
    }
    #[test]
    fn old_fn_macro_indices_are_removed_without_affecting_other_actions() {
        let mut map = vec![0; 512];
        map[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
        map[40..44].copy_from_slice(&[3, 1, 1, 5]);
        map[44..48].copy_from_slice(&[0x0d, 0, 0, 0]);
        let output = without_macros(&map).unwrap();
        assert_eq!(&output[40..44], &[0; 4]);
        assert_eq!(&output[44..48], &[0x0d, 0, 0, 0]);
        assert_eq!(&output[508..], &[0, 0, 0x5a, 0xa5]);
    }
    #[test]
    fn receiver_keymap_omits_reserved_write_entries() {
        let mut source = vec![0; 504];
        source[..4].copy_from_slice(&[0, 0, 0, 0x29]);
        let map = normalize_keymap(&source, Transport::Receiver).unwrap();
        assert_eq!(&map[..504], &source);
        assert_eq!(&map[504..], &[0, 0, 0, 0, 0, 0, 0x5a, 0xa5]);
        assert!(normalize_keymap(&source[..500], Transport::Receiver).is_err());
        assert!(normalize_keymap(&source, Transport::Usb).is_err());
    }
    #[test]
    fn read_request_fixture() {
        assert_eq!(
            &receiver_read(0x44, 0).unwrap()[..5],
            &[0x13, 0x44, 1, 0, 0]
        );
        assert_eq!(receiver_read(0x44, 0).unwrap()[19], 0x58);
    }
    #[test]
    fn receiver_boundary() {
        let p = receiver_write(4, 0, &[5; 128]).unwrap();
        assert_eq!(p.len(), 10);
        assert_eq!(p[9][4], 2);
        assert_eq!(
            p[9][19],
            p[9][..19].iter().fold(0u8, |a, b| a.wrapping_add(*b))
        );
    }
    #[test]
    fn usb_boundary() {
        let p = usb_reports(5, 0, 513, Some(&[7; 513])).unwrap();
        assert_eq!(&p[0][..8], &[6, 5, 0, 0, 2, 0, 0, 2]);
        assert_eq!(&p[1][..8], &[6, 5, 0, 0, 2, 1, 1, 0]);
        assert_eq!(p[1][8], 7);
    }
    #[test]
    fn effect_slots_and_preservation() {
        let mut b = block();
        b[100] = 42;
        let s = Lighting {
            effect: 3,
            brightness: 9,
            speed: 4,
            color: 7,
        };
        let result = s.apply(&b).unwrap();
        assert_eq!(result[0x3e], 9);
        assert_eq!(result[0x3f], 0x47);
        assert_eq!(result[100], 42);
        assert_eq!(b[10], 0);
        assert_eq!(Lighting::read(&result).unwrap(), s);
    }
    #[test]
    fn invalid_settings_rejected() {
        assert!(
            Lighting {
                effect: 19,
                brightness: 9,
                speed: 4,
                color: 7
            }
            .apply(&block())
            .is_err()
        );
        assert!(sleep_settings(&[0; 128], 60).is_err());
        assert!(receiver_write(4, 16, &[1]).is_err());
        assert!(usb_reports(4, 0, 1, Some(&[0; 2])).is_err());
    }
    #[test]
    fn keymap_preserves_other_keys() {
        let mut b = vec![0; 512];
        b[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
        b[40] = 55;
        let r = remap(&b, 1, [0, 0, 0, 4]).unwrap();
        assert_eq!(r[40], 55);
        assert_eq!(&r[4..8], &[0, 0, 0, 4]);
        assert!(remap(&b, 126, [0; 4]).is_err());
    }
    #[test]
    fn rgb_planes() {
        let p = color_planes(&[[1, 2, 3]; 126]);
        assert_eq!(&p[..126], &[1; 126]);
        assert_eq!(&p[126..252], &[2; 126]);
        assert_eq!(&p[252..378], &[3; 126]);
        assert_eq!(&p[378..], &[0; 6]);
    }
}
