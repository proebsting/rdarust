# Findings from porting rdapy to Rust

Notes for the rdapy maintainers, gathered while building
[rdarust](https://github.com/proebsting/rdarust), a line-by-line port of
rdapy's analytics.

Everything here was found by making the two implementations agree to 1e-9 on
real data and chasing down every place they did not. All line numbers are
against **`2b702b4`** ("Tweaked doc"), verified with Python 3.13.15, NumPy
2.5.3, SciPy 1.18.1, shapely 2.1.2.

A note on tone: this is a list of things worth a look, not a list of things
that are wrong with rdapy. The port reproduces rdapy's results to
floating-point noise across ~8,500 conformance cases, and the scores CSV it
produces is byte-identical. Most of what follows is small.

---

## 1. Defects that change results

### 1.1 `minimum_bounding_rectangle` can miss the minimum

`rdapy/compactness/pypoly/miniumareaboundingrectangle.py:28`

```python
hull_points = points[ConvexHull(points).vertices]
edges = hull_points[1:] - hull_points[:-1]
```

`ConvexHull.vertices` is an **unclosed** vertex list, so the hull's closing
edge -- from the last vertex back to the first -- never enters `edges`, and
its direction is never tried.

A minimum-area enclosing rectangle always has a side flush with some hull
edge, so skipping an edge can skip the answer. It does: for shape `16` of
`testdata/compactness/first20`, the omitted closing edge is exactly the one
giving the minimum, and the rectangle returned is **0.77% larger than the true
minimum** (0.78627382 against 0.78020531).

Effect: the `bbox` feature is up to ~0.5% low, which moves a KIWYSI rank by
under a tenth of a point on a 1-100 scale. Small, but it is a wrong answer
rather than a rounding difference, and it will vary unpredictably with qhull's
choice of starting vertex.

Suggested fix -- close the ring before differencing:

```python
hull_points = points[ConvexHull(points).vertices]
closed = np.vstack([hull_points, hull_points[:1]])
edges = closed[1:] - closed[:-1]
```

### 1.2 `canonical_to_assignments.py` is broken on GerryChain 1.0

`scripts/formats/canonical_to_assignments.py:37`

```python
recom_graph.nodes[node].get(args.geoid) for node in list(recom_graph.nodes())
```

GerryChain 1.0.0 made `Graph.nodes` a property rather than a method, so
calling it raises immediately:

```
TypeError: As of GerryChain version 1.0.0, `Graph.nodes` is a property, not a
method. Use `graph.nodes` without parentheses; use `graph.node_data(node_id)`
for node attributes.
```

`requirements.txt` pins nothing, so a fresh install gets 1.0.0 and the script
cannot run at all. Dropping the parentheses fixes it. GerryChain is imported
by this one script and nothing else, so nothing else is affected.

### 1.3 A missing adjacency graph yields an infinite Reock

Input files written for the scoring pipeline carry no `neighbors` key -- the
graph is a separate file, which is good practice, since it can be corrected
without rewriting the data. But `aggregate_shapes_by_district` reads adjacency
from the graph it is handed, and if that graph is empty or missing, no
precinct is ever found to be on a district boundary. Every district then gets
an empty exterior, `wl_make_circle` returns radius 0, the diameter is 0, and

```
reock = area / (pi * 0) = inf
```

which flows into the plan's average Reock and its 0-100 compactness rating.
rdapy scores the plan and reports the result.

Worth an explicit check: a district with no boundary points is a data error,
not a compactness of infinity.

---

## 2. Failure modes that hide errors

### 2.1 Blanket exception handling drops plans silently

`rdapy/aggregate/aggregate.py:106`, `rdapy/score/analyze.py:124`

Both stream loops wrap each record in `except Exception`, print to stderr, and
continue. That is reasonable resilience for a long ensemble run, but it
catches real data errors too -- notably the `ValueError("Populated geoid ...
not in the plan!")` that `aggregate_data_by_district` raises.

The consequence is that a plan disappears from the output and the only
evidence is that the CSV has fewer rows than the input, which nothing checks.

Two cheap improvements: count the skipped records and report the total at
exit, and consider making the permissive behaviour opt-in rather than the
default.

### 2.2 `calc_mean_median_difference` tests its argument for truthiness

`rdapy/partisan/bias.py:283`

```python
benchmark: float = Vf if Vf else statistics.mean(Vf_array)
```

The intent is "use the statewide share if one was given". But a statewide
Democratic share of exactly `0.0` is falsy, so it silently falls through to
the average-district benchmark and computes `MM'` where `MM` was asked for.
A shutout is unlikely in real data, but `Vf is None` says what is meant.

---

## 3. Fragility worth knowing about

### 3.1 The enclosing circle draws on a module-global RNG

`rdapy/compactness/smallestenclosingcircle.py:10`

`random.seed(42)` runs at import, and `do_make_circle` shuffles from that
shared stream on every call. So the shuffle a given district gets depends on
how many calls preceded it, and in what order -- scoring district 5 depends on
districts 1 through 4 having been scored first.

**In practice this appears harmless.** The minimum enclosing circle is unique,
and on a real district boundary (6,020 points from NC) the result is
bit-identical across six different RNG states. The concerns are that nothing
guarantees this, and that it rules out scoring plans in parallel, since
workers would interleave their draws.

A local `random.Random(42)` per call would make results reproducible and
parallel-safe at no cost. (rdarust uses a fixed permutation and resolves
collinear triples directly rather than retrying; it agrees with rdapy to
1.2e-16 and is bit-identical on 10 of 11 test cases.)

### 3.2 Geodesic hole handling depends on winding

`rdapy/compactness/pypoly/polygonattributes.py:89`

```python
area += area_of_hole
```

This is correct only because a hole wound opposite its exterior gives a
negative geodesic area. A GeoJSON whose interior rings are wound the same way
as the exterior -- which the format permits and some producers emit -- would
*add* the hole instead of subtracting it. A `-= abs(...)`, or an explicit
orientation check, would make the intent independent of the input's winding.

### 3.3 The metadata version is compared against a string

`rdapy/base/datasets.py:32`

```python
if "version" not in metadata or metadata["version"] == "1":
```

`map_scoring_data.py` declares `--version` with `type=str` and **no default**,
so a freshly generated data map records `"version": null`. That happens to
work -- `None != "1"`, so the map is read as current -- but a map recording a
numeric `1` would also be read as current, and reading a current file under
the legacy branch looks for unprefixed column names that do not exist. The
failure is a `KeyError` on a column name, which does not point at the version.

### 3.4 `update_aggregates` mutates its argument

`rdapy/score/analyze.py:490`

```python
new_aggs: Aggregates = aggs.copy()
new_aggs["shapes"][shapes_dataset].update(...)
new_aggs["census"][census_dataset].pop("CxD")
```

`dict.copy()` is shallow, so the nested dictionaries are shared: the returned
aggregates and the ones passed in are the same objects, and `CxD` is removed
from the caller's copy too. Harmless at the current call sites, but the
signature promises otherwise.

### 3.5 `is_embedded` has an unreachable branch

`rdapy/graph/embedded.py:45`

```python
neighboring_district: int | str = plan[neighbor]
# Assume that a missing district assignment means ... water-only
if neighboring_district == None:
```

`plan` is a plain dict, so `plan[neighbor]` raises `KeyError` before the
`None` check can run. The comment describes behaviour the code does not have;
`plan.get(neighbor)` would give it.

---

## 4. Approximations worth documenting

Neither of these is wrong -- both are reproduced faithfully in the port -- but
neither is obvious from the code, and both affect published numbers.

### 4.1 The geodesic "diameter" is not geodesic

`rdapy/compactness/pypoly/polygonattributes.py:118`

The enclosing circle is found in **degree** space, and the diameter is then
measured geodesically across it along each axis, keeping the larger. At North
Carolina's latitude a degree of longitude is about 82% of a degree of
latitude, so that circle is not a circle on the ellipsoid, and the figure is
an upper bound on the larger axis rather than a diameter. It feeds the
published geodesic Reock.

### 4.2 Population compactness is measured in degrees

`rdapy/compactness/energy.py:36`

`_squared_distance` squares raw differences in longitude and latitude, so the
metric is not isotropic: north-south displacement counts more than east-west
by about 1.2x at 35N, and the factor varies with latitude, which makes the
figure not strictly comparable between states.

### 4.3 Converted assignments come out in an order set iteration decides

`scripts/formats/canonical_to_assignments.py:47`

```python
districts: dict[int, set[int]] = defaultdict(set)
for precinct, district in enumerate(parsed_line["assignment"]):
    districts[district].add(precinct)
assignments = {geoids[index]: district
               for district, precincts in districts.items()
               for index in precincts}
```

Building an assignment map by grouping into sets and then flattening puts the
output keys in an order determined by CPython's set iteration for integers --
roughly, precinct index modulo the set's table size. Nothing downstream cares,
since the pairs are the same, but it makes the output non-obvious to diff and
ties the file's byte content to an implementation detail of the interpreter.
Iterating the assignment list directly gives precinct order and the same
result.

---

## 5. Work that is computed and discarded

### 5.1 `find_center` is dead on the scoring path

`rdapy/base/extract.py:154`, overwritten at
`scripts/data/extract_data.py:70`

`abstract_shape` computes a centre -- `shp.centroid`, a `contains` test, and a
`representative_point` fallback -- and `extract_data.py` then overwrites
`center` unconditionally with DRA's `labelx`/`labely`. Three geometry
operations per precinct, including the only point-in-polygon test in the
codebase, whose results nothing can observe.

### 5.2 Each precinct's convex hull is appended once per neighbour

`rdapy/aggregate/aggregate.py:613`

`exterior()` extends the district's point list with the precinct's **entire**
convex hull once for each qualifying neighbour, so a precinct bordering five
other districts contributes its hull five times. `wl_make_circle` dedupes its
input, so the circle is unaffected -- but the list handed to it is several
times larger than it needs to be, on the most expensive step of aggregation.
Appending once when any neighbour qualifies gives an identical circle.

### 5.3 The raw geometry is carried in the extracted data and never read

`extract_data.py` copies each feature's GeoJSON geometry into its output. For
North Carolina that is **9.4 MB of a 14.1 MB file (67%)**, and nothing in the
scoring pipeline reads it -- aggregation works entirely from the derived
`area`, `arcs`, `exterior` and `center`. Useful for debugging; expensive as a
default.

### 5.4 Unused parameters

`_calc_county_splitting(CxD, district_totals, county_totals)` never uses
`district_totals`; `_calc_district_splitting` never uses `county_totals`. Both
are marked *FOR TESTING*, so this is cosmetic -- the symmetry is presumably
deliberate.

---

## 6. Test-suite gaps

The suite passes cleanly (80 tests, ~13s). These are things it does not reach.

- **`calculate_mmd_simple` is never called.** `test_scorecard.py` scores with
  `mmd_scoring=False`, and nothing else exercises it -- yet it runs on every
  plan scored in production. Its boundary behaviour is worth pinning: exactly
  half of CVAP is *not* a majority, and a coalition summing to exactly half
  does not count.
- **`connected_subsets` and `is_consistent` are not imported by any test.**
  `connected_subsets` is reachable only through `generate_contiguity_mods`,
  which has no test either.
- **`testdata/examples/NC_congress_scores.csv` is stale.** It predates the
  `efficiency_gap_statewide` to `efficiency_gap_FPTP` rename and the addition
  of `geographic_advantage`, so it no longer matches what the pipeline
  produces. It is not asserted against, so nothing catches this.
- **`NC-116th-Congressional/expected.json`** has `avgPolsby` 0.24221440,
  where the current code computes 0.24189289. The test passes because it
  asserts to two decimal places. Worth knowing which of the two is intended.

---

## 7. Housekeeping

- **`scipy.ndimage.interpolation` is deprecated.**
  `miniumareaboundingrectangle.py:23` imports `rotate` from it -- an import
  that is never used. The namespace still exists in SciPy 1.18 but is slated
  for removal in 2.0, at which point `calc_bbox` and the whole KIWYSI path
  raise on import. Deleting the line fixes it.
- **`nptyping` is in `requirements.txt` and `setup.py` but never imported.**
  It is also unmaintained and pins older NumPy in some versions.
- `bin/distill` is a checked-in macOS arm64 binary (~1.5 MB), which limits the
  repository to that platform for whatever uses it.

---

## Appendix: things that looked wrong and were not

Recorded so nobody re-investigates them.

- **The naive shoelace formula is not good enough at these coordinates, but
  shapely already handles it.** Computing a precinct's area with a textbook
  shoelace sum gives 1.5e-8 relative error, because at -79 longitude every
  cross-product is around 2,800 while the answer is around 1e-4 and the sum
  cancels away eight digits. GEOS shifts to the first vertex internally, so
  `shp.area` is correct; only a reimplementation needs to know.
- **`est_seats` and `statistics.mean` differ deliberately.** `statistics.mean`
  sums exactly before dividing and is correctly rounded; the builtin `sum` is
  not. rdapy uses the former for turnout bias and mean-median and the latter
  for `est_seats` and average margin. That looks inconsistent but is fine --
  it just has to be matched exactly by any reimplementation.
- **The shared RNG in `wl_make_circle` does not appear to change answers.**
  See 3.1.
