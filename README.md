# rdarust

A Rust implementation of [dra2020/rdapy](https://github.com/dra2020/rdapy),
the redistricting analytics used by [Dave's Redistricting](https://davesredistricting.org/).

The goal is a library you can call from another Rust program -- pass a plan,
get scores back, no JSON or CSV in the middle -- with a command-line interface
on top for bulk ensemble scoring.

**Status: in progress.** Scoring and the command-line interface work, and
match rdapy byte for byte. Still to come: the geometry-dependent shape
compactness (KIWYSI) and the GeoJSON preprocessing steps.

Ported: the five DRA ratings; the partisan suite (Nagle's method, bias,
responsiveness); population deviation; county, district and COI splitting;
minority opportunity and majority-minority counts; the Reock and
Polsby-Popper formulas, population compactness and cut edges; contiguity and
embeddedness; the pipeline that feeds them -- reading precinct data,
aggregating by district, and scoring -- and the CLI.

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

Note that `rdarust` starts from extracted precinct data and an adjacency
graph. Producing those from a DRA GeoJSON needs a geometry library and is not
ported yet; use rdapy's `scripts/data/extract_data.py` for that step.

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
  rdarust-core/   scoring formulas and types; almost no dependencies
  rdarust-io/     JSONL and CSV
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

The scores CSV and the metadata JSON the CLI writes are **byte-identical** to
rdapy's, checked by a test against golden files recorded from rdapy's own
pipeline.

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

Reproduce with:

```bash
cargo run --release -p rdarust-io --example score_ensemble -- \
    vendor/rdapy/testdata/examples/NC_input_data.jsonl \
    vendor/rdapy/testdata/plans/NC_congress_plans.tagged.jsonl \
    NC congress vendor/rdapy/testdata/examples/NC_graph.json
```
