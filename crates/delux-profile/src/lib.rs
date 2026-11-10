//! Local profiles: the only record of what was configured, since the mouse
//! cannot be read back.

pub mod store;

use delux_proto::{
    encode_buttons, encode_power_light, encode_sensor, ButtonAction, ButtonMap, PollingRate,
    PowerLightConfig, SensorConfig, SLOT_COUNT,
};
use serde::{Deserialize, Serialize};

pub use store::{default_path, load, save, LoadOutcome, Settings, Store};

/// A mouse button that runs a command on the Mac. The slot is mapped in the
/// firmware to an otherwise unused key, which the app catches as a hotkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptBinding {
    pub slot: usize,
    /// HID usage of the key the slot emits (F13..F24).
    pub hotkey_usage: u8,
    pub command: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub polling: PollingRate,
    pub sensor: SensorConfig,
    pub power_light: PowerLightConfig,
    pub buttons: ButtonMap,
    #[serde(default)]
    pub scripts: Vec<ScriptBinding>,
}

impl Profile {
    pub fn default_named(name: &str) -> Self {
        Profile {
            name: name.to_string(),
            polling: PollingRate::Hz1000,
            sensor: SensorConfig::default_profile(),
            power_light: PowerLightConfig::default_profile(),
            buttons: ButtonMap::factory(),
            scripts: Vec::new(),
        }
    }

    /// The vendor driver's defaults: what "restore factory settings" sends.
    pub fn factory(name: &str) -> Self {
        let mut p = Self::default_named(name);
        for (stage, dpi) in p.sensor.stages.iter_mut().zip([400, 800, 1200, 1600, 3200, 5000]) {
            stage.dpi = dpi;
        }
        p.sensor.current_stage = 2;
        p.sensor.ripple = true;
        p.sensor.motion_sync = true;
        p
    }

    /// Every problem found, in Portuguese, ready to show.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.name.trim().is_empty() {
            errs.push("nome do perfil vazio".to_string());
        }
        if let Err(e) = encode_sensor(&self.sensor) {
            errs.push(format!("sensor: {e}"));
        }
        if let Err(e) = encode_power_light(&self.power_light) {
            errs.push(format!("luz/energia: {e}"));
        }
        if let Err(e) = encode_buttons(&self.buttons) {
            errs.push(format!("botões: {e}"));
        }
        let mut seen = Vec::new();
        for s in &self.scripts {
            if s.slot == 0 || s.slot >= SLOT_COUNT {
                errs.push(format!("script em slot inválido: {}", s.slot));
                continue;
            }
            if seen.contains(&s.slot) {
                errs.push(format!("slot {} tem mais de um script", s.slot));
            }
            seen.push(s.slot);
            let expected = ButtonAction::Key { modifiers: 0, usage: s.hotkey_usage };
            if self.buttons.0[s.slot] != expected {
                errs.push(format!("slot {} não emite a tecla do seu script", s.slot));
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }
}

/// One firmware command's worth of settings, in the order they are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Section {
    Sensor,
    Polling,
    PowerLight,
    Buttons,
}

impl Section {
    pub const ALL: [Section; 4] =
        [Section::Sensor, Section::Polling, Section::PowerLight, Section::Buttons];
}

/// Sections whose firmware packet differs between two profiles.
pub fn changed_sections(old: &Profile, new: &Profile) -> Vec<Section> {
    Section::ALL
        .into_iter()
        .filter(|s| match s {
            Section::Sensor => old.sensor != new.sensor,
            Section::Polling => old.polling != new.polling,
            Section::PowerLight => old.power_light != new.power_light,
            Section::Buttons => old.buttons != new.buttons,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use delux_proto::{ButtonAction, PollingRate};

    fn with_script(slot: usize) -> Profile {
        let mut p = Profile::default_named("t");
        if slot < p.buttons.0.len() {
            p.buttons.0[slot] = ButtonAction::Key { modifiers: 0, usage: 0x68 };
        }
        p.scripts.push(ScriptBinding {
            slot,
            hotkey_usage: 0x68,
            command: "true".into(),
            label: "x".into(),
        });
        p
    }

    #[test]
    fn default_profile_is_valid() {
        assert_eq!(Profile::default_named("Padrão").validate(), Ok(()));
    }

    #[test]
    fn factory_profile_matches_vendor_defaults() {
        let p = Profile::factory("Fábrica");
        assert_eq!(p.validate(), Ok(()));
        let dpis: Vec<u16> = p.sensor.stages.iter().filter(|s| s.enabled).map(|s| s.dpi).collect();
        assert_eq!(dpis, vec![400, 800, 1200, 1600, 3200, 5000]);
        assert_eq!(p.sensor.current_stage, 2);
        assert!(p.sensor.ripple && p.sensor.motion_sync && !p.sensor.angle_snap);
        assert_eq!(p.buttons, delux_proto::ButtonMap::factory());
        assert!(p.scripts.is_empty());
    }

    #[test]
    fn empty_name_is_invalid() {
        assert!(Profile::default_named("  ").validate().is_err());
    }

    #[test]
    fn bad_dpi_is_reported() {
        let mut p = Profile::default_named("t");
        p.sensor.stages[0].dpi = 123;
        let errs = p.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("dpi")), "{errs:?}");
    }

    #[test]
    fn script_on_valid_slot_is_ok() {
        assert_eq!(with_script(7).validate(), Ok(()));
    }

    #[test]
    fn script_on_slot_0_invalid() {
        assert!(with_script(0).validate().is_err());
    }

    #[test]
    fn script_on_slot_out_of_range_invalid() {
        assert!(with_script(18).validate().is_err());
    }

    #[test]
    fn script_slot_must_carry_its_hotkey() {
        let mut p = with_script(7);
        p.buttons.0[7] = ButtonAction::Back;
        assert!(p.validate().is_err());
    }

    #[test]
    fn duplicate_script_slot_invalid() {
        let mut p = with_script(7);
        p.scripts.push(p.scripts[0].clone());
        assert!(p.validate().is_err());
    }

    #[test]
    fn changed_sections_only_polling() {
        let a = Profile::default_named("t");
        let mut b = a.clone();
        b.polling = PollingRate::Hz500;
        assert_eq!(changed_sections(&a, &b), vec![Section::Polling]);
    }

    #[test]
    fn changed_sections_sensor_and_buttons_in_send_order() {
        let a = Profile::default_named("t");
        let mut b = a.clone();
        b.buttons.0[7] = ButtonAction::Disabled;
        b.sensor.stages[0].dpi = 800;
        assert_eq!(changed_sections(&a, &b), vec![Section::Sensor, Section::Buttons]);
    }

    #[test]
    fn script_command_change_touches_no_section() {
        let a = with_script(7);
        let mut b = a.clone();
        b.scripts[0].command = "false".into();
        assert_eq!(changed_sections(&a, &b), vec![]);
    }
}
