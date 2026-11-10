//! Estimate the mouse's report rate from arrival timestamps.

const WINDOW_US: u64 = 250_000;

/// Highest report rate seen in any 250 ms window, in Hz. `ts` is ascending µs.
pub fn peak_rate_hz(ts: &[u64]) -> u32 {
    if ts.len() < 2 {
        return 0;
    }
    let mut best = 0usize;
    let mut start = 0usize;
    for end in 0..ts.len() {
        while ts[end] - ts[start] >= WINDOW_US {
            start += 1;
        }
        best = best.max(end - start + 1);
    }
    (best as u64 * 1_000_000 / WINDOW_US) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_1000hz() {
        let ts: Vec<u64> = (0..3000).map(|i| i * 1000).collect(); // µs
        assert_eq!(peak_rate_hz(&ts), 1000);
    }

    #[test]
    fn burst_decides_not_average() {
        // 1 s idle, then 0.5 s at 500 Hz
        let mut ts = vec![0u64];
        ts.extend((0..250).map(|i| 1_000_000 + i * 2000));
        assert_eq!(peak_rate_hz(&ts), 500);
    }

    #[test]
    fn too_few_samples_is_zero() {
        assert_eq!(peak_rate_hz(&[]), 0);
        assert_eq!(peak_rate_hz(&[5]), 0);
    }
}
