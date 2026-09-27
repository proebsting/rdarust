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

---

# Deviations added with the CLI

## 14. A bad record stops the run

rdapy's `score_plans` and `aggregate_plans` wrap each input line in a bare
`except Exception`, log to stderr, and continue. A real data error -- the
`ValueError` raised when a populated precinct is missing from the plan --
therefore drops that plan from the output, and the only evidence is that the
CSV has fewer rows than the input.

`rdarust` stops, naming the line and the cause. `--continue-on-error`
restores rdapy's behaviour for diffing against it, and reports how many
records were skipped at exit so a silent drop stays visible.

## 15. Scoring runs in parallel

`score-all` scores plans across cores. Results are collected in input order,
so the output does not depend on the thread count, and no scoring state is
shared: the enclosing-circle routine here is deterministic and
order-independent, unlike rdapy's, whose shared RNG would make parallel
results depend on scheduling.

---

# Byte compatibility

These outputs are byte-identical to rdapy's, and a test checks it:

* the scores CSV, including column order, six-decimal float formatting and
  CRLF line endings;
* the `_metadata.json` written beside it, including its four-space indent and
  absence of a trailing newline.

The by-district JSONL is semantically identical and compared structurally
rather than byte for byte, which was a deliberate choice: matching it exactly
would mean reimplementing CPython's float `repr`, and the file is read by
programs rather than diffed by people.

---

# Deviations added with extraction

## 16. Shared borders are found by matching segments, not by clipping polygons

rdapy computes the border two precincts share as
`a.intersection(b).length` -- a full polygon clip, then the length of what
comes back. This matches their boundary *segments* instead and sums the ones
they both carry.

The two agree because precincts form a coverage: adjacent ones are cut from
the same boundary and carry bit-identical vertices. Measured against rdapy's
own output for all 15,058 adjacent pairs in North Carolina, the largest
disagreement is 7.8e-16 degrees.

It is also the more robust of the two. A clipping engine has to decide where
near-coincident edges intersect, which is where such engines go wrong;
segment matching has no such decision to make, and it needs no geometry
library at all.

**Where this would break:** a coverage with T-junctions, where one side of a
shared edge carries a vertex the other does not. The segments then differ and
part of the shared border is missed. `Coverage::overshared_segments` reports
the related pathology of overlapping shapes, and `extract-data` warns when it
finds any. Before relying on this for a state other than North Carolina, check
that the extracted arcs still sum to each precinct's perimeter.

## 17. The centroid calculation is skipped

`abstract_shape` computes a centre -- a centroid, falling back to a
representative point when the centroid falls outside the shape -- and
`extract_data.py` then overwrites it unconditionally with DRA's label
coordinates. Nothing can observe the result, so it is not computed. This also
removes the only need for point-in-polygon testing.

## 18. Neighbour order within the graph may differ

rdapy's neighbour lists come out of `libpysal`; these come out of the segment
index. The sets are identical -- verified for all 2,667 North Carolina nodes
-- but the order within a list can differ.

Order reaches one thing: the order per-neighbour arc lengths are summed when
computing the leftover state border. That is floating-point noise. It does not
reach the graph's meaning, and scores from the two orderings are byte-identical.

---

# Deviations added with shape compactness

## 19. The minimum bounding rectangle is the real minimum

rdapy's `minimum_bounding_rectangle` builds its list of candidate edge
directions like this:

```python
hull_points = points[ConvexHull(points).vertices]
edges = hull_points[1:] - hull_points[:-1]
```

`ConvexHull.vertices` is an *unclosed* list, so the hull's closing edge --
from the last vertex back to the first -- never appears. A minimum-area
rectangle always has a side flush with some hull edge, so dropping one edge
can drop the answer.

It does. For shape 16 of the `first20` set, the omitted edge is exactly the
one giving the true minimum, and rdapy's rectangle comes out 0.77% too large.

This computes the true minimum over every hull edge. Reproducing rdapy's
result instead would mean reproducing which vertex qhull happens to start
from, which is not a property worth depending on.

