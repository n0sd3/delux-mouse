//! F13..F24 global hotkeys: the keys script-bound mouse buttons emit.

use delux_core::controller::Hotkeys;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut};

const KEYS: [(u8, Code); 12] = [
    (0x68, Code::F13),
    (0x69, Code::F14),
    (0x6A, Code::F15),
    (0x6B, Code::F16),
    (0x6C, Code::F17),
    (0x6D, Code::F18),
    (0x6E, Code::F19),
    (0x6F, Code::F20),
    (0x70, Code::F21),
    (0x71, Code::F22),
    (0x72, Code::F23),
    (0x73, Code::F24),
];

fn shortcut(usage: u8) -> Option<Shortcut> {
    KEYS.iter()
        .find(|(u, _)| *u == usage)
        .map(|(_, code)| Shortcut::new(None, *code))
}

/// HID usage of the key a fired shortcut stands for.
pub fn usage_of(fired: &Shortcut) -> Option<u8> {
    KEYS.iter()
        .map(|(u, _)| *u)
        .find(|u| shortcut(*u).as_ref() == Some(fired))
}

pub struct TauriHotkeys(pub AppHandle);

impl Hotkeys for TauriHotkeys {
    fn register(&self, usage: u8) -> Result<(), String> {
        let sc = shortcut(usage).ok_or("tecla fora de F13..F24")?;
        let gs = self.0.global_shortcut();
        if gs.is_registered(sc) {
            return Ok(());
        }
        gs.register(sc)
            .map_err(|e| format!("atalho indisponível (em uso por outro app?): {e}"))
    }

    fn unregister(&self, usage: u8) {
        if let Some(sc) = shortcut(usage) {
            let _ = self.0.global_shortcut().unregister(sc);
        }
    }
}
