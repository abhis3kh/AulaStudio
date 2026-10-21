use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroEvent {
    pub usage: u8,
    pub pressed: bool,
    pub delay_ms: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroBinding {
    pub name: String,
    pub matrix: usize,
    pub events: Vec<MacroEvent>,
}

pub fn encode(bindings: &[MacroBinding]) -> Result<Vec<u8>, String> {
    if bindings.is_empty() || bindings.len() > 64 {
        return Err("Macro bank must contain 1 to 64 entries".into());
    }
    let mut unique = std::collections::HashSet::new();
    let mut bodies = Vec::new();
    for binding in bindings {
        if binding.matrix >= 126 || !unique.insert(binding.matrix) {
            return Err("Invalid or duplicate macro key".into());
        }
        let name = binding.name.encode_utf16().collect::<Vec<_>>();
        if name.is_empty() || name.len() > 30 || binding.events.is_empty() {
            return Err(
                "Each macro needs a name up to 30 UTF-16 units and at least one event".into(),
            );
        }
        let mut body = vec![(name.len() * 2) as u8];
        for unit in name {
            body.extend(unit.to_le_bytes())
        }
        let mut held = std::collections::HashSet::new();
        for event in &binding.events {
            if event.delay_ms > 0xfffff || event.usage == 0 {
                return Err("Invalid macro key or delay".into());
            }
            if event.pressed {
                if !held.insert(event.usage) {
                    return Err("Macro presses a key already held".into());
                }
            } else if !held.remove(&event.usage) {
                return Err("Macro releases a key that is not held".into());
            }
            let modifier = u8::from((0xe0..=0xe7).contains(&event.usage));
            body.extend([
                ((!event.pressed as u8) << 7) | (modifier << 4) | ((event.delay_ms >> 16) as u8),
                (event.delay_ms >> 8) as u8,
                event.delay_ms as u8,
                event.usage,
            ]);
        }
        if !held.is_empty() {
            return Err("Macro ends with keys still held".into());
        }
        bodies.push(body);
    }
    let mut offset = bindings.len() * 4;
    let total = offset + bodies.iter().map(Vec::len).sum::<usize>();
    if total > 4096 {
        return Err("Macros exceed the keyboard's 4 KB capacity".into());
    }
    let mut output = Vec::with_capacity(total);
    for body in &bodies {
        output.extend((offset as u16).to_le_bytes());
        output.extend((body.len() as u16).to_le_bytes());
        offset += body.len()
    }
    for body in bodies {
        output.extend(body)
    }
    Ok(output)
}
pub fn text_events(text: &str, delay_ms: u32) -> Result<Vec<MacroEvent>, String> {
    if text.is_empty() || delay_ms > 10000 {
        return Err("Enter text and a delay from 0 to 10000 ms".into());
    }
    let mut events = Vec::new();
    for character in text.chars() {
        let (usage, shift) = match character {
            'a'..='z' => (character as u8 - b'a' + 4, false),
            'A'..='Z' => (character as u8 - b'A' + 4, true),
            '1'..='9' => (character as u8 - b'1' + 0x1e, false),
            '0' => (0x27, false),
            ' ' => (0x2c, false),
            '\n' => (0x28, false),
            '\t' => (0x2b, false),
            _ => {
                let pairs = [
                    ('-', '_', 0x2d),
                    ('=', '+', 0x2e),
                    ('[', '{', 0x2f),
                    (']', '}', 0x30),
                    ('\\', '|', 0x31),
                    (';', ':', 0x33),
                    ('\'', '"', 0x34),
                    ('`', '~', 0x35),
                    (',', '<', 0x36),
                    ('.', '>', 0x37),
                    ('/', '?', 0x38),
                    ('1', '!', 0x1e),
                    ('2', '@', 0x1f),
                    ('3', '#', 0x20),
                    ('4', '$', 0x21),
                    ('5', '%', 0x22),
                    ('6', '^', 0x23),
                    ('7', '&', 0x24),
                    ('8', '*', 0x25),
                    ('9', '(', 0x26),
                    ('0', ')', 0x27),
                ];
                let matched = pairs
                    .iter()
                    .find(|(a, b, _)| *a == character || *b == character)
                    .ok_or_else(|| {
                        format!("Character {character:?} is unsupported by US-layout text macros")
                    })?;
                (matched.2, character == matched.1)
            }
        };
        if shift {
            events.push(MacroEvent {
                usage: 0xe1,
                pressed: true,
                delay_ms: 4,
            })
        }
        events.push(MacroEvent {
            usage,
            pressed: true,
            delay_ms,
        });
        events.push(MacroEvent {
            usage,
            pressed: false,
            delay_ms: 4,
        });
        if shift {
            events.push(MacroEvent {
                usage: 0xe1,
                pressed: false,
                delay_ms,
            })
        }
    }
    Ok(events)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_and_modifier_event_tags() {
        let bank = encode(&[MacroBinding {
            name: "m".into(),
            matrix: 78,
            events: text_events("A", 20).unwrap(),
        }])
        .unwrap();
        assert_eq!(&bank[..4], &[4, 0, 19, 0]);
        assert_eq!(&bank[7..11], &[0x10, 0, 4, 0xe1]);
        assert_eq!(&bank[11..15], &[0, 0, 20, 4]);
        assert_eq!(&bank[15..19], &[0x80, 0, 4, 4]);
        assert_eq!(&bank[19..23], &[0x90, 0, 20, 0xe1]);
    }
    #[test]
    fn unsupported_text_fails_without_silent_loss() {
        assert!(text_events("héllo", 20).is_err());
    }
    #[test]
    fn stuck_key_rejected() {
        assert!(
            encode(&[MacroBinding {
                name: "m".into(),
                matrix: 1,
                events: vec![MacroEvent {
                    usage: 4,
                    pressed: true,
                    delay_ms: 1
                }]
            }])
            .is_err()
        );
    }
}
