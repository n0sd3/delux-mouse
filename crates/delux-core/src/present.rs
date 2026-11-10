//! How device status is worded for the menu bar.

use delux_proto::BatteryState;

use crate::{Connection, Status};

/// Menu-bar text. A remembered battery value is shown in parentheses so it
/// never reads as a live one.
pub fn tray_title(s: &Status) -> String {
    let body = match (s.connection, s.battery) {
        (Connection::NoReceiver, _) => "—".to_string(),
        (Connection::Asleep, Some((_, pct))) => format!("({pct}%)"),
        (Connection::Asleep, None) => "zzz".to_string(),
        (Connection::Awake, None) => "…".to_string(),
        (Connection::Awake, Some((BatteryState::Charging | BatteryState::Full, pct))) => {
            format!("⚡{pct}%")
        }
        (Connection::Awake, Some((_, pct))) => format!("{pct}%"),
    };
    format!("🖱 {body}")
}

/// What the mouse has just changed about itself, compared with the previous
/// status: (DPI stage, polling, light mode). A value that merely lingers in
/// the status is not news and must not be acted on again.
pub fn live_delta(
    prev: Option<&Status>,
    now: &Status,
) -> (Option<u8>, Option<delux_proto::PollingRate>, Option<u8>) {
    (
        now.dpi_stage
            .filter(|_| prev.is_none_or(|p| p.dpi_stage != now.dpi_stage)),
        now.polling
            .filter(|_| prev.is_none_or(|p| p.polling != now.polling)),
        now.light_mode
            .filter(|_| prev.is_none_or(|p| p.light_mode != now.light_mode)),
    )
}

/// True when the battery has just dropped to or below `threshold` while not charging.
pub fn should_notify_low(prev: Option<u8>, now: u8, threshold: u8, charging: bool) -> bool {
    !charging && now <= threshold && prev.is_none_or(|p| p > threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Connection, Status};
    use delux_proto::BatteryState;

    fn st(connection: Connection, battery: Option<(BatteryState, u8)>) -> Status {
        Status {
            connection,
            model: Some(0x30),
            battery,
            battery_is_stale: battery.is_some() && connection != Connection::Awake,
            dpi_stage: None,
            polling: None,
            light_mode: None,
            transport: None,
        }
    }

    #[test]
    fn titles() {
        use BatteryState::*;
        use Connection::*;
        assert_eq!(tray_title(&st(NoReceiver, None)), "🖱 —");
        assert_eq!(tray_title(&st(NoReceiver, Some((Discharging, 84)))), "🖱 —");
        assert_eq!(tray_title(&st(Awake, Some((Discharging, 84)))), "🖱 84%");
        assert_eq!(tray_title(&st(Awake, Some((Charging, 40)))), "🖱 ⚡40%");
        assert_eq!(tray_title(&st(Awake, Some((Full, 100)))), "🖱 ⚡100%");
        assert_eq!(tray_title(&st(Awake, Some((Unknown(9), 50)))), "🖱 50%");
        assert_eq!(tray_title(&st(Asleep, Some((Discharging, 84)))), "🖱 (84%)");
        assert_eq!(tray_title(&st(Asleep, None)), "🖱 zzz");
        assert_eq!(tray_title(&st(Awake, None)), "🖱 …");
    }

    #[test]
    fn live_delta_reports_only_what_the_mouse_just_changed() {
        use crate::Connection::Awake;
        let mut a = st(Awake, Some((BatteryState::Discharging, 84)));
        a.dpi_stage = Some(1);
        // First status: everything it carries is news.
        assert_eq!(live_delta(None, &a), (Some(1), None, None));
        // Only the battery moved: the old stage must NOT be reported again,
        // or it would overwrite a stage the user has since picked in the app.
        let mut b = a.clone();
        b.battery = Some((BatteryState::Discharging, 83));
        assert_eq!(live_delta(Some(&a), &b), (None, None, None));
        let mut c = b.clone();
        c.dpi_stage = Some(2);
        c.light_mode = Some(3);
        assert_eq!(live_delta(Some(&b), &c), (Some(2), None, Some(3)));
    }

    #[test]
    fn low_battery_notifies_only_on_crossing() {
        assert!(should_notify_low(Some(21), 20, 20, false));
        assert!(should_notify_low(None, 15, 20, false));
        assert!(!should_notify_low(Some(20), 19, 20, false), "already below");
        assert!(!should_notify_low(Some(21), 20, 20, true), "charging");
        assert!(
            !should_notify_low(Some(50), 49, 20, false),
            "above threshold"
        );
    }
}
