//! Numeric behaviours inherited from CPython and NumPy.
//!
//! Each function here has a Rust-standard-library lookalike that is subtly
//! *different*, and using the lookalike silently corrupts downstream scores.
//! The differences are spelled out on each function.

/// CPython's `round(x)`: round half to **even**, not half away from zero.
///
/// `f64::round` rounds half away from zero, so `round(0.5)` is 1.0 there but
/// 0.0 in Python. This is observable in every DRA 0-100 rating, which are
/// produced by `Normalizer::rescale` as `round(unit_value * 100)`.
#[inline]
pub fn python_round(x: f64) -> f64 {
    let r = x.round_ties_even();
    // CPython's round(x) returns an *int*, which has no negative zero:
    // round(-0.5) is 0, not -0.0. f64::round_ties_even yields -0.0 here.
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

/// CPython's `round(x)` returning an integer, as the ratings code needs.
#[inline]
pub fn python_round_i64(x: f64) -> i64 {
    python_round(x) as i64
}

/// CPython's `round(x, ndigits)`: correctly-rounded to `ndigits` decimal
/// places with ties going to even.
///
/// This rounds the *decimal* value, so it cannot be done as
/// `(x * 10f64.powi(n)).round() / 10f64.powi(n)` -- that introduces a second
/// rounding error. Formatting and reparsing gives CPython's answer because
/// both directions are correctly rounded.
///
/// `trim_scores(precision=4)` applies this to every reported score.
pub fn python_round_to(x: f64, ndigits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    // format! with a precision is correctly rounded, ties to even; parse is
    // correctly rounded. The round trip therefore matches CPython's
    // _Py_dg_dtoa / _Py_dg_strtod pair.
    format!("{:.*}", ndigits, x).parse::<f64>().unwrap_or(x)
}

/// NumPy's `np.arange` for floats.
///
/// The obvious implementations are both wrong, and each one happens to be
/// right for *some* inputs, which makes this easy to get away with until it
/// silently isn't:
///
/// * `start + i * step` is wrong whenever `(start + step) - start != step`.
/// * Accumulating `v += step` is wrong whenever it does equal `step`.
///
/// NumPy's actual `DOUBLE_fill` writes the first two entries, recomputes the
/// stride from them, and multiplies out from there:
///
/// ```text
/// v[0] = start
/// v[1] = start + step
/// v[i] = start + i * delta,  where delta = v[1] - v[0]
/// ```
///
/// The length is `ceil((stop - start) / step)` evaluated in floating point.
///
/// `shift_range()` feeds every seats-votes curve in the partisan suite, so an
/// error here propagates into seats bias, votes bias and global symmetry.
pub fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let raw_len = ((stop - start) / step).ceil();
    if !raw_len.is_finite() || raw_len <= 0.0 {
        return Vec::new();
    }
    let n = raw_len as usize;
    let mut out = Vec::with_capacity(n);

    out.push(start);
    if n == 1 {
        return out;
    }
    let second = start + step;
    out.push(second);

    let delta = second - start;
    for i in 2..n {
        out.push(start + (i as f64) * delta);
    }
    out
}

/// `shift_range()` from `rdapy.partisan.utils`: vote shifts across the middle
/// of the seats-votes curve in half-percent steps, 101 points.
///
/// Note that 0.5 is *not* exactly present in the result -- the accumulated
/// midpoint is 0.5000000000000002 -- which is why `est_seats_bias` locates it
/// with `math.isclose` rather than equality.
pub fn shift_range() -> Vec<f64> {
    let lower = 25.0 / 100.0;
    let upper = 75.0 / 100.0;
    let step = (1.0 / 100.0) / 2.0;
    let epsilon = 1.0e-12;
    arange(lower, upper + epsilon, step)
}

/// CPython's `math.isclose`.
///
/// Both tolerance terms are live. rdapy's splitting reductions call this with
/// `abs_tol=1e-6` but leave `rel_tol` at its default `1e-9`, so an
/// implementation that honours only `abs_tol` is wrong for large populations.
#[inline]
pub fn isclose(a: f64, b: f64, rel_tol: f64, abs_tol: f64) -> bool {
    if a == b {
        return true;
    }
    if a.is_infinite() || b.is_infinite() {
        return false;
    }
    let diff = (b - a).abs();
    diff <= (rel_tol * b).abs() || diff <= (rel_tol * a).abs() || diff <= abs_tol
}

/// `math.isclose` with CPython's default tolerances.
#[inline]
pub fn isclose_default(a: f64, b: f64) -> bool {
    isclose(a, b, 1e-9, 0.0)
}

/// The error function.
///
/// Not in Rust's standard library. `est_seat_probability` -- the most-used
/// formula in the partisan suite -- is
/// `0.5 * (1 + erf((Vf - 0.5) / (0.02 * sqrt(8))))`.
#[inline]
pub fn erf(x: f64) -> f64 {
    libm::erf(x)
}

/// rdapy's `approx_equal`: `|x - y| <= 0.5 * 10^-places`.
///
/// This is `pytest.approx(y, abs=...)`, which applies **only** the absolute
/// tolerance when no relative tolerance is given -- no hidden relative term.
#[inline]
pub fn approx_equal(x: f64, y: f64, places: i32) -> bool {
    (x - y).abs() <= 10f64.powi(-places) * 0.5
}
