//! HID transport for the 2.4G 8K receiver (1D57:FA65).
//!
//! One macOS HID device carries both directions: commands go out as report 4
//! (usage page 0xFF00), events come in as report 3.

use std::thread::sleep;
use std::time::Duration;

use delux_proto::{frames, parse_event, Event};

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

/// How the mouse is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Transport {
    /// 2.4G 8K receiver: commands framed into output report 4.
    Receiver,
    /// USB cable (mouse switched OFF, cable plugged): each command is a
    /// feature report whose ID is the command byte.
    Wired,
}

pub struct Device {
    dev: hidapi::HidDevice,
    transport: Transport,
}

/// Total length (ID included) of each wired feature report, from the wired
/// HID descriptor. Only the commands the vendor driver sends are listed.
fn wired_len(cmd: u8) -> Option<usize> {
    Some(match cmd {
        0x04 => 52,
        0x05 => 13,
        0x06 => 9,
        0x08 => 59,
        0x0C => 6,
        _ => return None,
    })
}

const WIRED_MACRO_REPORT: usize = 64;
const WIRED_MACRO_DATA: usize = 60;

/// Turn an inner packet into the feature report(s) the wired mouse expects.
pub fn wired_reports(inner: &[u8]) -> Result<Vec<Vec<u8>>, HidError> {
    let cmd = *inner
        .first()
        .ok_or_else(|| HidError::Io("pacote vazio".into()))?;
    if cmd == 0x09 {
        // Macro: `09 40 <slot> <chunk>` + 60 bytes of the packet body.
        let body = inner.get(3..).unwrap_or_default();
        return Ok(body
            .chunks(WIRED_MACRO_DATA)
            .enumerate()
            .map(|(i, chunk)| {
                let mut r = vec![0u8; WIRED_MACRO_REPORT];
                r[..4].copy_from_slice(&[0x09, 0x40, inner[2], i as u8]);
                r[4..4 + chunk.len()].copy_from_slice(chunk);
                r
            })
            .collect());
    }
    let len = wired_len(cmd)
        .ok_or_else(|| HidError::Io(format!("comando {cmd:#04x} não existe no modo cabo")))?;
    if inner.len() < len {
        return Err(HidError::Io(format!("pacote {cmd:#04x} curto demais")));
    }
    Ok(vec![inner[..len].to_vec()])
}

