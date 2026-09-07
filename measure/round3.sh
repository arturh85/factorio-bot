#!/usr/bin/env bash
# Round 3: the same four configurations, INTERLEAVED.
#
# Round 2 measured them back to back, and the box's load drifted from 3 to 34
# across the run -- base was measured on a quiet box and `thin` on a busy one,
# so every cross-configuration difference it showed is confounded with load.
# Wall time moved 2x on ONE configuration between repeats for that reason.
#
# The fix is per-configuration target directories (3.6 GB each) that PERSIST,
# so switching configuration costs only a warm rebuild. Each cycle touches all
# four within a few minutes, so whatever the box is doing is shared by all four
# rather than assigned to whichever ran while a neighbour was building.
set -uo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$WT/measure/bench.sh"

REPO=/home/arturh/projects/private/factorio-bot
MAP="$REPO/workspace/scripts/map.json"
export FACTORIO_BOT_WORLD_DUMP="$REPO/workspace/wrload/scripts/wr-census.json"

CONFIGS=(
  "base  ld    fat  1  s"
  "mold  mold  fat  1  s"
  "thin  mold  thin 16 s"
  "thin3 mold  thin 16 3"
)

cd "$WT" || exit 1
# ---- phase 1: one cold build per configuration, into its own target dir ----
for spec in "${CONFIGS[@]}"; do
  # shellcheck disable=SC2086
  set -- $spec
  N="$1"
  export CARGO_TARGET_DIR="$WT/measure/td-$N"
  bash "$WT/measure/apply.sh" "$2" "$3" "$4" "$5" >/dev/null || continue
  rm -rf "$CARGO_TARGET_DIR"
  timeit "$N" cold "" -- cargo build --release
  BIN="$CARGO_TARGET_DIR/release/factorio-bot"
  [ -x "$BIN" ] && record "$N" binsize "$(stat -c%s "$BIN")" - - bytes
  # build the oracle test binary now, untimed
  cargo test -p factorio-bot-core --release flow_graph --no-run \
    >"$WT/measure/r3-oracle-build-$N.log" 2>&1
done

# ---- phase 2: interleaved warm rebuilds ----
for cycle in 1 2 3; do
  for spec in "${CONFIGS[@]}"; do
    # shellcheck disable=SC2086
    set -- $spec
    N="$1"
    export CARGO_TARGET_DIR="$WT/measure/td-$N"
    bash "$WT/measure/apply.sh" "$2" "$3" "$4" "$5" >/dev/null || continue
    touch crates/planner/src/lib.rs
    timeit "$N" "warm-c$cycle" "" -- cargo build --release
  done
done

# ---- phase 3: interleaved runtime probes ----
for cycle in 1 2; do
  for spec in "${CONFIGS[@]}"; do
    # shellcheck disable=SC2086
    set -- $spec
    N="$1"
    export CARGO_TARGET_DIR="$WT/measure/td-$N"
    BIN="$CARGO_TARGET_DIR/release/factorio-bot"
    [ -x "$BIN" ] || continue
    timeit "$N" "plan-c$cycle" "" -- "$BIN" plan --world "$MAP" \
      --goal researched:automation --bots 1,2,3,4
    timeit "$N" "oracle-c$cycle" "" -- cargo test -p factorio-bot-core --release \
      production_rates_of_a_dumped_world -- --ignored --nocapture
  done
done

echo "== ROUND 3 COMPLETE"
