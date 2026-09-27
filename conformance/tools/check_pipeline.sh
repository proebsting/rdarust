#!/bin/bash
# Run the whole pipeline from a bare DRA GeoJSON, end to end.
#
# check_rustrecom.sh starts from precinct data and a seed plan that already
# exist. This starts from the one file DRA publishes and builds everything
# else: the data map, the adjacency graph, the precinct data, the ReCom dual
# graph, a seed plan, a chain, and scores.
#
# partigraph and rustrecom live outside this repo, so this is a script rather
# than a test.
#
#   PARTIGRAPH=~/work/partigraph-rust/target/release/partigraph-seedmap \
#   RUSTRECOM=~/ext/rustrecom/target/release/rustrecom \
#     conformance/tools/check_pipeline.sh
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
RDARUST="${RDARUST:-$REPO/target/release/rdarust}"
RUSTRECOM="${RUSTRECOM:-$HOME/ext/rustrecom/target/release/rustrecom}"
PARTIGRAPH="${PARTIGRAPH:-$HOME/work/partigraph-rust/target/release/partigraph-seedmap}"
GEOJSON="${GEOJSON:-$REPO/vendor/rdapy/testdata/examples/NC_vtd_datasets.geojson}"
STATE="${STATE:-NC}"
PLAN_TYPE="${PLAN_TYPE:-congress}"
DISTRICTS="${DISTRICTS:-14}"
# Reversible ReCom self-loops heavily -- that is the price of its
# reversibility guarantee -- so a short chain can accept no proposal at all
# and rustrecom exits with an error. A few hundred steps is the useful floor.
STEPS="${STEPS:-1000}"
SEED="${SEED:-20260927}"

