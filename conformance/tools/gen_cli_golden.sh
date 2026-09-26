#!/bin/bash
# Record rdapy's pipeline output as a golden file for the CLI to match.
#
# The Rust CLI aims to be byte-identical here: a scores CSV you can diff
# directly against Python's is the cheapest way to know the two agree on every
# metric, every format and every column.
#
#   conformance/tools/gen_cli_golden.sh
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
RDAPY="$REPO/vendor/rdapy"
VENV="${RDAPY_VENV:-$HOME/ext/rdapy/.venv}"
OUT="$REPO/conformance/cases/cli"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$OUT"
cd "$RDAPY"
export PYTHONPATH="$RDAPY"
PY="$VENV/bin/python"

DATA=testdata/examples/NC_input_data.jsonl
GRAPH=testdata/examples/NC_graph.json
PRE=testdata/examples/NC_congress_precomputed.json
PLANS=testdata/plans/NC_congress_plans.tagged.jsonl

run() {  # run <extra write args> <output name>
  local extra="$1" name="$2"
  cat "$PLANS" \
  | "$PY" scripts/score/aggregate.py --state NC --plan-type congress \
        --data "$DATA" --graph "$GRAPH" \
  | "$PY" scripts/score/score.py --state NC --plan-type congress \
        --data "$DATA" --graph "$GRAPH" --precomputed "$PRE" \
  | "$PY" scripts/score/write.py --data "$DATA" \
        --scores "$OUT/$name" --by-district "$TMP/bd.jsonl" $extra
  echo "  $name: $(wc -l < "$OUT/$name") lines"
}

echo "recording rdapy's pipeline output into $OUT/"
run ""           NC_scores.csv
run "--prefixes" NC_scores.prefixed.csv
rm -f "$OUT"/*_metadata.json
echo "done"
