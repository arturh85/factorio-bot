#!/usr/bin/env bash
# oracle.sh <config-name> [reps]
# The heaviest CPU-bound probe we have: the flow-graph oracle over a 2.9 GB
# world dump. Assumes the release build for <config-name> is already in
# measure/td (run-config.sh leaves it there), so this times the TEST, not a
# build. Run INSIDE `nix develop -c`.
set -uo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$WT/measure/bench.sh"

NAME="$1"; REPS="${2:-2}"
export CARGO_TARGET_DIR="$WT/measure/td"
# ABSOLUTE path -- a relative one dies with a bare NotFound.
export FACTORIO_BOT_WORLD_DUMP="/home/arturh/projects/private/factorio-bot/workspace/wrload/scripts/wr-census.json"

cd "$WT" || exit 1

# Build the test binary first, untimed, so the timed reps measure only the run.
cargo test -p factorio-bot-core --release flow_graph --no-run \
  >"$WT/measure/oracle-build-$NAME.log" 2>&1 || { echo "test build failed"; exit 1; }

for r in $(seq 1 "$REPS"); do
  timeit "$NAME" "oracle$r" "" -- cargo test -p factorio-bot-core --release \
    production_rates_of_a_dumped_world -- --ignored --nocapture
done
