#!/usr/bin/env python3
"""
Derive conformance cases by running rdapy's OWN test suite with its functions
instrumented.

Rather than transcribing ~4,600 lines of assertions by hand -- which risks
typos and goes stale -- this wraps the functions we have ported, runs rdapy's
pytest suite, and records every call: the real arguments the tests pass and
the value rdapy actually returns.

What comes out is a corpus covering exactly what rdapy's tests exercise,
including calls made deep inside rdapy that no assertion names directly.

The suite must pass for the trace to be trusted; if pytest fails, so does this.

  PYTHONPATH=vendor/rdapy ~/ext/rdapy/.venv/bin/python \
      conformance/tools/trace_rdapy_tests.py conformance/cases/traced
"""

import json
import math
import os
import platform
import subprocess
import sys
from collections import defaultdict

REPO = os.path.abspath(os.path.dirname(os.path.dirname(os.path.dirname(__file__))))
RDAPY = os.path.join(REPO, "vendor", "rdapy")
sys.path.insert(0, RDAPY)

import numpy as np  # noqa: E402

SCHEMA = "rdarust.cases/1"

# Functions to instrument, as (dotted module, attribute). A function is
# patched in every namespace that binds it, so calls through `from x import *`
# re-exports are caught too.
TARGETS = [
    # partisan/method.py
    "est_seat_probability",
    "est_district_responsiveness",
    "est_seats",
    "est_fptp_seats",
    "infer_sv_points",
    "infer_inverse_sv_points",
    "infer_geometric_seats_bias_points",
    # partisan/bias.py
    "calc_best_seats",
    "calc_disproportionality",
    "calc_disproportionality_from_best",
    "calc_efficiency_gap",
    "calc_gamma",
    "est_seats_bias",
    "est_votes_bias",
    "est_geometric_seats_bias",
    "calc_global_symmetry",
    "key_RV_points",
    "is_sweep",
    "calc_declination",
    "calc_mean_median_difference",
    "calc_turnout_bias",
    "calc_lopsided_outcomes",
    # partisan/responsiveness.py
    "count_competitive_districts",
    "est_competitive_districts",
    "est_district_competitiveness",
    "est_responsive_districts",
    "est_responsiveness",
    "calc_big_R",
    "calc_minimal_inverse_responsiveness",
    # partisan/more.py
    "calc_efficiency_gap_wasted_votes",
    "calc_average_margin",
    # equal/population.py
    "calc_population_deviation",
    # splitting/county.py
    "calc_splitting_metrics",
    "split_score",
    "_county_totals",
    "_district_totals",
    "_reduce_county_splits",
    "_reduce_district_splits",
    "_calc_county_weights",
    "_calc_district_weights",
    "_calc_county_fractions",
    "_calc_district_fractions",
    "_county_split_score",
    "_district_split_score",
    "_county_splitting",
    "_district_splitting",
    "_calc_county_splitting",
    "_calc_county_splitting_reduced",
    "_calc_district_splitting",
    "_calc_district_splitting_reduced",
    "_population_weight",
    "_reverse_weight",
    # splitting/coi.py
    "uncertainty_of_membership",
    "effective_splits",
    # minority/minority.py
    "calc_proportional_districts",
    "est_minority_opportunity",
    "calc_minority_metrics",
    # minority/majority_minority.py
    "calculate_mmd_simple",
    "_is_single_demo_mmd",
    "_is_coalition_mmd",
]

TRACE = defaultdict(list)
SEEN = defaultdict(set)


def jsonable(v, depth=0):
    """Convert a value to something JSON can hold, or raise."""
    if depth > 6:
        raise ValueError("too deep")
    if isinstance(v, bool) or v is None:
        return v
    if isinstance(v, (int,)):
        return int(v)
    if isinstance(v, float) or isinstance(v, np.floating):
        f = float(v)
        if math.isnan(f) or math.isinf(f):
            raise ValueError("non-finite")
        return f
    if isinstance(v, np.integer):
        return int(v)
    if isinstance(v, np.ndarray):
        # scipy's interp1d returns a 0-d array, whose .tolist() is a scalar.
        if v.ndim == 0:
            return jsonable(v.item(), depth + 1)
        return [jsonable(x, depth + 1) for x in v.tolist()]
    if isinstance(v, (list, tuple)):
        return [jsonable(x, depth + 1) for x in v]
    if isinstance(v, dict):
        return {str(k): jsonable(x, depth + 1) for k, x in v.items()}
    raise ValueError(f"unsupported type {type(v).__name__}")


def calling_test():
    """The pytest test function that called us directly, if any.

    A direct call from a test is an assertion the rdapy authors wrote by hand:
    a published reference value. A call from inside rdapy is incidental
    coverage. The two are worth keeping in different quantities.
    """
    try:
        f = sys._getframe(2)
    except ValueError:
        return None
    name = f.f_code.co_name
    if name.startswith("test_") or name.startswith("check_"):
        return f"{os.path.basename(f.f_code.co_filename)}::{name}"
    return None


