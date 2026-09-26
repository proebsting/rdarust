//! Competitiveness and responsiveness, ported from
//! `rdapy/partisan/responsiveness.py`.

use crate::partisan::method::{est_district_responsiveness, lower_bracket, upper_bracket};
use crate::partisan::{naive_sum, roughly_equal, EPSILON};

/// `Cn` -- districts whose vote share falls in `[45%, 55%]`.
pub fn count_competitive_districts(vf_array: &[f64]) -> i32 {
    vf_array.iter().filter(|&&v| v >= 0.45 && v <= 0.55).count() as i32
}

/// `cD` -- the expected number of competitive districts.
pub fn est_competitive_districts(vf_array: &[f64]) -> f64 {
    naive_sum(vf_array.iter().map(|&v| est_district_competitiveness(v)))
}

/// Competitiveness is a synonym for responsiveness at the district level.
#[inline]
pub fn est_district_competitiveness(vf: f64) -> f64 {
    est_district_responsiveness(vf)
}

/// `rD` -- the expected number of responsive districts.
pub fn est_responsive_districts(vf_array: &[f64]) -> f64 {
    naive_sum(vf_array.iter().map(|&v| est_district_responsiveness(v)))
}

/// little `r` -- the slope of the seats-votes curve at the statewide share.
///
/// The seat delta is normalised into a share before taking the slope, so the
/// result is dimensionless.
///
/// Returns `None` if the statewide share falls outside the inferred curve,
/// where rdapy raises.
pub fn est_responsiveness(vf: f64, sv_curve_pts: &[(f64, f64)], n: i32) -> Option<f64> {
    const VOTE_SHARE: usize = 0;

    let (v1, s1) = lower_bracket(sv_curve_pts, vf, VOTE_SHARE)?;
    let (v2, s2) = upper_bracket(sv_curve_pts, vf, VOTE_SHARE)?;

    Some(((s2 - s1) / n as f64) / (v2 - v1))
}

/// big `R` -- seats won per point of vote above 50%. Undefined at a tie.
pub fn calc_big_r(vf: f64, sf: f64) -> Option<f64> {
    if roughly_equal(vf, 0.5, EPSILON) {
        None
    } else {
        Some((sf - 0.5) / (vf - 0.5))
    }
}

/// `MIR` -- minimal inverse responsiveness, `1/r` less an ideal.
///
/// The ideal is stricter for a competitive state (r = 10) than a lopsided one
/// (r = 5). Undefined when the plan is entirely unresponsive.
pub fn calc_minimal_inverse_responsiveness(vf: f64, r: f64) -> Option<f64> {
    if roughly_equal(r, 0.0, EPSILON) {
        return None;
    }
    let ideal = if is_balanced(vf) { 0.1 } else { 0.2 };
    Some(((1.0 / r) - ideal).max(0.0))
}

/// Is the statewide vote share within `[45%, 55%]`?
pub fn is_balanced(vf: f64) -> bool {
    !(vf > 0.55 || vf < 0.45)
}
