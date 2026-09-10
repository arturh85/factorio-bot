//! Independent verification of direct machine-to-lab delivery via material
//! conservation.
//!
//! For the phase-1 red-science topology, observes one product assembler,
//! its sole outgoing inserter, and that inserter's destination lab. The
//! conservation equation counts actually delivered items across an interval
//! without trusting the planner's model.

use serde::{Deserialize, Serialize};

/// A reading of the direct output edge at one point in game time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeReading {
    /// Game tick when this reading was taken.
    pub tick: u64,
    /// Cumulative items the assembler has ever completed.
    pub completed_items: u64,
    /// Items currently sitting in the assembler's output slot.
    pub output_items: u64,
    /// Items currently held by the inserter.
    pub held_items: u64,
    /// Whether all preconditions for the conservation equation held
    /// continuously since the last reading (no recipe change, no manual
    /// removal, no alternate output path, etc.).
    pub continuity_valid: bool,
}

/// Whether the observed delivery satisfies the required threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryVerdict {
    /// The threshold was met in all windows.
    Achieved,
    /// The threshold was not met in at least one window.
    NotAchieved,
    /// The data is insufficient or the conservation assumptions were broken.
    Unknown,
}

/// Compute the number of items delivered between two edge readings using the
/// conservation equation:
///
/// ```text
/// delivered = (completed_b - completed_a)
///           + output_a + held_a
///           - output_b - held_b
/// ```
///
/// Returns `None` if the arithmetic overflows or counter regression is
/// detected (items should never decrease).
pub fn delivered_between(a: &EdgeReading, b: &EdgeReading) -> Option<u64> {
    // Use signed 128-bit intermediates to detect overflow and regression.
    let completed_delta = (b.completed_items as i128).checked_sub(a.completed_items as i128)?;
    let output_a = a.output_items as i128;
    let held_a = a.held_items as i128;
    let output_b = b.output_items as i128;
    let held_b = b.held_items as i128;

    let delivered: i128 = completed_delta
        .checked_add(output_a)?
        .checked_add(held_a)?
        .checked_sub(output_b)?
        .checked_sub(held_b)?;

    // Negative delivery means counter regression or stock was removed from
    // the pipeline by means other than delivery (manual removal, etc.).
    if delivered < 0 {
        return None;
    }

    // Should always fit in u64 for reasonable game values.
    u64::try_from(delivered).ok()
}

