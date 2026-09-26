//! Measures of partisan bias, ported from `rdapy/partisan/bias.py`.
//!
//! By convention `+` is Republican bias and `-` is Democratic.

use crate::numeric::{isclose_default, python_round};
use crate::partisan::method::{
    est_seat_probability, est_seats, infer_geometric_seats_bias_points,
};
use crate::partisan::{exact_mean, median, naive_sum, roughly_equal, EPSILON};
use crate::spline::{CubicSpline, SplineError};

/// `^S#` -- the whole number of Democratic seats closest to proportional.
///
/// The epsilon nudge makes an exact half round down rather than to even.
pub fn calc_best_seats(n: i32, vf: f64) -> i32 {
    python_round((n as f64 * vf) - EPSILON) as i32
}

/// `B%` -- deviation from proportionality, as proportional-minus-actual.
#[inline]
pub fn calc_disproportionality_from_best(est_sf: f64, best_sf: f64) -> f64 {
    best_sf - est_sf
}

/// `PR` -- raw disproportionality.
#[inline]
pub fn calc_disproportionality(vf: f64, sf: f64) -> f64 {
    vf - sf
}

/// `EG` -- the efficiency gap.
///
/// Written so that `+` is Republican bias, consistent with the other metrics.
/// This is *not* the more common `(Sf - 0.5) - 2*(Vf - 0.5)` arrangement.
#[inline]
pub fn calc_efficiency_gap(vf: f64, sf: f64) -> f64 {
    (2.0 * (vf - 0.5)) - (sf - 0.5)
}

/// `gamma` -- bias relative to what the plan's own responsiveness predicts.
#[inline]
pub fn calc_gamma(vf: f64, sf: f64, r: f64) -> f64 {
    0.5 + (r * (vf - 0.5)) - sf
}

/// `BS_50` -- seats bias at a tied statewide vote, in seats.
///
/// Reads the seats-votes curve at 50%. The curve never contains exactly 0.5
/// (accumulating the shift range lands on 0.5000000000000002), so the point is
/// located with `math.isclose` at its default tolerances, as rdapy does.
pub fn est_seats_bias(sv_curve_pts: &[(f64, f64)], n: i32) -> Option<f64> {
    let (_, d_seats) = sv_curve_pts
        .iter()
        .find(|pt| isclose_default(pt.0, 0.5))
        .copied()?;
    let r_seats = n as f64 - d_seats;
    Some((r_seats - d_seats) / 2.0)
}

/// `BV_50` -- votes bias: how far the statewide vote must move from 50% for
/// the seats to split evenly.
///
/// Interpolates the seats-votes curve the other way round, with seats as the
/// independent variable.
pub fn est_votes_bias(sv_curve_pts: &[(f64, f64)], n: i32) -> Result<f64, SplineError> {
    let half_seats = n as f64 / 2.0;

    let x: Vec<f64> = sv_curve_pts.iter().map(|p| p.0).collect();
    let y: Vec<f64> = sv_curve_pts.iter().map(|p| p.1).collect();

    let sp = CubicSpline::new(&y, &x)?;
    Ok(sp.eval(half_seats)? - 0.50)
}

/// `BS_V` -- geometric seats bias at the actual statewide vote share.
///
/// Returns a fractional number of seats, not a seat share.
pub fn est_geometric_seats_bias(
    vf: f64,
    d_sv_pts: &[(f64, f64)],
    r_sv_pts: &[(f64, f64)],
) -> Result<f64, SplineError> {
    let b_gs_pts = infer_geometric_seats_bias_points(d_sv_pts, r_sv_pts);

    let x: Vec<f64> = b_gs_pts.iter().map(|p| p.0).collect();
    let y: Vec<f64> = b_gs_pts.iter().map(|p| p.1).collect();

    CubicSpline::new(&x, &y)?.eval(vf)
}

/// `GS` -- global symmetry: the area between the two seats-votes curves.
///
/// Normalised by 100 so it is comparable to the unit square, even though the
/// curves are only inferred over `[0.25, 0.75]` at 101 points.
pub fn calc_global_symmetry(
    d_sv_pts: &[(f64, f64)],
    r_sv_pts: &[(f64, f64)],
    s50v: f64,
    n: i32,
) -> f64 {
    let mut g_sym = 0.0;
    for i in 0..d_sv_pts.len() {
        let d_sf = d_sv_pts[i].1 / n as f64;
        let r_sf = r_sv_pts[i].1 / n as f64;
        g_sym += (d_sf - r_sf).abs() / 2.0;
    }
    let sign = if s50v < 0.0 { -1.0 } else { 1.0 };
    (g_sym * sign) / 100.0
}

