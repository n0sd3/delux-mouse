//! Cmd 0x06 — polling rate.

use serde::{Deserialize, Serialize};

/// Polling rate; the discriminant is the firmware code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PollingRate {
    Hz125 = 1,
    Hz250,
    Hz500,
    Hz1000,
    Hz2000,
    Hz4000,
    Hz8000,
}

impl PollingRate {
    pub const ALL: [PollingRate; 7] = [
        Self::Hz125,
        Self::Hz250,
        Self::Hz500,
        Self::Hz1000,
        Self::Hz2000,
        Self::Hz4000,
        Self::Hz8000,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn hz(self) -> u16 {
        125 << (self.code() - 1)
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.code() == code)
    }

    pub fn from_hz(hz: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.hz() == hz)
    }
}

/// `06 09 01 B !B 00 00 00 00`. High nibble of `B` is the high-speed (8K
/// receiver) code; low nibble is the same rate capped at 1000 Hz, for the
/// low-speed path.
pub fn encode_polling(rate: PollingRate) -> Vec<u8> {
    let b = (rate.code() << 4) | rate.code().min(4);
    vec![0x06, 0x09, 0x01, b, !b, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hz8000_vector() {
        assert_eq!(
            encode_polling(PollingRate::Hz8000),
            vec![0x06, 0x09, 0x01, 0x74, 0x8B, 0, 0, 0, 0]
        );
    }

    #[test]
    fn hz1000_uses_both_nibbles() {
        assert_eq!(encode_polling(PollingRate::Hz1000)[3], 0x44);
        assert_eq!(encode_polling(PollingRate::Hz125)[3], 0x11);
    }

    #[test]
    fn complement_and_length_hold_for_every_rate() {
        for r in PollingRate::ALL {
            let p = encode_polling(r);
            assert_eq!(p.len(), 9);
            assert_eq!(p[3] ^ p[4], 0xFF);
        }
    }

    #[test]
    fn hz_roundtrip() {
        for r in PollingRate::ALL {
            assert_eq!(PollingRate::from_hz(r.hz()), Some(r));
        }
        assert_eq!(PollingRate::from_hz(300), None);
        assert_eq!(PollingRate::Hz8000.hz(), 8000);
        assert_eq!(PollingRate::from_code(7), Some(PollingRate::Hz8000));
        assert_eq!(PollingRate::from_code(0), None);
    }
}
