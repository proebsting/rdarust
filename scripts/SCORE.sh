#!/bin/bash
# Score an ensemble, from a DRA GeoJSON.
#
# Takes the same arguments as rdapy's scripts/score/SCORE.sh and runs the same
# four steps, all in Rust:
#
#   map-data -> extract-graph -> extract-data -> score-all
#
# The graph and precinct data are derived once per state, so pass --graph and
# --data pointing at existing files to skip straight to scoring.
set -euo pipefail

RDARUST="$(dirname "$0")/../target/release/rdarust"

MODE=all
CENSUS=T_20_CENS
VAP=V_20_VAP
CVAP=V_20_CVAP
ELECTIONS=E_16-20_COMP
PRECOMPUTED=""
EXPAND=""
PREFIXES=""
JOBS=""
GEOJSON=""
GRAPH=""
DATA=""

usage() {
  cat >&2 <<'USAGE'
usage: SCORE.sh --state XX --plan-type CHAMBER
                (--geojson PATH | --graph PATH --data PATH)
                --plans PATH --scores PATH --by-district PATH
                [--precomputed PATH] [--mode MODE]
                [--census D] [--vap D] [--cvap D] [--elections LIST]
                [--expand-composites] [--prefixes] [--jobs N]
USAGE
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --state)        STATE="$2";       shift 2 ;;
    --plan-type)    PLAN_TYPE="$2";   shift 2 ;;
    --geojson)      GEOJSON="$2";     shift 2 ;;
    --graph)        GRAPH="$2";       shift 2 ;;
    --data)         DATA="$2";        shift 2 ;;
    --plans)        PLANS="$2";       shift 2 ;;
    --scores)       SCORES="$2";      shift 2 ;;
    --by-district)  BY_DISTRICT="$2"; shift 2 ;;
    --precomputed)  PRECOMPUTED="$2"; shift 2 ;;
    --mode)         MODE="$2";        shift 2 ;;
    --census)       CENSUS="$2";      shift 2 ;;
    --vap)          VAP="$2";         shift 2 ;;
    --cvap)         CVAP="$2";        shift 2 ;;
    --elections)    ELECTIONS="$2";   shift 2 ;;
    --expand-composites) EXPAND="--expand-composites"; shift ;;
    --prefixes)     PREFIXES="--prefixes"; shift ;;
    --jobs)         JOBS="$2";        shift 2 ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
done

: "${STATE:?--state is required}"
: "${PLAN_TYPE:?--plan-type is required}"
: "${PLANS:?--plans is required}"
: "${SCORES:?--scores is required}"
: "${BY_DISTRICT:?--by-district is required}"

if [[ -z "$GEOJSON" && ( -z "$GRAPH" || -z "$DATA" ) ]]; then
  echo "give either --geojson, or both --graph and --data" >&2
  usage
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Derive the graph and precinct data from the GeoJSON unless given.
if [[ -n "$GEOJSON" ]]; then
  [[ -n "$GRAPH" ]] || GRAPH="$TMP/graph.json"
  [[ -n "$DATA"  ]] || DATA="$TMP/data.jsonl"

  "$RDARUST" map-data --geojson "$GEOJSON" --data-map "$TMP/data-map.json" \
      --census "$CENSUS" --vap "$VAP" --cvap "$CVAP" --elections "$ELECTIONS" \
      ${EXPAND:+$EXPAND}

  "$RDARUST" extract-graph --geojson "$GEOJSON" --graph "$GRAPH"

  "$RDARUST" extract-data --geojson "$GEOJSON" --data-map "$TMP/data-map.json" \
      --graph "$GRAPH" --data "$DATA"
fi

"$RDARUST" score-all \
  --state "$STATE" --plan-type "$PLAN_TYPE" \
  --data "$DATA" --graph "$GRAPH" --plans "$PLANS" \
  --scores "$SCORES" --by-district "$BY_DISTRICT" \
  --mode "$MODE" \
  ${PRECOMPUTED:+--precomputed "$PRECOMPUTED"} \
  ${PREFIXES:+$PREFIXES} \
  ${JOBS:+--jobs "$JOBS"}

echo
echo "Done!"
