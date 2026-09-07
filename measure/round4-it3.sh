#!/usr/bin/env bash
# The cell the owner's correction made the most interesting one, and which the
# original grid did not contain: lto = false with opt-level = 3.
#
# The brief that commissioned round 4 asked for an ITERATION profile beside an
# unchanged `release`. The owner inverted that -- "you keep using release all
# the time, it needs to be fast too" -- so the question became which settings
# `[profile.release]` ITSELF should carry, with the runtime floor as a hard
# constraint rather than a nice-to-have. `opt-level = 3` measured 2x faster
# planning at fat LTO for +67% build cpu; whether that build cost survives
# WITHOUT fat LTO is the open cell, and it is the one that would let `release`
# be cheap to build and still fast to run.
#
# Same target-dir-per-config layout as round4.sh so the warm and plan phases
# can pick it up.
set -uo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$WT/measure/bench.sh"

REPO=/home/arturh/projects/private/factorio-bot
MAP="$REPO/workspace/scripts/map.json"
OILMAP="$REPO/workspace/scripts/map-31337-explored.json"

N=it3
cd "$WT" || exit 1
export CARGO_TARGET_DIR="$WT/measure/td-$N"
bash "$WT/measure/apply.sh" mold none 4 3 >/dev/null || exit 1
rm -rf "$CARGO_TARGET_DIR"
AVAIL_G=$(df --output=avail -BG / | tail -1 | tr -dc '0-9')
if [ "${AVAIL_G:-0}" -lt 25 ]; then echo "ABORT: only ${AVAIL_G}G free"; exit 1; fi
timeit "$N" cold "" -- cargo build --release
BIN="$CARGO_TARGET_DIR/release/factorio-bot"
[ -x "$BIN" ] && record "$N" binsize "$(stat -c%s "$BIN")" - - bytes
if [ -x "$BIN" ]; then
  for g in researched:automation producing:automation-science-pack:6 \
           producing:logistic-science-pack:6; do
    "$BIN" plan --world "$MAP" --goal "$g" --bots 1,2,3,4 --steps \
      >"$WT/measure/plan-$N-${g//:/_}.log" 2>&1
  done
  "$BIN" plan --world "$OILMAP" --goal gathered:crude-oil --bots 1,2,3,4 \
    --steps >"$WT/measure/plan-$N-oil.log" 2>&1
fi
echo "== it3 cold COMPLETE"
