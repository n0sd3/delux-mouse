//! Statistics over raw mouse reports, to compare sensor settings by
//! measurement instead of feel.

/// Reports further apart than this belong to different strokes.
const STROKE_GAP_US: u64 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Arrival time, µs.
    pub t_us: u64,
    pub buttons: u8,
    pub dx: i16,
    pub dy: i16,
}

/// Mouse report of this device: buttons, X (i16 LE), Y (i16 LE), wheel, pan.
pub fn parse_mouse_report(t_us: u64, r: &[u8]) -> Option<Sample> {
    let [buttons, xl, xh, yl, yh, ..] = *r else {
        return None;
    };
    Some(Sample {
        t_us,
        buttons,
        dx: i16::from_le_bytes([xl, xh]),
        dy: i16::from_le_bytes([yl, yh]),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub reports: usize,
    /// Reports per second while moving (pauses between strokes excluded).
    pub rate_hz: f64,
    /// Share of moving reports with one axis exactly zero. Angle snapping
    /// pushes this up on straight strokes.
    pub axis_purity: f64,
    /// Travel on the minor axis over travel on the major axis.
    pub perpendicular_share: f64,
    /// Mean change between consecutive deltas over mean delta size:
    /// 0 = perfectly even motion. Smoothing (ripple control) and frame
    /// alignment (motion sync) should lower it.
    pub roughness: f64,
}

pub fn stats(v: &[Sample]) -> Stats {
    let moving: Vec<&Sample> = v.iter().filter(|s| s.dx != 0 || s.dy != 0).collect();
    let ratio = |num: f64, den: f64| if den > 0.0 { num / den } else { 0.0 };

    let pure = moving.iter().filter(|s| s.dx == 0 || s.dy == 0).count();
    let sum_x: f64 = moving.iter().map(|s| (s.dx as f64).abs()).sum();
    let sum_y: f64 = moving.iter().map(|s| (s.dy as f64).abs()).sum();

    let (mut active_us, mut intervals, mut change, mut size, mut pairs) =
        (0u64, 0u64, 0.0, 0.0, 0u64);
    for w in moving.windows(2) {
        let dt = w[1].t_us.saturating_sub(w[0].t_us);
        if dt >= STROKE_GAP_US {
            continue;
        }
        active_us += dt;
        intervals += 1;
        let (ax, ay, bx, by) = (
            w[0].dx as f64,
            w[0].dy as f64,
            w[1].dx as f64,
            w[1].dy as f64,
        );
        change += ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        size += ((ax * ax + ay * ay).sqrt() + (bx * bx + by * by).sqrt()) / 2.0;
        pairs += 1;
    }
    let _ = pairs;
    Stats {
        reports: moving.len(),
        rate_hz: ratio(intervals as f64 * 1e6, active_us as f64),
        axis_purity: ratio(pure as f64, moving.len() as f64),
        perpendicular_share: ratio(sum_x.min(sum_y), sum_x.max(sum_y)),
        roughness: ratio(change, size),
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clicks {
    /// How long each press of the button lasted.
    pub presses_ms: Vec<f64>,
    /// Time from each release to the next press.
    pub gaps_ms: Vec<f64>,
}

/// Press durations and release→press gaps of the button(s) in `mask`.
pub fn clicks(v: &[Sample], mask: u8) -> Clicks {
    let mut out = Clicks::default();
    let (mut down_at, mut up_at) = (None::<u64>, None::<u64>);
    for s in v {
        let down = s.buttons & mask != 0;
        match (down, down_at) {
            (true, None) => {
                if let Some(up) = up_at.take() {
                    out.gaps_ms.push((s.t_us - up) as f64 / 1000.0);
                }
                down_at = Some(s.t_us);
            }
            (false, Some(start)) => {
                out.presses_ms.push((s.t_us - start) as f64 / 1000.0);
                down_at = None;
                up_at = Some(s.t_us);
            }
            _ => {}
        }
    }
    out
}

pub fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let mid = s.len() / 2;
    if s.len() % 2 == 1 {
        s[mid]
    } else {
        (s[mid - 1] + s[mid]) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t_us: u64, dx: i16, dy: i16) -> Sample {
        Sample {
            t_us,
            buttons: 0,
            dx,
            dy,
        }
    }

    #[test]
    fn parses_a_7_byte_mouse_report() {
        // buttons, x lo, x hi, y lo, y hi, wheel, pan
        let p = parse_mouse_report(123, &[0x01, 0x05, 0x00, 0xFE, 0xFF, 0, 0]).unwrap();
        assert_eq!((p.t_us, p.buttons, p.dx, p.dy), (123, 1, 5, -2));
        assert!(parse_mouse_report(0, &[1, 2, 3]).is_none());
    }

    #[test]
    fn axis_purity_counts_moving_reports_with_one_axis_at_zero() {
        let v = vec![s(0, 5, 0), s(1, 4, 0), s(2, 3, 1), s(3, 0, 0), s(4, 0, 7)];
        // 4 moving reports; 3 of them are purely horizontal or vertical
        assert!((stats(&v).axis_purity - 0.75).abs() < 1e-9);
    }

    #[test]
    fn perpendicular_share_is_minor_axis_travel_over_major() {
        // mostly horizontal stroke: |dy| total 2 over |dx| total 20
        let v = vec![s(0, 10, 1), s(1, 10, -1)];
        assert!((stats(&v).perpendicular_share - 0.1).abs() < 1e-9);
    }

    #[test]
    fn roughness_is_zero_for_constant_speed_and_grows_with_jitter() {
        let smooth: Vec<Sample> = (0..50).map(|i| s(i * 1000, 10, 0)).collect();
        let rough: Vec<Sample> = (0..50)
            .map(|i| s(i * 1000, if i % 2 == 0 { 5 } else { 15 }, 0))
            .collect();
        assert!(stats(&smooth).roughness < 1e-9);
        assert!(
            (stats(&rough).roughness - 1.0).abs() < 0.05,
            "{}",
            stats(&rough).roughness
        );
    }

    #[test]
    fn rate_ignores_pauses_between_strokes() {
        // two bursts at 1000 Hz separated by half a second
        let mut v: Vec<Sample> = (0..100).map(|i| s(i * 1000, 1, 0)).collect();
        v.extend((0..100).map(|i| s(600_000 + i * 1000, 1, 0)));
        let r = stats(&v).rate_hz;
        assert!((r - 1000.0).abs() < 20.0, "{r}");
    }

    #[test]
    fn click_durations_and_gaps() {
        let b = |t_ms: u64, buttons: u8| Sample {
            t_us: t_ms * 1000,
            buttons,
            dx: 0,
            dy: 0,
        };
        // press 0–80 ms, press 200–260 ms (bit 0 = left button)
        let v = vec![b(0, 1), b(80, 0), b(200, 1), b(260, 0)];
        let c = clicks(&v, 0x01);
        assert_eq!(c.presses_ms, vec![80.0, 60.0]);
        assert_eq!(c.gaps_ms, vec![120.0]);
    }

    #[test]
    fn empty_input_gives_zeroes_not_nan() {
        let st = stats(&[]);
        assert_eq!(
            (st.reports, st.axis_purity, st.roughness, st.rate_hz),
            (0, 0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn median_works() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }
}
