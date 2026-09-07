#!/usr/bin/env bash
# it3's warm rebuilds and plan probes.
#
# Separate from round4.sh because that script's CONFIGS list was already being
# executed by a running bash when this cell was added, and bash re-reads a
# script by BYTE OFFSET while it runs -- editing a live script can splice the
# interpreter into the middle of a different line. So the cell gets its own
# file rather than an edit to a file in flight.
set -uo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$WT/measure/bench.sh"

REPO=/home/arturh/projects/private/factorio-bot
MAP="$REPO/workspace/scripts/map.json"

N=it3
cd "$WT" || exit 1
export CARGO_TARGET_DIR="$WT/measure/td-$N"
[ -d "$CARGO_TARGET_DIR" ] || { echo "no target dir for $N"; exit 1; }

for cycle in 1 2 3; do
  bash "$WT/measure/apply.sh" mold none 4 3 >/dev/null || exit 1
  touch crates/planner/src/lib.rs
  timeit "$N" "warm-c$cycle" "" -- cargo build --release
done

BIN="$CARGO_TARGET_DIR/release/factorio-bot"
for cycle in 1 2 3; do
  [ -x "$BIN" ] || break
  timeit "$N" "plan-c$cycle" "" -- "$BIN" plan --world "$MAP" \
    --goal researched:automation --bots 1,2,3,4
done
echo "== it3 warm+plan COMPLETE"
