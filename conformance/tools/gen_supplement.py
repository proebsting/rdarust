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

from rdapy import calc_coi_splitting, calculate_mmd_simple  # noqa: E402

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


def main():
    outdir = os.path.abspath(
        sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "conformance/cases/supplement")
    )
    os.makedirs(outdir, exist_ok=True)
    print(f"recording supplementary cases into {outdir}/")
    gen_mmd(outdir)
    gen_coi(outdir)
    print("done")


if __name__ == "__main__":
    main()
