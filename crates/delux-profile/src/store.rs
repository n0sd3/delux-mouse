//! TOML persistence.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Profile;

const DEFAULT_PROFILE_NAME: &str = "Padrão";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub low_battery_pct: u8,
    pub autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            low_battery_pct: 20,
            autostart: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    pub active: String,
    #[serde(default)]
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    /// What was last sent to the mouse. `None` until the first Apply: until
    /// then nothing shown in the UI reflects the mouse.
    #[serde(default)]
    pub last_applied: Option<Profile>,
    /// Sections of `last_applied` that did not actually reach the mouse
    /// (an Apply that stopped partway).
    #[serde(default)]
    pub unsent: Vec<crate::Section>,
}

impl Default for Store {
    fn default() -> Self {
        Store {
            active: DEFAULT_PROFILE_NAME.to_string(),
            settings: Settings::default(),
            profiles: vec![Profile::default_named(DEFAULT_PROFILE_NAME)],
            last_applied: None,
            unsent: Vec::new(),
        }
    }
}

impl Store {
    pub fn applied_once(&self) -> bool {
        self.last_applied.is_some()
    }

    pub fn active_profile(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|p| p.name == self.active)
            .expect("store invariant: active names an existing profile")
    }

    fn validate(&self) -> Result<(), String> {
        if self.profiles.is_empty() {
            return Err("nenhum perfil".into());
        }
        if !self.profiles.iter().any(|p| p.name == self.active) {
            return Err(format!("perfil ativo {:?} não existe", self.active));
        }
        for (i, p) in self.profiles.iter().enumerate() {
            if self.profiles[..i].iter().any(|q| q.name == p.name) {
                return Err(format!("perfil {:?} duplicado", p.name));
            }
            p.validate()
                .map_err(|e| format!("perfil {:?}: {}", p.name, e.join("; ")))?;
        }
        if !(1..=100).contains(&self.settings.low_battery_pct) {
            return Err("limiar de bateria fora de 1..100".into());
        }
        Ok(())
    }
}

pub struct LoadOutcome {
    pub store: Store,
    /// Set when the file on disk was unusable and defaults were loaded instead.
    pub warning: Option<String>,
}

/// Load the store. Never fails: an unreadable or invalid file is moved aside
/// to `<name>.bad` and defaults are returned with a warning.
pub fn load(path: &Path) -> LoadOutcome {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return LoadOutcome {
                store: Store::default(),
                warning: None,
            }
        }
        Err(e) => {
            return LoadOutcome {
                store: Store::default(),
                warning: Some(format!("não foi possível ler {}: {e}", path.display())),
            }
        }
    };
    let parsed = toml::from_str::<Store>(&text)
        .map_err(|e| e.message().to_string())
        .and_then(|s| s.validate().map(|()| s));
    match parsed {
        Ok(store) => LoadOutcome {
            store,
            warning: None,
        },
        Err(reason) => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let bad = PathBuf::from(bad);
            let kept = std::fs::rename(path, &bad).is_ok();
            let warning = if kept {
                format!(
                    "Arquivo de perfis inválido ({reason}). Perfis padrão carregados; o arquivo antigo foi guardado em {}.",
                    bad.display()
                )
            } else {
                format!("Arquivo de perfis inválido ({reason}). Perfis padrão carregados.")
            };
            LoadOutcome {
                store: Store::default(),
                warning: Some(warning),
            }
        }
    }
}

/// Atomic save: write a sibling temp file, then rename over the target.
pub fn save(path: &Path, store: &Store) -> std::io::Result<()> {
    let text = toml::to_string_pretty(store).map_err(std::io::Error::other)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// `~/Library/Application Support/delux-mouse/profiles.toml`
pub fn default_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Library/Application Support/delux-mouse/profiles.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Profile;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/profiles.toml");
        let mut store = Store::default();
        store.profiles.push(Profile::default_named("Jogo"));
        store.last_applied = Some(Profile::default_named("Padrão"));
        save(&path, &store).unwrap();
        let out = load(&path);
        assert_eq!(out.store, store);
        assert_eq!(out.warning, None);
    }

    #[test]
    fn missing_file_gives_default() {
        let dir = tempfile::tempdir().unwrap();
        let out = load(&dir.path().join("nope.toml"));
        assert_eq!(out.store.profiles.len(), 1);
        assert_eq!(out.store.profiles[0].name, "Padrão");
        assert_eq!(out.store.active, "Padrão");
        assert!(out.store.last_applied.is_none());
        assert_eq!(out.warning, None);
        assert_eq!(out.store.settings.low_battery_pct, 20);
        assert!(out.store.settings.autostart);
    }

    #[test]
    fn corrupt_file_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        std::fs::write(&path, "{{{").unwrap();
        let out = load(&path);
        assert_eq!(out.store, Store::default());
        assert!(out.warning.is_some());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("profiles.toml.bad")).unwrap(),
            "{{{"
        );
    }

    fn assert_rejected(edit: impl Fn(&str) -> String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        save(&path, &Store::default()).unwrap();
        let edited = edit(&std::fs::read_to_string(&path).unwrap());
        std::fs::write(&path, &edited).unwrap();
        let out = load(&path);
        assert_eq!(out.store, Store::default());
        assert!(out.warning.is_some());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("profiles.toml.bad")).unwrap(),
            edited
        );
    }

    #[test]
    fn out_of_range_dpi_is_rejected() {
        assert_rejected(|s| {
            assert!(s.contains("dpi = 400"));
            s.replacen("dpi = 400", "dpi = 123", 1)
        });
    }

    #[test]
    fn nine_stages_is_rejected() {
        assert_rejected(|s| {
            // duplicate the first stage table → 9 stages
            let start = s.find("[[profiles.sensor.stages]]").unwrap();
            let next = s[start + 1..].find("[[profiles.sensor.stages]]").unwrap() + start + 1;
            let first = s[start..next].to_string();
            format!("{}{}{}", &s[..start], first, &s[start..])
        });
    }

    #[test]
    fn nineteen_buttons_is_rejected() {
        assert_rejected(|s| {
            assert!(s.contains("\"ScrollDown\""));
            s.replacen("\"ScrollDown\"", "\"ScrollDown\", \"Disabled\"", 1)
        });
    }

    #[test]
    fn active_must_name_an_existing_profile() {
        assert_rejected(|s| s.replacen("active = \"Padrão\"", "active = \"Sumiu\"", 1));
    }
}
