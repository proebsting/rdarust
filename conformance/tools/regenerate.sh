#!/bin/bash
# Regenerate the whole conformance corpus from the pinned rdapy.
#
# Run this after moving the submodule. It records the rdapy commit and the
# dependency versions it read them from into cases/PROVENANCE.json, which a
# Rust test checks against the live submodule -- so a corpus recorded at one
# commit cannot quietly be used to validate another.
#
#   conformance/tools/regenerate.sh
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
RDAPY="$REPO/vendor/rdapy"
VENV="${RDAPY_VENV:-$HOME/ext/rdapy/.venv}"
PY="$VENV/bin/python"
CASES="$REPO/conformance/cases"

[[ -x "$PY" ]] || { echo "no rdapy interpreter at $PY; set RDAPY_VENV" >&2; exit 1; }
[[ -d "$RDAPY/rdapy" ]] || { echo "submodule not initialised: git submodule update --init" >&2; exit 1; }

cd "$REPO"
SHA="$(git -C "$RDAPY" rev-parse HEAD)"
DIRTY="$(git -C "$RDAPY" status --porcelain | wc -l | tr -d ' ')"
if [[ "$DIRTY" != "0" ]]; then
  echo "WARNING: $RDAPY has $DIRTY modified file(s); the corpus will not be" >&2
  echo "         reproducible from commit $SHA alone." >&2
fi
echo "regenerating against rdapy $SHA"

run() { echo "-- $1"; shift; PYTHONPATH="$RDAPY" "$PY" "$@"; }

run "numeric primitives"      conformance/tools/gen_primitives.py "$CASES/primitives"
run "DRA ratings"             conformance/tools/gen_rate.py       "$CASES/rate"
run "traced from rdapy tests" conformance/tools/trace_rdapy_tests.py "$CASES/traced"
run "supplementary cases"     conformance/tools/gen_supplement.py "$CASES/supplement"
run "whole scorecards"        conformance/tools/gen_scorecards.py "$CASES/scorecard"
run "shape compactness"       conformance/tools/gen_shapes.py     "$CASES/shapes"

echo "-- CLI golden files"; conformance/tools/gen_cli_golden.sh
echo "-- geographic baseline"; conformance/tools/gen_baseline.sh

# Record what all of that was read from.
"$PY" - "$SHA" "$DIRTY" > "$CASES/PROVENANCE.json" <<'PYEOF'
import json, platform, sys
from importlib.metadata import version, PackageNotFoundError

def v(name):
    try:
        return version(name)
    except PackageNotFoundError:
        return None

print(json.dumps({
    "note": ("What the conformance corpus was recorded from. A Rust test "
             "checks this against the live submodule; if they differ, either "
             "restore the submodule or run conformance/tools/regenerate.sh."),
    "rdapy_commit": sys.argv[1],
    "rdapy_dirty": sys.argv[2] != "0",
    "python": platform.python_version(),
    "packages": {n: v(n) for n in
                 ("numpy", "scipy", "shapely", "geopandas", "fiona",
                  "libpysal", "geographiclib", "gerrychain", "pytest")},
}, indent=2))
PYEOF

echo
echo "recorded $CASES/PROVENANCE.json"
git -C "$REPO" status --short conformance/cases | head -20
