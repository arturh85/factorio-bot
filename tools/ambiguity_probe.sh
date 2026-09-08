#!/usr/bin/env bash
# Probe for the "39 ways to make a processing unit" ambiguity work.
#
# Plans the rocket ladder against the fluid-carrying dump and the four
# baselines against the dumps each was taken on, printing the FULL refusal
# text for anything that refuses. Refusal text is the diagnostic; a summary
# of it is not.
#
# Usage: tools/ambiguity_probe.sh <binary> <dump-dir> [label]
set -u
BIN="${1:?binary}"
DUMPS="${2:?dump dir}"
LABEL="${3:-}"
NEW="$DUMPS/map-31337-water-and-oil.json"
T0="$DUMPS/map.json"
EXPLORED="$DUMPS/map-31337-explored.json"

echo "=== $LABEL ==="
echo "--- eight rungs, on map-31337-water-and-oil.json ---"
for goal in researched:rocket-silo have:rocket-silo:1 have:rocket-part:1 \
            have:low-density-structure:1 have:processing-unit:1 \
            have:rocket-fuel:1 have:plastic-bar:10 have:sulfur:10; do
  echo "### $goal"
  timeout 600 "$BIN" plan --world "$NEW" --goal "$goal" --bots 1,2,3,4 2>&1 \
    | grep -v '^$' | tail -20
done

echo "--- baselines ---"
for spec in "$T0 researched:automation" \
            "$T0 producing:automation-science-pack:6" \
            "$T0 producing:logistic-science-pack:6" \
            "$EXPLORED gathered:crude-oil"; do
  set -- $spec
  echo "### $2   ($(basename "$1"))"
  timeout 600 "$BIN" plan --world "$1" --goal "$2" --bots 1,2,3,4 2>&1 \
    | grep -iE "action|tick|makespan|refus|error|ambiguous" | tail -8
done
