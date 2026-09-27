# Conformance corpus

Language-neutral test cases shared by the Rust implementation and by rdapy.

The point is that neither implementation owns the expected values. They live
here as data, and each language has a small runner that reads them. Adding a
case means editing one JSON file; both implementations pick it up.

## Layout

```
conformance/
  cases/
    primitives/     numeric behaviours inherited from CPython / NumPy / SciPy
    rate/           the five DRA ratings, incl. rdapy's own test values
    traced/         recorded by running rdapy's test suite instrumented
    supplement/     functions rdapy's tests never call
  tools/
    gen_primitives.py      records the primitive reference values
    gen_rate.py            transcribes and verifies rdapy's rating tests
    trace_rdapy_tests.py   instruments rdapy and runs its pytest suite
    gen_supplement.py      covers what that suite leaves untouched
    check_recom_graph.py   runs GerryChain on a generated ReCom graph
```

## Where the expected values come from

Three sources, in descending order of authority:

1. **rdapy's own test suite.** `trace_rdapy_tests.py` wraps each ported
   function, runs rdapy's pytest suite, and records every call: the real
   arguments the tests pass and the value rdapy returns. Nothing is
   transcribed, so nothing can be transcribed wrong. A case carrying a `from`
   field was called *directly* by that test -- a published reference value --
   which the tracer distinguishes by walking the stack. Those are always kept;
   incidental interior calls are sampled under a size budget.

2. **Transcribed test tables.** `gen_rate.py` carries over the
   state-by-state splitting table and the 116th-Congress values from
   `test_rate.py`, which encode the spreadsheet DRA used to develop the
   ratings. Each transcription is checked against live rdapy before being
   recorded, so a typo fails the generator.

3. **Purpose-built cases.** `gen_supplement.py` covers functions rdapy's
   tests never reach -- `calculate_mmd_simple` runs on every plan in
   production but is never called by a test -- and functions whose real
   arguments are too large to check in, such as `calc_energy`, which rdapy
   only ever calls with the entire NC precinct set including geometry.

The tracer reports both gaps -- "not exercised by rdapy's tests" and "too
large to record as data" -- on every run. Those reports have already caught
three defects in the tracer itself, each of which had silently dropped a
whole category of call from the corpus.

## Case file format

```json
{
  "schema": "rdarust.cases/1",
  "suite":  "primitives",
  "function": "python_round",
  "note":   "why this matters and what goes wrong if you get it right-ish",
  "source": { "rdapy_commit": "...", "python": "...", "numpy": "...", "scipy": "..." },
  "cases":  [ { "name": "...", "input": [0.5], "expect": 0.0 } ]
}
```

`source` records exactly which versions produced the values, so a future
mismatch can be attributed rather than guessed at.

Some suites need a richer shape than `input`/`expect` -- the spline cases
carry `x`, `y`, `query`; the circle cases carry `points`. The runner for a
suite knows its shape.

## Regenerating

One script runs every generator and records what it read them from:

```bash
conformance/tools/regenerate.sh      # RDAPY_VENV=... to point at another venv
```

Regenerating against the same rdapy should produce no diff. If it does, either
a dependency version changed or something upstream moved -- investigate before
committing it.

## Provenance

Every expected value here came from one specific rdapy commit, recorded in
`cases/PROVENANCE.json` and stamped into each generated file.

Moving the submodule without regenerating would leave the tests passing while
checking this port against values that no longer describe the code it is
pinned to -- the one failure a test suite cannot report on its own. So
`crates/rdarust-core/tests/provenance.rs` compares the recorded commit against
the submodule as it stands, and fails with instructions if they differ. It
also checks the per-file stamps agree with each other, which catches half the
corpus being regenerated on its own.

Where the submodule is absent or git is unavailable -- a source export -- the
check reports that it could not compare and passes.

## Runners

* Rust: `crates/rdarust-core/tests/primitives.rs` (`cargo test`)
* Python: not yet written; will `import rdapy` and check the same files.
