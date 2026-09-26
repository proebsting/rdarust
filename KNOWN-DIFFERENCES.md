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

---

# Deviations added with the formula layer

## 5. Normalizer invariants return an error rather than raising

rdapy's `Normalizer` asserts that a value is in `[0, 1]` before inverting,
decaying or rescaling it, and `score_plans` catches the resulting
`AssertionError` and skips the plan. `rate::*` returns `RateError` instead.
Same observable outcome, no panic. On finite inputs the ratings cannot fail.

## 6. Undefined metrics are `Option`, not a None-or-float union

`calc_declination` (sweep, fewer than five districts, or a winning average
sitting exactly at 50%), `calc_lopsided_outcomes` (sweep), `calc_big_R` (tied
statewide vote) and `calc_minimal_inverse_responsiveness` (unresponsive plan)
all return `None` in rdapy. They return `Option<f64>` here.

## 7. `est_minority_opportunity` does not raise on a negative share

rdapy asserts `mf >= 0.0`. Here that is a `debug_assert!`. A negative share is
unreachable from the pipeline -- shares are counts over counts -- and the
realistic bad input is NaN, which propagates through
[`numeric::python_min`]/[`python_max`] exactly as it does in Python.

## 8. `connected_subsets` returns a deterministic order

rdapy returns a list of Python sets, so both the order of the components and
the order within each is whatever set iteration produces. Components come out
here ordered by first appearance with members sorted, so the result is stable
across runs.

## 9. `calc_energy` measures distance in degrees

Reproduced from rdapy, and worth knowing about: the population-compactness
metric squares raw differences in longitude and latitude, so a degree of
longitude counts the same as a degree of latitude regardless of the state's
latitude. At North Carolina's latitude a degree of longitude is about 82% of a
degree of latitude, so the metric is not isotropic. Not corrected -- it is a
relative measure and changing it would change every published score.

## 10. `spanning_tree_score` returns `None` where rdapy returns `-inf`

For a disconnected graph the reduced Laplacian is singular. rdapy returns
`float("-inf")`; this returns `None`.

---

# Deviations added with the pipeline layer

## 11. County-district matrix columns are in sorted order

rdapy derives its county ordering from a Python `set`, so the columns of the
`CxD` matrix come out in whatever order set iteration produces. Columns here
are sorted by FIPS code.

Column order changes only the order the weighted sums accumulate in, so the
splitting scores differ by floating-point noise (measured at ~1e-16). Counts
-- counties split, total splits -- are order-independent. The matrix itself is
not part of the published output: rdapy drops it from the aggregates before
writing them.

## 12. Boundary points are collected once per precinct, not once per neighbour

Building a district's boundary, rdapy appends a precinct's entire convex hull
once for *each* neighbour in another district, so a precinct bordering five
others contributes its hull five times. The enclosing-circle routine dedupes
its input, so the circle is identical either way; this appends it once.

## 13. A district with no boundary is an error, not an infinite Reock

If a district's boundary comes out empty its diameter is zero, and Reock --
area over the area of the bounding circle -- is infinity, which then flows
into the plan's average and its 0-100 compactness rating.

In practice this means the adjacency graph was not loaded: input files written
for the scoring pipeline carry no neighbour lists, because the graph is
maintained separately. rdapy scores such a plan and reports the result.
`AggregateError::DistrictHasNoBoundary` is returned instead.