**Effect:** the `bbox` feature differs by up to 0.5%, which moves a KIWYSI
rank by under a tenth of a point on a 1-100 scale. Measured against the model
authors' published predictions -- which is what rdapy's own tests check, to
within one whole rank -- the ranks here are within 0.02 and 0.09.

## 20. The geodesic diameter is rdapy's approximation, reproduced

`_get_geodesic_attributes_poly` finds the enclosing circle in *degree* space,
then measures geodesically across it along each axis and keeps the larger.
That is not a geodesic diameter; at 35N a degree of longitude is about 82% of
a degree of latitude, so the circle is not a circle on the ellipsoid.
Reproduced as-is, since it feeds the published Reock figures.

---

# Dependencies

`rdarust-core` depends only on `libm`, for `erf`. `rdarust-geo` needs no
dependencies for extraction; its `shapes` feature -- shape-based compactness
and the KIWYSI model -- adds `geo` for polygon union and `geographiclib-rs`
for geodesic measurement. Both are pure Rust: there is no C dependency
anywhere, and nothing links GEOS.

The union is the only operation needing a clipping engine, and `i_overlay`
(under `geo`) agrees with GEOS to 6e-9 relative on the symmetry features
across all 53 test shapes.

---

# Deviations added with the format converters

## 21. `from-canonical` keys assignments in precinct order

rdapy's `canonical_to_assignments.py` builds each plan by grouping precincts
into a `dict[int, set[int]]` by district and then flattening, so the output
keys come out in an order CPython's set iteration decides -- roughly precinct
index modulo the set's table size.

This iterates the assignment list directly, giving precinct order. The
assignments are identical: same geoids, same districts, same plan names,
verified across all 101 canonical test plans. Only the key order in the JSONL
differs, and nothing downstream reads it in order.

Note that rdapy's script does not currently run at all against GerryChain
1.0.0; see RDAPY-FINDINGS.md 1.2. The comparison above is against a corrected
reference implementing what the script intends.

## 22. Records are written with Python's JSON separators

Not a deviation but worth recording, since it is deliberate and load-bearing:
`json.dump` with no `indent` separates items with `", "` and keys from values
with `": "`, where serde_json writes neither. `rdarust-io` supplies a
formatter matching Python's, so the JSONL the stages exchange can be diffed
directly against rdapy's.

With it, `from-json`, `from-csvs` and `sample` are byte-identical to rdapy's
output. The by-district JSONL is identical but for last-ulp float differences
arising from county-column accumulation order (deviation 11).

---

# Deviations added with the geographic baseline and contiguity repair

## 23. The distance cache is gone

rdapy keeps a `DistanceLedger` caching the squared distance between each pair
of precincts. The quantity is a subtraction and two multiplications, so
computing it costs less than looking it up. Dropped; results are unchanged,
since it was a speed optimisation rather than a semantic one.

## 24. The connectivity assertion is not re-run on every step

`_nearest_connected_neighbor` asserts, on every precinct it yields, that the
set yielded so far is connected. The set grows to a district's worth of
precincts, and the check walks all of it, so the assertion dominates the run
time -- rdapy takes 67s for North Carolina, or 4.8s under `python -O`, with
identical output either way.

The property it checks holds by construction: a precinct is only ever taken
from the frontier of what has already been taken. It is not re-checked here.
The neighbourhood output is byte-identical.

## 25. Contiguity mods come out ordered by precinct

rdapy reports its spanning-tree edges ordered by island number, and island
numbers come from `connected_subsets`, which returns Python sets -- so the
order varies. These are ordered by the precincts they name, so the file is
stable and reviewable. The set of edges is identical; verified on a five-island
test case.

## 26. A mods row that is not an addition is rejected

`apply_contiguity_mods.py` reads the operation column and then ignores it, so
a row written as `-,a,b` adds the edge. Here only `+` (or an empty column) is
accepted, and anything else is an error naming the line. Applying a mods file
that only ever used `+` behaves identically.
