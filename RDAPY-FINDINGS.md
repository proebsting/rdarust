# Notes from porting rdapy to Rust

These are observations gathered while building
[rdarust](https://github.com/proebsting/rdarust), a port of rdapy's analytics
to Rust, offered in case any of them are useful.

They come out of one specific exercise: making the two implementations agree
to within 1e-9 on real data, and then chasing down every place they did not.
That is a much tighter bar than rdapy's own tests aim at, and it surfaces
things that are invisible at normal tolerances. Most of what follows is
small, several items are performance opportunities rather than errors, and a
few are simply worth writing down somewhere.

For context on how closely the two agree: the port reproduces rdapy's results
across about 8,500 conformance cases, and the scores CSV, the by-district
aggregates, the extracted precinct data, the adjacency graph, the
neighbourhood bitmaps and the format-converter output are all byte-identical
to rdapy's. The architecture made that possible — the separation between
aggregation and scoring, and the decision to reduce each precinct to a handful
of numbers before any plan is scored, are what let a second implementation
line up so precisely.

Line numbers are against commit **`2b702b4`** ("Tweaked doc"), verified with
Python 3.13.15, NumPy 2.5.3, SciPy 1.18.1, shapely 2.1.2, GerryChain 1.0.0.

---

## 1. Two places where a change would move a number

### 1.1 `minimum_bounding_rectangle` and the hull's closing edge

`rdapy/compactness/pypoly/miniumareaboundingrectangle.py:28`

```python
hull_points = points[ConvexHull(points).vertices]
edges = hull_points[1:] - hull_points[:-1]
```

`ConvexHull.vertices` is an unclosed list, so the edge from the last hull
vertex back to the first does not appear in `edges`, and its direction is
never tried as a candidate orientation.

Since a minimum-area enclosing rectangle always has a side flush with some
hull edge, leaving one edge out can leave out the answer. For shape `16` of
`testdata/compactness/first20` it does: the omitted closing edge is the one
that yields the minimum, and the rectangle returned is 0.77% larger
(0.78627382 against 0.78020531).

Downstream this moves the `bbox` feature by about 0.5% and a KIWYSI rank by
under a tenth of a point on a 1-100 scale, so it is well inside the tolerance
the tests use. Closing the ring first would settle it:

```python
hull_points = points[ConvexHull(points).vertices]
closed = np.vstack([hull_points, hull_points[:1]])
edges = closed[1:] - closed[:-1]
```

### 1.2 A guard for a district with no boundary

Input files written for the scoring pipeline carry no `neighbors` key, since
the graph is maintained separately — which is a good separation, because a
graph can then be corrected without regenerating the data.

One consequence is worth a guard. `aggregate_shapes_by_district` reads
adjacency from the graph it is handed, so if that graph is empty or was not
loaded, no precinct is ever found to sit on a district boundary. Every
district then gets an empty exterior, `wl_make_circle` returns radius 0, the
diameter is 0, and

```
reock = area / (pi * 0) = inf
```

which propagates into the plan's average Reock and its 0-100 compactness
rating. The run completes and reports a result. A district with no boundary
points is a reliable signal that something upstream went wrong, so it may be
worth raising there.

---

## 2. Performance opportunities

### 2.1 The connectivity assertion in the neighbourhood search

`rdapy/partisan/geographic.py:183`

```python
assert is_connected(list(yielded), graph), f"Yields must maintain connectivity: {next.geoid}"
```

This runs on every precinct the search yields, and `is_connected` walks the
whole set, which grows to a district's worth of precincts. It is the dominant
cost of `find_neighborhoods.py`.

Measured on North Carolina (2,666 precincts):

| | time |
| --- | --- |
| as written | 67.3 s |
| under `python -O` | 4.8 s |

with byte-identical output. The property being checked holds by construction
— a precinct only ever enters the frontier from a precinct already taken — so
it is an invariant check rather than a test of the input.

`make_neighborhood` and `_nearest_connected_neighbor` already take a `debug`
parameter and thread it through to each other, though nothing currently reads
it. Guarding this assertion with it looks like the intended design and would
be a one-line change for roughly a 14x speedup.

### 2.2 The convex hull is appended once per neighbour

`rdapy/aggregate/aggregate.py:613`

`exterior()` extends a district's point list with the precinct's entire convex
hull once for *each* neighbour in a different district, so a precinct
bordering five others contributes its hull five times.

`wl_make_circle` deduplicates its input, so the circle is unaffected — but the
list handed to it is several times larger than it needs to be, on the most
expensive step of aggregation. Appending once if any neighbour qualifies gives
an identical circle from a fraction of the memory.

### 2.3 Geometry in the extracted data file

`extract_data.py` copies each feature's GeoJSON geometry into its output. For
North Carolina that is 9.4 MB of a 14.1 MB file, and the scoring pipeline
never reads it: aggregation works entirely from the derived `area`, `arcs`,
`exterior` and `center`.

Very useful for debugging; possibly worth a flag rather than being
unconditional.

### 2.4 `find_center` is computed and then replaced

`rdapy/base/extract.py:154`, replaced at `scripts/data/extract_data.py:70`

`abstract_shape` computes a centre — `shp.centroid`, a `contains` test, and a
`representative_point` fallback — and `extract_data.py` then sets `center`
from DRA's `labelx`/`labely` regardless. That is the better centre, since
DRA's label point is guaranteed to lie inside the shape, so the replacement is
right; the earlier computation just no longer has a consumer. It is three
geometry operations per precinct, including the only point-in-polygon test in
the codebase.

### 2.5 The distance cache

`rdapy/base/distance.py`

`DistanceLedger` caches the squared distance between each pair of precincts.
The quantity is a subtraction and two multiplications, which in the port
turned out to cost less than a hash lookup, so it was dropped with no change
in results. Whether the same holds in Python is worth measuring — dictionary
lookups are relatively cheaper there — but it may be one less thing to carry.

---

## 3. Robustness suggestions

### 3.1 Blanket exception handling around each record

`rdapy/aggregate/aggregate.py:106`, `rdapy/score/analyze.py:124`

Both stream loops wrap each record in `except Exception`, log to stderr, and
continue. That is sensible resilience for a long ensemble run. It also catches
genuine data errors, notably the `ValueError("Populated geoid ... not in the
plan!")` that `aggregate_data_by_district` raises, and the effect is that a
plan leaves the output with no record of it beyond a stderr line.

Two cheap additions: count the skipped records and report the total at exit,
so a silent drop becomes visible, and consider making the permissive path
opt-in.

### 3.2 A truthiness test in `calc_mean_median_difference`

`rdapy/partisan/bias.py:283`

```python
benchmark: float = Vf if Vf else statistics.mean(Vf_array)
```

The intent is "use the statewide share if one was given". A statewide
Democratic share of exactly `0.0` is falsy, so it would fall through to the
average-district benchmark and compute `MM'` where `MM` was asked for. A
shutout is unlikely in real data; `Vf is None` says what is meant.

### 3.3 Geodesic hole handling depends on ring winding

`rdapy/compactness/pypoly/polygonattributes.py:89`

```python
area += area_of_hole
```

This is correct because a hole wound opposite its exterior yields a negative
geodesic area. GeoJSON permits either winding, and some producers emit
interior rings wound the same way as the exterior, which would add the hole
rather than subtract it. An explicit `-= abs(...)`, or an orientation check,
would make the intent independent of the input.

### 3.4 The metadata version is compared against a string

`rdapy/base/datasets.py:32`

```python
if "version" not in metadata or metadata["version"] == "1":
```

`map_scoring_data.py` declares `--version` with `type=str` and no default, so
a freshly generated data map records `"version": null`. That works out — `None
!= "1"`, so the map reads as current — but a map recording a numeric `1` would
also read as current, and reading a current file through the legacy branch
looks for unprefixed column names that do not exist. The resulting `KeyError`
names a column rather than the version, which makes it a slow thing to
diagnose.

### 3.5 `update_aggregates` shares structure with its argument

`rdapy/score/analyze.py:490`

```python
new_aggs: Aggregates = aggs.copy()
new_aggs["shapes"][shapes_dataset].update(...)
new_aggs["census"][census_dataset].pop("CxD")
```

`dict.copy()` is shallow, so the nested dictionaries are shared: the returned
aggregates and the ones passed in are the same objects, and `CxD` is removed
from the caller's as well. Harmless at the current call sites; a
`copy.deepcopy`, or a note in the docstring, would keep it that way.

### 3.6 An unreachable branch in `is_embedded`

`rdapy/graph/embedded.py:45`

```python
neighboring_district: int | str = plan[neighbor]
# Assume that a missing district assignment means ... water-only
if neighboring_district == None:
```

`plan` is a plain dict, so `plan[neighbor]` raises `KeyError` before the
`None` check can run. `plan.get(neighbor)` would give the behaviour the
comment describes.

### 3.7 An operation column that is read and then ignored

`scripts/graphs/apply_contiguity_mods.py`

`read_mods` keeps each row whole and `add_adjacency(graph, mod[1], mod[2])`
uses only columns 1 and 2, so a row written as `-,a,b` adds the edge. Since
every mods file in practice uses `+`, this has no effect today; validating the
column would keep it that way.

---

## 4. Approximations worth documenting

Neither of these is an error — both are reproduced faithfully in the port —
but neither is obvious from the code, and both feed published numbers.

### 4.1 The geodesic diameter

`rdapy/compactness/pypoly/polygonattributes.py:118`

The enclosing circle is found in degree space, and the diameter is then
measured geodesically across it along each axis, keeping the larger. At North
Carolina's latitude a degree of longitude is about 82% of a degree of
latitude, so that circle is not a circle on the ellipsoid, and the result is
an upper bound on the larger axis rather than a diameter. It feeds the
geodesic Reock figures.

### 4.2 Population compactness is measured in degrees

`rdapy/compactness/energy.py:36`

`_squared_distance` squares raw differences in longitude and latitude, so the
metric is not isotropic: north-south displacement counts for more than
east-west by about 1.2x at 35N, and the factor varies with latitude, which
makes the figure not strictly comparable between states. As a relative measure
within one state it is unaffected.

### 4.3 Two places where output order comes from set iteration

`scripts/formats/canonical_to_assignments.py:47` builds each plan by grouping
precincts into a `dict[int, set[int]]` and flattening, which orders the output
keys by CPython's set iteration for integers. `generate_contiguity_mods`
numbers its islands from `connected_subsets`, which returns Python sets, and
then sorts its spanning-tree edges by those numbers.

In both cases the content is the same and nothing downstream depends on the
order. It does mean the files are awkward to diff between runs. Iterating the
assignment list directly, and sorting mods by the precincts they name, gives
stable output.

---

## 5. Compatibility

### 5.1 `canonical_to_assignments.py` and GerryChain 1.0

`scripts/formats/canonical_to_assignments.py:37`

```python
recom_graph.nodes[node].get(args.geoid) for node in list(recom_graph.nodes())
```

GerryChain 1.0.0 made `Graph.nodes` a property rather than a method, so this
raises on the call:

```
TypeError: As of GerryChain version 1.0.0, `Graph.nodes` is a property, not a
method. Use `graph.nodes` without parentheses ...
```

`requirements.txt` does not pin a version, so a fresh install gets 1.0.0.
Dropping the parentheses fixes it. GerryChain is imported by this one script
and nothing else.

### 5.2 `scipy.ndimage.interpolation` is deprecated

`miniumareaboundingrectangle.py:23` imports `rotate` from it. The import is
unused, and the namespace still exists in SciPy 1.18, but it is slated for
removal in 2.0 — at which point `calc_bbox` and the KIWYSI path would raise on
import. Deleting the line resolves it ahead of time.

### 5.3 `nptyping`

Declared in `requirements.txt` and `setup.py`, never imported. It is also no
longer maintained and pins older NumPy in some versions.

### 5.4 `bin/distill`

A checked-in macOS arm64 binary, which limits whatever uses it to that
platform.

---

## 6. Test coverage opportunities

The suite passes cleanly — 80 tests in about 13 seconds. These are areas it
does not currently reach.

- **`calculate_mmd_simple` is never called.** `test_scorecard.py` scores with
  `mmd_scoring=False` and nothing else exercises it, though it runs on every
  plan scored in production. Its boundary behaviour is worth pinning: exactly
  half of CVAP is not a majority, and a coalition summing to exactly half does
  not count.
- **`connected_subsets` and `is_consistent` are not imported by any test.**
  `connected_subsets` is reachable only through `generate_contiguity_mods`,
  which has no test either.
- **`testdata/examples/NC_congress_scores.csv` has drifted.** It predates the
  `efficiency_gap_statewide` to `efficiency_gap_FPTP` rename and the addition
  of `geographic_advantage`, so it no longer matches what the pipeline
  produces. Nothing asserts against it, so nothing flags it.
- **`testdata/examples/NC_congress_precomputed.json` does not correspond to
  the `NC_input_data.jsonl` beside it.** It carries 22 elections against the
  data file's 7, and the shared election names hold different vote counts:
  `E_20_AG` matches to the last digit, the rest differ by a few percent.
  Regenerating it from the checked-in data would make the pair consistent.
- **`NC-116th-Congressional/expected.json`** records `avgPolsby` as
  0.24221440 where the current code computes 0.24189289. The test passes
  because it asserts to two decimal places; worth knowing which figure is
  intended.

---

## Appendix: things that looked like problems and were not

Recorded so nobody spends time re-investigating them.

- **The shoelace formula needs care at these coordinates, but shapely already
  takes it.** Computing a precinct's area with a textbook shoelace sum gives
  1.5e-8 relative error, because at -79 longitude every cross-product is
  around 2,800 while the answer is around 1e-4, and the sum cancels away eight
  digits. GEOS shifts to the ring's first vertex internally, so `shp.area` is
  correct. Only a reimplementation needs to know.
- **The mix of `statistics.mean` and the builtin `sum` is fine.**
  `statistics.mean` sums exactly before dividing and is correctly rounded; the
  builtin is not. rdapy uses the former for turnout bias and mean-median, the
  latter for `est_seats` and average margin. That looks inconsistent but the
  values are correct either way — it just has to be matched exactly by a
  reimplementation.
- **The shared RNG in `wl_make_circle` does not appear to change answers.**
  `random.seed(42)` at import means the shuffle a district gets depends on how
  many calls preceded it. On a real district boundary (6,020 points) the
  circle is bit-identical across six different RNG states, which is what the
  uniqueness of the minimum enclosing circle would predict. A local
  `random.Random(42)` would make that guaranteed rather than observed, and
  would allow scoring plans in parallel, but nothing is wrong today.
