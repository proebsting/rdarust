# Known differences from rdapy

Every place this port deliberately behaves differently from
[dra2020/rdapy](https://github.com/dra2020/rdapy), and why.

The default is bug-for-bug compatibility: if rdapy produces a value, this
produces the same value. Deviation happens only where rdapy raises, or where
matching would mean reproducing an implementation accident rather than a
behaviour.

Acceptance bar: floats agree to ~1e-9 relative; integers and the 0-100 DRA
ratings agree exactly.

---

## 1. Minimum enclosing circle: deterministic, no retry loop

**rdapy** (`compactness/smallestenclosingcircle.py`) dedupes its input through
a `set`, shuffles with a module-global RNG seeded once at import
(`random.seed(42)`), and retries up to ten times when three points come out
colinear.

Because the RNG is global and shared, results depend on how many prior calls
consumed RNG state -- so scoring district 5 depends on districts 1-4 having
been scored first, in that order.

**Here:** a fixed-seed xorshift permutation, and colinear triples resolved
directly (the enclosing circle is then fixed by the two extreme points) rather
than by reshuffling.

**Why this is safe:** the minimum enclosing circle is mathematically unique.
Welzl's algorithm is correct under *any* permutation; randomisation only buys
expected-linear running time. The permutation therefore affects floating-point
noise and speed, not the answer.

**Measured:** worst relative error 1.2e-16 across the corpus, and bit-identical
to rdapy in 10 of 11 cases.

**Consequence:** results here are reproducible and order-independent, which
rdapy's are not. This is also what makes it safe to score plans in parallel.

## 2. Minimum enclosing circle: tolerant containment test

rdapy tests `hypot(dx, dy) <= r` with no slack, which can spin on degenerate
input. This uses a small relative slack (1e-13). Since the circle is unique,
slack changes only how much redundant work the search does.

## 3. Cubic spline out of range is an error, not an extrapolation

`scipy.interp1d` defaults to `bounds_error=True`, so rdapy raises a
`ValueError` if `est_votes_bias` or `est_geometric_seats_bias` is queried
outside the interpolation range. `CubicSpline::eval` returns
`Err(SplineError::OutOfBounds)` rather than extrapolating, which is the same
observable behaviour in Rust's idiom.

## 4. `python_round(-0.5)` returns `+0.0`

CPython's `round(x)` with no `ndigits` returns an **int**, which has no signed
zero: `round(-0.5)` is `0`. Rust's `f64::round_ties_even()` returns `-0.0`.
`python_round` normalises negative zero away to match.

---

# Rust-side hazards

Not differences from rdapy -- traps in the Rust ecosystem that would have
silently produced wrong answers.

## serde_json's default float parser loses the last bit

`serde_json` parses `0.39500000000000013` as `0.3950000000000002` -- one ulp
off -- where `str::parse::<f64>` is correct. Left unfixed this would have
corrupted every area, arc length, centroid and boundary coordinate read from
the input JSONL, before any scoring logic ran.

**Fix:** every crate that touches `serde_json` enables the `float_roundtrip`
feature. Do not remove it.

## `f64::round` is not Python's `round`

Rust rounds half away from zero; Python rounds half to even. `round(2.5)` is
3.0 in Rust and 2 in Python. This is directly observable in the 0-100 ratings,
which are `round(unit_value * 100)`. Use `numeric::python_round`. There is a
test that fails if someone "simplifies" it back.

## `erf` is not in Rust's standard library

Supplied by the `libm` crate. `est_seat_probability` -- the most-used formula
in the partisan suite -- is built on it.
