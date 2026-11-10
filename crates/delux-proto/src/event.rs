//! Report-ID-3 events sent by the mouse: `03, model, type, v0, v1`.

/// Model code of the M900 PRO 8K variant handled by driver page MS_4.
pub const MODEL_M900PRO_8K_MS4: u8 = 0x30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum BatteryState {
    Discharging,
    Charging,
    Full,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Battery { state: BatteryState, percent: u8 },
    /// 1-based DPI stage.
    DpiStage(u8),
    /// Polling code 1..=7 (125 Hz .. 8000 Hz).
    PollingRate(u8),
    Ack(u8),
    Other { kind: u8, v0: u8, v1: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub model: u8,
    pub kind: EventKind,
}

/// Decode one input report. `None` if it is not a report-3 event.
pub fn parse_event(report: &[u8]) -> Option<Event> {
    let [0x03, model, kind, v0, v1, ..] = *report else {
        return None;
    };
    let kind = match kind {
        0x40 | 0x41 => match v0 {
            1 => EventKind::Battery { state: BatteryState::Discharging, percent: v1.min(100) },
            3 => EventKind::Battery { state: BatteryState::Charging, percent: v1.min(100) },
            2 => EventKind::Battery { state: BatteryState::Full, percent: 100 },
            other => EventKind::Battery { state: BatteryState::Unknown(other), percent: v1.min(100) },
        },
        0x10 => EventKind::DpiStage(v0),
        0x20 => EventKind::PollingRate((v0 >> 4) & 0x0F),
        0x50 => EventKind::Ack(v0),
        kind => EventKind::Other { kind, v0, v1 },
    };
    Some(Event { model, kind })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_84() {
        let e = parse_event(&[0x03, 0x30, 0x40, 0x01, 0x54]).unwrap();
        assert_eq!(e.model, 0x30);
        assert_eq!(e.kind, EventKind::Battery { state: BatteryState::Discharging, percent: 84 });
    }

    #[test]
    fn dpi_stage_3() {
        assert_eq!(parse_event(&[0x03, 0x30, 0x10, 0x03, 0x00]).unwrap().kind, EventKind::DpiStage(3));
    }

    #[test]
    fn charging() {
        assert_eq!(
            parse_event(&[3, 0x30, 0x40, 3, 10]).unwrap().kind,
            EventKind::Battery { state: BatteryState::Charging, percent: 10 }
        );
    }

    #[test]
    fn full_is_100() {
        assert_eq!(
            parse_event(&[3, 0x30, 0x40, 2, 0]).unwrap().kind,
            EventKind::Battery { state: BatteryState::Full, percent: 100 }
        );
    }

    #[test]
    fn battery_type_41_and_percent_clamped() {
        assert_eq!(
            parse_event(&[3, 0x50, 0x41, 1, 250]).unwrap().kind,
            EventKind::Battery { state: BatteryState::Discharging, percent: 100 }
        );
    }

    #[test]
    fn unknown_battery_state_is_kept() {
        assert_eq!(
            parse_event(&[3, 0x30, 0x40, 9, 50]).unwrap().kind,
            EventKind::Battery { state: BatteryState::Unknown(9), percent: 50 }
        );
    }

    #[test]
    fn polling() {
        assert_eq!(parse_event(&[3, 0x30, 0x20, 0x70, 0]).unwrap().kind, EventKind::PollingRate(7));
    }

    #[test]
    fn ack_and_other() {
        assert_eq!(parse_event(&[3, 0x30, 0x50, 0, 0]).unwrap().kind, EventKind::Ack(0));
        assert_eq!(
            parse_event(&[3, 0x30, 0x70, 1, 2]).unwrap().kind,
            EventKind::Other { kind: 0x70, v0: 1, v1: 2 }
        );
    }

    #[test]
    fn rejects_short_and_wrong_id() {
        assert!(parse_event(&[3, 0x30]).is_none());
        assert!(parse_event(&[4, 0, 0, 0, 0]).is_none());
    }
}
