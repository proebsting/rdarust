#!/usr/bin/env python3
"""
Record rdapy's scorecards for real plans, as end-to-end golden cases.

The per-function corpus checks the formulas. This checks the whole pipeline:
reading precinct data, aggregating by district, and scoring -- on the actual
North Carolina and New Jersey data rdapy's own test_scorecard.py uses, plus a
sample of plans from the NC ensemble.

Plans are referenced by file and index rather than inlined; the Rust runner
reads the same files. Inlining ~2700 assignments per plan would add megabytes
and prove nothing extra.

  PYTHONPATH=vendor/rdapy ~/ext/rdapy/.venv/bin/python \
      conformance/tools/gen_scorecards.py conformance/cases/scorecard
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
    OUT_OF_STATE,
    aggregate_districts,
    collect_metadata,
    load_data,
    load_graph,
    read_csv,
    read_record,
    score_plan,
    smart_read,
    sorted_geoids,
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


def unpack_input_data(path):
    """rdapy's test helper: metadata, precincts and adjacency from one file."""
    metadata, precincts, graph = {}, [], {}
    with smart_read(path) as stream:
        for line in stream:
            rec = read_record(line)
            if "_tag_" not in rec:
                continue
            if rec["_tag_"] == "metadata":
                metadata = rec["properties"]
            else:
                data = rec["data"]
                if data["geoid"] != OUT_OF_STATE:
                    precincts.append(data)
                graph[data["geoid"]] = data["neighbors"]
    return metadata, precincts, graph


def plan_from_csv(path):
    rows = read_csv(path, [str, int])
    geoid_field = next(f for f in ("GEOID", "GEOID20", "GEOID30") if f in rows[0])
    district_field = next(f for f in ("District", "DISTRICT") if f in rows[0])
    return {str(r[geoid_field]): int(r[district_field]) for r in rows}


def plans_from_jsonl(path, limit):
    out = []
    with smart_read(path) as stream:
        for line in stream:
            rec = read_record(line)
            if rec.get("_tag_") != "plan":
                continue
            out.append({str(k): int(v) for k, v in rec["plan"].items()})
            if len(out) >= limit:
                break
    return out


