//! Cmd 0x06 — polling rate.

use serde::{Deserialize, Serialize};

/// Polling rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PollingRate {
    Hz125,
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

    pub fn hz(self) -> u16 {
        match self {
            Self::Hz125 => 125,
            Self::Hz250 => 250,
            Self::Hz500 => 500,
            Self::Hz1000 => 1000,
            Self::Hz2000 => 2000,
            Self::Hz4000 => 4000,
            Self::Hz8000 => 8000,
        }
    }

    pub fn from_hz(hz: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.hz() == hz)
    }

    /// One-hot firmware code of variant 0x30 (MS_4), used both in cmd 0x06
    /// and in the polling event (type 0x20).
    pub fn wire(self) -> u8 {
        match self {
            Self::Hz125 => 0x20,
            Self::Hz250 => 0x10,
            Self::Hz500 => 0x08,
            Self::Hz1000 => 0x04,
            Self::Hz2000 => 0x02,
            Self::Hz4000 => 0x01,
            Self::Hz8000 => 0x40,
        }
    }

    pub fn from_wire(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.wire() == code)
    }
}

/// `06 09 01 B !B 00 00 00 00`, `B` = one-hot rate code.
pub fn encode_polling(rate: PollingRate) -> Vec<u8> {
    let b = rate.wire();
    vec![0x06, 0x09, 0x01, b, !b, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    // Variant 0x30 (MS_4) uses a one-hot code, from the driver's apply code
    // at 0x4d8fef. The earlier (hs<<4)|low layout belongs to other variants.
    const TABLE: [(PollingRate, u8); 7] = [
        (PollingRate::Hz125, 0x20),
        (PollingRate::Hz250, 0x10),
        (PollingRate::Hz500, 0x08),
        (PollingRate::Hz1000, 0x04),
        (PollingRate::Hz2000, 0x02),
        (PollingRate::Hz4000, 0x01),
        (PollingRate::Hz8000, 0x40),
    ];

    #[test]
    fn hz8000_vector() {
        assert_eq!(
            encode_polling(PollingRate::Hz8000),
            vec![0x06, 0x09, 0x01, 0x40, 0xBF, 0, 0, 0, 0]
        );
    }

    #[test]
    fn every_rate_uses_its_one_hot_code() {
        for (rate, code) in TABLE {
            let p = encode_polling(rate);
            assert_eq!(p.len(), 9);
            assert_eq!(p[3], code, "{rate:?}");
            assert_eq!(p[3] ^ p[4], 0xFF);
            assert_eq!(PollingRate::from_wire(code), Some(rate));
        }
    }

    #[test]
    fn unknown_wire_codes_are_rejected() {
        for bad in [0x00u8, 0x03, 0x44, 0x74, 0x80] {
            assert_eq!(PollingRate::from_wire(bad), None, "{bad:#04x}");
        }
    }

    #[test]
    fn hz_roundtrip() {
        for r in PollingRate::ALL {
            assert_eq!(PollingRate::from_hz(r.hz()), Some(r));
        }
        assert_eq!(PollingRate::from_hz(300), None);
        assert_eq!(PollingRate::Hz8000.hz(), 8000);
    }
}
