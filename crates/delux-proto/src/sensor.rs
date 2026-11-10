//! Cmd 0x04 — DPI stages and sensor options (variant 0x30 / MS_4 layout).

use serde::{Deserialize, Serialize};

use crate::{checksum16, Error};

pub const STAGE_COUNT: usize = 8;
pub const DPI_MIN: u16 = 50;
pub const DPI_MAX: u16 = 26000;
pub const DPI_STEP: u16 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpiStage {
    pub enabled: bool,
    pub dpi: u16,
    pub color: Rgb,
}

/// Lift-off distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lod {
    Mm1 = 0,
    Mm2 = 1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorConfig {
    #[serde(deserialize_with = "crate::exact_array")]
    pub stages: [DpiStage; STAGE_COUNT],
    /// 1-based; must name an enabled stage.
    pub current_stage: u8,
    pub lod: Lod,
    pub ripple: bool,
    pub angle_snap: bool,
    pub motion_sync: bool,
}

impl SensorConfig {
    /// Starting profile: six enabled stages, two spare.
    pub fn default_profile() -> Self {
        let stage = |enabled, dpi, color| DpiStage {
            enabled,
            dpi,
            color,
        };
        SensorConfig {
            stages: [
                stage(true, 400, Rgb(255, 0, 0)),
                stage(true, 800, Rgb(0, 0, 255)),
                stage(true, 1600, Rgb(0, 255, 0)),
                stage(true, 3200, Rgb(255, 255, 0)),
                stage(true, 6400, Rgb(0, 255, 255)),
                stage(true, 12800, Rgb(255, 0, 255)),
                stage(false, 16000, Rgb(255, 128, 0)),
                stage(false, 26000, Rgb(255, 255, 255)),
            ],
            current_stage: 3,
            lod: Lod::Mm1,
            ripple: false,
            angle_snap: false,
            motion_sync: false,
        }
    }
}

/// Build the 56-byte inner packet. Layout: docs/protocol, "Variante 0x30".
pub fn encode_sensor(cfg: &SensorConfig) -> Result<Vec<u8>, Error> {
    for s in &cfg.stages {
        if s.dpi < DPI_MIN || s.dpi > DPI_MAX || !s.dpi.is_multiple_of(DPI_STEP) {
            return Err(Error::OutOfRange {
                field: "dpi",
                value: s.dpi as i64,
            });
        }
    }
    let current = cfg.current_stage as usize;
    if current == 0 || current > STAGE_COUNT || !cfg.stages[current - 1].enabled {
        return Err(Error::OutOfRange {
            field: "current_stage",
            value: cfg.current_stage as i64,
        });
    }

    let mut p = vec![0u8; 0x38];
    p[..3].copy_from_slice(&[0x04, 0x38, 0x01]);
    p[3] = cfg.lod as u8;
    p[4] = cfg.ripple as u8;
    p[6] = cfg.angle_snap as u8;
    p[7] = cfg.motion_sync as u8;
    for (i, s) in cfg.stages.iter().enumerate() {
        p[5] |= (s.enabled as u8) << i;
        let [lo, hi] = (s.dpi / DPI_STEP - 1).to_le_bytes();
        p[8 + i] = lo;
        p[16 + i] = hi;
        let Rgb(r, g, b) = s.color;
        p[25 + i * 3..28 + i * 3].copy_from_slice(&[r, g, b]);
    }
    p[24] = cfg.current_stage;
    p[49] = 0x03;
    let cs = checksum16(&p[3..50]).to_be_bytes();
    p[50..52].copy_from_slice(&cs);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum16;

    #[test]
    fn dpi_encoding() {
        // 1600 → 31 = 0x001F ; 26000 → 519 = 0x0207
        let mut c = SensorConfig::default_profile();
        c.stages[0].dpi = 1600;
        c.stages[1].dpi = 26000;
        let p = encode_sensor(&c).unwrap();
        assert_eq!(p.len(), 0x38);
        assert_eq!(&p[..3], &[0x04, 0x38, 0x01]);
        assert_eq!((p[8], p[16]), (0x1F, 0x00));
        assert_eq!((p[9], p[17]), (0x07, 0x02));
        assert_eq!(p[49], 0x03);
        assert_eq!(u16::from_be_bytes([p[50], p[51]]), checksum16(&p[3..50]));
        assert_eq!(&p[52..], &[0, 0, 0, 0]);
    }

    #[test]
    fn option_bytes_follow_ms4_layout() {
        let mut c = SensorConfig::default_profile();
        c.lod = Lod::Mm2;
        c.ripple = true;
        c.angle_snap = false;
        c.motion_sync = true;
        let p = encode_sensor(&c).unwrap();
        assert_eq!((p[3], p[4], p[6], p[7]), (1, 1, 0, 1));
    }

    #[test]
    fn default_profile_mask_stage_and_colors() {
        let c = SensorConfig::default_profile();
        let p = encode_sensor(&c).unwrap();
        assert_eq!(p[5], 0b0011_1111); // stages 1..=6 enabled
        assert_eq!(p[24], 3);
        let Rgb(r, g, b) = c.stages[7].color;
        assert_eq!(&p[46..49], &[r, g, b]);
    }

    #[test]
    fn rejects_bad_dpi() {
        for bad in [0u16, 49, 75, 26050] {
            let mut c = SensorConfig::default_profile();
            c.stages[0].dpi = bad;
            assert_eq!(
                encode_sensor(&c),
                Err(Error::OutOfRange {
                    field: "dpi",
                    value: bad as i64
                })
            );
        }
    }

    #[test]
    fn bad_dpi_on_disabled_stage_is_still_rejected() {
        let mut c = SensorConfig::default_profile();
        c.stages[7].dpi = 75;
        assert!(encode_sensor(&c).is_err());
    }

    #[test]
    fn rejects_current_stage_disabled_or_out_of_range() {
        for bad in [0u8, 7, 9] {
            // stage 7 exists but is disabled in the default profile
            let mut c = SensorConfig::default_profile();
            c.current_stage = bad;
            assert_eq!(
                encode_sensor(&c),
                Err(Error::OutOfRange {
                    field: "current_stage",
                    value: bad as i64
                })
            );
        }
    }

    #[test]
    fn rejects_no_stage_enabled() {
        let mut c = SensorConfig::default_profile();
        for s in &mut c.stages {
            s.enabled = false;
        }
        assert!(encode_sensor(&c).is_err());
    }
}
