//! App logic behind the UI: profiles on disk, what was applied, script bindings.
//! No UI and no HID here; both are injected.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use delux_profile::{changed_sections, Profile, ScriptBinding, Section, Settings, Store};
use delux_proto::{ButtonAction, ButtonMap};

use crate::scripts::usage_for_slot;
use crate::{ApplyError, DeviceService};

/// What the controller needs from the device.
pub trait Mouse: Send + Sync {
    fn apply(&self, profile: &Profile, sections: &[Section]) -> Result<(), ApplyError>;
    fn factory_reset(&self) -> Result<(), ApplyError>;
}

impl Mouse for DeviceService {
    fn apply(&self, profile: &Profile, sections: &[Section]) -> Result<(), ApplyError> {
        DeviceService::apply(self, profile, sections)
    }
    fn factory_reset(&self) -> Result<(), ApplyError> {
        DeviceService::factory_reset(self)
    }
}

/// Global hotkey registration, by HID keyboard usage.
pub trait Hotkeys: Send + Sync {
    fn register(&self, usage: u8) -> Result<(), String>;
    fn unregister(&self, usage: u8);
}

pub struct Controller {
    path: PathBuf,
    mouse: Arc<dyn Mouse>,
    keys: Arc<dyn Hotkeys>,
    store: Mutex<Store>,
    /// Something the user must see; kept until dismissed.
    warning: Mutex<Option<String>>,
}

impl Controller {
    /// Load profiles from `path` and register the active profile's hotkeys.
    pub fn load(path: PathBuf, mouse: Arc<dyn Mouse>, keys: Arc<dyn Hotkeys>) -> Self {
        let loaded = delux_profile::load(&path);
        let mut store = loaded.store;
        // Older files allocated script keys per profile; move them to the
        // fixed per-slot keys. The button map changes, so it shows as pending.
        let mut migrated = false;
        for p in &mut store.profiles {
            for s in &mut p.scripts {
                if let Some(usage) = usage_for_slot(s.slot).filter(|u| *u != s.hotkey_usage) {
                    s.hotkey_usage = usage;
                    p.buttons.0[s.slot] = ButtonAction::Key {
                        modifiers: 0,
                        usage,
                    };
                    migrated = true;
                }
            }
        }
        if migrated {
            let _ = delux_profile::save(&path, &store);
        }
        let c = Controller {
            path,
            mouse,
            keys,
            store: Mutex::new(store),
            warning: Mutex::new(loaded.warning),
        };
        let failed = c.register_active(&c.store.lock().unwrap());
        c.warn_dead_hotkeys(failed);
        c
    }

    /// Register the active profile's hotkeys; returns the labels that failed.
    fn register_active(&self, store: &Store) -> Vec<String> {
        store
            .active_profile()
            .scripts
            .iter()
            .filter(|s| self.keys.register(s.hotkey_usage).is_err())
            .map(|s| {
                if s.label.is_empty() {
                    s.command.clone()
                } else {
                    s.label.clone()
                }
            })
            .collect()
    }

    fn warn_dead_hotkeys(&self, failed: Vec<String>) {
        if !failed.is_empty() {
            *self.warning.lock().unwrap() = Some(format!(
                "Atalho indisponível (em uso por outro app?) para o script: {}. O botão não vai disparar esse script.",
                failed.join(", ")
            ));
        }
    }

    pub fn store(&self) -> Store {
        self.store.lock().unwrap().clone()
    }

    /// Current warning, if any. Stays until `dismiss_warning`.
    pub fn warning(&self) -> Option<String> {
        self.warning.lock().unwrap().clone()
    }

    pub fn dismiss_warning(&self) {
        *self.warning.lock().unwrap() = None;
    }

    fn persist(&self, store: &Store) -> Result<(), String> {
        delux_profile::save(&self.path, store)
            .map_err(|e| format!("não foi possível gravar perfis: {e}"))
    }

    fn pending_in(store: &Store, profile: &Profile) -> Vec<Section> {
        match &store.last_applied {
            None => Section::ALL.to_vec(),
            Some(last) => {
                let changed = changed_sections(last, profile);
                Section::ALL
                    .into_iter()
                    .filter(|s| changed.contains(s) || store.unsent.contains(s))
                    .collect()
            }
        }
    }

