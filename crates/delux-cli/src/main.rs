//! `delux` — diagnostic CLI for the Delux M900 PRO 8K receiver.

mod hex;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use delux_hid::Device;
use delux_proto::{frames, parse_event, Event, MODEL_M900PRO_8K_MS4};

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
    }
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
