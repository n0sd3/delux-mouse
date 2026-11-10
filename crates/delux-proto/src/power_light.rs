//! Cmd 0x05 — light, sleep timer and key debounce (variant 0x30 / MS_4 value ranges).

use serde::{Deserialize, Serialize};

use crate::{checksum16, Error, Rgb};

/// Byte 9 of the packet. Other variants use it as a light-off timer; for
/// this model the vendor driver has no control for it and always sends 2.
const BYTE9_MS4: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerLightConfig {
    /// Combo index 0..=11 (0 off, 1 static, 2 breathing, 3 neon, …).
    pub light_mode: u8,
    /// 4..=8.
    pub breath_speed: u8,
    /// 1..=8; only sent for light modes 1 and 9.
    pub brightness: u8,
    pub color: Rgb,
    /// Minutes of idle before the mouse sleeps, 1..=20.
    pub sleep_min: u8,
    /// Key debounce in ms: even values 2..=50 (the firmware counts 2 ms steps).
    pub debounce_ms: u8,
}

impl PowerLightConfig {
    /// The vendor driver's defaults for this model.
    pub fn default_profile() -> Self {
        PowerLightConfig {
            light_mode: 1,
            breath_speed: 6,
            brightness: 8,
            color: Rgb(255, 0, 0),
            sleep_min: 10,
            debounce_ms: 6,
        }
    }
}

fn check(field: &'static str, value: u8, min: u8, max: u8) -> Result<(), Error> {
    if value < min || value > max {
        return Err(Error::OutOfRange {
            field,
            value: value as i64,
        });
    }
    Ok(())
}

/// Build the 15-byte inner packet.
pub fn encode_power_light(cfg: &PowerLightConfig) -> Result<Vec<u8>, Error> {
    check("light_mode", cfg.light_mode, 0, 11)?;
    check("breath_speed", cfg.breath_speed, 4, 8)?;
    check("brightness", cfg.brightness, 1, 8)?;
    check("sleep_min", cfg.sleep_min, 1, 20)?;
    check("debounce_ms", cfg.debounce_ms, 2, 50)?;
    if !cfg.debounce_ms.is_multiple_of(2) {
        return Err(Error::OutOfRange {
            field: "debounce_ms",
            value: cfg.debounce_ms as i64,
        });
    }

    let brightness = if matches!(cfg.light_mode, 1 | 9) {
        cfg.brightness
    } else {
        8
    };
    let Rgb(r, g, b) = cfg.color;
    let mut p = vec![0u8; 0x0F];
    p[..3].copy_from_slice(&[0x05, 0x0F, 0x01]);
    p[3] = cfg.light_mode << 4;
    p[4] = (cfg.sleep_min & 0xF0) | (9 - cfg.breath_speed);
    p[5] = ((cfg.sleep_min & 0x0F) << 4) | brightness;
    p[6..9].copy_from_slice(&[r, g, b]);
    p[9] = BYTE9_MS4;
    p[10] = cfg.debounce_ms / 2;
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
            sleep_min: 0x12,
            debounce_ms: 6,
        }
    }

    #[test]
    fn sample_vector() {
        let p = encode_power_light(&sample()).unwrap();
        let cs = checksum16(&p[3..11]).to_be_bytes();
        // byte 9 is the constant 2 the vendor driver sends for this model;
        // debounce goes out in 2 ms steps (6 ms → 3).
        assert_eq!(
            p,
            vec![0x05, 0x0F, 0x01, 0x10, 0x15, 0x25, 1, 2, 3, 2, 3, cs[0], cs[1], 0, 0]
        );
    }

    #[test]
    fn matches_the_driver_example_for_sleep_1() {
        // From the driver analysis: sleep 1, mode 1, red, speed 6, debounce 3 steps.
        let cfg = PowerLightConfig {
            light_mode: 1,
            breath_speed: 6,
            brightness: 8,
            color: Rgb(0xFF, 0, 0),
            sleep_min: 1,
            debounce_ms: 6,
        };
        assert_eq!(
            encode_power_light(&cfg).unwrap(),
            vec![0x05, 0x0F, 0x01, 0x10, 0x03, 0x18, 0xFF, 0, 0, 0x02, 0x03, 0x01, 0x2F, 0, 0]
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
    fn default_is_valid_and_uses_driver_defaults() {
        let d = PowerLightConfig::default_profile();
        assert!(encode_power_light(&d).is_ok());
        assert_eq!((d.sleep_min, d.debounce_ms), (10, 6));
    }

    #[test]
    fn rejects_out_of_range_fields() {
        type Case = (&'static str, fn(&mut PowerLightConfig));
        let cases: [Case; 11] = [
            ("light_mode", |c| c.light_mode = 12),
            ("breath_speed", |c| c.breath_speed = 3),
            ("breath_speed", |c| c.breath_speed = 9),
            ("brightness", |c| c.brightness = 0),
            ("brightness", |c| c.brightness = 9),
            ("sleep_min", |c| c.sleep_min = 0),
            ("sleep_min", |c| c.sleep_min = 21),
            ("debounce_ms", |c| c.debounce_ms = 0),
            ("debounce_ms", |c| c.debounce_ms = 7),
            ("debounce_ms", |c| c.debounce_ms = 52),
            ("debounce_ms", |c| c.debounce_ms = 1),
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

    #[test]
    fn old_files_with_light_off_min_still_load() {
        let old = r#"light_mode = 1
breath_speed = 4
brightness = 8
color = [255, 0, 0]
light_off_min = 10
sleep_min = 10
debounce_ms = 6
"#;
        let cfg: PowerLightConfig = toml::from_str(old).unwrap();
        assert_eq!(cfg.sleep_min, 10);
    }
}