    /// Sections of `name` the mouse does not have yet.
    pub fn pending_sections(&self, name: &str) -> Vec<Section> {
        let store = self.store.lock().unwrap();
        store
            .profiles
            .iter()
            .find(|p| p.name == name)
            .map(|p| Self::pending_in(&store, p))
            .unwrap_or_default()
    }

    /// Send whatever the mouse is missing from `name`. Returns the sections sent.
    pub fn apply_profile(&self, name: &str) -> Result<Vec<Section>, ApplyError> {
        let mut store = self.store.lock().unwrap();
        let profile = store
            .profiles
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .ok_or_else(|| ApplyError::Invalid(vec![format!("perfil {name:?} não existe")]))?;
        let sections = Self::pending_in(&store, &profile);
        if sections.is_empty() {
            return Ok(sections);
        }
        let result = self.mouse.apply(&profile, &sections);
        match &result {
            Ok(()) => {
                store.last_applied = Some(profile);
                store.unsent.clear();
            }
            Err(ApplyError::Io { pending, .. }) => {
                store.last_applied = Some(profile);
                store.unsent = pending.clone();
            }
            Err(_) => return result.map(|()| sections),
        }
        let saved = self.persist(&store);
        result?;
        saved.map_err(|e| ApplyError::Invalid(vec![e]))?;
        Ok(sections)
    }

    /// Validate and store an edited profile (matched by name). Sends nothing.
    pub fn save_profile(&self, profile: Profile) -> Result<(), Vec<String>> {
        profile.validate()?;
        let mut store = self.store.lock().unwrap();
        let slot = store
            .profiles
            .iter_mut()
            .find(|p| p.name == profile.name)
            .ok_or_else(|| vec![format!("perfil {:?} não existe", profile.name)])?;
        // Script bindings change only through bind/unbind, which keep hotkeys in step.
        let scripts = slot.scripts.clone();
        *slot = Profile { scripts, ..profile };
        slot.validate()?;
        self.persist(&store).map_err(|e| vec![e])
    }

