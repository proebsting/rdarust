//! Community-of-interest splitting, using Sam Wang's metrics.
//! Ported from `rdapy/splitting/coi.py`.
//!
//! `splits` is a community's population distributed across districts, as
//! fractions summing to 1.

/// Shannon entropy of the split, in bits.
///
/// 0 for an intact community; log2(k) when spread evenly over k districts.
pub fn uncertainty_of_membership(splits: &[f64]) -> f64 {
    let result: f64 = -splits
        .iter()
        .filter(|&&x| x > 0.0)
        .map(|&x| x * x.log2())
        .sum::<f64>();

    // rdapy normalises away the negative zero that -1 * 0.0 produces.
    if result == 0.0 {
        0.0
    } else {
        result
    }
}

/// The effective number of extra pieces a community was broken into.
///
/// 0 for an intact community; k-1 when spread evenly over k districts.
pub fn effective_splits(splits: &[f64]) -> f64 {
    let sum_sq: f64 = splits.iter().filter(|&&x| x > 0.0).map(|&x| x * x).sum();
    (1.0 / sum_sq) - 1.0
}

/// Both metrics for one community.
#[derive(Debug, Clone, PartialEq)]
pub struct CoiSplitting {
    pub name: String,
    pub effective_splits: f64,
    pub uncertainty: f64,
}

pub fn calc_coi_splitting(communities: &[(String, Vec<f64>)]) -> Vec<CoiSplitting> {
    communities
        .iter()
        .map(|(name, splits)| CoiSplitting {
            name: name.clone(),
            effective_splits: effective_splits(splits),
            uncertainty: uncertainty_of_membership(splits),
        })
        .collect()
}
