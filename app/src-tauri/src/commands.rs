//! Commands the window calls. Thin wrappers over `delux_core::controller`.

use delux_core::scripts::RunRecord;
use delux_core::{ApplyError, Status};
use delux_profile::{Profile, Section, Settings, Store};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{tray, Ctx};

/// Physical button behind each firmware slot (verified on hardware 2026-11-10).
/// Empty name = no physical button on this mouse.
const SLOT_NAMES: [&str; 18] = [
    "Clique esquerdo",
    "Clique direito",
    "Clique do meio",
    "Botão DPI",
    "",
    "",
    "Lateral da frente",
    "Lateral de trás",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "Scroll para cima",
    "Scroll para baixo",
];

/// Settings sent as the vendor driver sends them, but whose effect could
/// not be confirmed by measurement on hardware.
const UNVERIFIED: [&str; 3] = ["ripple", "motion_sync", "debounce_ms"];

#[derive(Serialize)]
pub struct UiState {
    status: Status,
    store: Store,
    /// Sections of the active profile the mouse does not have yet.
    pending: Vec<Section>,
    slot_names: Vec<&'static str>,
    unverified: Vec<&'static str>,
    /// Shown until the user dismisses it.
    load_warning: Option<String>,
}

#[derive(Serialize)]
pub struct ApplyFailure {
    kind: &'static str,
    message: String,
}

impl From<ApplyError> for ApplyFailure {
    fn from(e: ApplyError) -> Self {
        let kind = match &e {
            ApplyError::NoReceiver => "no_receiver",
            ApplyError::Asleep => "asleep",
            ApplyError::UnsupportedModel(_) => "unsupported_model",
            ApplyError::Invalid(_) => "invalid",
            ApplyError::Io { .. } => "io",
        };
        let message = match &e {
            ApplyError::NoReceiver => "Receptor não conectado.".to_string(),
            ApplyError::Asleep => {
                "Mouse dormindo ou desligado. Mexa o mouse e tente de novo.".to_string()
            }
            ApplyError::UnsupportedModel(m) => {
                format!("Este modelo de mouse ({m:#04x}) não é suportado; nada foi enviado.")
            }
            ApplyError::Invalid(errs) => errs.join("; "),
            ApplyError::Io { message, .. } => {
                format!("Envio interrompido ({message}). O que faltou continua pendente.")
            }
        };
        ApplyFailure { kind, message }
    }
}

#[tauri::command]
pub fn get_state(ctx: State<Ctx>) -> UiState {
    let store = ctx.controller.store();
    UiState {
        status: ctx.svc.status(),
        pending: ctx.controller.pending_sections(&store.active),
        store,
        slot_names: SLOT_NAMES.to_vec(),
        unverified: UNVERIFIED.to_vec(),
        load_warning: ctx.controller.warning(),
    }
}

#[tauri::command]
pub fn save_profile(ctx: State<Ctx>, profile: Profile) -> Result<(), Vec<String>> {
    ctx.controller.save_profile(profile)
}

/// Save an edit and send it to the mouse right away (there is no Apply button).
#[tauri::command(async)]
pub fn update_profile(ctx: State<Ctx>, profile: Profile) -> Result<Vec<Section>, ApplyFailure> {
    Ok(ctx.controller.update_profile(profile)?)
}

#[tauri::command(async)]
pub fn apply_profile(ctx: State<Ctx>, name: String) -> Result<Vec<Section>, ApplyFailure> {
    Ok(ctx.controller.apply_profile(&name)?)
}

#[tauri::command(async)]
pub fn set_active_profile(
    app: AppHandle,
    ctx: State<Ctx>,
    name: String,
) -> Result<Vec<Section>, ApplyFailure> {
    let result = ctx.controller.set_active_profile(&name);
    tray::refresh_menu(&app);
    Ok(result?)
}

#[tauri::command]
pub fn create_profile(app: AppHandle, ctx: State<Ctx>, name: String) -> Result<(), String> {
    ctx.controller.create_profile(&name)?;
    tray::refresh_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_profile(app: AppHandle, ctx: State<Ctx>, name: String) -> Result<(), String> {
    ctx.controller.delete_profile(&name)?;
    tray::refresh_menu(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn factory_reset(ctx: State<Ctx>) -> Result<(), ApplyFailure> {
    Ok(ctx.controller.factory_reset()?)
}

#[tauri::command]
pub fn update_settings(app: AppHandle, ctx: State<Ctx>, settings: Settings) -> Result<(), String> {
    ctx.controller.update_settings(settings.clone())?;
    crate::sync_autostart(&app, settings.autostart);
    Ok(())
}

#[tauri::command(async)]
pub fn bind_script(
    ctx: State<Ctx>,
    profile: String,
    slot: usize,
    command: String,
    label: String,
) -> Result<(), String> {
    ctx.controller.bind_script(&profile, slot, &command, &label)
}

#[tauri::command(async)]
pub fn unbind_script(ctx: State<Ctx>, profile: String, slot: usize) -> Result<(), String> {
    ctx.controller.unbind_script(&profile, slot)
}

#[tauri::command]
pub fn script_log(ctx: State<Ctx>) -> Vec<RunRecord> {
    ctx.runner.log()
}

#[tauri::command]
pub fn dismiss_warning(ctx: State<Ctx>) {
    ctx.controller.dismiss_warning();
}
