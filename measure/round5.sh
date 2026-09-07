#!/usr/bin/env bash
# Round 5: is a DEV build good enough to work in?
#
# The owner: "Maybe also just change that guidance, I see no reasons for release
# builds ever, only when CI creates real releases." That makes the question
# neither "which iteration profile to add" nor "what should release carry", but:
# what does a developer or agent TYPE, and can `cargo build` / `cargo test`
# carry the offline `plan` loop that this project actually iterates in?
#
# The decisive number is PLAN CPU on a dev build. CLAUDE.md records debug as
# 4-7x slower at runtime, which is precisely why `--release` became the habit.
# If `opt-level = 1` on our crates closes most of that, the guidance changes; if
# nothing cheap gets within a small factor of release's ~1-2 s, it does not, and
# saying so is the answer.
#
# The grid separates three things the discussion usually conflates:
#   OUR opt-level   -- recompiled on every edit, where build cost lives
#   DEPS opt-level  -- built once, cached by sccache, so nearly free to raise
#   debug info      -- a build cost in its own right, and the one nobody prices
#
# `dtoday` is the profile as shipped (0 / "z" / debug on). Note "z" on deps is
# the suspicious cell: it costs build time AND gives up runtime, so if it is
# wrong it is wrong twice -- which is exactly why it gets measured rather than
# assumed.
set -uo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$WT/measure/bench.sh"

REPO=/home/arturh/projects/private/factorio-bot
MAP="$REPO/workspace/scripts/map.json"

#  name    ours deps debug
CONFIGS=(
  "dtoday   0    z    true"
  "d1       1    2    true"
  "d1nodbg  1    2    0"
  "d2       2    3    true"
)

cd "$WT" || exit 1
phase="${1:-all}"

if [ "$phase" = all ] || [ "$phase" = cold ]; then
for spec in "${CONFIGS[@]}"; do
  # shellcheck disable=SC2086
  set -- $spec
  N="$1"
  export CARGO_TARGET_DIR="$WT/measure/tdd-$N"
  bash "$WT/measure/apply-dev.sh" "$2" "$3" "$4" >/dev/null || continue
  rm -rf "$CARGO_TARGET_DIR"
  AVAIL_G=$(df --output=avail -BG / | tail -1 | tr -dc '0-9')
  if [ "${AVAIL_G:-0}" -lt 25 ]; then echo "ABORT: only ${AVAIL_G}G free"; exit 1; fi
  timeit "$N" cold "" -- cargo build
  BIN="$CARGO_TARGET_DIR/debug/factorio-bot"
  [ -x "$BIN" ] && record "$N" binsize "$(stat -c%s "$BIN")" - - bytes
  # correctness gate on the dev binary too: an opt-level must not move a plan,
  # and a dev build is a different codegen path, not merely a slower one.
  if [ -x "$BIN" ]; then
    for g in researched:automation producing:automation-science-pack:6 \
             producing:logistic-science-pack:6; do
      "$BIN" plan --world "$MAP" --goal "$g" --bots 1,2,3,4 --steps \
        >"$WT/measure/plan-$N-${g//:/_}.log" 2>&1
    done
  fi
done
fi

# Interleaved, for the reason round 3 learned: measuring configurations back to
# back on this box assigns its mood to whichever one was unlucky.
if [ "$phase" = all ] || [ "$phase" = warm ]; then
for cycle in 1 2 3; do
  for spec in "${CONFIGS[@]}"; do
    # shellcheck disable=SC2086
    set -- $spec
    N="$1"
    export CARGO_TARGET_DIR="$WT/measure/tdd-$N"
    [ -d "$CARGO_TARGET_DIR" ] || continue
    bash "$WT/measure/apply-dev.sh" "$2" "$3" "$4" >/dev/null || continue
    touch crates/planner/src/lib.rs
    timeit "$N" "warm-c$cycle" "" -- cargo build
  done
done
fi

if [ "$phase" = all ] || [ "$phase" = plan ]; then
for cycle in 1 2 3; do
  for spec in "${CONFIGS[@]}"; do
    # shellcheck disable=SC2086
    set -- $spec
    N="$1"
    BIN="$WT/measure/tdd-$N/debug/factorio-bot"
    [ -x "$BIN" ] || continue
    timeit "$N" "plan-c$cycle" "" -- "$BIN" plan --world "$MAP" \
      --goal researched:automation --bots 1,2,3,4
  done
done
fi

# `cargo test --workspace` is plausibly the single most-run command in this
# repo -- the gate every agent runs after every merge -- so its cost is a
# first-class part of the answer, not a footnote. Timed on the already-warm
# target dir so this measures the test build and run, not a cold start.
if [ "$phase" = all ] || [ "$phase" = test ]; then
for spec in "${CONFIGS[@]}"; do
  # shellcheck disable=SC2086
  set -- $spec
  N="$1"
  export CARGO_TARGET_DIR="$WT/measure/tdd-$N"
  [ -d "$CARGO_TARGET_DIR" ] || continue
  bash "$WT/measure/apply-dev.sh" "$2" "$3" "$4" >/dev/null || continue
  timeit "$N" "testbuild" "" -- cargo test --workspace --no-run
  timeit "$N" "testrun" "" -- cargo test --workspace
done
fi

echo "== ROUND 5 ($phase) COMPLETE"
