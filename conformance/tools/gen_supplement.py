#!/usr/bin/env python3
"""
Record cases for rdapy functions its own test suite never calls.

The tracer reports these as "not exercised by rdapy's tests". They are real
parts of the scoring path -- calculate_mmd_simple runs on every plan scored
through score_plan with MMD scoring on -- so leaving them unverified would be
a hole, not a saving.

Output goes to conformance/cases/supplement/, which the Rust runner reads
alongside conformance/cases/traced/.

  PYTHONPATH=vendor/rdapy ~/ext/rdapy/.venv/bin/python \
      conformance/tools/gen_supplement.py conformance/cases/supplement
"""

import json
import os
import platform
import subprocess
import sys

REPO = os.path.abspath(os.path.dirname(os.path.dirname(os.path.dirname(__file__))))
RDAPY = os.path.join(REPO, "vendor", "rdapy")
sys.path.insert(0, RDAPY)

import numpy as np  # noqa: E402

from rdapy import (  # noqa: E402
    calc_coi_splitting,
    calc_energy,
    calculate_mmd_simple,
    connected_subsets,
    is_consistent,
)

SCHEMA = "rdarust.cases/1"


def provenance():
    try:
        sha = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=RDAPY, text=True
        ).strip()
    except Exception:
        sha = "unknown"
    return {"rdapy_commit": sha, "python": platform.python_version(),
            "numpy": np.__version__, "platform": platform.platform()}


