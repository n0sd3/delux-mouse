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
