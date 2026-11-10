//! Runs the shell commands bound to mouse buttons.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use delux_profile::ScriptBinding;
use serde::Serialize;

/// Output kept per run; the rest is read and discarded.
pub const LOG_CAP: usize = 64 * 1024;
const LOG_LEN: usize = 50;

/// Slots that may carry a script, and the key each one emits: slot 3 → F13
/// (HID usage 0x68) … slot 14 → F24 (0x73). The key is fixed per slot so a
/// button means the same key in every profile. (Full-size Apple keyboards
/// have F13–F19; pressing those would run the script too.)
const FIRST_SCRIPT_SLOT: usize = 3;
const LAST_SCRIPT_SLOT: usize = 14;
const F13_USAGE: u8 = 0x68;

pub fn usage_for_slot(slot: usize) -> Option<u8> {
    (FIRST_SCRIPT_SLOT..=LAST_SCRIPT_SLOT)
        .contains(&slot)
        .then(|| F13_USAGE + (slot - FIRST_SCRIPT_SLOT) as u8)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunRecord {
    pub label: String,
    /// Unix time, ms.
    pub started_ms: u64,
    /// `None` while running; -1 if killed by a signal.
    pub exit: Option<i32>,
    pub output: String,
    pub truncated: bool,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    running: HashMap<usize, Arc<Mutex<Child>>>,
    /// Newest first.
    log: VecDeque<(u64, RunRecord)>,
}

impl Inner {
    fn push(&mut self, rec: RunRecord) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.log.push_front((id, rec));
        self.log.truncate(LOG_LEN);
        id
    }
}

#[derive(Default, Clone)]
pub struct ScriptRunner {
    inner: Arc<Mutex<Inner>>,
}

