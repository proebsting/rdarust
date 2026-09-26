#!/usr/bin/env python3
"""
Record reference values for rdapy's five 0-100 DRA ratings.

Two kinds of case are emitted:

  origin "rdapy-test"  -- transcribed from rdapy's own test_rate.py. These are
                          the authoritative values: DRA developed the ratings
                          in a spreadsheet and the test encodes that table.
                          Each transcription is CHECKED against live rdapy, so
                          a typo here fails the generator rather than silently
                          weakening the corpus.
  origin "sweep"       -- recorded from rdapy over a broad input range, to
                          catch divergence away from the hand-picked points.

Every case carries rdapy's exact value in "expect"; cases from the test suite
also carry the literal the test asserts in "published", with the tolerance the
test used.

  PYTHONPATH=vendor/rdapy ~/ext/rdapy/.venv/bin/python \
      conformance/tools/gen_rate.py conformance/cases/rate
"""

import json
import os
import platform
import subprocess
import sys

import numpy as np

sys.path.insert(0, "vendor/rdapy")

from rdapy import (  # noqa: E402
    EPSILON,
    rate_competitiveness,
    rate_compactness,
    rate_county_splitting,
    rate_district_splitting,
    rate_minority_opportunity,
    rate_polsby,
    rate_proportionality,
    rate_reock,
    rate_splitting,
)
from rdapy.rate import (  # noqa: E402
    AVG_SV_ERROR,
    POLSBY_MAX,
    POLSBY_MIN,
    REOCK_MAX,
    REOCK_MIN,
    Normalizer,
    adjust_deviation,
    best_target,
    extra_bonus,
    is_antimajoritarian,
)

SCHEMA = "rdarust.cases/1"
FAILURES = []


def provenance():
    try:
        sha = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd="vendor/rdapy", text=True
        ).strip()
    except Exception:
        sha = "unknown"
    return {"rdapy_commit": sha, "python": platform.python_version(),
            "numpy": np.__version__, "platform": platform.platform()}


class Suite:
    def __init__(self, function, note):
        self.function = function
        self.note = note
        self.cases = []

    def published(self, fn, args, asserted, places=None, name=None):
        """A case transcribed from rdapy's tests; verify it against rdapy."""
        actual = fn(*args)
        if places is None:
            ok = actual == asserted
        else:
            ok = abs(actual - asserted) <= 0.5 * 10 ** (-places)
        if not ok:
            FAILURES.append(
                f"{self.function}{tuple(args)}: rdapy gives {actual!r}, "
                f"test_rate.py asserts {asserted!r}"
                + (f" (places={places})" if places is not None else "")
            )
        case = {"input": list(args), "expect": actual,
                "published": asserted, "origin": "rdapy-test"}
        if places is not None:
            case["places"] = places
        if name:
            case["name"] = name
        self.cases.append(case)

    def sweep(self, fn, args):
        self.cases.append(
            {"input": list(args), "expect": fn(*args), "origin": "sweep"}
        )

    def emit(self, outdir):
        doc = {"schema": SCHEMA, "suite": "rate", "function": self.function,
               "note": self.note, "source": provenance(), "cases": self.cases}
        path = os.path.join(outdir, f"{self.function}.json")
        with open(path, "w") as f:
            json.dump(doc, f, indent=2)
            f.write("\n")
        pub = sum(1 for c in self.cases if c["origin"] == "rdapy-test")
        print(f"  {self.function}: {len(self.cases)} cases ({pub} from rdapy's tests)")


### NORMALIZER PRIMITIVES ###


def norm_op(op, raw, *args):
    """Apply one Normalizer step and return the work-in-progress value."""
    n = Normalizer(raw)
    getattr(n, op)(*args)
    return n.wip_num


def norm_rescale(raw):
    n = Normalizer(raw)
    n.rescale()
    return n.normalized_num


