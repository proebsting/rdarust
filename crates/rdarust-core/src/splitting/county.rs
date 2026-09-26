//! County-district splitting, building on Moon Duchin's work.
//! Ported from `rdapy/splitting/county.py`.
//!
//! The central object is the `CxD` matrix: rows are districts, columns are
//! counties, entries are population. Note that entries are floats, not
//! integers -- populations get disaggregated to blocks and reaggregated, so
//! they can come out fractional. That is why "is this a whole county" is an
//! approximate comparison rather than an equality.

use crate::numeric::isclose;
use crate::partisan::naive_sum;

/// Rows are districts, columns are counties.
pub type CxD = Vec<Vec<f64>>;

/// How close a part must be to a whole before it counts as unsplit.
///
/// rdapy passes `abs_tol=1e-6` to `math.isclose` but leaves `rel_tol` at its
/// default, so both terms are live -- which matters, because county
/// populations run into the millions and the relative term dominates there.
const SPLIT_ABS_TOL: f64 = 1e-6;
const SPLIT_REL_TOL: f64 = 1e-9;

/// The county and district splitting scores for a plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplittingMetrics {
    pub county: f64,
    pub district: f64,
}

pub fn calc_splitting_metrics(cxd: &[Vec<f64>], reverse_weight: bool) -> SplittingMetrics {
    let d_t = district_totals(cxd);
    let c_t = county_totals(cxd);
    SplittingMetrics {
        county: calc_county_splitting_reduced(cxd, &d_t, &c_t, reverse_weight),
        district: calc_district_splitting_reduced(cxd, &d_t, &c_t),
    }
}

/// Moon Duchin's raw split score: the sum of the square roots of the parts.
///
/// An unsplit county scores 1.0; the more evenly a county is divided, the
/// higher the score. An empty split list also scores 1.0.
pub fn split_score(split: &[f64]) -> f64 {
    if split.is_empty() {
        return 1.0;
    }
    naive_sum(split.iter().map(|&x| x.sqrt()))
}

/// Total population of each county (column).
pub fn county_totals(cxd: &[Vec<f64>]) -> Vec<f64> {
    let n_c = cxd[0].len();
    let n_d = cxd.len();
    let mut totals = vec![0.0; n_c];
    for j in 0..n_c {
        for i in 0..n_d {
            totals[j] += cxd[i][j];
        }
    }
    totals
}

/// Total population of each district (row).
pub fn district_totals(cxd: &[Vec<f64>]) -> Vec<f64> {
    let n_c = cxd[0].len();
    let n_d = cxd.len();
    let mut totals = vec![0.0; n_d];
    for j in 0..n_c {
        for i in 0..n_d {
            totals[i] += cxd[i][j];
        }
    }
    totals
}

#[inline]
fn is_whole(part: f64, whole: f64) -> bool {
    isclose(part, whole, SPLIT_REL_TOL, SPLIT_ABS_TOL)
}

/// Fold whole districts up into a dummy district 0, county by county.
///
/// A district lying entirely within one county does not split that county, so
/// it is consolidated away before scoring.
pub fn reduce_county_splits(cxd: &[Vec<f64>], d_totals: &[f64]) -> CxD {
    let mut out: CxD = Vec::with_capacity(cxd.len() + 1);
    out.push(vec![0.0; cxd[0].len()]);
    out.extend(cxd.iter().cloned());

    let n_c = out[0].len();
    let n_d = out.len();

    for j in 0..n_c {
        for i in 1..n_d {
            let split_total = out[i][j];
            if split_total > 0.0 && is_whole(split_total, d_totals[i - 1]) {
                out[0][j] += split_total;
                out[i][j] = 0.0;
            }
        }
    }
    out
}

/// Fold whole counties left into a dummy county 0, district by district.
pub fn reduce_district_splits(cxd: &[Vec<f64>], c_totals: &[f64]) -> CxD {
    let mut out: CxD = cxd
        .iter()
        .map(|row| {
            let mut r = Vec::with_capacity(row.len() + 1);
            r.push(0.0);
            r.extend_from_slice(row);
            r
        })
        .collect();

    let n_c = out[0].len();
    let n_d = out.len();

    for i in 0..n_d {
        for j in 1..n_c {
            let split_total = out[i][j];
            if split_total > 0.0 && is_whole(split_total, c_totals[j - 1]) {
                out[i][0] += split_total;
                out[i][j] = 0.0;
            }
        }
    }
    out
}

/// Weight counties by population share.
pub fn population_weight(county_pop: f64, _n_counties: usize, state_pop: f64) -> f64 {
    county_pop / state_pop
}

