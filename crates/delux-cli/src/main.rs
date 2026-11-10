//! `delux` — diagnostic CLI for the Delux M900 PRO 8K receiver.

mod hex;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use delux_hid::Device;
use delux_proto::{
    encode_buttons, encode_factory_reset, encode_polling, encode_power_light, encode_sensor,
    frames, parse_event, ButtonAction, ButtonMap, Event, Lod, PollingRate, PowerLightConfig,
    SensorConfig, MODEL_M900PRO_8K_MS4, SLOT_COUNT,
};

#[derive(Parser)]
#[command(name = "delux", about = "Diagnóstico do Delux M900 PRO 8K")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Escuta e imprime eventos do mouse (só leitura).
    Listen {
        #[arg(long, default_value_t = 30)]
        secs: u64,
    },
    /// Envia um pacote interno em hex (ex.: "06 09 01 74 8B 00 00 00 00").
    SendRaw {
        hex: String,
        /// Sem isto, só mostra os frames que seriam enviados.
        #[arg(long)]
        yes: bool,
    },
    /// Define o polling rate (125, 250, 500, 1000, 2000, 4000, 8000).
    SetPolling {
        hz: u16,
        #[arg(long)]
        yes: bool,
    },
    /// Define os estágios de DPI (ex.: 400,3200,12800); demais opções no padrão.
    SetDpi {
        #[arg(value_delimiter = ',')]
        dpis: Vec<u16>,
        #[arg(long, default_value_t = 1)]
        stage: u8,
        #[arg(long)]
        lod2: bool,
        #[arg(long)]
        ripple: bool,
        #[arg(long)]
        angle_snap: bool,
        #[arg(long)]
        motion_sync: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Define sleep, debounce e luz; o que não for passado fica no padrão.
    SetPower {
        #[arg(long)]
        sleep: Option<u8>,
        #[arg(long)]
        debounce: Option<u8>,
        #[arg(long)]
        light_mode: Option<u8>,
        #[arg(long)]
        brightness: Option<u8>,
        #[arg(long)]
        yes: bool,
    },
    /// Envia o mapa de fábrica com um slot alterado. Ação: left, right, middle,
    /// back, forward, dpi, off, key:<usage hex>, raw:<3 bytes hex>.
    SetButton {
        slot: usize,
        action: String,
        #[arg(long)]
        yes: bool,
    },
    /// Envia o mapa de botões de fábrica.
    ResetButtons {
        #[arg(long)]
        yes: bool,
    },
    /// Reset de fábrica (cmd 0x0C).
    Reset {
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::Listen { secs } => listen(secs),
        Cmd::SendRaw { hex, yes } => send(&hex::parse_hex(&hex)?, yes),
        Cmd::SetPolling { hz, yes } => {
            let rate = PollingRate::from_hz(hz).ok_or(format!("polling inválido: {hz}"))?;
            send(&encode_polling(rate), yes)
        }
        Cmd::SetDpi { dpis, stage, lod2, ripple, angle_snap, motion_sync, yes } => {
            let mut cfg = SensorConfig::default_profile();
            if dpis.is_empty() || dpis.len() > cfg.stages.len() {
                return Err("informe de 1 a 8 valores de DPI".into());
            }
            for (i, s) in cfg.stages.iter_mut().enumerate() {
                s.enabled = i < dpis.len();
                if let Some(d) = dpis.get(i) {
                    s.dpi = *d;
                }
            }
            cfg.current_stage = stage;
            cfg.lod = if lod2 { Lod::Mm2 } else { Lod::Mm1 };
            cfg.ripple = ripple;
            cfg.angle_snap = angle_snap;
            cfg.motion_sync = motion_sync;
            send(&encode_sensor(&cfg).map_err(|e| e.to_string())?, yes)
        }
        Cmd::SetPower { sleep, debounce, light_mode, brightness, yes } => {
            let mut cfg = PowerLightConfig::default_profile();
            cfg.sleep_min = sleep.unwrap_or(cfg.sleep_min);
            cfg.debounce_ms = debounce.unwrap_or(cfg.debounce_ms);
            cfg.light_mode = light_mode.unwrap_or(cfg.light_mode);
            cfg.brightness = brightness.unwrap_or(cfg.brightness);
            send(&encode_power_light(&cfg).map_err(|e| e.to_string())?, yes)
        }
        Cmd::SetButton { slot, action, yes } => {
            if slot == 0 || slot >= SLOT_COUNT {
                return Err(format!("slot deve ser 1..{}", SLOT_COUNT - 1));
            }
            let mut map = ButtonMap::factory();
            map.0[slot] = parse_action(&action)?;
            send(&encode_buttons(&map).map_err(|e| e.to_string())?, yes)
        }
        Cmd::ResetButtons { yes } => {
            send(&encode_buttons(&ButtonMap::factory()).map_err(|e| e.to_string())?, yes)
        }
        Cmd::Reset { yes } => send(&encode_factory_reset(), yes),
    }
}

