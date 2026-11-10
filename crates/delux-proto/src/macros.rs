//! Cmd 0x09 — a macro stored in the mouse for one button (variant 0x30 / MS_4).
//!
//! One packet = one macro = one button slot. Layout (driver `FUN_004d0590`):
//! `09 6F slot+1 mode 00 00 00 loops count records… sum16`, records are
//! `[flag | delay/10, code]` with flag 0x01 = press, 0x81 = release.

use serde::{Deserialize, Serialize};

use crate::{checksum16, Error, SLOT_COUNT};

/// Steps the vendor driver allows per macro (the packet could hold 50).
pub const MAX_STEPS: usize = 46;
pub const DELAY_MIN_MS: u16 = 10;
pub const DELAY_MAX_MS: u16 = 1270;

/// Mouse buttons inside a macro; keyboard keys are plain HID usages,
/// modifiers included (0xE0..=0xE7).
pub const MOUSE_LEFT: u8 = 0xF1;
pub const MOUSE_RIGHT: u8 = 0xF2;
pub const MOUSE_MIDDLE: u8 = 0xF3;
pub const MOUSE_FORWARD: u8 = 0xF4;
pub const MOUSE_BACK: u8 = 0xF5;
pub const MODIFIER_LEFT_GUI: u8 = 0xE3;

