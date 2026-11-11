//! `delux` — diagnostic CLI for the Delux M900 PRO 8K receiver.

mod hex;
mod motion;
mod rate;

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
    /// Aplica um perfil salvo (todas as seções) pelo serviço de dispositivo.
    Apply {
        /// Nome do perfil; padrão: o perfil ativo.
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Mede a taxa real de relatórios do mouse (mova o mouse durante a medição).
    MeasureRate {
        #[arg(long, default_value_t = 6)]
        secs: u64,
    },
    /// Mostra os relatórios de teclado que o receptor emite (botões remapeados).
    ListenKeys {
        #[arg(long, default_value_t = 20)]
        secs: u64,
    },
    /// Mede estatísticas do movimento bruto (mova o mouse durante a medição).
    MeasureMotion {
        #[arg(long, default_value_t = 8)]
        secs: u64,
    },
    /// Mede duração dos cliques do botão esquerdo (clique várias vezes).
    MeasureClicks {
        #[arg(long, default_value_t = 8)]
        secs: u64,
    },
    /// Envia o mapa de fábrica com os três botões extras trocados (diagnóstico).
    /// Cada função é uma ação de `set-button` (ex.: raw:100003, key:68, back).
    SetExtraButtons {
        dpi: String,
        front: String,
        back: String,
        #[arg(long)]
        yes: bool,
    },
    /// Mostra o que o mouse emite em todas as interfaces, sem o movimento puro.
    Capture {
        #[arg(long, default_value_t = 8)]
        secs: u64,
    },
    /// Modo cabo, só leitura: pede ao mouse os feature reports declarados.
    WiredRead,
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
        Cmd::SetDpi {
            dpis,
            stage,
            lod2,
            ripple,
            angle_snap,
            motion_sync,
            yes,
        } => {
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
        Cmd::SetPower {
            sleep,
            debounce,
            light_mode,
            brightness,
            yes,
        } => {
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
        Cmd::ResetButtons { yes } => send(
            &encode_buttons(&ButtonMap::factory()).map_err(|e| e.to_string())?,
            yes,
        ),
        Cmd::Apply { profile, yes } => apply(profile, yes),
        Cmd::MeasureRate { secs } => {
            println!("medindo {secs}s — mova o mouse sem parar …");
            let ts = delux_hid::capture_mouse_report_times(secs).map_err(|e| e.to_string())?;
            println!(
                "{} relatórios; pico ≈ {} Hz",
                ts.len(),
                rate::peak_rate_hz(&ts)
            );
            Ok(())
        }
        Cmd::ListenKeys { secs } => {
            println!("escutando teclas {secs}s …");
            for r in delux_hid::capture_keyboard_reports(secs).map_err(|e| e.to_string())? {
                println!("{}", hex::format_hex(&r));
            }
            Ok(())
        }
        Cmd::MeasureMotion { secs } => {
            let st = motion::stats(&mouse_samples(secs)?);
            println!(
                "relatorios={} taxa={:.0}Hz eixo_puro={:.3} perpendicular={:.4} aspereza={:.4}",
                st.reports, st.rate_hz, st.axis_purity, st.perpendicular_share, st.roughness
            );
            Ok(())
        }
        Cmd::MeasureClicks { secs } => {
            let c = motion::clicks(&mouse_samples(secs)?, 0x01);
            let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
            println!(
                "cliques={} duracao_mediana={:.1}ms duracao_min={:.1}ms intervalo_mediano={:.1}ms intervalo_min={:.1}ms",
                c.presses_ms.len(),
                motion::median(&c.presses_ms),
                if c.presses_ms.is_empty() { 0.0 } else { min(&c.presses_ms) },
                motion::median(&c.gaps_ms),
                if c.gaps_ms.is_empty() { 0.0 } else { min(&c.gaps_ms) },
            );
            Ok(())
        }
        Cmd::Capture { secs } => {
            let mut last: Option<(String, Vec<u8>)> = None;
            for mut r in delux_hid::capture_reports(secs, false).map_err(|e| e.to_string())? {
                // Skip the 2 s battery heartbeat.
                if r.data.first() == Some(&0x03) && r.data.get(2) == Some(&0x40) {
                    continue;
                }
                // Mouse interface: blank out X/Y so only buttons, wheel and
                // pan remain; plain movement then collapses into repeats.
                if r.source == "0001:02" && r.data.len() >= 5 {
                    r.data[1..5].fill(0);
                }
                if last.as_ref() == Some(&(r.source.clone(), r.data.clone())) {
                    continue;
                }
                println!(
                    "[{:8.3}s] {} {}",
                    r.t_us as f64 / 1e6,
                    r.source,
                    hex::format_hex(&r.data)
                );
                last = Some((r.source, r.data));
            }
            Ok(())
        }
        Cmd::SetExtraButtons {
            dpi,
            front,
            back,
            yes,
        } => {
            // Slots verified on hardware: 3 = DPI button, 6 = front side, 7 = back side.
            let mut map = ButtonMap::factory();
            map.0[3] = parse_action(&dpi)?;
            map.0[6] = parse_action(&front)?;
            map.0[7] = parse_action(&back)?;
            send(&encode_buttons(&map).map_err(|e| e.to_string())?, yes)
        }
        Cmd::WiredRead => {
            // (report id, declared data length) from the wired HID descriptor.
            let ids = [
                (0x04, 51),
                (0x05, 12),
                (0x06, 8),
                (0x07, 7),
                (0x08, 58),
                (0x0A, 7),
                (0x0B, 7),
                (0x0C, 5),
                (0x10, 7),
                (0xA0, 7),
            ];
            for (id, r) in delux_hid::wired_read_feature_reports(&ids).map_err(|e| e.to_string())? {
                match r {
                    Ok(bytes) => println!("{id:#04x}: {}", hex::format_hex(&bytes)),
                    Err(e) => println!("{id:#04x}: erro: {e}"),
                }
            }
            Ok(())
        }
        Cmd::Reset { yes } => send(&encode_factory_reset(), yes),
    }
}

fn apply(name: Option<String>, yes: bool) -> Result<(), String> {
    use delux_core::{Connection, DeviceService};
    use delux_profile::Section;

    let loaded = delux_profile::load(&delux_profile::default_path());
    if let Some(w) = loaded.warning {
        eprintln!("aviso: {w}");
    }
    let name = name.unwrap_or_else(|| loaded.store.active.clone());
    let profile = loaded
        .store
        .profiles
        .iter()
        .find(|p| p.name == name)
        .ok_or(format!("perfil {name:?} não existe"))?;
    println!("perfil {name:?}: {} seções", Section::ALL.len());
    if !yes {
        println!("(não enviado — use --yes para enviar)");
        return Ok(());
    }
    let svc = DeviceService::spawn_default(Box::new(|_| {}));
    let start = Instant::now();
    while svc.status().connection != Connection::Awake {
        if start.elapsed() > Duration::from_secs(10) {
            return Err(
                "mouse sem sinal há 10 s. Mexa o mouse e tente de novo; nada enviado".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    svc.apply(profile, &Section::ALL)
        .map_err(|e| e.to_string())?;
    println!("aplicado.");
    Ok(())
}

fn mouse_samples(secs: u64) -> Result<Vec<motion::Sample>, String> {
    Ok(delux_hid::capture_reports(secs, true)
        .map_err(|e| e.to_string())?
        .iter()
        .filter_map(|r| motion::parse_mouse_report(r.t_us, &r.data))
        .collect())
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
                    [usage] => ButtonAction::Key {
                        modifiers: 0,
                        usage: *usage,
                    },
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
                    println!(
                        "[{t:7.3}s] model={model:#04x} {kind:?}   ({})",
                        hex::format_hex(&raw)
                    )
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
    println!("transporte: {:?}", dev.transport());
    wait_awake(&dev)?;
    dev.write_packet(inner).map_err(|e| e.to_string())?;
    println!("enviado.");
    Ok(())
}

/// Block until a fresh event from a supported mouse arrives (max 10 s).
fn wait_awake(dev: &Device) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Some(ev) = dev.read_event(200).map_err(|e| e.to_string())? {
            if ev.model != MODEL_M900PRO_8K_MS4 {
                return Err(format!(
                    "modelo {:#04x} não suportado; nada enviado",
                    ev.model
                ));
            }
            return Ok(());
        }
    }
    Err("mouse sem sinal há 10 s (dormindo?). Mexa o mouse e tente de novo; nada enviado".into())
}
