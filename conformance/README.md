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
  tools/
    gen_primitives.py    records the primitive reference values
```

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

Needs the rdapy virtualenv and the pinned checkout in `vendor/rdapy`:

```bash
PYTHONPATH=vendor/rdapy ~/ext/rdapy/.venv/bin/python \
    conformance/tools/gen_primitives.py conformance/cases/primitives
```

Regenerating should produce no diff. If it does, either a dependency version
changed or something upstream moved -- investigate before committing it.

## Runners

* Rust: `crates/rdarust-core/tests/primitives.rs` (`cargo test`)
* Python: not yet written; will `import rdapy` and check the same files.
