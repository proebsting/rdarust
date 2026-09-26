#!/usr/bin/env python3
"""
Record reference values for the low-level numeric behaviours that rdapy
inherits from CPython / NumPy / SciPy, and that a Rust port must reproduce.

These are NOT rdapy functions -- they are the foundations rdapy's formulas
sit on. Getting any of them subtly wrong silently corrupts downstream scores,
so they are pinned first and independently.

Run with the rdapy venv's interpreter and PYTHONPATH pointed at the pinned
rdapy checkout:

  PYTHONPATH=vendor/rdapy ~/ext/rdapy/.venv/bin/python \
      conformance/tools/gen_primitives.py conformance/cases/primitives
"""

import json
import math
import os
import platform
import subprocess
import sys

import numpy as np
import scipy
from scipy.interpolate import interp1d

SCHEMA = "rdarust.cases/1"


def provenance():
    try:
        sha = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd="vendor/rdapy", text=True
        ).strip()
    except Exception:
        sha = "unknown"
    return {
        "rdapy_commit": sha,
        "python": platform.python_version(),
        "numpy": np.__version__,
        "scipy": scipy.__version__,
        "platform": platform.platform(),
    }


def emit(outdir, function, cases, note):
    doc = {
        "schema": SCHEMA,
        "suite": "primitives",
        "function": function,
        "note": note,
        "source": provenance(),
        "cases": cases,
    }
    path = os.path.join(outdir, f"{function}.json")
    with open(path, "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    print(f"  {function}: {len(cases)} cases -> {path}")


### 1. round() to integer: half-to-even, NOT half-away-from-zero ###


def gen_python_round(outdir):
    xs = [
        0.0, 0.5, 1.5, 2.5, 3.5, 4.5, -0.5, -1.5, -2.5,
        0.49999999999999994, 1.0000000000000002,
        12.5, 13.5, 99.5, 100.5, 0.1, 0.9, -0.1, -0.9,
        2.675, 1234567.5, 1234568.5,
    ]
    # Values that arise from Normalizer.rescale(): round(unit * 100)
    rng = np.random.default_rng(20260926)
    xs += [float(v) * 100.0 for v in rng.random(200)]
    xs += [i / 2.0 for i in range(0, 201)]  # every exact .0 and .5 in [0, 100]
    cases = [{"input": [x], "expect": round(x)} for x in xs]
    emit(outdir, "python_round", cases,
         "CPython round(x) -> int, round-half-to-even. Rust f64::round() is "
         "half-away-from-zero and is WRONG here; use round_ties_even().")


### 2. round(x, ndigits): used by trim_scores(precision=4) ###


def gen_python_round_to(outdir):
    rng = np.random.default_rng(1729)
    cases = []
    for nd in (1, 2, 3, 4, 6):
        vals = list(rng.random(120) * rng.choice([1.0, 10.0, 100.0], 120))
        vals += [0.5 / 10**nd, 1.5 / 10**nd, 2.5 / 10**nd]
        vals += [1.0005, 2.675, 0.125, 0.135, 1.0 / 3.0, 2.0 / 3.0]
        for v in vals:
            v = float(v)
            cases.append({"input": [v, nd], "expect": round(v, nd)})
    emit(outdir, "python_round_to", cases,
         "CPython round(x, ndigits): correctly-rounded decimal, ties-to-even. "
         "Used by trim_scores(precision=4) on every reported score.")


### 3. np.arange: cumulative accumulation, not start + i*step ###


def gen_numpy_arange(outdir):
    specs = [
        # The one that matters: partisan.utils.shift_range()
        (25 / 100, 75 / 100 + 1.0e-12, (1 / 100) / 2),
        (0.0, 1.0, 0.1),
        (0.0, 1.0, 0.125),
        (0.25, 0.75, 0.005),
        (-1.0, 1.0, 0.03),
        (0.0, 0.3, 0.1),
    ]
    cases = []
    for start, stop, step in specs:
        arr = np.arange(start, stop, step)
        cases.append({
            "input": [float(start), float(stop), float(step)],
            "expect": [float(v) for v in arr],
        })
    emit(outdir, "numpy_arange", cases,
         "np.arange for floats accumulates (v[i] = v[i-1] + step); it is NOT "
         "start + i*step. Length is ceil((stop-start)/step). shift_range() "
         "feeds every seats-votes curve, so a 1-ulp error propagates widely.")


### 4. math.erf: the core of Nagle's seat-probability formula ###


def gen_erf(outdir):
    xs = [0.0, 1e-12, 1e-8, 0.5, -0.5, 1.0, -1.0, 2.0, 3.0, 4.0, 5.0, 6.0,
          -6.0, 0.02, 25.0, -25.0]
    # The exact arguments est_seat_probability() produces: (Vf - 0.5)/(0.02*sqrt(8))
    denom = 0.02 * math.sqrt(8)
    xs += [(vf / 1000.0 - 0.5) / denom for vf in range(300, 701, 7)]
    rng = np.random.default_rng(42)
    xs += [float(v) for v in (rng.random(150) * 12.0 - 6.0)]
    cases = [{"input": [float(x)], "expect": math.erf(float(x))} for x in xs]
    emit(outdir, "erf", cases,
         "math.erf. est_seat_probability(Vf) = 0.5*(1+erf((Vf-0.5)/(0.02*sqrt(8)))) "
         "is the single most-used formula in the partisan suite.")


### 5. math.isclose: two-term semantics, both tolerances ###


def gen_math_isclose(outdir):
    pairs = [
        (1.0, 1.0), (1.0, 1.0 + 1e-10), (1.0, 1.0 + 1e-8),
        (0.0, 0.0), (0.0, 1e-7), (0.0, 1e-12),
        (1e6, 1e6 + 1e-3), (1e6, 1e6 + 1.0),
        (100.0, 100.0000001), (-5.0, -5.0000000001),
        (1e-9, 0.0), (3.0, 3.0000000000001),
    ]
    tols = [(1e-09, 0.0), (1e-09, 1e-06), (0.0, 1e-06)]
    cases = []
    for a, b in pairs:
        for rel, abs_ in tols:
            cases.append({
                "input": [a, b, rel, abs_],
                "expect": math.isclose(a, b, rel_tol=rel, abs_tol=abs_),
            })
    emit(outdir, "math_isclose", cases,
         "math.isclose(a, b, rel_tol=1e-09, abs_tol=0.0). The splitting "
         "reductions call it with abs_tol=1e-6 but leave rel_tol at its "
         "default, so BOTH terms are live.")


### 6. not-a-knot cubic spline == scipy interp1d(kind='cubic') ###


def gen_spline(outdir):
    cases = []

    def add(name, x, y, q):
        f = interp1d(np.asarray(x), np.asarray(y), kind="cubic")
        cases.append({
            "name": name,
            "x": [float(v) for v in x],
            "y": [float(v) for v in y],
            "query": [float(v) for v in q],
            "expect": [float(v) for v in f(np.asarray(q))],
        })

    add("linear-data", [0, 1, 2, 3, 4], [0, 1, 2, 3, 4], [0.5, 1.5, 2.5, 3.5])
    add("quadratic", [0, 1, 2, 3, 4, 5], [0, 1, 4, 9, 16, 25],
        [0.25, 1.75, 2.5, 4.9])
    rng = np.random.default_rng(7)
    xs = np.linspace(0.0, 1.0, 11)
    ys = np.sort(rng.random(11))
    add("random-monotone", xs, ys, list(np.linspace(0.02, 0.98, 25)))

    # The real shapes: est_votes_bias inverts an S-V curve (seats -> votes),
    # est_geometric_seats_bias interpolates a bias curve (votes -> seats).
    sys.path.insert(0, "vendor/rdapy")
    from rdapy import infer_sv_points, infer_inverse_sv_points  # noqa: E402

    profile = json.load(
        open("vendor/rdapy/testdata/partisan/nagle/partisan-NC-2012.json")
    )
    rv = profile["byDistrict"]
    vf = profile["statewide"]
    n = len(rv)
    d_pts = infer_sv_points(vf, rv, True)
    r_pts = infer_inverse_sv_points(d_pts, n)

    vx = [p[0] for p in d_pts]
    sy = [p[1] for p in d_pts]
    add("sv-curve-votes-to-seats", vx, sy, [0.30, 0.42, vf, 0.55, 0.70])
    # est_votes_bias: interp1d(y, x) -- seats as the independent variable
    add("sv-curve-seats-to-votes", sy, vx, [n / 4.0, n / 2.0, 3.0 * n / 4.0])
    bgs = [0.5 * (r_pts[i][1] - d_pts[i][1]) for i in range(len(d_pts))]
    add("geometric-seats-bias", vx, bgs, [0.35, vf, 0.60])

    emit(outdir, "not_a_knot_spline", cases,
         "scipy.interpolate.interp1d(kind='cubic') is bit-identical to "
         "make_interp_spline(x, y, k=3), i.e. a not-a-knot cubic B-spline. "
         "Used by est_votes_bias and est_geometric_seats_bias.")


### 7. Welzl minimum enclosing circle ###


def gen_welzl(outdir):
    sys.path.insert(0, "vendor/rdapy")
    from rdapy import wl_make_circle  # noqa: E402

    cases = []

    def add(name, pts):
        c = wl_make_circle([(float(a), float(b)) for a, b in pts])
        cases.append({
            "name": name,
            "points": [[float(a), float(b)] for a, b in pts],
            "expect": {"x": float(c.x), "y": float(c.y), "r": float(c.r)},
        })

    add("single-point", [(1.0, 2.0)])
    add("two-points", [(0.0, 0.0), (2.0, 0.0)])
    add("three-corners", [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)])
    add("colinear-three", [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)])
    add("colinear-many", [(float(i), 0.0) for i in range(12)])
    add("duplicates", [(0.0, 0.0), (0.0, 0.0), (1.0, 1.0), (1.0, 1.0)])
    add("unit-square", [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])

    rng = np.random.default_rng(99)
    add("random-50", list(map(tuple, rng.random((50, 2)))))
    add("random-500", list(map(tuple, rng.random((500, 2)) * 37.0 - 11.0)))
    # Points on a circle: the degenerate-ish case for MEC
    add("on-a-circle", [
        (math.cos(2 * math.pi * i / 64), math.sin(2 * math.pi * i / 64))
        for i in range(64)
    ])

    # A real district exterior: the actual shape of the workload, in lon/lat.
    data_path = "vendor/rdapy/testdata/score/NC_input_data.v1.jsonl"
    ext = []
    with open(data_path) as f:
        for line in f:
            rec = json.loads(line)
            if rec.get("_tag_") != "precinct":
                continue
            ext.extend(rec["data"]["exterior"])
            if len(ext) > 8000:
                break
    add("nc-exterior-8k", ext)

    emit(outdir, "welzl_min_enclosing_circle", cases,
         "rdapy's wl_make_circle: dedupes points, shuffles with a seeded "
         "global RNG, then runs iterative Welzl. The circle is mathematically "
         "unique, so a deterministic port must agree to ~1e-9 without "
         "reproducing the shuffle.")


def main():
    outdir = sys.argv[1] if len(sys.argv) > 1 else "conformance/cases/primitives"
    os.makedirs(outdir, exist_ok=True)
    print(f"recording primitive reference values into {outdir}/")
    gen_python_round(outdir)
    gen_python_round_to(outdir)
    gen_numpy_arange(outdir)
    gen_erf(outdir)
    gen_math_isclose(outdir)
    gen_spline(outdir)
    gen_welzl(outdir)
    print("done")


if __name__ == "__main__":
    main()
