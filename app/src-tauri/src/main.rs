//! Delux Mouse — menu-bar app for the Delux M900 PRO 8K.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod hotkeys;
mod tray;

use std::sync::{Arc, Mutex};

use delux_core::controller::Controller;
use delux_core::present::should_notify_low;
use delux_core::scripts::ScriptRunner;
use delux_core::{ApplyError, Connection, DeviceService, Status};
use delux_proto::BatteryState;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::ShortcutState;
use tauri_plugin_notification::NotificationExt;

pub struct Ctx {
    pub controller: Arc<Controller>,
    pub svc: DeviceService,
    pub runner: ScriptRunner,
    /// Last live battery percentage, for the low-battery crossing.
    last_pct: Mutex<Option<u8>>,
}

pub fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Switch profile from the menu bar; tell the user if the mouse could not take it.
/// Runs on its own thread: sending a profile takes seconds and must not
/// freeze the menu bar.
pub fn switch_profile(app: &AppHandle, name: &str) {
    let (app, name) = (app.clone(), name.to_string());
    std::thread::spawn(move || {
        let ctx = app.state::<Ctx>();
        let result = ctx.controller.set_active_profile(&name);
        tray::refresh_menu(&app);
        let _ = app.emit("store-changed", ());
        match result {
            Ok(_) => {}
            Err(ApplyError::Asleep) => notify(
                &app,
                "Perfil pendente",
                &format!("“{name}” será aplicado quando você mexer o mouse e clicar em Aplicar."),
            ),
            Err(e) => notify(&app, "Perfil não aplicado", &e.to_string()),
        }
        if let Some(w) = ctx.controller.warning() {
            notify(&app, "Delux Mouse", &w);
            ctx.controller.dismiss_warning();
        }
    });
}

pub fn sync_autostart(app: &AppHandle, enabled: bool) {
    // A debug build would register the build-tree binary; only real installs autostart.
    if cfg!(debug_assertions) {
        return;
    }
    let launcher = app.autolaunch();
    let _ = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
}

fn on_status(app: &AppHandle, status: &Status) {
    tray::set_status(app, status);
    let _ = app.emit("status", status);
    let Some(ctx) = app.try_state::<Ctx>() else {
        return;
    };
    if status.connection != Connection::Awake {
        return;
    }
    if let Some((state, pct)) = status.battery {
        let charging = matches!(state, BatteryState::Charging | BatteryState::Full);
        let threshold = ctx.controller.store().settings.low_battery_pct;
        let mut last = ctx.last_pct.lock().unwrap();
        if should_notify_low(*last, pct, threshold, charging) {
            notify(app, "Bateria do mouse baixa", &format!("{pct}% restantes."));
        }
        *last = Some(pct);
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    // A key can fire before setup has finished.
                    let Some(ctx) = app.try_state::<Ctx>() else {
                        return;
                    };
                    let binding = hotkeys::usage_of(shortcut)
                        .and_then(|u| ctx.controller.script_for_usage(u));
                    if let Some(binding) = binding {
                        ctx.runner.trigger(&binding);
                        let _ = app.emit("script-run", ());
                    }
                })
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let status_handle = handle.clone();
            let svc = DeviceService::spawn_default(Box::new(move |s| on_status(&status_handle, s)));
            let controller = Arc::new(Controller::load(
                delux_profile::default_path(),
                Arc::new(svc.clone()),
                Arc::new(hotkeys::TauriHotkeys(handle.clone())),
            ));
            let autostart = controller.store().settings.autostart;
            app.manage(Ctx {
                controller,
                svc,
                runner: ScriptRunner::default(),
                last_pct: Mutex::new(None),
            });
            tray::create(&handle)?;
            // Edits made while the mouse slept are sent as soon as it is awake.
            let retry = handle.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let ctx = retry.state::<Ctx>();
                let active = ctx.controller.store().active;
                let owes = !ctx.controller.pending_sections(&active).is_empty();
                if owes
                    && ctx.svc.status().connection == Connection::Awake
                    && ctx.controller.apply_pending().is_ok()
                {
                    let _ = retry.emit("store-changed", ());
                }
            });
            // The poll thread may have published its first status before the tray existed.
            let ctx = handle.state::<Ctx>();
            tray::set_status(&handle, &ctx.svc.status());
            if let Some(w) = ctx.controller.warning() {
                notify(&handle, "Delux Mouse", &w);
                ctx.controller.dismiss_warning();
            }
            sync_autostart(&handle, autostart);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the app alive in the menu bar.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::save_profile,
            commands::update_profile,
            commands::apply_profile,
            commands::set_active_profile,
            commands::create_profile,
            commands::delete_profile,
            commands::factory_reset,
            commands::update_settings,
            commands::bind_script,
            commands::unbind_script,
            commands::script_log,
            commands::dismiss_warning,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Delux Mouse")
        .run(|app, event| {
            // Covers every way out (menu, Cmd+Q, logout): no script outlives the app.
            if let tauri::RunEvent::Exit = event {
                if let Some(ctx) = app.try_state::<Ctx>() {
                    ctx.runner.kill_all();
                }
            }
        });
}