/// Verify that at least `minimum` items were delivered in each of `windows`
/// consecutive windows of `window_ticks` ticks, given a set of readings at
/// exact boundaries and a commissioning reading.
///
/// Returns:
/// - `DeliveryVerdict::Achieved` if every window meets the threshold
/// - `DeliveryVerdict::NotAchieved` if any window falls short
/// - `DeliveryVerdict::Unknown` if missing readings, broken continuity, or
///   arithmetic errors prevent a determination
///
/// The commissioning reading (`readings[0]`) is taken at the moment
/// commissioning is declared. Each subsequent reading is at an exact window
/// boundary. Adjacent windows share endpoints without double counting: the
/// delivery in window `i` is `delivered_between(readings[i], readings[i+1])`.
pub fn verify_windows(readings: &[EdgeReading], minimum: u64, window_ticks: u64, windows: usize) -> DeliveryVerdict {
    // Need commissioning + one reading per window.
    let required_readings = windows + 1;
    if readings.len() < required_readings {
        return DeliveryVerdict::Unknown;
    }

    // Verify continuity for every reading after commissioning.
    for reading in &readings[1..] {
        if !reading.continuity_valid {
            return DeliveryVerdict::Unknown;
        }
    }

    // Check each window.
    for i in 0..windows {
        let a = &readings[i];
        let b = &readings[i + 1];

        // Verify the window ticks align.
        let actual_ticks = b.tick.checked_sub(a.tick).unwrap_or(0);
        if actual_ticks != window_ticks {
            return DeliveryVerdict::Unknown;
        }

        match delivered_between(a, b) {
            Some(delivered) if delivered >= minimum => continue,
            Some(_) => return DeliveryVerdict::NotAchieved,
            None => return DeliveryVerdict::Unknown,
        }
    }

    DeliveryVerdict::Achieved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(tick: u64, completed: u64, output: u64, held: u64, valid: bool) -> EdgeReading {
        EdgeReading {
            tick,
            completed_items: completed,
            output_items: output,
            held_items: held,
            continuity_valid: valid,
        }
    }

    #[test]
    fn packs_still_in_the_arm_have_not_been_delivered() {
        let a = reading(0, 0, 0, 0, true);
        let b = reading(3600, 6, 0, 1, true);
        assert_eq!(delivered_between(&a, &b), Some(5));
    }

    #[test]
    fn full_pipeline_delivery() {
        // 6 completed, all 6 in output slot -> nothing was actually moved.
        let a = reading(0, 0, 0, 0, true);
        let b = reading(3600, 6, 6, 0, true);
        assert_eq!(delivered_between(&a, &b), Some(0));
    }

    #[test]
    fn counter_regression_returns_none() {
        let a = reading(0, 10, 0, 0, true);
        let b = reading(3600, 5, 0, 0, true);
        assert!(delivered_between(&a, &b).is_none());
    }

    #[test]
    fn overflow_detection() {
        let a = reading(0, u64::MAX, 0, 0, true);
        let b = reading(3600, 0, 0, 1, true);
        // completed went from MAX to 0 -> regression, should be None
        assert!(delivered_between(&a, &b).is_none());
    }

    #[test]
    fn verify_windows_accepts_sufficient_delivery() {
        let readings = vec![
            reading(0, 0, 0, 0, true),      // commissioning
            reading(3600, 10, 0, 0, true),  // window 1: 10 delivered
            reading(7200, 20, 0, 0, true),  // window 2: 10 delivered
            reading(10800, 30, 0, 0, true), // window 3: 10 delivered
            reading(14400, 40, 0, 0, true), // window 4: 10 delivered
            reading(18000, 50, 0, 0, true), // window 5: 10 delivered
        ];
        assert_eq!(verify_windows(&readings, 6, 3600, 5), DeliveryVerdict::Achieved);
    }

    #[test]
    fn verify_windows_rejects_insufficient() {
        let readings = vec![
            reading(0, 0, 0, 0, true),
            reading(3600, 10, 0, 0, true),   // enough
            reading(7200, 15, 0, 0, true),   // only 5
            reading(10800, 25, 0, 0, true),
            reading(14400, 35, 0, 0, true),
            reading(18000, 45, 0, 0, true),
        ];
        assert_eq!(verify_windows(&readings, 6, 3600, 5), DeliveryVerdict::NotAchieved);
    }

    #[test]
    fn missing_readings_are_unknown() {
        let readings = vec![
            reading(0, 0, 0, 0, true),
            reading(3600, 10, 0, 0, true),
            // only 2 readings for 5 windows
        ];
        assert_eq!(verify_windows(&readings, 6, 3600, 5), DeliveryVerdict::Unknown);
    }

    #[test]
    fn broken_continuity_is_unknown() {
        let readings = vec![
            reading(0, 0, 0, 0, true),
            reading(3600, 10, 0, 0, false), // continuity broken
            reading(7200, 20, 0, 0, true),
        ];
        assert_eq!(verify_windows(&readings, 6, 3600, 1), DeliveryVerdict::Unknown);
    }

    #[test]
    fn total_known_six_five_five_five_five() {
        // Only 6 in first window, 5 in remaining -> NotAchieved
        let readings = vec![
            reading(0, 0, 0, 0, true),
            reading(3600, 6, 0, 0, true),
            reading(7200, 11, 0, 0, true),
            reading(10800, 16, 0, 0, true),
            reading(14400, 21, 0, 0, true),
            reading(18000, 26, 0, 0, true),
        ];
        assert_eq!(verify_windows(&readings, 6, 3600, 5), DeliveryVerdict::NotAchieved);
    }

    #[test]
    fn total_known_six_six_six_six_six() {
        // 6 in every window -> Achieved
        let readings = vec![
            reading(0, 0, 0, 0, true),
            reading(3600, 6, 0, 0, true),
            reading(7200, 12, 0, 0, true),
            reading(10800, 18, 0, 0, true),
            reading(14400, 24, 0, 0, true),
            reading(18000, 30, 0, 0, true),
        ];
        assert_eq!(verify_windows(&readings, 6, 3600, 5), DeliveryVerdict::Achieved);
    }

    #[test]
    fn output_pipeline_stock_does_not_earn_credit() {
        // Initial stock of 2 in output: pipeline has 2 items that predate
        // the experiment. The conservation equation subtracts pipeline stock.
        // Each window: completed_delta=6, pipeline stable at 2, delivered=6.
        let readings = vec![
            reading(0, 0, 2, 0, true),     // commissioning: 2 pre-existing in output
            reading(3600, 6, 2, 0, true),  // 6 completed, pipeline still 2 -> 6 delivered
            reading(7200, 12, 2, 0, true),
            reading(10800, 18, 2, 0, true),
            reading(14400, 24, 2, 0, true),
            reading(18000, 30, 2, 0, true),
        ];
        assert_eq!(verify_windows(&readings, 6, 3600, 5), DeliveryVerdict::Achieved);
    }
}
