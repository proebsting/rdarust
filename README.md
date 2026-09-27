# rdarust

A Rust implementation of [dra2020/rdapy](https://github.com/dra2020/rdapy),
the redistricting analytics used by [Dave's Redistricting](https://davesredistricting.org/).

The goal is a library you can call from another Rust program -- pass a plan,
get scores back, no JSON or CSV in the middle -- with a command-line interface
on top for bulk ensemble scoring.

**Status: the analytics are ported.** The whole pipeline works, from a DRA
GeoJSON to a scores CSV, with no Python involved -- and the CSV is
byte-identical to the one rdapy produces from the same input. Shape-based
compactness, including the KIWYSI model, is in too.

Not ported: a handful of one-off experiment scripts and shell helpers.

Ported: the five DRA ratings; the partisan suite (Nagle's method, bias,
responsiveness); population deviation; county, district and COI splitting;
minority opportunity and majority-minority counts; the Reock and
Polsby-Popper formulas, population compactness and cut edges; contiguity and
embeddedness; shape-based compactness and the KIWYSI model; the pipeline that
feeds them -- reading a GeoJSON, building the adjacency graph, aggregating by
district, and scoring -- and the CLI.

## The command line

```bash
cargo build --release

target/release/rdarust score-all \
    --state NC --plan-type congress \
    --data NC_input_data.jsonl --graph NC_graph.json \
    --plans NC_congress_plans.jsonl \
    --scores scores.csv --by-district by-district.jsonl
```

The three stages rdapy uses are also available separately, with the same
arguments, so an existing pipeline works unchanged:

```bash
cat plans.jsonl | rdarust aggregate ... | rdarust score ... | rdarust write ...
```

`scripts/score/{aggregate,score,write}` are drop-in replacements for rdapy's
scripts of the same name. `score-all` does the same work in one process,
skipping the JSONL round-trip between stages, and scores across cores.

Starting from a DRA GeoJSON instead, `scripts/SCORE.sh` runs the whole chain
and takes rdapy's arguments:

```bash
scripts/SCORE.sh --state NC --plan-type congress \
    --geojson NC_2020_vtd.datasets.geojson \
    --plans NC_congress_plans.jsonl \
    --scores scores.csv --by-district by-district.jsonl
```

which is these four steps:

```bash
rdarust map-data      --geojson NC.geojson --data-map NC_data_map.json
rdarust extract-graph --geojson NC.geojson --graph NC_graph.json
rdarust extract-data  --geojson NC.geojson --data-map NC_data_map.json \
                      --graph NC_graph.json --data NC_input_data.jsonl
rdarust score-all     --state NC --plan-type congress ...
```

The first three are once per state.

Compactness measured from district shapes, rather than from aggregates, is a
separate command, and is the only way to get a KIWYSI rank:

```bash
rdarust compactness --geojson NC_districts.geojson
```

## Getting plans in

Ensembles arrive in several shapes. Each converter writes the tagged JSONL
that `aggregate` and `score-all` read:

```bash
rdarust from-json      --input plans.legacy.json          # one JSON, a `plans` list
rdarust from-csvs      --files 'csvs/*.csv' --state NC    # one CSV per plan
rdarust from-canonical --graph recom_graph.json           # GerryTools canonical
rdarust sample -k 100                                     # keep every 100th record
```

`from-json`, `from-csvs` and `sample` produce byte-identical output to
rdapy's. `from-canonical` produces the same assignments, in a different key
order -- see [KNOWN-DIFFERENCES.md](KNOWN-DIFFERENCES.md).

## Repairing a graph

A state's precincts are often not all connected to each other -- islands, and
precincts reachable only across water. Scoring needs a connected graph:

```bash
rdarust check-graph      --state NC --data NC_input_data.jsonl --graph NC_graph.json
rdarust contiguity-mods  --graph NC_graph.json --geojson NC.geojson --output mods.csv
rdarust apply-mods       --graph NC_graph.json --mods mods.csv --output NC_graph.fixed.json
```

`contiguity-mods` proposes the fewest edges that would connect the state,
joining islands at their closest pair of border precincts and choosing which
islands to join with a minimum spanning tree. The output is a CSV meant to be
reviewed before it is applied.

## Running a chain

The dual graph a ReCom implementation eats is one file in networkx adjacency
format: integer node ids, a population on every node, and no border node --
rdapy keeps adjacency and precinct data separately and represents the state
border as a pseudo-node, which would otherwise be treated as a real unit
adjacent to half the state.

```bash
rdarust to-recom-graph --state NC \
    --data NC_input_data.jsonl --graph NC_graph.json \
    --assignment starting_plan.jsonl \
    --output NC_recom_graph.json
```

Each node carries `GEOID`, `COUNTY` and `TOTAL_POP`. The chain itself reads
only the population column; the geoid is what turns a plan of node indices
back into precinct assignments, and the county is there for region-aware
ReCom. The graph must be fully connected, so the command refuses a
disconnected one and points at `contiguity-mods`.

`--assignment` stamps a plan you already have onto the nodes as a starting
point. It is optional, but required by
[rustrecom](https://github.com/mggg/rustrecom), whose `--assignment-col` has
no default. No seed is *generated*: producing a balanced starting plan is its
own algorithm, and both GerryChain and rustrecom's own tooling already have
one.

### With rustrecom

```bash
rustrecom chain --graph-json NC_recom_graph.json \
    --assignment-col INITIAL --pop-col TOTAL_POP \
    --n-steps 100000 --n-threads 8 --rng-seed 1 \
    --tol 0.02 --variant cut-edges-ust \
    --writer canonical > chain.jsonl

rdarust from-canonical --graph NC_recom_graph.json --input chain.jsonl \
  | rdarust score-all --state NC --plan-type congress \
        --data NC_input_data.jsonl --graph NC_graph.json \
        --plans - --scores scores.csv --by-district by-district.jsonl
```

`--writer canonical` matters: rustrecom's default writer emits summary
statistics with no assignments at all.

One wrinkle it handles for you. rustrecom normalises district labels to
0-based internally -- `Partition::from_assignments` subtracts the lowest --
and writes them out that way whatever the seed used, while scoring numbers
districts from 1. `from-canonical` shifts them back and says so;
`--keep-district-numbers` declines. The shift is an offset, not a relabelling,
so district identity survives it.

`conformance/tools/check_rustrecom.sh` runs that whole loop and checks every
plan from the chain comes out scored.

### With GerryChain

GerryChain reads the same graph, and seeds a chain itself:

```python
seed = recursive_tree_part(graph, range(14), ideal, "TOTAL_POP", 0.02)
```

so `--assignment` is unnecessary there.
`conformance/tools/check_recom_graph.py` loads a generated graph through
GerryChain and runs a chain on it, which is the part a Rust test cannot check.

## The geographic baseline

How many seats each party would win from geography alone, which
`geographic_advantage` scores a plan against. Both steps are once per state:

```bash
rdarust find-neighborhoods    --state NC --plan-type congress \
        --data NC_input_data.jsonl --graph NC_graph.json \
        --output NC_neighborhoods.jsonl
rdarust precompute-baselines  --state NC --plan-type congress \
        --data NC_input_data.jsonl \
        --neighborhoods NC_neighborhoods.jsonl \
        --output NC_precomputed.json
```

The result is what `score-all --precomputed` reads. Output is byte-identical
to rdapy's.

## Using it as a library

```rust
use rdarust_core::aggregate::{Aggregates, Mode};
use rdarust_core::score::ScoreOptions;
use rdarust_io::{load_graph, load_input_data};

// Built once per state.
let ctx = load_input_data("NC_input_data.jsonl")?
    .with_graph(load_graph("NC_graph.json")?)
    .into_context("NC", "congress", None)?;

// Then per plan. `plan` is a slice of district numbers indexed by precinct.
let opts = ScoreOptions::default();
let mut aggs = Aggregates::new(&ctx);          // reused across plans
let scorecard = ctx.score_into(&plan, &opts, &mut aggs)?;

println!("{}", scorecard.shapes.unwrap().reock);
```

Geoid-keyed plans convert once with `ctx.plan_from_assignments(...)`. Nothing
goes through JSON or CSV on this path.

## Layout

```
crates/
  rdarust-core/   scoring formulas and types; libm is its only dependency
  rdarust-geo/    planar geometry; no dependencies unless shape compactness
                  is enabled, which adds geo and geographiclib-rs
  rdarust-io/     GeoJSON, JSONL and CSV
  rdarust-cli/    the `rdarust` binary
conformance/      language-neutral test corpus shared with rdapy
vendor/rdapy/     rdapy pinned as a submodule, for test data and reference values
```

## Building

```bash
git submodule update --init   # rdapy, for test data
cargo test
```

## How correctness is established

Expected values are not written by hand and not owned by either
implementation. They are recorded from rdapy (and from CPython / NumPy /
SciPy) into `conformance/cases/`, and both a Rust runner and a Python runner
check against the same files. See [conformance/README.md](conformance/README.md).

The bar is ~1e-9 relative on floats and exact agreement on integers and the
0-100 ratings -- far tighter than rdapy's own tests, which assert 1 to 4
decimal places. Deliberate deviations are listed in
[KNOWN-DIFFERENCES.md](KNOWN-DIFFERENCES.md).

Current margins against that bar:

| primitive | worst relative error |
| --- | --- |
| `erf` vs CPython | 1.1e-16 |
| not-a-knot cubic spline vs SciPy | 3.3e-16 |
| minimum enclosing circle vs rdapy | 1.2e-16 (bit-identical in 10/11 cases) |
| the five DRA ratings | 0 (bit-identical, all 2381 cases) |
| the formula layer, 70 functions | 9.9e-14 |
| whole scorecards, 28 real plans | 7.9e-12 |
| shape compactness, 53 shapes | 6.1e-9 |

The scores CSV and the metadata JSON the CLI writes are **byte-identical** to
rdapy's, checked by a test against golden files recorded from rdapy's own
pipeline.

Holding the two implementations to that bar surfaced a handful of things worth
reporting back upstream -- a few that would move a number, several performance
opportunities, and some notes on approximations that are easy to miss from the
code. Those are written up for the rdapy maintainers here:

<https://claude.ai/code/artifact/20fde9f4-9c4a-49ec-89b1-d7c75936767c>

[RDAPY-FINDINGS.md](RDAPY-FINDINGS.md) summarises what that covers and points
at it. The document is the only copy, so it can be revised as people comment.

## Performance

Scoring the 101-plan NC congressional ensemble -- 2,666 precincts, 7 election
datasets, every metric -- on one core:

| | rdapy | rdarust, 1 core | rdarust, 10 cores |
| --- | --- | --- | --- |
| per plan | ~100 ms | 1.4 ms | 0.45 ms |
| whole 101-plan run | 10.3 s | 0.30 s | 0.18 s |

rdapy's figure covers its three-process pipeline, which serialises the full
by-district aggregates to JSONL between stages. Both rdarust figures are the
complete `score-all` run: loading the precinct data, reading the plans,
scoring and writing. Per-plan figures come from a 1,010-plan run, where the
fixed loading cost no longer dominates.

Extrapolating to 100,000 plans: roughly 3 hours for rdapy against about a
minute.

The once-per-state preprocessing, on North Carolina's 2,666 precincts:

| | rdapy | rdarust |
| --- | --- | --- |
| extract the adjacency graph | 4.2 s | 0.76 s |
| extract the precinct data | 3.6 s | 0.40 s |
| grow 2,666 neighbourhoods | 67.3 s | 0.16 s |
| compute the baseline | 2.2 s | 0.04 s |

Neighbourhood growing is the outlier. Most of rdapy's 67s is a connectivity
assertion that re-checks the whole set on every step; running it under
`python -O` takes it to 4.8s with identical output.

Reproduce with:

```bash
cargo run --release -p rdarust-io --example score_ensemble -- \
    vendor/rdapy/testdata/examples/NC_input_data.jsonl \
    vendor/rdapy/testdata/plans/NC_congress_plans.tagged.jsonl \
    NC congress vendor/rdapy/testdata/examples/NC_graph.json
```
