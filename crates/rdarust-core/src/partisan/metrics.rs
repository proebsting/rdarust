//! The full partisan scorecard for one election, ported from
//! `rdapy/partisan/partisan.py`.

use crate::partisan::bias::{self, KeyRvPoints};
use crate::partisan::method::{self, infer_inverse_sv_points, infer_sv_points};
use crate::partisan::more;
use crate::partisan::responsiveness as resp;
use crate::spline::SplineError;

/// Everything rdapy's `calc_partisan_metrics` produces.
///
/// Biases follow rdapy's sign convention: `+` is Republican, `-` Democratic.
/// Seat *counts* are fractional where they come from seat probabilities;
/// fields named `_pct` or `_share` are fractions of the whole.
#[derive(Debug, Clone, PartialEq)]
pub struct PartisanMetrics {
    /// `^S#` -- whole seats closest to proportional.
    pub best_seats: i32,
    /// `S#` -- expected seats from seat probabilities.
    pub estimated_seats: f64,
    /// `S!` -- seats under first past the post.
    pub fptp_seats: i32,
    /// `B%` -- deviation from proportionality.
    pub deviation: f64,
    /// `TO` -- turnout bias.
    pub turnout_bias: f64,
    /// `BS_50` -- seats bias at a tied vote, as a fraction of all seats.
    pub seats_bias: Option<f64>,
    /// `BV_50` -- votes bias.
    pub votes_bias: f64,
    /// `decl` -- declination, in degrees. `None` where undefined.
    pub declination: Option<f64>,
    pub key_rv_points: KeyRvPoints,
    /// `GS` -- global symmetry.
    pub global_symmetry: f64,
    pub gamma: f64,
    /// `EG` from expected seats.
    pub efficiency_gap: f64,
    /// `EG` from first-past-the-post seats.
    pub efficiency_gap_fptp: f64,
    /// `BS_V` -- geometric seats bias, as a fraction of all seats.
    pub geometric_seats_bias: f64,
    /// `PR` -- raw disproportionality.
    pub disproportionality: f64,
    /// `MM` -- mean-median against the statewide share.
    pub mean_median_statewide: f64,
    /// `MM'` -- mean-median against the average district share.
    pub mean_median_average_district: f64,
    /// `LO` -- lopsided outcomes. `None` for a sweep.
    pub lopsided_outcomes: Option<f64>,

    /// `Cn` -- districts in [45%, 55%].
    pub competitive_district_count: i32,
    /// `cD` -- expected competitive districts.
    pub competitive_districts: f64,
    /// `R` -- seats won per point of vote above 50%.
    pub big_r: Option<f64>,
    /// `r` -- slope of the seats-votes curve at the statewide share.
    pub little_r: f64,
    /// `MIR` -- minimal inverse responsiveness.
    pub mir: Option<f64>,
    /// `rD` -- expected responsive districts.
    pub responsive_districts: f64,
    pub responsive_district_pct: f64,
    pub average_margin: f64,

    pub d_sv_points: Vec<(f64, f64)>,
    pub r_sv_points: Vec<(f64, f64)>,
    /// Mean share in Democratic-won districts, `None` if they won none.
    pub average_d_vf: Option<f64>,
    /// Mean share in Republican-won districts. Ties count as Republican wins.
    pub average_r_vf: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PartisanError {
    /// An inferred seats-votes curve could not be interpolated.
    Spline(SplineError),
    /// The statewide share fell outside the inferred curve, so little `r`
    /// has no bracket.
    NoResponsivenessBracket(f64),
}

impl From<SplineError> for PartisanError {
    fn from(e: SplineError) -> Self {
        PartisanError::Spline(e)
    }
}

impl std::fmt::Display for PartisanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PartisanError::Spline(e) => write!(f, "seats-votes interpolation: {e}"),
            PartisanError::NoResponsivenessBracket(vf) => {
                write!(f, "statewide share {vf} is outside the inferred curve")
            }
        }
    }
}

