//! Words and numbers of the galactic header that do not depend on Bevy: the month clock.

/// The galactic month shown in the header ("MONTH: n") and how far it has progressed (0..1, the green bar).
///
/// Data: the Remake calls one fiscal cycle (`Fiscal_Cycle_Time_In_Secs`) a month (`TEXT_CYCLES` "Month", `TEXT_LOG_CYCLE_01`
/// "Month ##0", "credits per month" in every upkeep text). The simulation counts that cycle as its week: `completed` is its week
/// counter, `time` the galactic seconds and `cycle_seconds` the length of one cycle. The counter is the same one the advisor's
/// "Month n" lines use, so the header shows month n + 1 while the last line reads "Month n".
pub(crate) fn month_clock(completed: u64, time: f64, cycle_seconds: f32) -> (u32, f32) {
    let month = u32::try_from(completed).unwrap_or(u32::MAX).saturating_add(1);
    let progress = if cycle_seconds > 0.0 { (time / f64::from(cycle_seconds)).fract() as f32 } else { 0.0 };
    (month, progress.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_counts_fiscal_cycles_as_months() {
        // A new game: month 1, nothing of it gone.
        assert_eq!(month_clock(0, 0.0, 45.0), (1, 0.0));
        // 130 cycles of 45 s are over and the 131st is half done: the advisor's last line said "Month 130".
        let (month, progress) = month_clock(130, 130.0 * 45.0 + 22.5, 45.0);
        assert_eq!(month, 131);
        assert!((progress - 0.5).abs() < 1e-4, "{progress}");
    }

    #[test]
    fn a_cycle_without_length_shows_no_progress() {
        assert_eq!(month_clock(3, 100.0, 0.0), (4, 0.0));
    }
}
