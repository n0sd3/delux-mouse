//! Cmd 0x08 — button map (18 slots × 3 bytes).

use serde::{Deserialize, Serialize};

use crate::{checksum16, Error};

pub const SLOT_COUNT: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonAction {
    Disabled,
    Left,
    Right,
    Middle,
    Back,
    Forward,
    DoubleClick,
    Fire,
    ScrollUp,
    ScrollDown,
    DpiCycle,
    DpiUp,
    DpiDown,
    /// HID modifier bitmask + HID keyboard usage.
    Key { modifiers: u8, usage: u8 },
    /// Media / system function code, 0x15..=0x26.
    Media(u8),
    /// Any other firmware code, passed through untouched.
    Raw([u8; 3]),
}

impl ButtonAction {
    pub fn to_bytes(&self) -> [u8; 3] {
        let simple = |code| [code, 0, 0];
        match *self {
            Self::Disabled => simple(0x01),
            Self::Left => simple(0x02),
            Self::Right => simple(0x03),
            Self::Middle => simple(0x04),
            Self::Back => simple(0x05),
            Self::Forward => simple(0x06),
            Self::DoubleClick => simple(0x07),
            Self::Fire => simple(0x08),
            Self::ScrollUp => simple(0x09),
            Self::ScrollDown => simple(0x0A),
            Self::DpiCycle => simple(0x0D),
            Self::DpiUp => simple(0x0E),
            Self::DpiDown => simple(0x0F),
            Self::Key { modifiers, usage } => [0x11, modifiers, usage],
            Self::Media(code) => simple(code),
            Self::Raw(bytes) => bytes,
        }
    }

    pub fn from_bytes(b: [u8; 3]) -> Self {
        match b {
            [0x01, 0, 0] => Self::Disabled,
            [0x02, 0, 0] => Self::Left,
            [0x03, 0, 0] => Self::Right,
            [0x04, 0, 0] => Self::Middle,
            [0x05, 0, 0] => Self::Back,
            [0x06, 0, 0] => Self::Forward,
            [0x07, 0, 0] => Self::DoubleClick,
            [0x08, 0, 0] => Self::Fire,
            [0x09, 0, 0] => Self::ScrollUp,
            [0x0A, 0, 0] => Self::ScrollDown,
            [0x0D, 0, 0] => Self::DpiCycle,
            [0x0E, 0, 0] => Self::DpiUp,
            [0x0F, 0, 0] => Self::DpiDown,
            [0x11, modifiers, usage] => Self::Key { modifiers, usage },
            [code @ 0x15..=0x26, 0, 0] => Self::Media(code),
            other => Self::Raw(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ButtonMap(pub [ButtonAction; SLOT_COUNT]);

impl ButtonMap {
    /// The vendor driver's default map.
    pub fn factory() -> Self {
        use ButtonAction::*;
        let mut m = [Disabled; SLOT_COUNT];
        m[0] = Left;
        m[1] = Right;
        m[2] = Middle;
        m[3] = DpiCycle;
        m[5] = Media(0x18);
        m[6] = Forward;
        m[7] = Back;
        m[16] = ScrollUp;
        m[17] = ScrollDown;
        ButtonMap(m)
    }
}

/// Build the 59-byte inner packet. Slot 0 must stay left click so the mouse
/// can never be left without a primary button.
pub fn encode_buttons(map: &ButtonMap) -> Result<Vec<u8>, Error> {
    if map.0[0] != ButtonAction::Left {
        return Err(Error::OutOfRange { field: "slot 0 must stay left click", value: 0 });
    }
    let mut p = vec![0u8; 0x3B];
    p[..3].copy_from_slice(&[0x08, 0x3B, 0x01]);
    for (i, a) in map.0.iter().enumerate() {
        p[3 + i * 3..6 + i * 3].copy_from_slice(&a.to_bytes());
    }
    let cs = checksum16(&p[3..57]).to_be_bytes();
    p[57..59].copy_from_slice(&cs);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum16;

    #[test]
    fn key_bytes() {
        assert_eq!(ButtonAction::Key { modifiers: 0, usage: 0x68 }.to_bytes(), [0x11, 0x00, 0x68]);
    }

    #[test]
    fn roundtrip_every_variant() {
        let all = [
            ButtonAction::Disabled,
            ButtonAction::Left,
            ButtonAction::Right,
            ButtonAction::Middle,
            ButtonAction::Back,
            ButtonAction::Forward,
            ButtonAction::DoubleClick,
            ButtonAction::Fire,
            ButtonAction::ScrollUp,
            ButtonAction::ScrollDown,
            ButtonAction::DpiCycle,
            ButtonAction::DpiUp,
            ButtonAction::DpiDown,
            ButtonAction::Key { modifiers: 0x01, usage: 0x06 },
            ButtonAction::Media(0x18),
            ButtonAction::Raw([0x29, 0x00, 0x03]),
        ];
        for a in all {
            assert_eq!(ButtonAction::from_bytes(a.to_bytes()), a, "{a:?}");
        }
    }

    #[test]
    fn known_codes() {
        assert_eq!(ButtonAction::Disabled.to_bytes(), [0x01, 0, 0]);
        assert_eq!(ButtonAction::Back.to_bytes(), [0x05, 0, 0]);
        assert_eq!(ButtonAction::Forward.to_bytes(), [0x06, 0, 0]);
        assert_eq!(ButtonAction::DpiCycle.to_bytes(), [0x0D, 0, 0]);
        assert_eq!(ButtonAction::from_bytes([0x1B, 0, 0]), ButtonAction::Media(0x1B));
    }

    #[test]
    fn factory_map_encodes() {
        let m = ButtonMap::factory();
        let p = encode_buttons(&m).unwrap();
        assert_eq!(p.len(), 0x3B);
        assert_eq!(&p[..3], &[0x08, 0x3B, 0x01]);
        assert_eq!(&p[3..6], &[0x02, 0, 0]); // slot 0 = left
        assert_eq!(&p[12..15], &[0x0D, 0, 0]); // slot 3 = DPI cycle
        assert_eq!(&p[21..24], &[0x06, 0, 0]); // slot 6 = forward
        assert_eq!(&p[24..27], &[0x05, 0, 0]); // slot 7 = back
        assert_eq!(&p[51..54], &[0x09, 0, 0]); // slot 16 = scroll up
        assert_eq!(u16::from_be_bytes([p[57], p[58]]), checksum16(&p[3..57]));
    }

    #[test]
    fn left_click_slot_is_protected() {
        let mut m = ButtonMap::factory();
        m.0[0] = ButtonAction::Disabled;
        assert_eq!(
            encode_buttons(&m),
            Err(Error::OutOfRange { field: "slot 0 must stay left click", value: 0 })
        );
    }
}
