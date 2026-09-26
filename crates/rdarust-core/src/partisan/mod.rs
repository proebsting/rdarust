//! Partisan bias and responsiveness, ported from `rdapy/partisan/`.
//!
//! The metrics implement John Nagle's method: district vote shares are turned
//! into fractional seat probabilities, and a seats-votes curve is inferred by
//! shifting the statewide vote share across `[0.25, 0.75]`.
//!
//! All vote shares here are **Democratic** shares, and the sign convention
//! throughout is rdapy's: `+` means Republican bias, `-` means Democratic.
//!
//! Several metrics are genuinely undefined for some plans -- declination for a
//! sweep, big R at exactly 50% -- and rdapy returns `None`. Those return
//! [`Option`] here.

pub mod bias;
pub mod method;
pub mod more;
pub mod responsiveness;

/// rdapy's `EPSILON`, used as a nearness threshold rather than a machine
/// epsilon.
pub const EPSILON: f64 = 1.0 / 1_000_000.0;

/// rdapy's `roughly_equal`: a strict `<` comparison, not `<=`.
#[inline]
pub fn roughly_equal(x: f64, y: f64, tolerance: f64) -> bool {
    (x - y).abs() < tolerance
}

/// `statistics.mean`.
///
/// CPython's `statistics.mean` sums exactly (as rationals) before dividing, so
/// it is correctly rounded -- unlike the builtin `sum(xs) / len(xs)`, which
/// accumulates error. Neumaier summation reproduces it.
///
/// The distinction matters: rdapy uses `statistics.mean` for turnout bias and
/// the mean-median difference, but the *builtin* `sum` for `est_seats` and
/// the average margin. Those must stay naive.
pub fn exact_mean(xs: &[f64]) -> f64 {
    neumaier_sum(xs) / xs.len() as f64
}

/// Compensated summation, recovering the low-order bits that plain
/// accumulation drops.
pub fn neumaier_sum(xs: &[f64]) -> f64 {
    let mut sum = 0.0f64;
    let mut c = 0.0f64;
    for &x in xs {
        let t = sum + x;
        if sum.abs() >= x.abs() {
            c += (sum - t) + x;
        } else {
            c += (x - t) + sum;
        }
        sum = t;
    }
    sum + c
}

/// Plain left-to-right summation, matching Python's builtin `sum`.
///
/// Deliberately *not* compensated. Where rdapy uses the builtin, reproducing
/// its accumulation order reproduces its rounding.
#[inline]
pub fn naive_sum(xs: impl IntoIterator<Item = f64>) -> f64 {
    xs.into_iter().fold(0.0, |a, b| a + b)
}

/// `statistics.median`: the middle value, or the mean of the two middle
/// values for an even count.
pub fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("median of NaN"));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}