/// Don Leake's reverse weighting: `(S - c) / ((n - 1) S)`.
///
/// Weights *small* counties more heavily, on the argument that splitting a
/// small county is the greater offence.
pub fn reverse_weight(county_pop: f64, n_counties: usize, state_pop: f64) -> f64 {
    (state_pop - county_pop) / ((n_counties as f64 - 1.0) * state_pop)
}

pub fn calc_county_weights(county_totals: &[f64], reverse: bool) -> Vec<f64> {
    let n_c = county_totals.len();
    let c_total = naive_sum(county_totals.iter().copied());
    county_totals
        .iter()
        .map(|&pop| {
            if reverse {
                reverse_weight(pop, n_c, c_total)
            } else {
                population_weight(pop, n_c, c_total)
            }
        })
        .collect()
}

pub fn calc_district_weights(district_totals: &[f64]) -> Vec<f64> {
    let d_total = naive_sum(district_totals.iter().copied());
    district_totals.iter().map(|&t| t / d_total).collect()
}

/// Each entry as a fraction of its county's population.
pub fn calc_county_fractions(cxd: &[Vec<f64>], county_totals: &[f64]) -> CxD {
    let n_d = cxd.len();
    let n_c = cxd[0].len();
    let mut f = vec![vec![0.0; n_c]; n_d];
    for j in 0..n_c {
        for i in 0..n_d {
            f[i][j] = if county_totals[j] > 0.0 {
                cxd[i][j] / county_totals[j]
            } else {
                0.0
            };
        }
    }
    f
}

/// Each entry as a fraction of its district's population.
pub fn calc_district_fractions(cxd: &[Vec<f64>], district_totals: &[f64]) -> CxD {
    let n_d = cxd.len();
    let n_c = cxd[0].len();
    let mut g = vec![vec![0.0; n_c]; n_d];
    for j in 0..n_c {
        for i in 0..n_d {
            g[i][j] = if district_totals[i] > 0.0 {
                cxd[i][j] / district_totals[i]
            } else {
                0.0
            };
        }
    }
    g
}

/// Split score for one county, over all districts.
pub fn county_split_score(j: usize, f: &[Vec<f64>]) -> f64 {
    let splits: Vec<f64> = (0..f.len()).map(|i| f[i][j]).collect();
    split_score(&splits)
}

/// Split score for one district, over all counties.
pub fn district_split_score(i: usize, g: &[Vec<f64>]) -> f64 {
    split_score(&g[i])
}

/// Population-weighted sum of county split scores.
pub fn county_splitting(f: &[Vec<f64>], w: &[f64]) -> f64 {
    let num_c = f[0].len();
    let mut e = 0.0;
    for j in 0..num_c {
        e += w[j] * county_split_score(j, f);
    }
    e
}

/// Population-weighted sum of district split scores.
pub fn district_splitting(g: &[Vec<f64>], x: &[f64]) -> f64 {
    let mut e = 0.0;
    for i in 0..g.len() {
        e += x[i] * district_split_score(i, g);
    }
    e
}

/// County splitting with whole districts consolidated away. This is the
/// score DRA reports.
pub fn calc_county_splitting_reduced(
    cxd: &[Vec<f64>],
    district_totals: &[f64],
    county_totals: &[f64],
    reverse: bool,
) -> f64 {
    let r_c = reduce_county_splits(cxd, district_totals);
    let f = calc_county_fractions(&r_c, county_totals);
    let w = calc_county_weights(county_totals, reverse);
    county_splitting(&f, &w)
}

/// Unreduced county splitting. rdapy keeps this for testing only.
pub fn calc_county_splitting(cxd: &[Vec<f64>], county_totals: &[f64]) -> f64 {
    let f = calc_county_fractions(cxd, county_totals);
    let w = calc_county_weights(county_totals, false);
    county_splitting(&f, &w)
}

/// District splitting with whole counties consolidated away.
pub fn calc_district_splitting_reduced(
    cxd: &[Vec<f64>],
    district_totals: &[f64],
    county_totals: &[f64],
) -> f64 {
    let r_d = reduce_district_splits(cxd, county_totals);
    let g = calc_district_fractions(&r_d, district_totals);
    let x = calc_district_weights(district_totals);
    district_splitting(&g, &x)
}

/// Unreduced district splitting. rdapy keeps this for testing only.
pub fn calc_district_splitting(cxd: &[Vec<f64>], district_totals: &[f64]) -> f64 {
    let g = calc_district_fractions(cxd, district_totals);
    let x = calc_district_weights(district_totals);
    district_splitting(&g, &x)
}
