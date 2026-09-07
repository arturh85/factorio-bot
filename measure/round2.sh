#!/usr/bin/env bash
# The real dataset. One pass over every configuration, same harness, same
# order, wall AND cpu seconds, load recorded either side of every timing.
# Run INSIDE `nix develop -c`.
set -uo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$WT/measure/bench.sh"

TD="$WT/measure/td"
export CARGO_TARGET_DIR="$TD"
BIN="$TD/release/factorio-bot"
REPO=/home/arturh/projects/private/factorio-bot
MAP="$REPO/workspace/scripts/map.json"
XMAP="$REPO/workspace/scripts/map-31337-explored.json"
export FACTORIO_BOT_WORLD_DUMP="$REPO/workspace/wrload/scripts/wr-census.json"

# name linker lto cu opt
CONFIGS=(
  "base  ld    fat  1  s"
  "mold  mold  fat  1  s"
  "thin  mold  thin 16 s"
  "thin3 mold  thin 16 3"
)

cd "$WT" || exit 1

for spec in "${CONFIGS[@]}"; do
  # shellcheck disable=SC2086
  set -- $spec
  N="$1"
  bash "$WT/measure/apply.sh" "$2" "$3" "$4" "$5" >"$WT/measure/applied-$N.txt" 2>&1 || continue

  rm -rf "$TD"
  timeit "$N" cold "" -- cargo build --release
  [ -x "$BIN" ] || { echo "!! no binary for $N"; continue; }
  record "$N" binsize "$(stat -c%s "$BIN")" - - bytes

  # three warm rebuilds -- the actual iteration loop
  for r in 1 2 3; do
    touch crates/planner/src/lib.rs
    timeit "$N" "warm$r" "" -- cargo build --release
  done

  # correctness: the four offline baselines, full --steps output
  for g in researched:automation producing:automation-science-pack:6 \
           producing:logistic-science-pack:6; do
    "$BIN" plan --world "$MAP" --goal "$g" --bots 1,2,3,4 --steps \
      >"$WT/measure/r2-plan-$N-${g//:/_}.log" 2>&1
  done
  "$BIN" plan --world "$XMAP" --goal gathered:crude-oil --bots 1,2,3,4 --steps \
    >"$WT/measure/r2-plan-$N-oil.log" 2>&1

  # runtime probe A: the ~4 s planner loop everyone iterates in
  for s in 1 2 3; do
    timeit "$N" "plan$s" "" -- "$BIN" plan --world "$MAP" \
      --goal researched:automation --bots 1,2,3,4
  done

  # runtime probe B: the heaviest CPU-bound thing we have
  cargo test -p factorio-bot-core --release flow_graph --no-run \
    >"$WT/measure/r2-oracle-build-$N.log" 2>&1
  for s in 1 2; do
    timeit "$N" "oracle$s" "" -- cargo test -p factorio-bot-core --release \
      production_rates_of_a_dumped_world -- --ignored --nocapture
  done
  echo "== finished $N"
done
echo "== ROUND 2 COMPLETE"
