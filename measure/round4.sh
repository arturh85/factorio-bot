#!/usr/bin/env bash
# Round 4: an ITERATION profile, swept.
#
# Round 2/3 asked what the SHIPPING profile costs. This asks a different
# question: the owner said "build time is more important to us than runtime
# optimizations/speed", so what does a profile built for the edit-compile-plan
# loop cost, and what does it cost the loop?
#
# The candidate is lto=false / opt-level=1 / codegen-units=4 -- cu matched to
# `jobs = 4`, because jobs caps rustc PROCESSES and codegen-units multiplies
# threads INSIDE each one, and cu=16 reached ~14x parallelism while the config
# file still read `jobs = 4`. It is a hypothesis, so it is swept rather than
# tested: opt 0/1/2 at cu=4, plus cu=8 at opt=1, against today's shipped
# fat/cu1/opt-s and against fat/cu1/opt-3 (the shipping change on branch
# `fat-lto-at-opt-three`).
#
# Per-configuration PERSISTENT target directories, warm rebuilds INTERLEAVED,
# for round 3's reason: the box's load drifts by a factor of ten across an
# hour, so measuring configurations back to back assigns the box's mood to
# whichever config was unlucky.
set -uo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$WT/measure/bench.sh"

REPO=/home/arturh/projects/private/factorio-bot
MAP="$REPO/workspace/scripts/map.json"
OILMAP="$REPO/workspace/scripts/map-31337-explored.json"

# name   linker lto  cu opt
CONFIGS=(
  "base   mold   fat  1  s"
  "fat3   mold   fat  1  3"
  "it1    mold   none 4  1"
  "it0    mold   none 4  0"
  "it2    mold   none 4  2"
  "it1c8  mold   none 8  1"
)

cd "$WT" || exit 1

phase="${1:-all}"

# ---- phase 1: one cold build per configuration, into its own target dir ----
if [ "$phase" = all ] || [ "$phase" = cold ]; then
for spec in "${CONFIGS[@]}"; do
  # shellcheck disable=SC2086
  set -- $spec
  N="$1"
  export CARGO_TARGET_DIR="$WT/measure/td-$N"
  bash "$WT/measure/apply.sh" "$2" "$3" "$4" "$5" >/dev/null || continue
  rm -rf "$CARGO_TARGET_DIR"
  # Disk guard. Written as a plain assignment on purpose: the first version
  # put the test at the end of a PIPELINE, so the pipeline's exit status was
  # the status of a `[ ] &&` that was FALSE whenever there was plenty of room
  # -- and `|| exit 1` then aborted the whole sweep on a healthy disk, in
  # under a second, with rc=0. A guard that fires when the thing it guards
  # against is absent is worse than no guard.
  AVAIL_G=$(df --output=avail -BG / | tail -1 | tr -dc '0-9')
  if [ "${AVAIL_G:-0}" -lt 25 ]; then
    echo "ABORT: only ${AVAIL_G}G free on /"; exit 1
  fi
  timeit "$N" cold "" -- cargo build --release
  BIN="$CARGO_TARGET_DIR/release/factorio-bot"
  [ -x "$BIN" ] && record "$N" binsize "$(stat -c%s "$BIN")" - - bytes
  # correctness gate: four offline baselines, full --steps, on THIS binary
  if [ -x "$BIN" ]; then
    for g in researched:automation producing:automation-science-pack:6 \
             producing:logistic-science-pack:6; do
      "$BIN" plan --world "$MAP" --goal "$g" --bots 1,2,3,4 --steps \
        >"$WT/measure/plan-$N-${g//:/_}.log" 2>&1
    done
    "$BIN" plan --world "$OILMAP" --goal gathered:crude-oil --bots 1,2,3,4 \
      --steps >"$WT/measure/plan-$N-oil.log" 2>&1
  fi
done
fi

# ---- phase 2: interleaved warm rebuilds ----
if [ "$phase" = all ] || [ "$phase" = warm ]; then
for cycle in 1 2 3; do
  for spec in "${CONFIGS[@]}"; do
    # shellcheck disable=SC2086
    set -- $spec
    N="$1"
    export CARGO_TARGET_DIR="$WT/measure/td-$N"
    [ -d "$CARGO_TARGET_DIR" ] || continue
    bash "$WT/measure/apply.sh" "$2" "$3" "$4" "$5" >/dev/null || continue
    touch crates/planner/src/lib.rs
    timeit "$N" "warm-c$cycle" "" -- cargo build --release
  done
done
fi

# ---- phase 3: interleaved runtime probes ----
if [ "$phase" = all ] || [ "$phase" = plan ]; then
for cycle in 1 2 3; do
  for spec in "${CONFIGS[@]}"; do
    # shellcheck disable=SC2086
    set -- $spec
    N="$1"
    BIN="$WT/measure/td-$N/release/factorio-bot"
    [ -x "$BIN" ] || continue
    timeit "$N" "plan-c$cycle" "" -- "$BIN" plan --world "$MAP" \
      --goal researched:automation --bots 1,2,3,4
  done
done
fi

echo "== ROUND 4 ($phase) COMPLETE"