def gen_normalizer(outdir):
    s = Suite("normalizer_invert", "Normalizer.invert: 1 - x on a unit value.")
    for x, want in [(0.75 / 100, 0.9925), (0.0, 1.0), (1.0, 0.0)]:
        s.published(lambda v: norm_op("invert", v), (x,), want, places=6)
    for x in np.linspace(0.0, 1.0, 21):
        s.sweep(lambda v: norm_op("invert", v), (float(x),))
    s.emit(outdir)

    s = Suite("normalizer_clip", "Normalizer.clip: endpoints may arrive in either order.")
    for x, want in [(0.3773, 0.3773), (0.2, 0.25), (0.55, 0.5), (0.25, 0.25), (0.5, 0.5)]:
        s.published(lambda v, a, b: norm_op("clip", v, a, b), (x, 0.25, 0.50), want, places=6)
    for x in np.linspace(-0.5, 1.5, 21):
        s.sweep(lambda v, a, b: norm_op("clip", v, a, b), (float(x), 0.25, 0.50))
        s.sweep(lambda v, a, b: norm_op("clip", v, a, b), (float(x), 0.50, 0.25))
    s.emit(outdir)

    s = Suite("normalizer_rebase", "Normalizer.rebase: delta from a baseline; may go negative.")
    for x, want in [(1.0, 0.0), (1.5, 0.5), (0.5, -0.5), (-2.0, -3.0)]:
        s.published(lambda v, b: norm_op("rebase", v, b), (x, 1.0), want, places=6)
    s.emit(outdir)

    s = Suite("normalizer_unitize",
              "Normalizer.unitize: divides by (end - begin) AS GIVEN, not by "
              "(hi - lo), then takes the absolute value. Reversed endpoints "
              "are used by the proportionality and splitting ratings.")
    for x, want in [(1.5, 0.5), (1.0, 0.0), (2.0, 1.0)]:
        s.published(lambda v, a, b: norm_op("unitize", v, a, b), (x, 1.0, 2.0), want, places=6)
    for x in np.linspace(0.0, 0.2, 11):
        s.sweep(lambda v, a, b: norm_op("unitize", v, a, b), (float(x), 0.20, 0.0))
        s.sweep(lambda v, a, b: norm_op("unitize", v, a, b), (float(x), 0.0, 0.20))
    s.emit(outdir)

    s = Suite("normalizer_decay", "Normalizer.decay: square a unit value.")
    for x, want in [(0.90, 0.81), (0.0, 0.0), (1.0, 1.0)]:
        s.published(lambda v: norm_op("decay", v), (x,), want, places=6)
    s.emit(outdir)

    s = Suite("normalizer_rescale",
              "Normalizer.rescale: round(x * 100) with CPython's round, so "
              "ties go to even. This is where banker's rounding is visible.")
    for x, want in [(0.63, 63), (0.6372, 64), (0.6312, 63), (0.0, 0), (1.0, 100)]:
        s.published(norm_rescale, (x,), want)
    # Every exact half-percent tie, where half-to-even and half-away differ.
    for i in range(0, 201):
        s.sweep(norm_rescale, (i / 200.0,))
    for x in np.linspace(0.0, 1.0, 97):
        s.sweep(norm_rescale, (float(x),))
    s.emit(outdir)


### RATING HELPERS ###