fn parse_action(s: &str) -> Result<ButtonAction, String> {
    Ok(match s {
        "left" => ButtonAction::Left,
        "right" => ButtonAction::Right,
        "middle" => ButtonAction::Middle,
        "back" => ButtonAction::Back,
        "forward" => ButtonAction::Forward,
        "dpi" => ButtonAction::DpiCycle,
        "off" => ButtonAction::Disabled,
        _ => {
            if let Some(h) = s.strip_prefix("key:") {
                match hex::parse_hex(h)?.as_slice() {
                    [usage] => ButtonAction::Key { modifiers: 0, usage: *usage },
                    _ => return Err("key:<1 byte hex>".into()),
                }
            } else if let Some(h) = s.strip_prefix("raw:") {
                match hex::parse_hex(h)?.as_slice() {
                    [a, b, c] => ButtonAction::Raw([*a, *b, *c]),
                    _ => return Err("raw:<3 bytes hex>".into()),
                }
            } else {
                return Err(format!("ação desconhecida: {s}"));
            }
        }
    })
}

fn listen(secs: u64) -> Result<(), String> {
    let dev = Device::open().map_err(|e| e.to_string())?;
    println!("escutando {secs}s …");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        if let Some(raw) = dev.read_raw(100).map_err(|e| e.to_string())? {
            let t = start.elapsed().as_secs_f32();
            match parse_event(&raw) {
                Some(Event { model, kind }) => {
                    println!("[{t:7.3}s] model={model:#04x} {kind:?}   ({})", hex::format_hex(&raw))
                }
                None => println!("[{t:7.3}s] ? ({})", hex::format_hex(&raw)),
            }
        }
    }
    Ok(())
}

/// Build frames for `inner`, show them, and send only with `yes` and only to
/// an awake mouse of the supported model.
fn send(inner: &[u8], yes: bool) -> Result<(), String> {
    let frames = frames(inner).map_err(|e| e.to_string())?;
    for (i, f) in frames.iter().enumerate() {
        println!("frame {i}: {}", hex::format_hex(f));
    }
    if !yes {
        println!("(não enviado — use --yes para enviar)");
        return Ok(());
    }
    let dev = Device::open().map_err(|e| e.to_string())?;
    wait_awake(&dev)?;
    dev.write_frames(&frames).map_err(|e| e.to_string())?;
    println!("enviado.");
    Ok(())
}

/// Block until a fresh event from a supported mouse arrives (max 10 s).
fn wait_awake(dev: &Device) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Some(ev) = dev.read_event(200).map_err(|e| e.to_string())? {
            if ev.model != MODEL_M900PRO_8K_MS4 {
                return Err(format!("modelo {:#04x} não suportado; nada enviado", ev.model));
            }
            return Ok(());
        }
    }
    Err("mouse sem sinal há 10 s (dormindo?). Mexa o mouse e tente de novo; nada enviado".into())
}
