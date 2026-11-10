//! HID transport for the 2.4G 8K receiver (1D57:FA65).
//!
//! One macOS HID device carries both directions: commands go out as report 4
//! (usage page 0xFF00), events come in as report 3.

use std::thread::sleep;
use std::time::Duration;

use delux_proto::{parse_event, Event, Frame};

pub const VID: u16 = 0x1D57;
pub const PID: u16 = 0xFA65;
const VENDOR_USAGE_PAGE: u16 = 0xFF00;

/// Pause after each frame and after each whole command, as the vendor driver does.
const CHUNK_PAUSE: Duration = Duration::from_millis(200);
const COMMAND_PAUSE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HidError {
    #[error("receptor 2.4G 8K (1D57:FA65) não encontrado")]
    NotFound,
    #[error("erro HID: {0}")]
    Io(String),
}

impl From<hidapi::HidError> for HidError {
    fn from(e: hidapi::HidError) -> Self {
        HidError::Io(e.to_string())
    }
}

pub struct Device {
    dev: hidapi::HidDevice,
}

impl Device {
    /// Open the receiver's vendor collection.
    pub fn open() -> Result<Device, HidError> {
        let api = hidapi::HidApi::new()?;
        let info = api
            .device_list()
            .find(|d| {
                d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == VENDOR_USAGE_PAGE
            })
            .ok_or(HidError::NotFound)?;
        let dev = api.open_path(info.path())?;
        Ok(Device { dev })
    }

    /// Wait up to `timeout_ms` for one input report. `Ok(None)` on timeout or
    /// on a report that is not a report-3 event.
    pub fn read_raw(&self, timeout_ms: i32) -> Result<Option<Vec<u8>>, HidError> {
        let mut buf = [0u8; 128];
        let n = self.dev.read_timeout(&mut buf, timeout_ms)?;
        Ok((n > 0).then(|| buf[..n].to_vec()))
    }

    pub fn read_event(&self, timeout_ms: i32) -> Result<Option<Event>, HidError> {
        Ok(self.read_raw(timeout_ms)?.and_then(|r| parse_event(&r)))
    }

    /// Send all frames of one command, with the driver's pacing.
    pub fn write_frames(&self, frames: &[Frame]) -> Result<(), HidError> {
        for f in frames {
            self.dev.write(f)?;
            sleep(CHUNK_PAUSE);
        }
        sleep(COMMAND_PAUSE);
        Ok(())
    }
}

/// Read raw reports from the mouse-movement interface for `secs` and return
/// arrival times in µs. Diagnostic only; macOS may require Input Monitoring.
pub fn capture_mouse_report_times(secs: u64) -> Result<Vec<u64>, HidError> {
    let api = hidapi::HidApi::new()?;
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == 1 && d.usage() == 2
        })
        .ok_or(HidError::NotFound)?;
    let dev = api.open_path(info.path())?;
    let start = std::time::Instant::now();
    let mut buf = [0u8; 64];
    let mut times = Vec::new();
    while start.elapsed() < Duration::from_secs(secs) {
        if dev.read_timeout(&mut buf, 50)? > 0 {
            times.push(start.elapsed().as_micros() as u64);
        }
    }
    Ok(times)
}

/// Read raw reports from every keyboard interface of the receiver for `secs`.
/// Diagnostic only: shows which key a remapped button emits.
pub fn capture_keyboard_reports(secs: u64) -> Result<Vec<Vec<u8>>, HidError> {
    let api = hidapi::HidApi::new()?;
    let mut paths = Vec::new();
    for d in api.device_list() {
        let is_kbd = d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == 1 && d.usage() == 6;
        if is_kbd && !paths.contains(&d.path().to_owned()) {
            paths.push(d.path().to_owned());
        }
    }
    if paths.is_empty() {
        return Err(HidError::NotFound);
    }
    let devs = paths.iter().map(|p| api.open_path(p)).collect::<Result<Vec<_>, _>>()?;
    let start = std::time::Instant::now();
    let mut buf = [0u8; 64];
    let mut out = Vec::new();
    while start.elapsed() < Duration::from_secs(secs) {
        for dev in &devs {
            let n = dev.read_timeout(&mut buf, 20)?;
            if n > 0 {
                out.push(buf[..n].to_vec());
            }
        }
    }
    Ok(out)
}