def gen_helpers(outdir):
    s = Suite("is_antimajoritarian",
              "Did the minority of voters win a majority of seats? Needs to "
              "clear AVG_SV_ERROR to count.")
    for vf, sf, want in [
        (0.5 - AVG_SV_ERROR - EPSILON, 0.50 + EPSILON, True),
        (0.5 + AVG_SV_ERROR + EPSILON, 0.5 - EPSILON, True),
        (0.51, 0.53, False),
        (0.49, 0.47, False),
        (0.5 - EPSILON, 0.5 - EPSILON, False),
    ]:
        s.published(is_antimajoritarian, (vf, sf), want)
    for vf in np.linspace(0.3, 0.7, 17):
        for sf in np.linspace(0.0, 1.0, 11):
            s.sweep(is_antimajoritarian, (float(vf), float(sf)))
    s.emit(outdir)

    s = Suite("extra_bonus", "The winner's bonus allowed before deviation counts.")
    for vf, want in [(0.50, 0.0), (0.55, 0.10 / 2), (0.60, 0.20 / 2),
                     (0.45, 0.10 / 2), (0.40, 0.20 / 2)]:
        s.published(extra_bonus, (vf,), want, places=6)
    for vf in np.linspace(0.2, 0.8, 25):
        s.sweep(extra_bonus, (float(vf),))
    s.emit(outdir)

    s = Suite("adjust_deviation",
              "Discount deviation by the winner's bonus only when the bias "
              "runs WITH the statewide vote; bias against the winner is left "
              "alone. Real 116th-Congress values.")
    for name, vf, bias, want in [
        ("CA", 0.64037, -0.1714, -0.0310),
        ("NC", 0.488799, 0.2268, 0.2156),
        ("OH", 0.486929, 0.2367, 0.2236),
        ("PA", 0.51148, 0.0397, 0.0397),
        ("TX", 0.436994, 0.1216, 0.0586),
    ]:
        s.published(adjust_deviation, (vf, bias, extra_bonus(vf)), want, places=4, name=name)
    for vf in np.linspace(0.35, 0.65, 13):
        for bias in np.linspace(-0.3, 0.3, 13):
            s.sweep(adjust_deviation, (float(vf), float(bias), extra_bonus(float(vf))))
    s.emit(outdir)

    s = Suite("best_target",
              "The practically-best splitting score given counts of counties "
              "and districts; some splitting is unavoidable when one greatly "
              "outnumbers the other.")
    for nc, nd, want in [
        (67, 7, 1.02), (67, 35, 1.10), (67, 105, 1.13),
        (15, 9, 1.11), (15, 30, 1.09),
        (58, 53, 1.18), (58, 40, 1.13), (58, 80, 1.14),
        (64, 7, 1.02), (64, 35, 1.11), (64, 65, 1.19),
        (8, 5, 1.10), (8, 36, 1.04), (8, 151, 1.01),
        (159, 14, 1.02), (159, 56, 1.07), (159, 180, 1.18),
        (99, 4, 1.01), (99, 50, 1.10), (99, 100, 1.20),
        (120, 6, 1.01), (120, 38, 1.06), (120, 100, 1.17),
        (67, 18, 1.05), (67, 50, 1.15), (67, 203, 1.07),
        (95, 9, 1.02), (95, 33, 1.07), (95, 99, 1.19),
        (254, 36, 1.03), (254, 31, 1.02), (254, 150, 1.12),
        (133, 11, 1.02), (133, 40, 1.06), (133, 100, 1.15),
    ]:
        s.published(best_target, (nc, nd), want, places=2)
    s.emit(outdir)


### THE FIVE RATINGS ###


