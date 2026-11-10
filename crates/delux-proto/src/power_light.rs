//! Cmd 0x05 — light, sleep timer and key debounce.

use serde::{Deserialize, Serialize};

use crate::{checksum16, Error, Rgb};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerLightConfig {
    /// Combo index 0..=11 (0 off, 1 static, 2 breathing, 3 neon, …).
    pub light_mode: u8,
    /// 4..=8.
    pub breath_speed: u8,
    /// 1..=8; only sent for light modes 1 and 9.
    pub brightness: u8,
    pub color: Rgb,
    /// Minutes until the light turns off, 1..=60.
    pub light_off_min: u8,
    /// Minutes until the mouse sleeps, 1..=60.
    pub sleep_min: u8,
    /// 1..=50.
    pub debounce_ms: u8,
}

impl PowerLightConfig {
    pub fn default_profile() -> Self {
        PowerLightConfig {
            light_mode: 1,
            breath_speed: 4,
            brightness: 8,
            color: Rgb(255, 0, 0),
            light_off_min: 10,
            sleep_min: 10,
            debounce_ms: 6,
        }
    }
}

fn check(field: &'static str, value: u8, min: u8, max: u8) -> Result<(), Error> {
    if value < min || value > max {
        return Err(Error::OutOfRange { field, value: value as i64 });
    }
    Ok(())
}

/// Build the 15-byte inner packet.
pub fn encode_power_light(cfg: &PowerLightConfig) -> Result<Vec<u8>, Error> {
    check("light_mode", cfg.light_mode, 0, 11)?;
    check("breath_speed", cfg.breath_speed, 4, 8)?;
    check("brightness", cfg.brightness, 1, 8)?;
    check("light_off_min", cfg.light_off_min, 1, 60)?;
    check("sleep_min", cfg.sleep_min, 1, 60)?;
    check("debounce_ms", cfg.debounce_ms, 1, 50)?;

    let brightness = if matches!(cfg.light_mode, 1 | 9) { cfg.brightness } else { 8 };
    let Rgb(r, g, b) = cfg.color;
    let mut p = vec![0u8; 0x0F];
    p[..3].copy_from_slice(&[0x05, 0x0F, 0x01]);
    p[3] = cfg.light_mode << 4;
    p[4] = (cfg.sleep_min & 0xF0) | (9 - cfg.breath_speed);
    p[5] = ((cfg.sleep_min & 0x0F) << 4) | brightness;
    p[6..9].copy_from_slice(&[r, g, b]);
    p[9] = cfg.light_off_min;
    p[10] = cfg.debounce_ms;
    let cs = checksum16(&p[3..11]).to_be_bytes();
    p[11..13].copy_from_slice(&cs);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum16;

    fn sample() -> PowerLightConfig {
        PowerLightConfig {
            light_mode: 1,
            breath_speed: 4,
            brightness: 5,
            color: Rgb(1, 2, 3),
            light_off_min: 10,
            sleep_min: 0x12,
            debounce_ms: 6,
        }
    }

    #[test]
    fn sample_vector() {
        let p = encode_power_light(&sample()).unwrap();
        let cs = checksum16(&p[3..11]).to_be_bytes();
        assert_eq!(
            p,
            vec![0x05, 0x0F, 0x01, 0x10, 0x15, 0x25, 1, 2, 3, 10, 6, cs[0], cs[1], 0, 0]
        );
    }

    #[test]
    fn brightness_only_sent_for_modes_1_and_9() {
        let mut c = sample();
        c.light_mode = 2;
        assert_eq!(encode_power_light(&c).unwrap()[5] & 0x0F, 8);
        c.light_mode = 9;
        assert_eq!(encode_power_light(&c).unwrap()[5] & 0x0F, 5);
    }

    #[test]
    fn default_is_valid() {
        assert!(encode_power_light(&PowerLightConfig::default_profile()).is_ok());
    }

    #[test]
    fn rejects_out_of_range_fields() {
        let cases: [(&str, fn(&mut PowerLightConfig)); 10] = [
            ("light_mode", |c| c.light_mode = 12),
            ("breath_speed", |c| c.breath_speed = 3),
            ("breath_speed", |c| c.breath_speed = 9),
            ("brightness", |c| c.brightness = 0),
            ("brightness", |c| c.brightness = 9),
            ("light_off_min", |c| c.light_off_min = 0),
            ("light_off_min", |c| c.light_off_min = 61),
            ("sleep_min", |c| c.sleep_min = 0),
            ("sleep_min", |c| c.sleep_min = 61),
            ("debounce_ms", |c| c.debounce_ms = 51),
        ];
        for (field, mutate) in cases {
            let mut c = sample();
            mutate(&mut c);
            match encode_power_light(&c) {
                Err(Error::OutOfRange { field: f, .. }) => assert_eq!(f, field),
                other => panic!("{field}: expected OutOfRange, got {other:?}"),
            }
        }
    }
}
