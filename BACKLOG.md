# Known gaps

Things this port does not do, deliberately deferred. Each was characterised
before being set aside, so picking one up does not mean starting from scratch.

## 1. Zipped JSONL input

rdapy's `smart_read` transparently opens a `.zip` containing a `.jsonl`;
`rdarust_io::records::smart_reader` handles only stdin and plain files.

Affects every stream command: `aggregate --input`, `score --input`,
`score-all --plans`, `precompute-baselines --neighborhoods`,
`from-canonical --input`. Ensembles and neighbourhood files are distributed
zipped -- `testdata/neighborhoods_2020.zip` is 23 MB, and rdapy carries an
`UNZIP-ENSEMBLE.sh` for the ensembles.

**Workaround:** unzip first.

**To do it:** `smart_reader` grows a `.zip` branch that finds the single
`.jsonl` member and streams it. rdapy warns and takes the first when an
archive holds several. Contained; one function and a dependency on a zip
crate.

## 2. The `compress` plan encoding

`bin/distill` converts between three plan encodings -- `canonical`,
`assignment` and `compress`. `from-canonical` covers the first; `compress` is
not read or written here.

It is a delta encoding built around ReCom's structure. Line 1 is the ReCom
graph in networkx adjacency form -- the same shape `to-recom-graph` writes.
Each plan after it records only the districts that changed:

```
plan 1: 14 districts recorded     <- the full starting plan
plan 2:  2 districts recorded     <- a ReCom step changes exactly two
plan 3:  2 districts recorded
```

with each district as `{"district": 10, "precincts": [...]}`. For a
100k-plan ensemble that is roughly a sevenfold saving before xz, which is why
large runs ship this way (`DUMP_ENSEMBLE.sh` and `RESCORE-SPLITTING.sh` both
pipe `xz --decompress | bin/distill`).

**Worth knowing:** `bin/distill` is a prebuilt macOS arm64 binary with no
source in rdapy, so on any other platform a `compress` ensemble cannot be read
at all today, with or without this port.

**To do it:** read line 1 as the graph, carry a running assignment, and apply
each record's districts over it. Writing it is the same in reverse. The
semantics to pin down are what a district's `precincts` list means when a
precinct moves out of one district and into another in the same step.

## 3. Shapefile input for shape compactness

`rdarust compactness` takes GeoJSON; rdapy's `load_shapes` reads ESRI
shapefiles through fiona. District shapes arriving as `.shp` need converting
first.

---

# Not gaps

Recorded so they are not mistaken for gaps later.

- **`SHARD.sh`, `CONCAT_FILES.sh`, `JOIN_CSVS.sh`** exist because scoring in
  Python is slow enough to need manual sharding across processes, and each
  mode run separately then stitched back. `score-all --jobs` does every mode
  across every core in one pass, so there is nothing to shard or rejoin.
- **`make_census_json.py`** is a geoid-to-population shim its own docstring
  describes as being "so that other legacy scripts don't need to be modified".
- **`EXTRACT-GRAPHS.py`, `NEIGHBORHOODS.py`, `PRECOMPUTE.py`** are for-loops
  over states around commands this port already has.
- **`GET-GEOJSON.sh`, `UNZIP-GEOJSON.sh`** are curl and unzip.
- **`scripts/experiments/*`** are one-off investigations.
