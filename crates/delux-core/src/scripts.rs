//! Runs the shell commands bound to mouse buttons.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use delux_profile::ScriptBinding;
use serde::Serialize;

/// Output kept per run; the rest is read and discarded.
pub const LOG_CAP: usize = 64 * 1024;
const LOG_LEN: usize = 50;

/// HID usages of F13..=F24: keys no Mac keyboard has.
const HOTKEY_USAGES: std::ops::RangeInclusive<u8> = 0x68..=0x73;

/// First F13..F24 usage not in `used`.
pub fn next_free_usage(used: &[u8]) -> Option<u8> {
    HOTKEY_USAGES.into_iter().find(|u| !used.contains(u))
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
        // Login shell so the user's PATH applies; stderr joins stdout.
        let spawned = Command::new("/bin/zsh")
            .arg("-lc")
            .arg(format!("{{ {}\n}} 2>&1", binding.command))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
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

        let shared = self.inner.clone();
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            let mut truncated = false;
            if let Some(mut out) = stdout {
                let mut buf = [0u8; 8192];
                while let Ok(n) = out.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let room = LOG_CAP - kept.len();
                    kept.extend_from_slice(&buf[..n.min(room)]);
                    truncated |= n > room;
                }
            }
            let exit = child
                .lock()
                .unwrap()
                .wait()
                .ok()
                .and_then(|s| s.code())
                .unwrap_or(-1);
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

    /// Kill every running script (app shutdown).
    pub fn kill_all(&self) {
        for child in self.inner.lock().unwrap().running.values() {
            let _ = child.lock().unwrap().kill();
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
    use std::time::{Duration, Instant};

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
    fn next_free_usage_walks_f13_to_f24() {
        assert_eq!(next_free_usage(&[]), Some(0x68));
        assert_eq!(next_free_usage(&[0x68, 0x6A]), Some(0x69));
        let all: Vec<u8> = (0x68..=0x73).collect();
        assert_eq!(next_free_usage(&all), None);
    }
}
