//! Majority-minority district counts, ported from
//! `rdapy/minority/majority_minority.py`.
//!
//! Counted from citizen voting-age population, in three mutually exclusive
//! buckets: Black-alone, Hispanic-alone, and coalition districts where neither
//! group is a majority on its own but together they are.

/// Counts in the three buckets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MmdCounts {
    pub mmd_black: i32,
    pub mmd_hispanic: i32,
    pub mmd_coalition: i32,
}

/// Is one group alone a majority of citizen voting-age population?
#[inline]
pub fn is_single_demo_mmd(demo_cvap: f64, total_cvap: f64) -> bool {
    (demo_cvap / total_cvap) > 0.5
}

/// Is this a coalition district: each group at or below half on its own, but
/// together above half?
pub fn is_coalition_mmd(demo_cvaps: &[f64], total_cvap: f64) -> bool {
    demo_cvaps.iter().all(|&d| d / total_cvap <= 0.5)
        && demo_cvaps.iter().sum::<f64>() / total_cvap > 0.5
}

/// Count majority-minority districts.
///
/// The three tests are applied in order and are mutually exclusive: a district
/// counted as Black-alone is not also considered for the others.
pub fn calculate_mmd_simple(
    black_cvap: &[f64],
    hispanic_cvap: &[f64],
    total_cvap: &[f64],
) -> MmdCounts {
    let mut counts = MmdCounts::default();

    for ((&black, &hispanic), &total) in black_cvap
        .iter()
        .zip(hispanic_cvap.iter())
        .zip(total_cvap.iter())
    {
        if is_single_demo_mmd(black, total) {
            counts.mmd_black += 1;
        } else if is_single_demo_mmd(hispanic, total) {
            counts.mmd_hispanic += 1;
        } else if is_coalition_mmd(&[black, hispanic], total) {
            counts.mmd_coalition += 1;
        }
    }

    counts
}
