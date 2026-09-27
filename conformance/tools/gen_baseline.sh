#!/bin/bash
# Record rdapy's geographic baseline for the checked-in North Carolina data.
#
# The neighbourhoods themselves are already in the submodule
# (testdata/examples/NC_congress_neighborhoods.jsonl) and are compared against
# directly. Only the baseline computed from them is recorded here.
#
# Note the submodule's NC_congress_precomputed.json does NOT correspond to its
# NC_input_data.jsonl -- it was produced from a larger dataset carrying 22
# elections rather than 7, and the shared election names hold different vote
# counts. This regenerates it from the data actually checked in.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
RDAPY="$REPO/vendor/rdapy"
VENV="${RDAPY_VENV:-$HOME/ext/rdapy/.venv}"
OUT="$REPO/conformance/cases/baseline"

mkdir -p "$OUT"
cd "$RDAPY"
export PYTHONPATH="$RDAPY"

"$VENV/bin/python" scripts/geographic-baseline/precompute_baselines.py \
    --state NC --plan-type congress \
    --data testdata/examples/NC_input_data.jsonl \
    --neighborhoods testdata/examples/NC_congress_neighborhoods.jsonl \
    > "$OUT/NC_precomputed.json"

echo "recorded $(python3 -c "import json;print(len(json.load(open('$OUT/NC_precomputed.json'))['geographic_baseline']))") elections"
