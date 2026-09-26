//! Estimated minority opportunity, ported from `rdapy/minority/minority.py`.

use crate::numeric::{python_max, python_min, python_round};
use crate::partisan::method::est_seat_probability;
use crate::partisan::naive_sum;

/// The demographics rdapy tracks, in the order it iterates them.
///
/// The order is load-bearing: rdapy sums opportunity over a `defaultdict`
/// populated in this sequence, and floating-point addition is not
/// associative, so a different order gives a different last bit.
pub const DEMOGRAPHICS: [&str; 7] = [
    "white", "minority", "black", "hispanic", "pacific", "asian", "native",
];

/// Voting-age-population shares for one district, or statewide.
///
/// A struct rather than a map: the scoring loop touches this per district per
/// plan, and string hashing there is pure overhead.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DemoShares {
    pub white: f64,
    pub minority: f64,
    pub black: f64,
    pub hispanic: f64,
    pub pacific: f64,
    pub asian: f64,
    pub native: f64,
}

impl DemoShares {
    /// Read a share by its index in [`DEMOGRAPHICS`].
    pub fn get(&self, idx: usize) -> f64 {
        match idx {
            0 => self.white,
            1 => self.minority,
            2 => self.black,
            3 => self.hispanic,
            4 => self.pacific,
            5 => self.asian,
            6 => self.native,
            _ => panic!("demographic index {idx} out of range"),
        }
    }

    pub fn set(&mut self, idx: usize, v: f64) {
        match idx {
            0 => self.white = v,
            1 => self.minority = v,
            2 => self.black = v,
            3 => self.hispanic = v,
            4 => self.pacific = v,
            5 => self.asian = v,
            6 => self.native = v,
            _ => panic!("demographic index {idx} out of range"),
        }
    }
}

/// The whole number of districts a statewide share would make proportional.
pub fn calc_proportional_districts(proportion: f64, n_districts: i32) -> i32 {
    python_round(proportion * n_districts as f64) as i32
}

/// The probability that a district with minority share `mf` elects a
/// minority-preferred candidate.
///
/// Minority shares are shifted up before being run through the seat-probability
/// curve, so a 37% Black district scores like a 52% one -- roughly a 70%
/// chance. Demographics other than Black (and the combined minority figure)
/// get half the shift.
///
/// With `clip`, anything below 37% scores zero outright; DRA's revised
/// ratings pass `clip = false` and keep the smooth tail.
pub fn est_minority_opportunity(mf: f64, demo: Option<&str>, clip: bool) -> f64 {
    debug_assert!(!(mf < 0.0), "minority share must be non-negative, got {mf}");

    let low_end = 0.37;
    let mut shift = 0.15;
    let dilution = 0.50;
    if let Some(d) = demo {
        if d != "black" && d != "minority" {
            shift *= dilution;
        }
    }

    let wip = mf + shift;

    if clip {
        if mf < low_end {
            0.0
        } else {
            python_min(est_seat_probability(wip), 1.0)
        }
    } else {
        python_max(python_min(est_seat_probability(wip), 1.0), 0.0)
    }
}

/// Opportunity and coalition district counts, against what would be
/// proportional.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinorityMetrics {
    /// Expected districts where some single minority has opportunity.
    pub opportunity_districts: f64,
    /// Districts that would be proportional, summed over single minorities.
    pub proportional_opportunities: i32,
    /// Expected districts where minorities combined have opportunity.
    pub coalition_districts: f64,
    /// Districts that would be proportional for minorities combined.
    pub proportional_coalitions: i32,
}

pub fn calc_minority_metrics(
    statewide: &DemoShares,
    by_district: &[DemoShares],
    clip: bool,
) -> MinorityMetrics {
    let n_districts = by_district.len() as i32;

    // Indices 1..7: everything but "white".
    let tracked = 1..DEMOGRAPHICS.len();

    let mut proportional = [0i32; DEMOGRAPHICS.len()];
    for d in tracked.clone() {
        proportional[d] = calc_proportional_districts(statewide.get(d), n_districts);
    }

    let mut oppty = [0.0f64; DEMOGRAPHICS.len()];
    for district in by_district {
        for d in tracked.clone() {
            oppty[d] += est_minority_opportunity(district.get(d), Some(DEMOGRAPHICS[d]), clip);
        }
    }

    // "minority" is the coalition figure and is excluded from the per-demographic
    // totals; summed in DEMOGRAPHICS order, as rdapy does.
    let singles: Vec<usize> = tracked.filter(|&d| DEMOGRAPHICS[d] != "minority").collect();

    MinorityMetrics {
        opportunity_districts: naive_sum(singles.iter().map(|&d| oppty[d])),
        proportional_opportunities: singles.iter().map(|&d| proportional[d]).sum(),
        coalition_districts: oppty[1],
        proportional_coalitions: proportional[1],
    }
}