[[ -x "$RDARUST" ]]    || { echo "no rdarust at $RDARUST; cargo build --release" >&2; exit 1; }
[[ -x "$RUSTRECOM" ]]  || { echo "no rustrecom at $RUSTRECOM; set RUSTRECOM" >&2; exit 1; }
[[ -x "$PARTIGRAPH" ]] || { echo "no seed generator at $PARTIGRAPH; set PARTIGRAPH" >&2; exit 1; }
[[ -f "$GEOJSON" ]]    || { echo "no GeoJSON at $GEOJSON; set GEOJSON" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"
# Nothing but the published file to start with, as a user would have it.
cp "$GEOJSON" state.geojson

echo "== 1. data map"
"$RDARUST" map-data --geojson state.geojson --data-map data_map.json

echo "== 2. adjacency graph"
"$RDARUST" extract-graph --geojson state.geojson --graph graph.json

echo "== 3. precinct data"
"$RDARUST" extract-data --geojson state.geojson \
    --data-map data_map.json --graph graph.json --data data.jsonl

echo "== 4. ReCom dual graph"
"$RDARUST" to-recom-graph --state "$STATE" --plan-type "$PLAN_TYPE" \
    --data data.jsonl --graph graph.json --output recom.json

echo "== 5. seed plan, $DISTRICTS districts"
"$PARTIGRAPH" --graph recom.json --output seeded.json \
    --parts "$DISTRICTS" --pop-col TOTAL_POP --epsilon 0.01 --seed "$SEED"

echo "== 6. $STEPS ReCom steps"
"$RUSTRECOM" chain --graph-json seeded.json \
    --assignment-col district --pop-col TOTAL_POP \
    --n-steps "$STEPS" --n-threads 1 --batch-size 1 \
    --rng-seed "$SEED" --tol 0.05 --variant reversible --balance-ub 30 \
    --writer canonical > canonical.jsonl

echo "== 7. back to geoid assignments"
"$RDARUST" from-canonical --graph seeded.json \
    --input canonical.jsonl --output plans.jsonl

echo "== 8. scoring"
"$RDARUST" score-all --state "$STATE" --plan-type "$PLAN_TYPE" \
    --data data.jsonl --graph graph.json --plans plans.jsonl \
    --scores scores.csv --by-district by_district.jsonl

echo "== 9. geographic baseline"
"$RDARUST" find-neighborhoods --state "$STATE" --plan-type "$PLAN_TYPE" \
    --data data.jsonl --graph graph.json --output neighborhoods.jsonl
"$RDARUST" precompute-baselines --state "$STATE" --plan-type "$PLAN_TYPE" \
    --data data.jsonl --graph graph.json \
    --neighborhoods neighborhoods.jsonl --output baselines.json
"$RDARUST" check-neighborhoods --state "$STATE" --plan-type "$PLAN_TYPE" \
    --data data.jsonl --graph graph.json --neighborhoods neighborhoods.jsonl
"$RDARUST" score-all --state "$STATE" --plan-type "$PLAN_TYPE" \
    --data data.jsonl --graph graph.json --plans plans.jsonl \
    --precomputed baselines.json \
    --scores scores_geo.csv --by-district by_district_geo.jsonl

# The staged path and the fused one share their aggregation and scoring but
# not their plumbing, so disagreement means the plumbing drifted.
echo "== 10. staged path agrees with the fused one"
"$RDARUST" sample --input plans.jsonl --output some.jsonl -k 20
"$RDARUST" aggregate --state "$STATE" --plan-type "$PLAN_TYPE" \
    --data data.jsonl --graph graph.json --input some.jsonl --output agg.jsonl
"$RDARUST" score --state "$STATE" --plan-type "$PLAN_TYPE" \
    --data data.jsonl --graph graph.json --input agg.jsonl --output scored.jsonl
"$RDARUST" write --input scored.jsonl --data data.jsonl \
    --scores staged.csv --by-district staged_bd.jsonl
"$RDARUST" score-all --state "$STATE" --plan-type "$PLAN_TYPE" \
    --data data.jsonl --graph graph.json --plans some.jsonl \
    --scores fused.csv --by-district fused_bd.jsonl
cmp staged.csv fused.csv
cmp staged_bd.jsonl fused_bd.jsonl

echo
python3 - "$STEPS" "$DISTRICTS" <<'PY'
import csv, json, sys

steps, districts = int(sys.argv[1]), int(sys.argv[2])
fail = []

plans = [json.loads(l) for l in open("plans.jsonl")]
if len(plans) != steps:
    fail.append(f"{len(plans)} plans from a {steps}-step chain")

# from-canonical shifts rustrecom's 0-based numbering onto rdarust's, which
# starts at 1 because 0 means unassigned. Getting this wrong yields a
# plausible plan rather than an error, so check it explicitly.
seen = {d for p in plans for d in p["plan"].values()}
if seen != set(range(1, districts + 1)):
    fail.append(f"districts are {sorted(seen)[:4]}..., not 1..{districts}")

# A chain that never moved would score 1000 identical plans and look fine.
first, last = plans[0]["plan"], plans[-1]["plan"]
moved = sum(1 for g in first if first[g] != last[g])
if moved == 0:
    fail.append("the first and last plan are identical; the chain never moved")

base = list(csv.DictReader(open("scores.csv")))
geo = list(csv.DictReader(open("scores_geo.csv")))
if len(base) != len(plans):
    fail.append(f"{len(base)} rows scored from {len(plans)} plans")
if len({r["name"] for r in base}) != len(base):
    fail.append("scored plans do not have distinct names")
blank = sorted({c for r in base for c in r if r[c] == ""})
if blank:
    fail.append(f"columns with no value: {blank}")

# The baseline adds a column and must disturb nothing else.
added = [c for c in geo[0] if c not in base[0]]
if added != ["geographic_advantage"]:
    fail.append(f"the baseline added {added}, not just geographic_advantage")
if any(g[c] != b[c] for g, b in zip(geo, base) for c in base[0]):
    fail.append("the baseline changed a column it should not have")

print(f"{len(base)} plans scored, {len(base[0])} columns "
      f"({len(geo[0])} with the baseline)")
print(f"districts 1..{districts}, {moved} of {len(first)} precincts moved "
      f"between the first and last plan")
print("staged and fused output byte-identical")

if fail:
    print("\nFAIL:")
    for f in fail:
        print(f"  {f}")
    raise SystemExit(1)
print("\nOK")
PY