def emit(outdir, function, cases, note):
    doc = {"schema": SCHEMA, "suite": "supplement", "function": function,
           "note": note, "source": provenance(), "cases": cases}
    with open(os.path.join(outdir, f"{function}.json"), "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    print(f"  {function}: {len(cases)} cases")


def gen_mmd(outdir):
    """Districts spanning all three buckets plus the boundaries between them."""
    rng = np.random.default_rng(31)
    scenarios = [
        # (black, hispanic, total) per district -- hand-picked edges first.
        ([60, 10, 10], [10, 60, 10], [100, 100, 100]),   # black, hispanic, neither
        ([50, 30], [10, 30], [100, 100]),                # exactly half is NOT a majority
        ([51, 30], [10, 30], [100, 100]),                # just over half is
        ([30, 25], [25, 30], [100, 100]),                # coalition both ways
        ([0, 0], [0, 0], [100, 100]),                    # no minority population
        ([50, 50], [50, 50], [100, 100]),                # both exactly half
        ([34, 33], [33, 34], [100, 100]),                # coalition just over half
        ([25, 25], [25, 25], [100, 100]),                # coalition exactly half: no
    ]
    cases = []
    for black, hispanic, total in scenarios:
        # The 0th element is the statewide total, which rdapy skips.
        aggs = {
            "black_cvap": [sum(black)] + list(black),
            "hispanic_cvap": [sum(hispanic)] + list(hispanic),
            "total_cvap": [sum(total)] + list(total),
        }
        cases.append({"input": [aggs], "expect": calculate_mmd_simple(aggs)})

    for _ in range(60):
        n = int(rng.integers(2, 15))
        total = [int(x) for x in rng.integers(1000, 50000, n)]
        black = [int(rng.random() * t) for t in total]
        hispanic = [int(rng.random() * (t - b)) for t, b in zip(total, black)]
        aggs = {
            "black_cvap": [sum(black)] + black,
            "hispanic_cvap": [sum(hispanic)] + hispanic,
            "total_cvap": [sum(total)] + total,
        }
        cases.append({"input": [aggs], "expect": calculate_mmd_simple(aggs)})

    emit(outdir, "calculate_mmd_simple", cases,
         "Majority-minority district counts in three mutually exclusive "
         "buckets. Not reached by rdapy's tests, which score with "
         "mmd_scoring=False, but it runs on every plan in production.")


def gen_coi(outdir):
    """Communities split across districts, including intact and even splits."""
    scenarios = [
        ("intact", [1.0]),
        ("even-two", [0.5, 0.5]),
        ("even-four", [0.25, 0.25, 0.25, 0.25]),
        ("paper-a", [0.33, 0.33, 0.34]),
        ("paper-b", [0.92, 0.05, 0.03]),
        ("with-zeros", [0.6, 0.0, 0.4, 0.0]),
        ("tiny-tail", [0.999, 0.001]),
    ]
    communities = [{"name": n, "splits": s} for n, s in scenarios]
    result = calc_coi_splitting(communities)
    cases = [{"input": [communities], "expect": result}]
    emit(outdir, "calc_coi_splitting", cases,
         "Sam Wang's community-of-interest splitting metrics. A thin wrapper "
         "over uncertainty_of_membership and effective_splits that rdapy's "
         "tests call only through its parts.")


def gen_energy(outdir):
    """Population compactness on small synthetic states.

    rdapy's tests do call calc_energy, but only with the full NC precinct set
    -- every record carrying its geometry -- which is megabytes per call and
    useless as a checked-in case. These are the same shape, small.
    """
    rng = np.random.default_rng(17)
    cases = []

    def add(pops, centers, districts):
        precincts = [
            {"geoid": f"p{i:04d}", "TOTAL_POP": int(p), "center": [float(c[0]), float(c[1])]}
            for i, (p, c) in enumerate(zip(pops, centers))
        ]
        assignments = {f"p{i:04d}": int(d) for i, d in enumerate(districts)}
        cases.append({
            "input": [assignments, precincts, "TOTAL_POP"],
            "expect": calc_energy(assignments, precincts, "TOTAL_POP"),
        })

    # Two compact districts side by side.
    add([100, 100, 100, 100],
        [(-80.0, 35.0), (-80.1, 35.0), (-81.0, 35.0), (-81.1, 35.0)],
        [1, 1, 2, 2])
    # The same populations, interleaved: strictly less compact.
    add([100, 100, 100, 100],
        [(-80.0, 35.0), (-80.1, 35.0), (-81.0, 35.0), (-81.1, 35.0)],
        [1, 2, 1, 2])
    # A district whose precincts share one point: zero energy contribution.
    add([50, 50], [(-79.0, 36.0), (-79.0, 36.0)], [1, 1])
    # Lopsided populations, so the centroid sits near the heavy precinct.
    add([1, 10_000], [(-79.0, 36.0), (-79.5, 36.5)], [1, 1])

    for _ in range(25):
        n = int(rng.integers(6, 60))
        d = int(rng.integers(2, 6))
        pops = [int(x) for x in rng.integers(1, 20000, n)]
        centers = [(float(x), float(y)) for x, y in
                   zip(rng.uniform(-84, -75, n), rng.uniform(33, 37, n))]
        # Every district must be non-empty, or rdapy divides by zero.
        districts = [1 + (i % d) for i in range(n)]
        rng.shuffle(districts)
        add(pops, centers, districts)

    emit(outdir, "calc_energy", cases,
         "Population compactness. rdapy's tests only ever call this with the "
         "entire NC precinct set including geometry, which is far too large to "
         "check in, so these are small synthetic states of the same shape.")


def grid_graph(rows, cols, prefix="n"):
    g = {}
    for r in range(rows):
        for c in range(cols):
            nbrs = []
            for dr, dc in ((0, 1), (1, 0), (0, -1), (-1, 0)):
                rr, cc = r + dr, c + dc
                if 0 <= rr < rows and 0 <= cc < cols:
                    nbrs.append(f"{prefix}{rr}_{cc}")
            g[f"{prefix}{r}_{c}"] = nbrs
    return g


def norm_subsets(subsets):
    """Sets to sorted lists, then order the components deterministically.

    rdapy returns a list of Python sets, whose iteration order is not a
    property worth reproducing.
    """
    return sorted((sorted(s) for s in subsets), key=lambda c: c[0] if c else "")


def gen_graph(outdir):
    """Connectivity helpers rdapy's tests never call."""
    g4 = grid_graph(4, 4)

    subsets_cases = []
    # One connected block.
    subsets_cases.append({"input": [sorted(g4.keys()), g4],
                          "expect": norm_subsets(connected_subsets(sorted(g4.keys()), g4))})
    # Two disjoint blocks: opposite corners of the grid.
    left = [f"n{r}_{c}" for r in range(2) for c in range(2)]
    right = [f"n{r}_{c}" for r in range(2, 4) for c in range(2, 4)]
    subsets_cases.append({"input": [left + right, g4],
                          "expect": norm_subsets(connected_subsets(left + right, g4))})
    # Isolated singletons.
    single = ["n0_0", "n2_2", "n0_3"]
    subsets_cases.append({"input": [single, g4],
                          "expect": norm_subsets(connected_subsets(single, g4))})
    # A chain, and the whole grid minus a cut column.
    chain = [f"n0_{c}" for c in range(4)]
    subsets_cases.append({"input": [chain, g4], "expect": norm_subsets(connected_subsets(chain, g4))})
    split = [k for k in sorted(g4.keys()) if not k.endswith("_1")]
    subsets_cases.append({"input": [split, g4], "expect": norm_subsets(connected_subsets(split, g4))})
    emit(outdir, "connected_subsets", subsets_cases,
         "Connected components of a node set. Only reachable in rdapy through "
         "generate_contiguity_mods, which has no test.")

    consistent_cases = []
    consistent_cases.append({"input": [g4], "expect": is_consistent(g4)})
    broken = {k: list(v) for k, v in g4.items()}
    broken["n0_0"] = [n for n in broken["n0_0"] if n != "n0_1"]
    consistent_cases.append({"input": [broken], "expect": is_consistent(broken)})
    tri = {"a": ["b", "c"], "b": ["a", "c"], "c": ["a", "b"]}
    consistent_cases.append({"input": [tri], "expect": is_consistent(tri)})
    one_way = {"a": ["b"], "b": []}
    consistent_cases.append({"input": [one_way], "expect": is_consistent(one_way)})
    emit(outdir, "is_consistent", consistent_cases,
         "Is every edge reciprocated? Not imported by any rdapy test.")


def main():
    outdir = os.path.abspath(
        sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "conformance/cases/supplement")
    )
    os.makedirs(outdir, exist_ok=True)
    print(f"recording supplementary cases into {outdir}/")
    gen_mmd(outdir)
    gen_coi(outdir)
    gen_energy(outdir)
    gen_graph(outdir)
    print("done")


if __name__ == "__main__":
    main()