impl ScriptRunner {
    /// Start the binding's command. Returns false (and does nothing) if the
    /// same button's previous run is still going.
    pub fn trigger(&self, binding: &ScriptBinding) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.running.contains_key(&binding.slot) {
            return false;
        }
        let started_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let mut rec = RunRecord {
            label: binding.label.clone(),
            started_ms,
            exit: None,
            output: String::new(),
            truncated: false,
        };
        // Login shell so the user's PATH applies; stderr joins stdout. Own
        // process group so everything the script starts can be killed with it.
        let spawned = Command::new("/bin/zsh")
            .arg("-lc")
            .arg(format!("{{ {}\n}} 2>&1", binding.command))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                rec.exit = Some(-1);
                rec.output = format!("não foi possível iniciar: {e}");
                inner.push(rec);
                return true;
            }
        };
        let stdout = child.stdout.take();
        let child = Arc::new(Mutex::new(child));
        let id = inner.push(rec);
        let slot = binding.slot;
        inner.running.insert(slot, child.clone());
        drop(inner);

        // Reader: collects output until the pipe closes, which may be long
        // after the shell exits if it left something running in the background.
        let captured = Arc::new(Mutex::new((Vec::new(), false)));
        let sink = captured.clone();
        std::thread::spawn(move || {
            let Some(mut out) = stdout else { return };
            let mut buf = [0u8; 8192];
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut c = sink.lock().unwrap();
                let room = LOG_CAP - c.0.len();
                c.0.extend_from_slice(&buf[..n.min(room)]);
                c.1 |= n > room;
            }
        });

        // Waiter: the run ends when the shell exits, not when the pipe closes.
        let shared = self.inner.clone();
        std::thread::spawn(move || {
            let exit = loop {
                match child.lock().unwrap().try_wait() {
                    Ok(Some(status)) => break status.code().unwrap_or(-1),
                    Ok(None) => {}
                    Err(_) => break -1,
                }
                std::thread::sleep(Duration::from_millis(25));
            };
            // Let the reader drain what the shell wrote just before exiting.
            std::thread::sleep(Duration::from_millis(100));
            let (kept, truncated) = captured.lock().unwrap().clone();
            let mut inner = shared.lock().unwrap();
            inner.running.remove(&slot);
            if let Some((_, rec)) = inner.log.iter_mut().find(|(i, _)| *i == id) {
                rec.exit = Some(exit);
                rec.output = String::from_utf8_lossy(&kept).into_owned();
                rec.truncated = truncated;
            }
        });
        true
    }

    /// Recent runs, newest first.
    pub fn log(&self) -> Vec<RunRecord> {
        self.inner
            .lock()
            .unwrap()
            .log
            .iter()
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// Kill every running script and whatever it started (app shutdown).
    pub fn kill_all(&self) {
        for child in self.inner.lock().unwrap().running.values() {
            let mut child = child.lock().unwrap();
            // Negative pid = the script's whole process group.
            let _ = Command::new("/bin/kill")
                .args(["-TERM", "--", &format!("-{}", child.id())])
                .status();
            let _ = child.kill();
        }
    }

    #[cfg(test)]
    fn push_for_test(&self, rec: RunRecord) {
        self.inner.lock().unwrap().push(rec);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn binding(slot: usize, command: &str) -> ScriptBinding {
        ScriptBinding {
            slot,
            hotkey_usage: 0x68,
            command: command.into(),
            label: format!("s{slot}"),
        }
    }

    fn wait_done(r: &ScriptRunner) -> RunRecord {
        let start = Instant::now();
        loop {
            if let Some(rec) = r.log().into_iter().find(|rec| rec.exit.is_some()) {
                return rec;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "script never finished"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn runs_and_captures() {
        let r = ScriptRunner::default();
        assert!(r.trigger(&binding(7, "echo oi")));
        let rec = wait_done(&r);
        assert_eq!(rec.exit, Some(0));
        assert_eq!(rec.output, "oi\n");
        assert!(!rec.truncated);
        assert_eq!(rec.label, "s7");
    }

    #[test]
    fn captures_stderr_and_exit_code() {
        let r = ScriptRunner::default();
        assert!(r.trigger(&binding(7, "echo err 1>&2; exit 3")));
        let rec = wait_done(&r);
        assert_eq!(rec.exit, Some(3));
        assert_eq!(rec.output, "err\n");
    }

    #[test]
    fn output_is_capped() {
        let r = ScriptRunner::default();
        assert!(r.trigger(&binding(7, "yes | head -c 200000")));
        let rec = wait_done(&r);
        assert!(rec.output.len() <= LOG_CAP);
        assert!(rec.truncated);
    }

    #[test]
    fn hung_script_does_not_block() {
        let r = ScriptRunner::default();
        let start = Instant::now();
        assert!(r.trigger(&binding(7, "sleep 30")));
        assert!(start.elapsed() < Duration::from_millis(100));
        assert!(!r.trigger(&binding(7, "echo again")), "same slot is busy");
        assert!(r.trigger(&binding(6, "echo other")), "other slot is free");
        r.kill_all();
    }

    #[test]
    fn slot_is_free_again_after_finish() {
        let r = ScriptRunner::default();
        assert!(r.trigger(&binding(7, "true")));
        wait_done(&r);
        let start = Instant::now();
        while !r.trigger(&binding(7, "true")) {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn log_keeps_last_50() {
        let r = ScriptRunner::default();
        for i in 0..60 {
            r.push_for_test(RunRecord {
                label: format!("{i}"),
                started_ms: i,
                exit: Some(0),
                output: String::new(),
                truncated: false,
            });
        }
        let log = r.log();
        assert_eq!(log.len(), 50);
        assert_eq!(log[0].label, "59", "newest first");
    }

    #[test]
    fn usage_for_slot_maps_3_to_14_onto_f13_to_f24() {
        assert_eq!(usage_for_slot(3), Some(0x68));
        assert_eq!(usage_for_slot(7), Some(0x6C));
        assert_eq!(usage_for_slot(14), Some(0x73));
        assert_eq!(usage_for_slot(2), None);
        assert_eq!(usage_for_slot(15), None);
    }

    #[test]
    fn background_child_does_not_hold_the_button() {
        // `server &` keeps the output pipe open long after the shell exits.
        let r = ScriptRunner::default();
        assert!(r.trigger(&binding(7, "sleep 30 & echo started")));
        let rec = wait_done(&r);
        assert_eq!(rec.exit, Some(0));
        assert!(rec.output.contains("started"), "{:?}", rec.output);
        assert!(
            r.trigger(&binding(7, "true")),
            "button is free once the shell exits"
        );
        r.kill_all();
    }
}