def gen_ratings(outdir):
    s = Suite("rate_proportionality",
              "Deviation from proportionality, after allowing a winner's "
              "bonus. Antimajoritarian outcomes score 0 outright.")
    for args, want in [
        ((0.00, 0.5, 0.5), 100), ((0.05, 0.5, 0.5), 75), ((0.10, 0.5, 0.5), 50),
        ((0.20, 0.5, 0.5), 0), ((0.25, 0.5, 0.5), 0),
        ((0.01, 0.48 - EPSILON, 0.5 + EPSILON), 0),
        ((0.01, 1 - 0.48 + EPSILON, 1 - 0.5 - EPSILON), 0),
    ]:
        s.published(rate_proportionality, args, want)
    for name, args, want in [
        ("CA 116th", (-0.1714, 0.6404, 43.0850 / 53), 84),
        ("CO 116th", (0.0006, 0.5286, 3.9959 / 7), 100),
        ("IL 116th", (-0.0585, 0.5838, 12.0531 / 18), 100),
        ("MA 116th", (-0.3331, 0.6321, 8.9985 / 9), 0),
        ("MD 116th", (-0.2500, 0.6336, 7.0000 / 8), 42),
        ("NC 116th", (0.2268, 0.4888, 3.0512 / 13), 0),
        ("OH 116th", (0.2367, 0.4869, 4.2120 / 16), 0),
        ("SC 116th", (0.2857, 0.4072, 1 / 7), 4),
        ("TN 116th", (0.1111, 0.3802, 2 / 9), 100),
        ("TX 116th", (0.1216, 0.4370, 11.6218 / 36), 71),
    ]:
        s.published(rate_proportionality, args, want, name=name)
    for dev in np.linspace(-0.35, 0.35, 15):
        for vf in np.linspace(0.35, 0.65, 13):
            s.sweep(rate_proportionality, (float(dev), float(vf), float(vf) + float(dev)))
    s.emit(outdir)

    s = Suite("rate_competitiveness", "Competitive-district share; 75% is a perfect score.")
    for cdf, want in [(0.00, 0), (0.25, 33), (0.50, 67), (0.75, 100), (0.80, 100)]:
        s.published(rate_competitiveness, (cdf,), want)
    for x in np.linspace(0.0, 1.0, 81):
        s.sweep(rate_competitiveness, (float(x),))
    s.emit(outdir)

    s = Suite("rate_minority_opportunity",
              "Opportunity districts against the proportional number, with "
              "coalition districts contributing half of any surplus.")
    for args, want in [
        ((1, 0, 0, 0), 0), ((0, 10, 0, 0), 0), ((5, 10, 0, 0), 50),
        ((10, 10, 0, 0), 100), ((11, 10, 0, 0), 100),
        ((12.56, 18, 22.92, 18), 85),
    ]:
        s.published(rate_minority_opportunity, args, want)
    rng = np.random.default_rng(5)
    for _ in range(200):
        pod = float(rng.integers(0, 15))
        pcd = float(rng.integers(0, 15))
        s.sweep(rate_minority_opportunity,
                (float(rng.random() * 18), pod, float(rng.random() * 25), pcd))
    s.emit(outdir)

    s = Suite("rate_reock", "Reock dispersion, unitized over [0.25, 0.50].")
    for x, want in [(0.3848, 54), (0.3373, 35), (REOCK_MIN, 0), (REOCK_MAX, 100),
                    (REOCK_MIN - EPSILON, 0), (REOCK_MAX + EPSILON, 100)]:
        s.published(rate_reock, (x,), want)
    for x in np.linspace(0.0, 0.75, 76):
        s.sweep(rate_reock, (float(x),))
    s.emit(outdir)

    s = Suite("rate_polsby", "Polsby-Popper indentation, unitized over [0.10, 0.50].")
    for x, want in [(0.1860, 21), (0.2418, 35), (POLSBY_MIN, 0), (POLSBY_MAX, 100),
                    (POLSBY_MIN - EPSILON, 0), (POLSBY_MAX + EPSILON, 100)]:
        s.published(rate_polsby, (x,), want)
    for x in np.linspace(0.0, 0.75, 76):
        s.sweep(rate_polsby, (float(x),))
    s.emit(outdir)

    s = Suite("rate_compactness", "Even blend of the Reock and Polsby ratings.")
    s.published(rate_compactness, (30, 60), 45)
    for r in range(0, 101, 7):
        for p in range(0, 101, 11):
            s.sweep(rate_compactness, (r, p))
    s.emit(outdir)

    # The state-by-state splitting table from DRA's development spreadsheet.
    table = [
        ("AL", 67, [(7, 1.1100, 1.4470, 73, 38), (35, 1.4200, 1.4500, 12, 37),
                    (105, 1.6400, 1.2600, 0, 64)]),
        ("AZ", 15, [(9, 1.3520, 1.4240, 33, 43), (30, 1.7100, 1.2000, 0, 70)]),
        ("CA", 58, [(53, 1.7890, 1.2530, 0, 87), (40, 1.7400, 1.3400, 0, 65),
                    (80, 1.7000, 1.1900, 0, 87)]),
        ("CO", 64, [(7, 1.1960, 1.5010, 47, 24), (35, 1.2100, 1.0800, 72, 99),
                    (65, 1.2500, 1.0700, 87, 99)]),
        ("CT", 8, [(5, 1.4800, 1.5310, 0, 16), (36, 2.0800, 1.1700, 0, 62),
                   (151, 1.6800, 1.0500, 0, 88)]),
        ("GA", 159, [(14, 1.2960, 1.6400, 17, 0), (56, 1.5800, 1.3900, 0, 52),
                     (180, 1.7800, 1.2700, 0, 76)]),
        ("IA", 99, [(4, 1.0000, 1.0000, 100, 100), (50, 1.3300, 1.2900, 36, 77),
                    (100, 1.2600, 1.2000, 85, 99)]),
        ("KY", 120, [(6, 1.0360, 1.2230, 92, 94), (38, 1.2100, 1.0800, 58, 99),
                     (100, 1.4300, 1.2000, 31, 99)]),
        ("PA", 67, [(18, 1.1780, 1.4080, 63, 47), (50, 1.5200, 1.3000, 1, 75),
                    (203, 1.6500, 1.1300, 0, 82), (203, 1.3100, 1.1800, 72, 67)]),
        ("TN", 95, [(9, 1.0710, 1.2670, 84, 83), (33, 1.1400, 1.1000, 79, 99),
                    (99, 1.1000, 1.1400, 99, 99)]),
        ("TX", 254, [(36, 1.5790, 1.4280, 0, 42), (31, 1.4600, 1.3300, 0, 67),
                     (150, 1.0800, 1.0400, 99, 99)]),
        ("VA", 133, [(11, 1.2140, 1.6900, 41, 0), (40, 1.6400, 1.7000, 0, 0),
                     (100, 1.8400, 1.4200, 0, 44)]),
    ]

    sc = Suite("rate_county_splitting",
               "County splitting. 100 is reserved for plans that split nothing.")
    sd = Suite("rate_district_splitting",
               "District splitting: the mirror of county splitting.")
    for xx, nc, rows in table:
        for nd, craw, draw, cwant, dwant in rows:
            sc.published(rate_county_splitting, (craw, nc, nd), cwant, name=f"{xx} {nd}")
            sd.published(rate_district_splitting, (draw, nc, nd), dwant, name=f"{xx} {nd}")
    for raw in np.linspace(1.0, 2.0, 21):
        for nc, nd in [(67, 7), (15, 30), (99, 100), (254, 150), (8, 151)]:
            sc.sweep(rate_county_splitting, (float(raw), nc, nd))
            sd.sweep(rate_district_splitting, (float(raw), nc, nd))
    sc.emit(outdir)
    sd.emit(outdir)

    s = Suite("rate_splitting",
              "Even blend of the two splitting ratings; 100 again reserved "
              "for a plan that splits nothing at all.")
    for args, want in [((99, 100), 99), ((100, 100), 100)]:
        s.published(rate_splitting, args, want)
    for c in range(0, 101, 5):
        for d in range(0, 101, 5):
            s.sweep(rate_splitting, (c, d))
    s.emit(outdir)


def main():
    outdir = sys.argv[1] if len(sys.argv) > 1 else "conformance/cases/rate"
    os.makedirs(outdir, exist_ok=True)
    print(f"recording rate reference values into {outdir}/")
    gen_normalizer(outdir)
    gen_helpers(outdir)
    gen_ratings(outdir)
    if FAILURES:
        print("\nTRANSCRIPTION MISMATCHES (case transcribed from test_rate.py "
              "disagrees with live rdapy):", file=sys.stderr)
        for f in FAILURES:
            print(f"  {f}", file=sys.stderr)
        sys.exit(1)
    print("done -- every transcribed case verified against rdapy")


if __name__ == "__main__":
    main()
