//! The five families of score, ported from `rdapy/score/categories.py`.

use crate::aggregate::Aggregates;
use crate::compactness::{polsby_formula, reock_formula};
use crate::equal::calc_population_deviation;
use crate::minority::opportunity::{
    calc_minority_metrics, DemoShares, MinorityMetrics, DEMOGRAPHICS,
};
use crate::partisan::metrics::{calc_partisan_metrics, PartisanError};
use crate::partisan::more::calc_efficiency_gap_wasted_votes;
use crate::partisan::naive_sum;
use crate::score::ElectionScores;
use crate::splitting::county;

/// Population deviation: the spread between largest and smallest district as
/// a fraction of the ideal.
///
/// `pop_by_district[0]` is the statewide total, `1..=n` the districts.
pub fn general_category(pop_by_district: &[i64], n_districts: usize) -> f64 {
    let districts = &pop_by_district[1..=n_districts];
    let max_pop = *districts.iter().max().expect("a district");
    let min_pop = *districts.iter().min().expect("a district");
    // rdapy truncates rather than rounds the ideal district size.
    let target = (pop_by_district[0] as f64 / n_districts as f64).trunc() as i64;
    calc_population_deviation(max_pop, min_pop, target)
}

/// Every partisan score for one election.
///
/// Ratings are left at zero; [`crate::context::Context::score_aggregates`]
/// fills them in, because they depend on the district count.
pub fn partisan_category(
    dataset: &str,
    dem_by_district: &[i64],
    tot_by_district: &[i64],
    baseline_whole_seats: Option<f64>,
) -> Result<ElectionScores, PartisanError> {
    let total_d = dem_by_district[0];
    let total = tot_by_district[0];
    let dem = &dem_by_district[1..];
    let tot = &tot_by_district[1..];
    let rep: Vec<i64> = tot.iter().zip(dem.iter()).map(|(t, d)| t - d).collect();

    let vf = total_d as f64 / total as f64;
    let vf_array: Vec<f64> = dem
        .iter()
        .zip(tot.iter())
        .map(|(&d, &t)| d as f64 / t as f64)
        .collect();

    let m = calc_partisan_metrics(vf, &vf_array)?;

    Ok(ElectionScores {
        dataset: dataset.to_string(),
        estimated_vote_pct: vf,
        pr_deviation: m.deviation,
        estimated_seats: m.estimated_seats,
        fptp_seats: m.fptp_seats,
        disproportionality: m.disproportionality,
        efficiency_gap: m.efficiency_gap,
        efficiency_gap_fptp: m.efficiency_gap_fptp,
        // Needs actual vote counts rather than shares, so it sits outside
        // calc_partisan_metrics.
        efficiency_gap_wasted_votes: calc_efficiency_gap_wasted_votes(dem, &rep),
        seats_bias: m.seats_bias,
        votes_bias: m.votes_bias,
        geometric_seats_bias: m.geometric_seats_bias,
        declination: m.declination,
        mean_median_statewide: m.mean_median_statewide,
        mean_median_average_district: m.mean_median_average_district,
        turnout_bias: m.turnout_bias,
        lopsided_outcomes: m.lopsided_outcomes,
        competitive_district_count: m.competitive_district_count,
        competitive_districts: m.competitive_districts,
        average_margin: m.average_margin,
        responsiveness: m.little_r,
        responsive_districts: m.responsive_districts,
        overall_responsiveness: m.big_r,
        geographic_advantage: baseline_whole_seats.map(|w| m.estimated_seats - w),
        proportionality: 0,
        competitiveness: 0,
    })
}

/// Minority opportunity from voting-age population counts.
///
/// `names` are the metadata field keys in order; the first is the total and
/// the rest are shares of it. rdapy reduces each to its leading word --
/// `black_vap` becomes `black` -- to line up with [`DEMOGRAPHICS`].
pub fn minority_category(
    vap_by_district: &[Vec<i64>],
    names: &[String],
    n_districts: usize,
) -> MinorityMetrics {
    let demo_index = |name: &str| -> Option<usize> {
        let simple = name.split('_').next()?.to_lowercase();
        DEMOGRAPHICS.iter().position(|d| *d == simple)
    };

    let total = &vap_by_district[0];

    let mut statewide = DemoShares::default();
    for (k, name) in names.iter().enumerate().skip(1) {
        if let Some(idx) = demo_index(name) {
            statewide.set(idx, vap_by_district[k][0] as f64 / total[0] as f64);
        }
    }

    let mut by_district = Vec::with_capacity(n_districts);
    for d in 1..=n_districts {
        let mut shares = DemoShares::default();
        for (k, name) in names.iter().enumerate().skip(1) {
            if let Some(idx) = demo_index(name) {
                shares.set(idx, vap_by_district[k][d] as f64 / total[d] as f64);
            }
        }
        by_district.push(shares);
    }

    // The revised ratings keep the smooth tail below 37% rather than clipping.
    calc_minority_metrics(&statewide, &by_district, false)
}

/// Reock and Polsby-Popper by district, written into the aggregates.
///
/// Element 0 of each becomes the plan-wide average, which is what gets rated.
pub fn compactness_category(aggs: &mut Aggregates, n_districts: usize) {
    let mut tot_reock = 0.0;
    let mut tot_polsby = 0.0;

    for d in 1..=n_districts {
        let reock = reock_formula(aggs.area[d], aggs.diameter[d] / 2.0);
        let polsby = polsby_formula(aggs.area[d], aggs.perimeter[d]);
        aggs.reock[d] = reock;
        aggs.polsby_popper[d] = polsby;
        tot_reock += reock;
        tot_polsby += polsby;
    }

    aggs.reock[0] = tot_reock / n_districts as f64;
    aggs.polsby_popper[0] = tot_polsby / n_districts as f64;
}

/// County and district splitting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplittingCategory {
    pub county_splitting: f64,
    pub district_splitting: f64,
    /// Counties divided between two or more districts.
    pub counties_split: i32,
    /// Total splits, counting a county in `k` districts as `k - 1`.
    pub county_splits: i32,
}

/// Compute splitting, writing the per-district split scores into
/// `district_splitting` with element 0 holding the plan-wide score.
pub fn splitting_category(
    cxd: &[Vec<f64>],
    reverse_weight: bool,
    district_splitting: &mut [f64],
) -> SplittingCategory {
    let metrics = county::calc_splitting_metrics(cxd, reverse_weight);

    // Rows are districts, columns counties.
    let mut counties_split = 0;
    let mut county_splits = 0;
    for j in 0..cxd[0].len() {
        let parts = cxd.iter().filter(|row| row[j] > 0.0).count() as i32;
        if parts > 1 {
            counties_split += 1;
            county_splits += parts - 1;
        }
    }

    // rdapy recomputes these intermediates rather than threading them out of
    // calc_splitting_metrics; so does this, for the same reason.
    let d_t = county::district_totals(cxd);
    let c_t = county::county_totals(cxd);
    let r_d = county::reduce_district_splits(cxd, &c_t);
    let g = county::calc_district_fractions(&r_d, &d_t);

    district_splitting[0] = metrics.district;
    for i in 0..g.len() {
        district_splitting[i + 1] = county::district_split_score(i, &g);
    }

    SplittingCategory {
        county_splitting: metrics.county,
        district_splitting: metrics.district,
        counties_split,
        county_splits,
    }
}

/// Sum of a slice with plain accumulation, matching Python's builtin `sum`.
#[allow(dead_code)]
fn sum(xs: &[f64]) -> f64 {
    naive_sum(xs.iter().copied())
}
