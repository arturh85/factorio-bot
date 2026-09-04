#!/usr/bin/env bash
#
# Generate N maps at different seeds and score each one.
#
# Workstream 0b of docs/superpowers/plans/2026-09-03-closing-the-idle-gap.md.
# Walking was 20.3% of the reference run -- 14,330 ticks, 3.98 minutes, for
# bot 1 alone -- against a target of researching `automation` in under nine
# minutes. A comparison against a world-record time means nothing on a spawn
# whose ore is far away, so the map has to be a fair one before any timing is
# worth quoting.
#
# THIS SCRIPT LAUNCHES FACTORIO. Everything else in this workstream is offline;
# this is the one part that is not, because generating a map requires the game.
# Do not run it while another agent holds the workspace or is building.
#
#   ./tools/seed_search.sh /tmp/seedsearch 20260903 20260904 20260905 ...
#
# WHY A SCRATCH WORKSPACE IS THE FIRST ARGUMENT AND NOT A DEFAULT
#
# `--seed` only takes effect together with `--new`. It reaches Factorio as
# `--map-gen-seed` on a `--create`, and `--create` runs only when `level.zip`
# is absent, which only `--new` causes -- this was silently ignored until
# 61ec7364 and now warns loudly. **`--new` deletes the map.** Pointed at the
# primary workspace this destroys whatever is there, so the path is required,
# checked against the default, and never guessed.
#
# The workspace is REUSED across seeds on purpose: a fresh one re-extracts the
# Factorio archive, which is 8-10 minutes per instance. `--new` deletes only
# the save.
#
# WHAT IT COSTS
#
# Per seed: one server start (~12-17 s) plus initial chunk discovery (~7 s at
# one chunk per tick) plus process teardown. No graphical client -- `--clients
# 0` -- so no 26 s of sprite loading and no connect wait. Scoring itself is
# offline and takes a fraction of a second.
#
# WHAT IT DELIBERATELY DOES NOT DO
#
# It does not pick a seed. It writes one JSON verdict per seed and prints a
# ranking; choosing and freezing a seed is a decision with consequences for
# every number quoted afterwards (see `BENCHMARK_SEED` in the justfile, which
# argues the other way -- that a searched-for seed stops being comparable to
# the manual baseline). Read both before changing the benchmark.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: $0 <scratch-workspace-path> <seed> [seed ...]" >&2
  exit 2
fi

WORKSPACE="$1"
shift
SEEDS=("$@")

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${SEED_SEARCH_OUT:-$WORKSPACE/scores}"

# The workspace this repository actually runs against. Refusing it is the whole
# safety property of this script: `--new` there deletes a map that may be the
# subject of a run in progress.
DEFAULT_WORKSPACE="$HOME/.local/share/factorio-bot-dev/workspace"
RELEASE_WORKSPACE="$HOME/.local/share/factorio-bot/workspace"
for forbidden in "$DEFAULT_WORKSPACE" "$RELEASE_WORKSPACE" "$REPO/workspace"; do
  if [ "$(realpath -m "$WORKSPACE")" = "$(realpath -m "$forbidden")" ]; then
    echo "refusing to run a seed search in $forbidden: --new deletes its map" >&2
    exit 1
  fi
done

mkdir -p "$OUT"

# Debug build: iterating. The mod is a symlink to the repo's copy in a debug
# build, so an edit to mods/BotBridge is what the next run loads; a release
# build embeds a snapshot at compile time. Nothing here is timed, so there is
# no reason to pay for --release.
# Overridable so a search can run against a COPY of the binary while another
# agent is still building in this checkout. cargo cannot write a running
# binary ("Text file busy"), so a search holding target/debug hostage for ten
# minutes breaks whoever is compiling. Copy it aside and point BIN at the copy.
BIN="${BIN:-$REPO/target/debug/factorio-bot}"
if [ ! -x "$BIN" ]; then
  echo "building $BIN" >&2
  nix develop -c cargo build --no-default-features --features cli,lua
fi

for seed in "${SEEDS[@]}"; do
  echo "=== seed $seed ==="
  # --new is what makes --seed mean anything. --clients 0 spawns no graphical
  # client; --bots 4 only decides how many bots the world seeds players for.
  nix develop -c "$BIN" lua dump_map.lua \
    --clients 0 --bots 4 \
    --seed "$seed" --new \
    --workspace-path "$WORKSPACE"

  dump="$WORKSPACE/scripts/map.json"
  if [ ! -f "$dump" ]; then
    echo "seed $seed produced no dump at $dump -- skipping" >&2
    continue
  fi
  # Moved, not copied: the next seed writes to the same path, and a stale
  # map.json scored as a new seed is exactly the silent failure this whole
  # plan keeps finding.
  mv "$dump" "$OUT/map-$seed.json"

  # --bots is explicit rather than defaulted off the dump: a `--clients 0` run
  # may leave the dump with no players, and a missing roster silently skips the
  # makespan tier instead of failing.
  nix develop -c "$BIN" score-map \
    --world "$OUT/map-$seed.json" \
    --bots 1,2,3,4 \
    --json > "$OUT/score-$seed.json"

  nix develop -c "$BIN" score-map --world "$OUT/map-$seed.json" --bots 1,2,3,4
done

echo
echo "=== ranking (viable seeds only, lower walk_score is better) ==="
python3 - "$OUT" <<'PY'
import json
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
rows = []
for path in sorted(out.glob("score-*.json")):
    seed = path.stem.removeprefix("score-")
    verdict = json.loads(path.read_text())
    score = verdict["score"]
    rows.append(
        {
            "seed": seed,
            "viable": score["verdict"] == "Viable",
            "walk": score.get("walk_score"),
            "makespan": (verdict.get("plan") or {}).get("makespan"),
            "charted": f"{score['charting']['covered']}/{score['charting']['probes']}",
            "why": (
                ""
                if score["verdict"] == "Viable"
                else ",".join(score["verdict"]["Incomplete"]["missing"])
            ),
            "refused": verdict.get("plan_error") or "",
        }
    )

# Unviable seeds sort last rather than being dropped: "every seed tried was
# unviable" is a finding about the search, and a ranking that hid them would
# print an empty table and look like no seeds had been run.
rows.sort(key=lambda r: (not r["viable"], r["walk"] is None, r["walk"] or 0))
print(f"{'seed':<12}{'viable':<8}{'walk':>8}{'makespan':>10}  {'charted':<9}notes")
for row in rows:
    walk = row["walk"] if row["walk"] is not None else "-"
    makespan = row["makespan"] if row["makespan"] is not None else "-"
    note = row["why"] or row["refused"]
    print(
        f"{row['seed']:<12}{str(row['viable']):<8}{walk:>8}{makespan:>10}"
        f"  {row['charted']:<9}{note[:60]}"
    )
if not rows:
    print("(nothing scored)")
PY