/// The four corner points of the declination diagram.
///
/// District vote shares are Democratic shares, so party A is Republican and
/// party B is Democratic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyRvPoints {
    /// Democratic seat share.
    pub sb: f64,
    /// Midpoint of the Republican-won seats.
    pub ra: f64,
    /// Midpoint of the Democratic-won seats.
    pub rb: f64,
    /// Average Republican vote share in Republican-won districts.
    pub va: f64,
    /// Average Democratic vote share in Democratic-won districts, as a
    /// Republican share.
    pub vb: f64,
}

pub fn key_rv_points(vf_array: &[f64]) -> KeyRvPoints {
    let n_districts = vf_array.len() as f64;
    let est_s = est_seats(vf_array);

    let sb = est_s / n_districts;
    let ra = (1.0 + sb) / 2.0;
    let rb = sb / 2.0;

    let va = naive_sum(
        vf_array
            .iter()
            .map(|&v| est_seat_probability(1.0 - v) * (1.0 - v)),
    ) / (n_districts - est_s);
    let vb =
        1.0 - naive_sum(vf_array.iter().map(|&v| est_seat_probability(v) * v)) / est_s;

    // Clamp away floating-point overshoot past the tied point.
    KeyRvPoints {
        sb,
        ra,
        rb,
        va: va.max(0.50),
        vb: vb.min(0.50),
    }
}

/// Did one party win every seat?
pub fn is_sweep(sf: f64, n_districts: usize) -> bool {
    let one_district = 1.0 / n_districts as f64;
    sf > (1.0 - one_district) || sf < one_district
}

#[inline]
pub fn radians_to_degrees(radians: f64) -> f64 {
    radians * (180.0 / std::f64::consts::PI)
}

/// `decl` -- declination, in degrees.
///
/// The angle between the two halves of the rank-vote diagram. Undefined, and
/// so `None`, for a sweep, for fewer than five districts, or when either
/// winning average sits exactly at 50%.
pub fn calc_declination(vf_array: &[f64]) -> Option<f64> {
    let k = key_rv_points(vf_array);

    let sweep = is_sweep(k.sb, vf_array.len());
    let too_few_districts = vf_array.len() < 5;
    let va_at_50 = roughly_equal(k.va - 0.5, 0.0, EPSILON);
    let vb_at_50 = roughly_equal(0.5 - k.vb, 0.0, EPSILON);

    if sweep || too_few_districts || va_at_50 || vb_at_50 {
        return None;
    }

    let l_tan = (k.sb - k.rb) / (0.5 - k.vb);
    let r_tan = (k.ra - k.sb) / (k.va - 0.5);

    Some(radians_to_degrees(r_tan.atan()) - radians_to_degrees(l_tan.atan()))
}

/// `MM` and `MM'` -- the mean-median difference.
///
/// Pass `Some(vf)` for the statewide version, `None` for the average-district
/// one.
///
/// Note the quirk this reproduces: rdapy writes `Vf if Vf else mean(...)`, a
/// truthiness test, so a statewide share of exactly 0.0 falls through to the
/// average-district benchmark.
pub fn calc_mean_median_difference(vf_array: &[f64], vf: Option<f64>) -> f64 {
    let benchmark = match vf {
        Some(v) if v != 0.0 => v,
        _ => exact_mean(vf_array),
    };
    benchmark - median(vf_array)
}

/// `TO` -- turnout bias: statewide share minus the average district share.
pub fn calc_turnout_bias(vf: f64, vf_array: &[f64]) -> f64 {
    vf - exact_mean(vf_array)
}

/// `LO` -- lopsided outcomes, a measure of packing.
///
/// Positive means party B's voters are the more packed. Undefined for a sweep.
pub fn calc_lopsided_outcomes(vf_array: &[f64]) -> Option<f64> {
    let k = key_rv_points(vf_array);
    if is_sweep(k.sb, vf_array.len()) {
        return None;
    }
    Some((0.5 - k.vb) - (k.va - 0.5))
}
