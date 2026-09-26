# rdarust

A Rust implementation of [dra2020/rdapy](https://github.com/dra2020/rdapy),
the redistricting analytics used by [Dave's Redistricting](https://davesredistricting.org/).

The goal is a library you can call from another Rust program -- pass a plan,
get scores back, no JSON or CSV in the middle -- with a command-line interface
on top for bulk ensemble scoring.

**Status: in progress.** The numeric foundations and the whole formula layer
are in place and verified against Python. The plan-scoring pipeline and the
CLI are not written yet.

Ported so far: the five DRA ratings; the partisan suite (Nagle's method, bias,
responsiveness); population deviation; county, district and COI splitting;
minority opportunity and majority-minority counts; the Reock and
Polsby-Popper formulas, population compactness and cut edges; contiguity and
embeddedness. Still to come: the scoring pipeline (`Context` + `score`), the
CLI, and the geometry-dependent shape compactness and preprocessing.

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

## Baseline to beat

rdapy scores the 101-plan NC congressional ensemble (2,666 precincts, 7
election datasets) in 10.3s wall / 13s CPU, about 100ms per plan.