const PACKET_LEN: usize = 0x6F;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroStep {
    /// Press (true) or release (false).
    pub down: bool,
    /// HID keyboard usage 0x04..=0xE7, or one of the `MOUSE_*` codes.
    pub code: u8,
    /// Wait after this step: 10, 30, 50 … 1270 ms (see `snap_delay_ms`).
    pub delay_ms: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Repeat {
    /// Play this many times per press, 1..=255.
    Times(u8),
    /// Keep repeating until any key or button is pressed.
    UntilKeyPressed,
    /// Keep repeating while the button is held.
    WhileHeld,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Macro {
    pub steps: Vec<MacroStep>,
    pub repeat: Repeat,
}

/// Nearest delay the firmware can represent. The delay shares its byte with
/// the press/release flag, whose lowest bit is always set, so only odd
/// multiples of 10 ms exist.
pub fn snap_delay_ms(ms: u32) -> u16 {
    let tens = ((ms + 5) / 10).clamp(1, 127) as u16;
    // Round to the nearest odd count; ties go down.
    let odd = if tens % 2 == 1 {
        tens
    } else if ms as u16 >= tens * 10 {
        tens + 1
    } else {
        tens - 1
    };
    odd.clamp(1, 127) * 10
}

fn valid_code(code: u8) -> bool {
    matches!(code, 0x04..=0xE7 | MOUSE_LEFT..=MOUSE_BACK)
}

/// Build the 111-byte inner packet for the macro on `slot` (0-based).
/// Rejects anything that could leave a key stuck down.
pub fn encode_macro(slot: usize, m: &Macro) -> Result<Vec<u8>, Error> {
    if slot == 0 || slot >= SLOT_COUNT {
        return Err(Error::OutOfRange {
            field: "macro slot",
            value: slot as i64,
        });
    }
    if m.steps.is_empty() || m.steps.len() > MAX_STEPS {
        return Err(Error::OutOfRange {
            field: "macro steps",
            value: m.steps.len() as i64,
        });
    }
    let (mode, loops) = match m.repeat {
        Repeat::Times(0) => {
            return Err(Error::OutOfRange {
                field: "macro repeat",
                value: 0,
            })
        }
        Repeat::Times(n) => (0, n),
        Repeat::UntilKeyPressed => (1, 1),
        Repeat::WhileHeld => (2, 1),
    };

    let mut p = vec![0u8; PACKET_LEN];
    p[0] = 0x09;
    p[1] = PACKET_LEN as u8;
    p[2] = slot as u8 + 1;
    p[3] = mode;
    p[7] = loops;
    p[8] = m.steps.len() as u8;
    let mut held: Vec<u8> = Vec::new();
    for (i, s) in m.steps.iter().enumerate() {
        if !valid_code(s.code) {
            return Err(Error::OutOfRange {
                field: "macro key",
                value: s.code as i64,
            });
        }
        if snap_delay_ms(s.delay_ms as u32) != s.delay_ms {
            return Err(Error::OutOfRange {
                field: "macro delay",
                value: s.delay_ms as i64,
            });
        }
        match (s.down, held.iter().position(|c| *c == s.code)) {
            (true, None) => held.push(s.code),
            (false, Some(at)) => {
                held.remove(at);
            }
            (true, Some(_)) => {
                return Err(Error::OutOfRange {
                    field: "macro presses a key twice",
                    value: s.code as i64,
                })
            }
            (false, None) => {
                return Err(Error::OutOfRange {
                    field: "macro releases a key not pressed",
                    value: s.code as i64,
                })
            }
        }
        let flag = if s.down { 0x01 } else { 0x81 };
        p[9 + i * 2] = flag | (s.delay_ms / 10) as u8;
        p[10 + i * 2] = s.code;
    }
    if let Some(code) = held.first() {
        return Err(Error::OutOfRange {
            field: "macro leaves a key down",
            value: *code as i64,
        });
    }
    let cs = checksum16(&p[3..PACKET_LEN - 2]).to_be_bytes();
    p[PACKET_LEN - 2..].copy_from_slice(&cs);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{checksum16, frames};

    fn step(down: bool, code: u8, delay_ms: u16) -> MacroStep {
        MacroStep {
            down,
            code,
            delay_ms,
        }
    }

    fn ab() -> Macro {
        Macro {
            steps: vec![
                step(true, 0x04, 50),
                step(false, 0x04, 10),
                step(true, 0x05, 50),
                step(false, 0x05, 10),
            ],
            repeat: Repeat::Times(1),
        }
    }

    #[test]
    fn driver_example_a_then_b_on_back_button() {
        // Slot index 7 (back side button) → slot byte 8.
        let p = encode_macro(7, &ab()).unwrap();
        assert_eq!(p.len(), 0x6F);
        assert_eq!(
            &p[..17],
            &[
                0x09, 0x6F, 0x08, 0, 0, 0, 0, 0x01, 0x04, 0x05, 0x04, 0x81, 0x04, 0x05, 0x05, 0x81,
                0x05
            ]
        );
        assert!(p[17..0x6D].iter().all(|b| *b == 0));
        assert_eq!(&p[0x6D..], &[0x01, 0x23]);
        assert_eq!(
            u16::from_be_bytes([p[0x6D], p[0x6E]]),
            checksum16(&p[3..0x6D])
        );
    }

    #[test]
    fn frames_match_the_driver_example() {
        let f = frames(&encode_macro(7, &ab()).unwrap()).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(&f[0][..3], &[0x04, 0x40, 0x00]);
        assert_eq!(&f[0][62..], &[0x01, 0xE7]);
        assert_eq!(&f[1][..3], &[0x04, 0x39, 0x01]);
        assert_eq!(&f[1][53..57], &[0x01, 0x23, 0x00, 0x62]);
    }

    #[test]
    fn repeat_modes() {
        let mut m = ab();
        m.repeat = Repeat::Times(7);
        let p = encode_macro(7, &m).unwrap();
        assert_eq!((p[3], p[7]), (0, 7));
        m.repeat = Repeat::UntilKeyPressed;
        let p = encode_macro(7, &m).unwrap();
        assert_eq!((p[3], p[7]), (1, 1));
        m.repeat = Repeat::WhileHeld;
        let p = encode_macro(7, &m).unwrap();
        assert_eq!((p[3], p[7]), (2, 1));
    }

    #[test]
    fn mouse_buttons_and_modifiers_are_plain_codes() {
        let m = Macro {
            steps: vec![
                step(true, MODIFIER_LEFT_GUI, 10),
                step(true, MOUSE_LEFT, 10),
                step(false, MOUSE_LEFT, 10),
                step(false, MODIFIER_LEFT_GUI, 10),
            ],
            repeat: Repeat::Times(1),
        };
        let p = encode_macro(6, &m).unwrap();
        assert_eq!(p[2], 7);
        assert_eq!(
            &p[8..17],
            &[4, 0x01, 0xE3, 0x01, 0xF1, 0x81, 0xF1, 0x81, 0xE3]
        );
    }

    #[test]
    fn delay_must_be_an_exact_firmware_value() {
        // The delay shares a byte with the up/down flag whose lowest bit is
        // always set, so only 10, 30, 50 … 1270 ms exist.
        for bad in [0u16, 20, 55, 1280, 2000] {
            let m = Macro {
                steps: vec![step(true, 4, bad), step(false, 4, 10)],
                repeat: Repeat::Times(1),
            };
            assert_eq!(
                encode_macro(7, &m),
                Err(Error::OutOfRange {
                    field: "macro delay",
                    value: bad as i64
                })
            );
        }
        assert_eq!(snap_delay_ms(0), 10);
        assert_eq!(snap_delay_ms(20), 30);
        assert_eq!(snap_delay_ms(54), 50);
        assert_eq!(snap_delay_ms(9999), 1270);
        let ok = Macro {
            steps: vec![step(true, 4, 1270), step(false, 4, 10)],
            repeat: Repeat::Times(1),
        };
        assert_eq!(encode_macro(7, &ok).unwrap()[9], 0x7F);
    }

    #[test]
    fn a_key_left_down_is_rejected() {
        let m = Macro {
            steps: vec![step(true, 4, 10)],
            repeat: Repeat::Times(1),
        };
        assert_eq!(
            encode_macro(7, &m),
            Err(Error::OutOfRange {
                field: "macro leaves a key down",
                value: 4
            })
        );
    }

    #[test]
    fn release_without_press_and_double_press_are_rejected() {
        let up_first = Macro {
            steps: vec![step(false, 4, 10)],
            repeat: Repeat::Times(1),
        };
        assert!(encode_macro(7, &up_first).is_err());
        let twice = Macro {
            steps: vec![step(true, 4, 10), step(true, 4, 10), step(false, 4, 10)],
            repeat: Repeat::Times(1),
        };
        assert!(encode_macro(7, &twice).is_err());
    }

    #[test]
    fn limits() {
        let empty = Macro {
            steps: vec![],
            repeat: Repeat::Times(1),
        };
        assert!(encode_macro(7, &empty).is_err());
        let mut steps = Vec::new();
        for _ in 0..24 {
            steps.push(step(true, 4, 10));
            steps.push(step(false, 4, 10));
        }
        let too_long = Macro {
            steps,
            repeat: Repeat::Times(1),
        };
        assert_eq!(too_long.steps.len(), 48);
        assert_eq!(
            encode_macro(7, &too_long),
            Err(Error::OutOfRange {
                field: "macro steps",
                value: 48
            })
        );
        assert!(encode_macro(0, &ab()).is_err(), "left click slot");
        assert!(encode_macro(18, &ab()).is_err(), "slot out of range");
        let zero_times = Macro {
            repeat: Repeat::Times(0),
            ..ab()
        };
        assert!(encode_macro(7, &zero_times).is_err());
        let bad_code = Macro {
            steps: vec![step(true, 0x03, 10), step(false, 0x03, 10)],
            repeat: Repeat::Times(1),
        };
        assert!(
            encode_macro(7, &bad_code).is_err(),
            "code 03 is the long-delay marker"
        );
    }
}
