//! The core of John Nagle's method: seat probabilities and inferred
//! seats-votes curves. Ported from `rdapy/partisan/method.rs`.

use crate::numeric::{erf, shift_range};
use crate::partisan::naive_sum;

/// The probability that a district with Democratic vote share `vf` elects a
/// Democrat.
///
/// A normal CDF with a 2% standard deviation, so the transition from safe-R to
/// safe-D happens over roughly 45-55%. This is the single most-used formula in
/// the suite.
#[inline]
pub fn est_seat_probability(vf: f64) -> f64 {
    0.5 * (1.0 + erf((vf - 0.50) / (0.02 * 8f64.sqrt())))
}

/// How much a district's expected seat share moves when the vote moves.
///
/// Peaks at 1.0 for a 50-50 district and falls to 0 for safe seats.
#[inline]
pub fn est_district_responsiveness(vf: f64) -> f64 {
    1.0 - 4.0 * (est_seat_probability(vf) - 0.5).powi(2)
}

/// `S#` -- expected Democratic seats, summing per-district probabilities.
///
/// Uses plain accumulation to match Python's builtin `sum`.
pub fn est_seats(vf_array: &[f64]) -> f64 {
    naive_sum(vf_array.iter().map(|&v| est_seat_probability(v)))
}

/// `S!` -- Democratic seats under first past the post. Ties go to Republicans.
pub fn est_fptp_seats(vf_array: &[f64]) -> i32 {
    vf_array.iter().filter(|&&v| v > 0.5).count() as i32
}

/// Infer the points of a seats-votes curve by shifting the statewide vote
/// share across `[0.25, 0.75]` in half-percent steps.
///
/// Returns `(vote share, fractional seats)` -- seats, not seat share.
pub fn infer_sv_points(vf: f64, vf_array: &[f64], proportional: bool) -> Vec<(f64, f64)> {
    shift_range()
        .into_iter()
        .map(|shifted_vf| {
            let shifted = shift_districts(vf, vf_array, shifted_vf, proportional);
            (shifted_vf, est_seats(&shifted))
        })
        .collect()
}

/// Move every district's vote share to be consistent with a new statewide
/// share.
pub fn shift_districts(
    vf: f64,
    vf_array: &[f64],
    shifted_vf: f64,
    proportional: bool,
) -> Vec<f64> {
    if proportional {
        shift_districts_proportionally(vf, vf_array, shifted_vf)
    } else {
        shift_districts_uniformly(vf, vf_array, shifted_vf)
    }
}

/// Add the same delta to every district.
pub fn shift_districts_uniformly(vf: f64, vf_array: &[f64], shifted_vf: f64) -> Vec<f64> {
    let shift = shifted_vf - vf;
    vf_array.iter().map(|&v| v + shift).collect()
}

/// Scale each district's share, so safe districts move less than marginal
/// ones. Shifting down scales the Democratic share; shifting up scales the
/// Republican share. This is the default.
pub fn shift_districts_proportionally(vf: f64, vf_array: &[f64], shifted_vf: f64) -> Vec<f64> {
    if shifted_vf < vf {
        let proportion = shifted_vf / vf;
        vf_array.iter().map(|&v| v * proportion).collect()
    } else if shifted_vf > vf {
        let proportion = (1.0 - shifted_vf) / (1.0 - vf);
        vf_array.iter().map(|&v| 1.0 - (1.0 - v) * proportion).collect()
    } else {
        vf_array.to_vec()
    }
}

/// The Republican seats-votes curve: the Democratic one reflected through the
/// centre, then re-sorted into increasing vote share.
pub fn infer_inverse_sv_points(sv_pts: &[(f64, f64)], n: i32) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = sv_pts
        .iter()
        .map(|&(v_d, s_d)| (1.0 - v_d, n as f64 - s_d))
        .collect();
    // Python sorts on the vote share alone, and its sort is stable.
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("sort of NaN"));
    out
}

/// The bias-of-geometric-seats curve: half the gap between the two
/// seats-votes curves, indexed by Democratic vote share.
pub fn infer_geometric_seats_bias_points(
    d_sv_pts: &[(f64, f64)],
    r_sv_pts: &[(f64, f64)],
) -> Vec<(f64, f64)> {
    d_sv_pts
        .iter()
        .zip(r_sv_pts.iter())
        .map(|(&(v_d, s_d), &(_v_r, s_r))| (v_d, 0.5 * (s_r - s_d)))
        .collect()
}

/// The last curve point at or below `value`, comparing on component `idx`.
pub fn lower_bracket(sv_curve_pts: &[(f64, f64)], value: f64, idx: usize) -> Option<(f64, f64)> {
    sv_curve_pts
        .iter()
        .filter(|pt| component(pt, idx) <= value)
        .next_back()
        .copied()
}

/// The first curve point at or above `value`, comparing on component `idx`.
pub fn upper_bracket(sv_curve_pts: &[(f64, f64)], value: f64, idx: usize) -> Option<(f64, f64)> {
    sv_curve_pts
        .iter()
        .find(|pt| component(pt, idx) >= value)
        .copied()
}

#[inline]
fn component(pt: &(f64, f64), idx: usize) -> f64 {
    if idx == 0 {
        pt.0
    } else {
        pt.1
    }
}
