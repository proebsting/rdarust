//! Extra partisan metrics that rdapy computes but DRA itself does not.
//! Ported from `rdapy/partisan/more.py`.

/// The efficiency gap computed from wasted votes rather than the statewide
/// formula.
///
/// A vote is wasted if it was cast for a loser, or for a winner beyond the
/// threshold needed to win. Needs actual vote counts, not shares, which is why
/// it is computed apart from the rest of the partisan suite.
///
/// Returns `None` if no votes were cast, where rdapy asserts.
pub fn calc_efficiency_gap_wasted_votes(
    dem_by_district: &[i64],
    rep_by_district: &[i64],
) -> Option<f64> {
    if dem_by_district.len() != rep_by_district.len() {
        return None;
    }
    let total_votes: i64 = dem_by_district.iter().sum::<i64>()
        + rep_by_district.iter().sum::<i64>();
    if total_votes <= 0 {
        return None;
    }

    let mut dem_wasted: i64 = 0;
    let mut rep_wasted: i64 = 0;

    for (&d, &r) in dem_by_district.iter().zip(rep_by_district.iter()) {
        // Integer floor division, as in Python. Ties count as Republican wins.
        let threshold = (d + r).div_euclid(2) + 1;
        if d > r {
            dem_wasted += d - threshold;
            rep_wasted += r;
        } else {
            rep_wasted += r - threshold;
            dem_wasted += d;
        }
    }

    Some((dem_wasted - rep_wasted) as f64 / total_votes as f64)
}

/// The average margin of victory across districts.
///
/// Plain accumulation, matching Python's builtin `sum`.
pub fn calc_average_margin(vf_array: &[f64]) -> f64 {
    crate::partisan::naive_sum(vf_array.iter().map(|&v| (v - 0.5000).abs()))
        / vf_array.len() as f64
}