def jsonable(v):
    if isinstance(v, dict):
        return {str(k): jsonable(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [jsonable(x) for x in v]
    if isinstance(v, (np.floating,)):
        return float(v)
    if isinstance(v, (np.integer,)):
        return int(v)
    return v


def score(xx, plan_type, data_path, plan, *, mmd, reverse):
    metadata_in, precincts, graph = unpack_input_data(data_path)
    geoids = sorted_geoids(precincts)
    meta = collect_metadata(xx, plan_type, geoids)

    aggs = aggregate_districts(
        plan, precincts, graph, meta, which="all", data_metadata=metadata_in
    )
    card, updated = score_plan(
        plan, aggs,
        data=precincts, graph=graph, metadata=meta, data_map=metadata_in,
        mode="all", mmd_scoring=mmd, precomputed={},
        reverse_weight_splitting=reverse,
    )
    # Keep only the by-district series scoring produces; the raw aggregates
    # are an input, not a result.
    by_district = {}
    for t in ("shapes", "census"):
        if t in updated:
            for ds, series in updated[t].items():
                for k in ("reock", "polsby_popper", "district_splitting"):
                    if k in series:
                        by_district[k] = jsonable(series[k])
    return jsonable(card), by_district


def score_v2(xx, plan_type, data_path, graph_path, precomputed_path, plan,
             *, mmd, reverse):
    """The pipeline the CLI uses: data and graph in separate files.

    Input files written for scoring carry no neighbour lists -- the graph is
    maintained separately -- so this exercises a different assembly path from
    the legacy single-file form, as well as several elections at once and
    majority-minority scoring.
    """
    data_map, precincts = load_data(data_path)
    graph = load_graph(graph_path)
    geoids = sorted_geoids(precincts)
    meta = collect_metadata(xx, plan_type, geoids)

    precomputed = {}
    if precomputed_path:
        with open(precomputed_path) as f:
            precomputed = json.load(f)

    aggs = aggregate_districts(
        plan, precincts, graph, meta, which="all", data_metadata=data_map
    )
    card, updated = score_plan(
        plan, aggs,
        data=precincts, graph=graph, metadata=meta, data_map=data_map,
        mode="all", mmd_scoring=mmd, precomputed=precomputed,
        reverse_weight_splitting=reverse,
    )
    by_district = {}
    for t in ("shapes", "census"):
        if t in updated:
            for ds, series in updated[t].items():
                for k in ("reock", "polsby_popper", "district_splitting"):
                    if k in series:
                        by_district[k] = jsonable(series[k])
    return jsonable(card), by_district


def main():
    outdir = os.path.abspath(
        sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "conformance/cases/scorecard")
    )
    os.makedirs(outdir, exist_ok=True)
    os.chdir(RDAPY)

    cases = []

    # The two baseline plans rdapy's own test_scorecard.py checks.
    for xx in ("NC", "NJ"):
        data = f"testdata/score/{xx}_input_data.v1.jsonl"
        plan_path = f"testdata/score/{xx}20C_baseline_100.csv"
        plan = plan_from_csv(plan_path)
        card, by_district = score(xx, "congress", data, plan, mmd=False, reverse=False)
        cases.append({
            "name": f"{xx} baseline",
            "state": xx, "plan_type": "congress", "data": data,
            "plan": {"kind": "csv", "path": plan_path},
            "options": {"mmd_scoring": False, "reverse_weight_splitting": False},
            "expect": card, "by_district": by_district,
        })
        print(f"  {xx} baseline")

    # Reverse-weighted county splitting, which rdapy's tests never exercise
    # end to end.
    plan = plan_from_csv("testdata/score/NC20C_baseline_100.csv")
    card, by_district = score(
        "NC", "congress", "testdata/score/NC_input_data.v1.jsonl",
        plan, mmd=False, reverse=True,
    )
    cases.append({
        "name": "NC baseline, reverse-weighted splitting",
        "state": "NC", "plan_type": "congress",
        "data": "testdata/score/NC_input_data.v1.jsonl",
        "plan": {"kind": "csv", "path": "testdata/score/NC20C_baseline_100.csv"},
        "options": {"mmd_scoring": False, "reverse_weight_splitting": True},
        "expect": card, "by_district": by_district,
    })
    print("  NC baseline (reverse-weighted)")

    # A sample of real ensemble plans: these vary far more than the baselines.
    ensemble = "testdata/plans/NC_congress_plans.tagged.jsonl"
    plans = plans_from_jsonl(ensemble, 20)
    for i, plan in enumerate(plans):
        card, by_district = score(
            "NC", "congress", "testdata/score/NC_input_data.v1.jsonl",
            plan, mmd=False, reverse=False,
        )
        cases.append({
            "name": f"NC ensemble plan {i}",
            "state": "NC", "plan_type": "congress",
            "data": "testdata/score/NC_input_data.v1.jsonl",
            "plan": {"kind": "jsonl", "path": ensemble, "index": i},
            "options": {"mmd_scoring": False, "reverse_weight_splitting": False},
            "expect": card, "by_district": by_district,
        })
    print(f"  {len(plans)} NC ensemble plans")

    # The pipeline layout: data and graph in separate files, seven elections,
    # CVAP present so majority-minority scoring runs, and a precomputed
    # geographic baseline.
    v2_data = "testdata/examples/NC_input_data.jsonl"
    v2_graph = "testdata/examples/NC_graph.json"
    v2_pre = "testdata/examples/NC_congress_precomputed.json"
    for i, plan in enumerate(plans_from_jsonl(ensemble, 5)):
        card, by_district = score_v2(
            "NC", "congress", v2_data, v2_graph, v2_pre, plan, mmd=True, reverse=False,
        )
        cases.append({
            "name": f"NC multi-dataset plan {i}",
            "state": "NC", "plan_type": "congress",
            "data": v2_data, "graph": v2_graph, "precomputed": v2_pre,
            "plan": {"kind": "jsonl", "path": ensemble, "index": i},
            "options": {"mmd_scoring": True, "reverse_weight_splitting": False},
            "expect": card, "by_district": by_district,
        })
    print("  5 NC multi-dataset plans (7 elections, MMD, geographic baseline)")

    doc = {
        "schema": SCHEMA, "suite": "scorecard",
        "note": ("End-to-end scorecards from rdapy on real data. Plans are "
                 "referenced by file and index, relative to vendor/rdapy."),
        "source": provenance(), "cases": cases,
    }
    path = os.path.join(outdir, "scorecards.json")
    with open(path, "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    print(f"\n{len(cases)} scorecards -> {path}")


if __name__ == "__main__":
    main()