impl Device {
    /// Open the mouse: over the cable if it is there (as the vendor driver
    /// prefers), otherwise through the receiver.
    pub fn open() -> Result<Device, HidError> {
        let api = hidapi::HidApi::new()?;
        let find = |pid: u16, usage_page: u16| {
            api.device_list().find(|d| {
                d.vendor_id() == VID && d.product_id() == pid && d.usage_page() == usage_page
            })
        };
        let (info, transport) = find(PID_WIRED, WIRED_COMMAND_USAGE_PAGE)
            .map(|i| (i, Transport::Wired))
            .or_else(|| find(PID, VENDOR_USAGE_PAGE).map(|i| (i, Transport::Receiver)))
            .ok_or(HidError::NotFound)?;
        let dev = api.open_path(info.path())?;
        Ok(Device { dev, transport })
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// Wait up to `timeout_ms` for one input report.
    pub fn read_raw(&self, timeout_ms: i32) -> Result<Option<Vec<u8>>, HidError> {
        let mut buf = [0u8; 128];
        let n = self.dev.read_timeout(&mut buf, timeout_ms)?;
        Ok((n > 0).then(|| buf[..n].to_vec()))
    }

    /// `Ok(None)` on timeout or on a report that is not a report-3 event.
    pub fn read_event(&self, timeout_ms: i32) -> Result<Option<Event>, HidError> {
        Ok(self.read_raw(timeout_ms)?.and_then(|r| parse_event(&r)))
    }

    /// Send one command (inner packet), with the vendor driver's pacing.
    pub fn write_packet(&self, inner: &[u8]) -> Result<(), HidError> {
        match self.transport {
            Transport::Receiver => {
                let frames = frames(inner).map_err(|e| HidError::Io(e.to_string()))?;
                for f in &frames {
                    self.dev.write(f)?;
                    sleep(CHUNK_PAUSE);
                }
            }
            Transport::Wired => {
                let reports = wired_reports(inner)?;
                let chunked = reports.len() > 1;
                for r in &reports {
                    self.dev.send_feature_report(r)?;
                    if chunked {
                        sleep(CHUNK_PAUSE);
                    }
                }
            }
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
        let is_kbd =
            d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == 1 && d.usage() == 6;
        if is_kbd && !paths.contains(&d.path().to_owned()) {
            paths.push(d.path().to_owned());
        }
    }
    if paths.is_empty() {
        return Err(HidError::NotFound);
    }
    let devs = paths
        .iter()
        .map(|p| api.open_path(p))
        .collect::<Result<Vec<_>, _>>()?;
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

/// Wired mode: the mouse itself on USB (switch OFF, cable plugged).
pub const PID_WIRED: u16 = 0xFA73;
const WIRED_COMMAND_USAGE_PAGE: u16 = 0x000B;

/// One feature-report read: (report id, bytes or error text).
pub type FeatureRead = (u8, Result<Vec<u8>, String>);

/// Read-only diagnostic: ask the wired mouse for its feature reports
/// (standard HID GET_REPORT). Returns (report id, bytes or error text).
pub fn wired_read_feature_reports(ids: &[(u8, usize)]) -> Result<Vec<FeatureRead>, HidError> {
    let api = hidapi::HidApi::new()?;
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == VID
                && d.product_id() == PID_WIRED
                && d.usage_page() == WIRED_COMMAND_USAGE_PAGE
        })
        .ok_or(HidError::NotFound)?;
    let dev = api.open_path(info.path())?;
    Ok(ids
        .iter()
        .map(|(id, len)| {
            let mut buf = vec![0u8; len + 1];
            buf[0] = *id;
            let r = dev
                .get_feature_report(&mut buf)
                .map(|n| buf[..n].to_vec())
                .map_err(|e| e.to_string());
            (*id, r)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use delux_proto::{
        encode_buttons, encode_factory_reset, encode_macro, encode_polling, encode_power_light,
        encode_sensor, ButtonMap, Macro, MacroStep, PollingRate, PowerLightConfig, Repeat,
        SensorConfig,
    };

    // Lengths come from the wired HID descriptor (report data + 1 for the ID).
    #[test]
    fn wired_plain_commands_are_the_inner_packet_cut_to_the_declared_length() {
        let polling = encode_polling(PollingRate::Hz1000);
        assert_eq!(wired_reports(&polling).unwrap(), vec![polling.clone()]);

        let sensor = encode_sensor(&SensorConfig::default_profile()).unwrap();
        let r = wired_reports(&sensor).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(
            r[0],
            sensor[..52],
            "checksum ends at byte 52; padding dropped"
        );

        let power = encode_power_light(&PowerLightConfig::default_profile()).unwrap();
        assert_eq!(wired_reports(&power).unwrap(), vec![power[..13].to_vec()]);

        let buttons = encode_buttons(&ButtonMap::factory()).unwrap();
        assert_eq!(wired_reports(&buttons).unwrap(), vec![buttons.clone()]);

        assert_eq!(
            wired_reports(&encode_factory_reset()).unwrap(),
            vec![vec![0x0C, 0x0A, 0x01, 0xFE, 0x01, 0xFE]]
        );
    }

    #[test]
    fn wired_macro_goes_as_two_64_byte_chunks() {
        let m = Macro {
            steps: vec![
                MacroStep {
                    down: true,
                    code: 4,
                    delay_ms: 50,
                },
                MacroStep {
                    down: false,
                    code: 4,
                    delay_ms: 10,
                },
            ],
            repeat: Repeat::Times(1),
        };
        let inner = encode_macro(7, &m).unwrap();
        let r = wired_reports(&inner).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].len(), 64);
        assert_eq!(&r[0][..4], &[0x09, 0x40, 0x08, 0x00]);
        assert_eq!(&r[0][4..], &inner[3..63]);
        assert_eq!(r[1].len(), 64);
        assert_eq!(&r[1][..4], &[0x09, 0x40, 0x08, 0x01]);
        assert_eq!(&r[1][4..52], &inner[63..111]);
        assert!(r[1][52..].iter().all(|b| *b == 0));
    }

    #[test]
    fn wired_refuses_commands_the_driver_never_sends() {
        assert!(wired_reports(&[0x22, 0x05, 0x01, 0, 0]).is_err());
        assert!(wired_reports(&[]).is_err());
    }
}
