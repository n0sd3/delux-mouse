//! Menu-bar item: battery title and profile menu.

use delux_core::present::tray_title;
use delux_core::Status;
use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::{show_window, Ctx};

const TRAY_ID: &str = "main";
const PROFILE_PREFIX: &str = "profile:";

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let store = app.state::<Ctx>().controller.store();
    let mut menu = MenuBuilder::new(app);
    for p in &store.profiles {
        let item = CheckMenuItemBuilder::with_id(format!("{PROFILE_PREFIX}{}", p.name), &p.name)
            .checked(p.name == store.active)
            .build(app)?;
        menu = menu.item(&item);
    }
    let open = MenuItemBuilder::with_id("open", "Abrir ajustes…").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Sair").build(app)?;
    menu.separator().item(&open).separator().item(&quit).build()
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .title("🖱 …")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if let Some(name) = id.strip_prefix(PROFILE_PREFIX) {
                crate::switch_profile(app, name);
            } else if id == "open" {
                show_window(app);
            } else if id == "quit" {
                app.state::<Ctx>().runner.kill_all();
                app.exit(0);
            }
        })
        .build(app)?;
    Ok(())
}

/// Rebuild the profile list (after create/delete/switch).
pub fn refresh_menu(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), build_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

pub fn set_status(app: &AppHandle, status: &Status) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_title(Some(tray_title(status)));
    }
}