def make_wrapper(name, orig):
    def wrapper(*args, **kwargs):
        result = orig(*args, **kwargs)
        try:
            rec = {
                "input": jsonable(list(args)),
                "expect": jsonable(result),
            }
            if kwargs:
                rec["kwargs"] = jsonable(kwargs)
            origin = calling_test()
            if origin:
                rec["from"] = origin
            key = json.dumps([rec["input"], rec.get("kwargs")], sort_keys=True)
            if key not in SEEN[name]:
                SEEN[name].add(key)
                TRACE[name].append(rec)
        except Exception:
            # A call we cannot represent as data -- skip it rather than
            # recording something misleading.
            pass
        return result

    wrapper.__name__ = name
    wrapper.__wrapped_by_tracer__ = True
    return wrapper


# Keeping every internal call would put megabytes of redundant cases in the
# repo -- est_seat_probability alone gets called 42k times, mostly on shifted
# copies of the same curve. Cases asserted directly by a test are always kept;
# incidental ones are sampled at an even stride.
MAX_INCIDENTAL = 1500

# Cases whose arguments are whole seats-votes curves cost ~4KB each, so a
# count-based cap alone still produces megabytes. Budget by payload instead.
BUDGET_BYTES = 150_000
MIN_INCIDENTAL = 5


def sample(cases):
    direct = [c for c in cases if "from" in c]
    incidental = [c for c in cases if "from" not in c]
    if not incidental:
        return direct

    # Measure as it will be written -- indented -- not compact.
    # Measure every case, not just the first. Case sizes vary hugely within a
    # single function -- the splitting helpers are called both on tiny
    # hand-built matrices and on a 15x100 state matrix -- so estimating from
    # one sample lets a function blow the budget by an order of magnitude.
    sizes = [len(json.dumps(c, indent=2)) for c in incidental]

    def stride_pick(k):
        step = len(incidental) / k
        return [int(i * step) for i in range(k)]

    cap = min(MAX_INCIDENTAL, len(incidental))
    while cap > MIN_INCIDENTAL:
        if sum(sizes[i] for i in stride_pick(cap)) <= BUDGET_BYTES:
            break
        cap = max(MIN_INCIDENTAL, cap * 3 // 4)

    if cap < len(incidental):
        incidental = [incidental[i] for i in stride_pick(cap)]
    return direct + incidental


def patch_everywhere():
    """Wrap each target in every rdapy namespace that binds it."""
    import rdapy  # noqa: F401

    modules = [m for n, m in sys.modules.items()
               if n == "rdapy" or (n or "").startswith("rdapy.")]
    originals = {}
    count = 0
    for name in TARGETS:
        orig = None
        for m in modules:
            if hasattr(m, name) and callable(getattr(m, name)):
                orig = getattr(m, name)
                break
        if orig is None:
            print(f"  WARNING: {name} not found in rdapy", file=sys.stderr)
            continue
        originals[name] = orig
        wrapper = make_wrapper(name, orig)
        for m in modules:
            if getattr(m, name, None) is orig:
                setattr(m, name, wrapper)
                count += 1
    print(f"instrumented {len(originals)} functions across {count} bindings")
    return originals


def provenance():
    try:
        sha = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=RDAPY, text=True
        ).strip()
    except Exception:
        sha = "unknown"
    return {"rdapy_commit": sha, "python": platform.python_version(),
            "numpy": np.__version__, "platform": platform.platform()}


def main():
    outdir = os.path.abspath(
        sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "conformance/cases/traced")
    )
    os.makedirs(outdir, exist_ok=True)

    originals = patch_everywhere()

    # rdapy's tests use paths relative to its own root.
    os.chdir(RDAPY)
    import pytest  # noqa: E402

    print("running rdapy's test suite with instrumentation ...")
    rc = pytest.main(["-q", "--no-header", "-p", "no:cacheprovider", "test"])
    if rc != 0:
        print(f"\nrdapy's suite failed (exit {rc}); not recording a trace.",
              file=sys.stderr)
        sys.exit(1)

    src = provenance()
    total = 0
    for name in sorted(TRACE):
        cases = sample(TRACE[name])
        n_direct = sum(1 for c in cases if "from" in c)
        doc = {
            "schema": SCHEMA,
            "suite": "traced",
            "function": name,
            "note": ("Recorded by instrumenting rdapy and running its own test "
                     "suite. Arguments are exactly what rdapy's tests exercise. "
                     "Cases with a \"from\" field were asserted directly by that "
                     "test; the rest are incidental interior calls, sampled."),
            "source": src,
            "cases": cases,
        }
        with open(os.path.join(outdir, f"{name}.json"), "w") as f:
            json.dump(doc, f, indent=2)
            f.write("\n")
        total += len(cases)
        print(f"  {name}: {len(cases)} cases ({n_direct} asserted directly by a test)")

    missed = [n for n in originals if n not in TRACE]
    if missed:
        print(f"\nnot exercised by rdapy's tests: {', '.join(sorted(missed))}")
    print(f"\n{total} cases recorded into {outdir}/")


if __name__ == "__main__":
    main()
