#!/usr/bin/env bash
# run-config.sh <name> <linker> <lto> <cu> <opt> [reps]
# One configuration, end to end. Run INSIDE `nix develop -c`.
set -uo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$WT/measure/bench.sh"

NAME="$1"; LINKER="$2"; LTO="$3"; CU="$4"; OPT="$5"; REPS="${6:-2}"
TD="$WT/measure/td"
export CARGO_TARGET_DIR="$TD"
BIN="$TD/release/factorio-bot"
MAP="$WT/../../workspace/scripts/map.json"

bash "$WT/measure/apply.sh" "$LINKER" "$LTO" "$CU" "$OPT" >/dev/null || exit 1

cd "$WT" || exit 1

for r in $(seq 1 "$REPS"); do
  rm -rf "$TD"
  timeit "$NAME" "cold$r" "" -- cargo build --release
  [ -x "$BIN" ] || { echo "NO BINARY for $NAME rep$r"; break; }
  if [ "$r" = 1 ]; then
    record "$NAME" "binsize" "$(stat -c%s "$BIN")" - - bytes
    # four offline baselines, on this binary
    # Four offline baselines, captured WITH --steps so the comparison across
    # configurations is byte-for-byte over the whole plan, not over two
    # summary numbers that could coincide.
    for g in researched:automation producing:automation-science-pack:6 \
             producing:logistic-science-pack:6; do
      "$BIN" plan --world "$MAP" --goal "$g" --bots 1,2,3,4 --steps \
        >"$WT/measure/plan-$NAME-${g//:/_}.log" 2>&1
    done
    "$BIN" plan --world "$WT/../../workspace/scripts/map-31337-explored.json" \
      --goal gathered:crude-oil --bots 1,2,3,4 --steps \
      >"$WT/measure/plan-$NAME-oil.log" 2>&1
    # timed plan probe, 3 samples
    for s in 1 2 3; do
      timeit "$NAME" "plan$s" "" -- "$BIN" plan --world "$MAP" \
        --goal researched:automation --bots 1,2,3,4
    done
  fi
  touch crates/planner/src/lib.rs
  timeit "$NAME" "warm$r" "" -- cargo build --release
done
echo "== done $NAME"
