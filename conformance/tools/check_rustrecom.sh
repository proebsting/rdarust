#!/bin/bash
# Run a chain through rustrecom and score the result, end to end.
#
# rustrecom lives outside this repo, so this is a script rather than a test.
# It builds a seeded ReCom graph, runs a short chain, converts the output back
# to geoid assignments, and scores every plan.
#
#   RUSTRECOM=~/ext/rustrecom/target/release/rustrecom \
#     conformance/tools/check_rustrecom.sh
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
RDARUST="${RDARUST:-$REPO/target/release/rdarust}"
RUSTRECOM="${RUSTRECOM:-$HOME/ext/rustrecom/target/release/rustrecom}"
V="$REPO/vendor/rdapy/testdata"
STEPS="${STEPS:-200}"

[[ -x "$RDARUST" ]]   || { echo "no rdarust at $RDARUST; cargo build --release" >&2; exit 1; }
[[ -x "$RUSTRECOM" ]] || { echo "no rustrecom at $RUSTRECOM; set RUSTRECOM" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "== building a seeded ReCom graph"
"$RDARUST" to-recom-graph --state NC \
    --data "$V/examples/NC_input_data.jsonl" \
    --graph "$V/examples/NC_graph.json" \
    --assignment "$V/plans/NC_congress_plans.tagged.jsonl" \
    --output "$TMP/graph.json"

echo "== running $STEPS ReCom steps"
"$RUSTRECOM" chain \
    --graph-json "$TMP/graph.json" \
    --assignment-col INITIAL \
    --pop-col TOTAL_POP \
    --n-steps "$STEPS" --n-threads 4 --rng-seed 20260927 \
    --tol 0.02 --batch-size 1 --variant cut-edges-ust \
    --writer canonical > "$TMP/canonical.jsonl"

echo "== converting to geoid assignments"
"$RDARUST" from-canonical --graph "$TMP/graph.json" \
    --input "$TMP/canonical.jsonl" --output "$TMP/plans.jsonl"

echo "== scoring"
"$RDARUST" score-all --state NC --plan-type congress \
    --data "$V/examples/NC_input_data.jsonl" \
    --graph "$V/examples/NC_graph.json" \
    --precomputed "$V/examples/NC_congress_precomputed.json" \
    --plans "$TMP/plans.jsonl" \
    --scores "$TMP/scores.csv" --by-district "$TMP/by-district.jsonl"

PLANS="$(grep -c . "$TMP/canonical.jsonl")"
SCORED="$(( $(wc -l < "$TMP/scores.csv") - 1 ))"
echo
if [[ "$PLANS" != "$SCORED" ]]; then
  echo "FAIL: $PLANS plans from the chain, $SCORED scored" >&2
  exit 1
fi
echo "OK: $SCORED of $PLANS plans scored"