impl std::error::Error for PartisanError {}

/// Compute every partisan metric from a statewide share and the shares by
/// district.
///
/// `vf` is the statewide Democratic share of the two-party vote; `vf_array`
/// the same by district. rdapy always shifts the curve proportionally, so
/// that is what this does.
pub fn calc_partisan_metrics(
    vf: f64,
    vf_array: &[f64],
) -> Result<PartisanMetrics, PartisanError> {
    let n = vf_array.len() as i32;
    let nf = n as f64;

    let best_seats = bias::calc_best_seats(n, vf);
    let best_sf = best_seats as f64 / nf;

    let fptp_seats = method::est_fptp_seats(vf_array);
    let est_seats = method::est_seats(vf_array);
    let est_sf = est_seats / nf;

    let d_sv_points = infer_sv_points(vf, vf_array, true);
    let r_sv_points = infer_inverse_sv_points(&d_sv_points, n);

    let seats_bias = bias::est_seats_bias(&d_sv_points, n).map(|b| b / nf);
    let votes_bias = bias::est_votes_bias(&d_sv_points, n)?;
    let geometric_seats_bias =
        bias::est_geometric_seats_bias(vf, &d_sv_points, &r_sv_points)? / nf;

    let little_r = resp::est_responsiveness(vf, &d_sv_points, n)
        .ok_or(PartisanError::NoResponsivenessBracket(vf))?;

    // Global symmetry is signed by the seats bias.
    let global_symmetry =
        bias::calc_global_symmetry(&d_sv_points, &r_sv_points, seats_bias.unwrap_or(0.0), n);

    let d_wins: Vec<f64> = vf_array.iter().copied().filter(|&v| v > 0.5).collect();
    // Ties are credited to Republicans.
    let r_wins: Vec<f64> = vf_array.iter().copied().filter(|&v| v <= 0.5).collect();

    let responsive_districts = resp::est_responsive_districts(vf_array);

    Ok(PartisanMetrics {
        best_seats,
        estimated_seats: est_seats,
        fptp_seats,
        deviation: bias::calc_disproportionality_from_best(est_sf, best_sf),
        turnout_bias: bias::calc_turnout_bias(vf, vf_array),
        seats_bias,
        votes_bias,
        declination: bias::calc_declination(vf_array),
        key_rv_points: bias::key_rv_points(vf_array),
        global_symmetry,
        gamma: bias::calc_gamma(vf, est_sf, little_r),
        efficiency_gap: bias::calc_efficiency_gap(vf, est_sf),
        efficiency_gap_fptp: bias::calc_efficiency_gap(vf, fptp_seats as f64 / nf),
        geometric_seats_bias,
        disproportionality: bias::calc_disproportionality(vf, est_sf),
        mean_median_statewide: bias::calc_mean_median_difference(vf_array, Some(vf)),
        mean_median_average_district: bias::calc_mean_median_difference(vf_array, None),
        lopsided_outcomes: bias::calc_lopsided_outcomes(vf_array),

        competitive_district_count: resp::count_competitive_districts(vf_array),
        competitive_districts: resp::est_competitive_districts(vf_array),
        big_r: resp::calc_big_r(vf, est_sf),
        little_r,
        mir: resp::calc_minimal_inverse_responsiveness(vf, little_r),
        responsive_districts,
        responsive_district_pct: responsive_districts / nf,
        average_margin: more::calc_average_margin(vf_array),

        d_sv_points,
        r_sv_points,
        average_d_vf: (!d_wins.is_empty())
            .then(|| crate::partisan::naive_sum(d_wins.iter().copied()) / d_wins.len() as f64),
        average_r_vf: (!r_wins.is_empty())
            .then(|| crate::partisan::naive_sum(r_wins.iter().copied()) / r_wins.len() as f64),
    })
}