    pub fn create_profile(&self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("nome vazio".into());
        }
        let mut store = self.store.lock().unwrap();
        if store.profiles.iter().any(|p| p.name == name) {
            return Err(format!("já existe um perfil {name:?}"));
        }
        store.profiles.push(Profile::default_named(name));
        self.persist(&store)
    }

    pub fn delete_profile(&self, name: &str) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        if store.profiles.len() == 1 {
            return Err("não é possível apagar o único perfil".into());
        }
        if store.active == name {
            return Err("não é possível apagar o perfil ativo".into());
        }
        let before = store.profiles.len();
        store.profiles.retain(|p| p.name != name);
        if store.profiles.len() == before {
            return Err(format!("perfil {name:?} não existe"));
        }
        self.persist(&store)
    }

    /// Make `name` the active profile and send it. The choice is kept even if
    /// the mouse cannot take it right now; the error says why it is pending.
    pub fn set_active_profile(&self, name: &str) -> Result<Vec<Section>, ApplyError> {
        {
            let mut store = self.store.lock().unwrap();
            if !store.profiles.iter().any(|p| p.name == name) {
                return Err(ApplyError::Invalid(vec![format!(
                    "perfil {name:?} não existe"
                )]));
            }
            if store.active != name {
                for s in &store.active_profile().scripts {
                    self.keys.unregister(s.hotkey_usage);
                }
                store.active = name.to_string();
                let failed = self.register_active(&store);
                self.warn_dead_hotkeys(failed);
                self.persist(&store)
                    .map_err(|e| ApplyError::Invalid(vec![e]))?;
            }
        }
        self.apply_profile(name)
    }

    /// Make button `slot` of `profile` run `command`. The hotkey is registered
    /// before anything is changed, so a conflict leaves everything as it was.
    pub fn bind_script(
        &self,
        profile: &str,
        slot: usize,
        command: &str,
        label: &str,
    ) -> Result<(), String> {
        let usage = usage_for_slot(slot).ok_or("este botão não pode ser usado para scripts")?;
        if command.trim().is_empty() {
            return Err("comando vazio".into());
        }
        let is_active = {
            let mut store = self.store.lock().unwrap();
            let is_active = store.active == profile;
            let p = store
                .profiles
                .iter_mut()
                .find(|p| p.name == profile)
                .ok_or_else(|| format!("perfil {profile:?} não existe"))?;
            if let Some(existing) = p.scripts.iter_mut().find(|s| s.slot == slot) {
                existing.command = command.to_string();
                existing.label = label.to_string();
                return self.persist(&store);
            }
            if is_active {
                self.keys.register(usage)?;
            }
            p.buttons.0[slot] = ButtonAction::Key {
                modifiers: 0,
                usage,
            };
            p.scripts.push(ScriptBinding {
                slot,
                hotkey_usage: usage,
                command: command.to_string(),
                label: label.to_string(),
            });
            self.persist(&store)?;
            is_active
        };
        if is_active {
            // If the mouse is asleep the button change stays pending; the UI shows that.
            let _ = self.apply_profile(profile);
        }
        Ok(())
    }

    /// Remove the script from `slot` and give the button its factory function back.
    pub fn unbind_script(&self, profile: &str, slot: usize) -> Result<(), String> {
        let is_active = {
            let mut store = self.store.lock().unwrap();
            let is_active = store.active == profile;
            let p = store
                .profiles
                .iter_mut()
                .find(|p| p.name == profile)
                .ok_or_else(|| format!("perfil {profile:?} não existe"))?;
            let idx = p
                .scripts
                .iter()
                .position(|s| s.slot == slot)
                .ok_or("este botão não tem script")?;
            let removed = p.scripts.remove(idx);
            p.buttons.0[slot] = ButtonMap::factory().0[slot];
            if is_active {
                self.keys.unregister(removed.hotkey_usage);
            }
            self.persist(&store)?;
            is_active
        };
        if is_active {
            let _ = self.apply_profile(profile);
        }
        Ok(())
    }

    /// The active profile's script for a hotkey, if any.
    pub fn script_for_usage(&self, usage: u8) -> Option<ScriptBinding> {
        let store = self.store.lock().unwrap();
        store
            .active_profile()
            .scripts
            .iter()
            .find(|s| s.hotkey_usage == usage)
            .cloned()
    }

    pub fn update_settings(&self, settings: Settings) -> Result<(), String> {
        if !(1..=100).contains(&settings.low_battery_pct) {
            return Err("limiar de bateria deve ficar entre 1 e 100".into());
        }
        let mut store = self.store.lock().unwrap();
        store.settings = settings;
        self.persist(&store)
    }

    /// Restore factory settings on the mouse. Afterwards the mouse holds the
    /// factory profile, so the active profile shows as pending where it differs.
    pub fn factory_reset(&self) -> Result<(), ApplyError> {
        let result = self.mouse.factory_reset();
        let unsent = match &result {
            Ok(()) => Vec::new(),
            // Stopped partway: part of the mouse is factory, the rest unknown.
            Err(ApplyError::Io { pending, .. }) => pending.clone(),
            Err(_) => return result,
        };
        let mut store = self.store.lock().unwrap();
        store.last_applied = Some(Profile::factory("factory"));
        store.unsent = unsent;
        let saved = self.persist(&store);
        result?;
        saved.map_err(|e| ApplyError::Invalid(vec![e]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use delux_proto::PollingRate;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeMouse {
        applied: Mutex<Vec<Vec<Section>>>,
        /// Next apply fails after sending this many sections.
        fail_after: Mutex<Option<usize>>,
        asleep: Mutex<bool>,
        reset_error: Mutex<Option<ApplyError>>,
    }

    impl Mouse for FakeMouse {
        fn apply(&self, _p: &Profile, sections: &[Section]) -> Result<(), ApplyError> {
            if *self.asleep.lock().unwrap() {
                return Err(ApplyError::Asleep);
            }
            if let Some(n) = self.fail_after.lock().unwrap().take() {
                self.applied.lock().unwrap().push(sections[..n].to_vec());
                return Err(ApplyError::Io {
                    sent: sections[..n].to_vec(),
                    pending: sections[n..].to_vec(),
                    message: "unplugged".into(),
                });
            }
            self.applied.lock().unwrap().push(sections.to_vec());
            Ok(())
        }
        fn factory_reset(&self) -> Result<(), ApplyError> {
            match self.reset_error.lock().unwrap().take() {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
    }

    #[derive(Default)]
    struct FakeHotkeys {
        registered: Mutex<Vec<u8>>,
        fail: Mutex<bool>,
    }

    impl Hotkeys for FakeHotkeys {
        fn register(&self, usage: u8) -> Result<(), String> {
            if *self.fail.lock().unwrap() {
                return Err("atalho em uso por outro app".into());
            }
            self.registered.lock().unwrap().push(usage);
            Ok(())
        }
        fn unregister(&self, usage: u8) {
            self.registered.lock().unwrap().retain(|u| *u != usage);
        }
    }

    struct Fx {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
        mouse: Arc<FakeMouse>,
        keys: Arc<FakeHotkeys>,
        c: Controller,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        let mouse = Arc::new(FakeMouse::default());
        let keys = Arc::new(FakeHotkeys::default());
        let c = Controller::load(path.clone(), mouse.clone(), keys.clone());
        Fx {
            _dir: dir,
            path,
            mouse,
            keys,
            c,
        }
    }

    fn sent(f: &Fx) -> Vec<Vec<Section>> {
        f.mouse.applied.lock().unwrap().clone()
    }

    #[test]
    fn first_apply_sends_everything_and_is_remembered() {
        let f = fx();
        assert!(!f.c.store().applied_once());
        assert_eq!(f.c.apply_profile("Padrão"), Ok(Section::ALL.to_vec()));
        assert_eq!(sent(&f), vec![Section::ALL.to_vec()]);
        assert!(f.c.store().applied_once());
        assert!(
            delux_profile::load(&f.path).store.applied_once(),
            "persisted"
        );
    }

    #[test]
    fn second_apply_without_change_sends_nothing() {
        let f = fx();
        f.c.apply_profile("Padrão").unwrap();
        assert_eq!(f.c.apply_profile("Padrão"), Ok(vec![]));
        assert_eq!(sent(&f).len(), 1);
    }

    #[test]
    fn apply_after_edit_sends_only_changed_sections() {
        let f = fx();
        f.c.apply_profile("Padrão").unwrap();
        let mut p = f.c.store().active_profile().clone();
        p.polling = PollingRate::Hz500;
        f.c.save_profile(p).unwrap();
        assert_eq!(f.c.apply_profile("Padrão"), Ok(vec![Section::Polling]));
    }

    #[test]
    fn partial_failure_retries_only_what_is_pending() {
        let f = fx();
        *f.mouse.fail_after.lock().unwrap() = Some(1);
        assert!(matches!(
            f.c.apply_profile("Padrão"),
            Err(ApplyError::Io { .. })
        ));
        assert_eq!(
            f.c.apply_profile("Padrão"),
            Ok(vec![
                Section::Polling,
                Section::PowerLight,
                Section::Buttons
            ])
        );
    }

    #[test]
    fn asleep_keeps_everything_pending() {
        let f = fx();
        *f.mouse.asleep.lock().unwrap() = true;
        assert_eq!(f.c.apply_profile("Padrão"), Err(ApplyError::Asleep));
        assert!(!f.c.store().applied_once());
        assert_eq!(f.c.pending_sections("Padrão"), Section::ALL.to_vec());
    }

    #[test]
    fn save_profile_rejects_invalid_and_keeps_disk() {
        let f = fx();
        let mut p = f.c.store().active_profile().clone();
        p.sensor.stages[0].dpi = 123;
        assert!(f.c.save_profile(p).is_err());
        assert_eq!(f.c.store().active_profile().sensor.stages[0].dpi, 400);
    }

    #[test]
    fn create_switch_delete_profiles() {
        let f = fx();
        f.c.create_profile("Jogo").unwrap();
        assert!(f.c.create_profile("Jogo").is_err(), "duplicate");
        assert!(f.c.create_profile(" ").is_err(), "blank");
        assert!(f.c.delete_profile("Padrão").is_err(), "active");
        f.c.apply_profile("Padrão").unwrap();
        f.c.set_active_profile("Jogo").unwrap();
        assert_eq!(f.c.store().active, "Jogo");
        f.c.delete_profile("Padrão").unwrap();
        assert!(f.c.delete_profile("Jogo").is_err(), "last one");
    }

    #[test]
    fn switching_profile_while_asleep_still_switches_and_reports() {
        let f = fx();
        f.c.create_profile("Jogo").unwrap();
        *f.mouse.asleep.lock().unwrap() = true;
        assert_eq!(f.c.set_active_profile("Jogo"), Err(ApplyError::Asleep));
        assert_eq!(
            f.c.store().active,
            "Jogo",
            "choice is kept; apply stays pending"
        );
    }

    #[test]
    fn bind_script_maps_button_registers_hotkey_and_applies_buttons() {
        let f = fx();
        f.c.apply_profile("Padrão").unwrap();
        f.c.bind_script("Padrão", 7, "open -a Calculator", "Calc")
            .unwrap();
        let p = f.c.store().active_profile().clone();
        assert_eq!(
            p.buttons.0[7],
            ButtonAction::Key {
                modifiers: 0,
                usage: 0x6C
            }
        );
        assert_eq!(p.scripts.len(), 1);
        assert_eq!(*f.keys.registered.lock().unwrap(), vec![0x6C]);
        assert_eq!(sent(&f).last().unwrap(), &vec![Section::Buttons]);
        assert_eq!(
            f.c.script_for_usage(0x6C).unwrap().command,
            "open -a Calculator"
        );
    }

    #[test]
    fn script_key_is_fixed_per_slot_across_profiles() {
        // A button must emit the same key in every profile: if a profile
        // switch is still pending on the mouse, the old map then triggers
        // either the right script or nothing — never another button's script.
        let f = fx();
        f.c.bind_script("Padrão", 7, "a", "A").unwrap();
        f.c.bind_script("Padrão", 3, "b", "B").unwrap();
        f.c.create_profile("Jogo").unwrap();
        f.c.bind_script("Jogo", 6, "c", "C").unwrap();
        let store = f.c.store();
        let usage = |profile: &str, slot: usize| {
            let p = store.profiles.iter().find(|p| p.name == profile).unwrap();
            p.scripts
                .iter()
                .find(|s| s.slot == slot)
                .unwrap()
                .hotkey_usage
        };
        assert_eq!(usage("Padrão", 3), 0x68);
        assert_eq!(usage("Padrão", 7), 0x6C);
        assert_eq!(usage("Jogo", 6), 0x6B);
    }

    #[test]
    fn factory_reset_partial_failure_marks_the_rest_pending() {
        let f = fx();
        f.c.apply_profile("Padrão").unwrap();
        let pending = vec![Section::Polling, Section::PowerLight, Section::Buttons];
        *f.mouse.reset_error.lock().unwrap() = Some(ApplyError::Io {
            sent: vec![Section::Sensor],
            pending: pending.clone(),
            message: "unplugged".into(),
        });
        assert!(f.c.factory_reset().is_err());
        let store = f.c.store();
        assert_eq!(
            store.last_applied.as_ref().map(|p| p.sensor.clone()),
            Some(Profile::factory("x").sensor)
        );
        assert_eq!(store.unsent, pending);
        assert!(f.c.pending_sections("Padrão").contains(&Section::Polling));
    }

    #[test]
    fn hotkey_failure_on_profile_switch_is_reported_and_stays_until_dismissed() {
        let f = fx();
        f.c.create_profile("Jogo").unwrap();
        f.c.bind_script("Jogo", 7, "x", "Calc").unwrap(); // not active: nothing registered yet
        assert_eq!(f.c.warning(), None);
        *f.keys.fail.lock().unwrap() = true;
        let _ = f.c.set_active_profile("Jogo");
        let w = f.c.warning().expect("warning about the dead hotkey");
        assert!(w.contains("Calc"), "{w}");
        assert_eq!(f.c.warning(), Some(w), "still there on the next read");
        f.c.dismiss_warning();
        assert_eq!(f.c.warning(), None);
    }

    #[test]
    fn rebinding_same_slot_replaces_command_and_keeps_key() {
        let f = fx();
        f.c.bind_script("Padrão", 7, "a", "A").unwrap();
        f.c.bind_script("Padrão", 7, "b", "B").unwrap();
        let p = f.c.store().active_profile().clone();
        assert_eq!(p.scripts.len(), 1);
        assert_eq!(p.scripts[0].command, "b");
        assert_eq!(*f.keys.registered.lock().unwrap(), vec![0x6C]);
    }

    #[test]
    fn hotkey_conflict_leaves_profile_untouched() {
        let f = fx();
        f.c.apply_profile("Padrão").unwrap();
        let before = std::fs::read_to_string(&f.path).unwrap();
        let sends = sent(&f).len();
        *f.keys.fail.lock().unwrap() = true;
        assert!(f.c.bind_script("Padrão", 7, "x", "X").is_err());
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), before);
        assert_eq!(sent(&f).len(), sends);
        assert_eq!(
            f.c.store().active_profile().buttons.0[7],
            ButtonAction::Back
        );
    }

    #[test]
    fn protected_and_bad_slots_rejected() {
        let f = fx();
        for slot in [0usize, 1, 2, 15, 16, 17, 18] {
            assert!(
                f.c.bind_script("Padrão", slot, "x", "X").is_err(),
                "slot {slot}"
            );
        }
        assert!(
            f.c.bind_script("Padrão", 7, "  ", "X").is_err(),
            "empty command"
        );
    }

    #[test]
    fn unbind_restores_factory_action_and_unregisters() {
        let f = fx();
        f.c.bind_script("Padrão", 7, "x", "X").unwrap();
        f.c.unbind_script("Padrão", 7).unwrap();
        let p = f.c.store().active_profile().clone();
        assert_eq!(p.buttons.0[7], ButtonAction::Back);
        assert!(p.scripts.is_empty());
        assert!(f.keys.registered.lock().unwrap().is_empty());
    }

    #[test]
    fn switching_profile_swaps_registered_hotkeys() {
        let f = fx();
        f.c.bind_script("Padrão", 7, "x", "X").unwrap();
        f.c.create_profile("Jogo").unwrap();
        f.c.set_active_profile("Jogo").unwrap();
        assert!(f.keys.registered.lock().unwrap().is_empty());
        f.c.set_active_profile("Padrão").unwrap();
        assert_eq!(*f.keys.registered.lock().unwrap(), vec![0x6C]);
    }

    #[test]
    fn load_moves_old_per_profile_keys_to_the_fixed_slot_key() {
        let f = fx();
        f.c.apply_profile("Padrão").unwrap();
        let mut store = f.c.store();
        store.profiles[0].buttons.0[7] = ButtonAction::Key {
            modifiers: 0,
            usage: 0x68,
        };
        store.profiles[0].scripts.push(ScriptBinding {
            slot: 7,
            hotkey_usage: 0x68,
            command: "x".into(),
            label: "X".into(),
        });
        store.last_applied = Some(store.profiles[0].clone());
        delux_profile::save(&f.path, &store).unwrap();
        let keys = Arc::new(FakeHotkeys::default());
        let c2 = Controller::load(f.path.clone(), f.mouse.clone(), keys.clone());
        assert_eq!(c2.store().active_profile().scripts[0].hotkey_usage, 0x6C);
        assert_eq!(*keys.registered.lock().unwrap(), vec![0x6C]);
        assert_eq!(c2.pending_sections("Padrão"), vec![Section::Buttons]);
    }

    #[test]
    fn load_registers_active_profiles_hotkeys() {
        let f = fx();
        f.c.bind_script("Padrão", 7, "x", "X").unwrap();
        let keys = Arc::new(FakeHotkeys::default());
        let _c2 = Controller::load(f.path.clone(), f.mouse.clone(), keys.clone());
        assert_eq!(*keys.registered.lock().unwrap(), vec![0x6C]);
    }
}
