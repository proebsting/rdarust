#!/bin/bash
# Score an ensemble, from already-extracted precinct data.
#
# rdapy's SCORE.sh starts from a DRA GeoJSON and extracts the precinct data
# and adjacency graph first. That extraction needs a geometry library and is
# not ported yet, so this starts one step later, from the data and graph those
# steps produce. Use rdapy's scripts/data/extract_data.py to produce them.
set -euo pipefail

MODE=all
PRECOMPUTED=""
PREFIXES=""
JOBS=""

usage() {
  cat >&2 <<'USAGE'
usage: SCORE.sh --state XX --plan-type CHAMBER --data PATH --graph PATH
                --plans PATH --scores PATH --by-district PATH
                [--precomputed PATH] [--mode MODE] [--prefixes] [--jobs N]
USAGE
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --state)       STATE="$2";       shift 2 ;;
    --plan-type)   PLAN_TYPE="$2";   shift 2 ;;
    --data)        DATA="$2";        shift 2 ;;
    --graph)       GRAPH="$2";       shift 2 ;;
    --plans)       PLANS="$2";       shift 2 ;;
    --scores)      SCORES="$2";      shift 2 ;;
    --by-district) BY_DISTRICT="$2"; shift 2 ;;
    --precomputed) PRECOMPUTED="$2"; shift 2 ;;
    --mode)        MODE="$2";        shift 2 ;;
    --prefixes)    PREFIXES="--prefixes"; shift ;;
    --jobs)        JOBS="$2";        shift 2 ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
done

: "${STATE:?--state is required}"
: "${PLAN_TYPE:?--plan-type is required}"
: "${DATA:?--data is required}"
: "${GRAPH:?--graph is required}"
: "${PLANS:?--plans is required}"
: "${SCORES:?--scores is required}"
: "${BY_DISTRICT:?--by-district is required}"

exec "$(dirname "$0")/../target/release/rdarust" score-all \
  --state "$STATE" --plan-type "$PLAN_TYPE" \
  --data "$DATA" --graph "$GRAPH" --plans "$PLANS" \
  --scores "$SCORES" --by-district "$BY_DISTRICT" \
  --mode "$MODE" \
  ${PRECOMPUTED:+--precomputed "$PRECOMPUTED"} \
  ${PREFIXES:+$PREFIXES} \
  ${JOBS:+--jobs "$JOBS"}
