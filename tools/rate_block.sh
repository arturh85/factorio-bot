#!/usr/bin/env bash
# Measure ONE searched layout in a live headless game, on a FRESH map.
#
#   tools/rate_block.sh <candidate> <instance-letter> [coal]
#   tools/rate_block.sh gen-5x4 a
#
# The offline half is `factorio-bot search`; this is the live half, and the
# reason the search's numbers can be checked at all. It:
#
#   1. asks `search --emit` for the candidate's blueprint string, so the block
#      measured is byte-for-byte the block scored;
#   2. writes it beside `rate_block.lua` in the instance's scripts directory;
#   3. runs the script headless at 10x on `workspace/headless-<letter>.toml`,
#      with `--seed 31337 --new` so every variant gets its own fresh map --
#      a block built on ground another block changed measures the sequence,
#      not the design;
#   4. prints the `RATE_BLOCK ...` line the script ends with.
#
# One instance per run: the four tomls own ports 4330-4333, never the default
# ones, and a peer session holds those.
#
# A DEBUG binary repoints <workspace>/mods/BotBridge at this checkout's mod
# for the run; the symlink is restored to the main checkout afterwards, so a
# later run from another tree is not handed a dangling link (see CLAUDE.md,
# "the two profiles share one workspace").
set -euo pipefail

CANDIDATE="${1:?candidate name, e.g. gen-5x4}"
INSTANCE="${2:?instance letter a-d}"
COAL="${3:-150}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="$(git -C "$HERE" rev-parse --path-format=absolute --git-common-dir | sed 's#/\.git$##')"
BIN="${BIN:-$HERE/target/debug/factorio-bot}"
WORLD="${WORLD:-$REPO/workspace/scripts/map-31337-explored-with-categories.json}"
SETTINGS="$REPO/workspace/headless-$INSTANCE.toml"
WS="$REPO/workspace/headless-$INSTANCE"
SPEED="${SPEED:-10}"

[ -x "$BIN" ] || { echo "no binary at $BIN" >&2; exit 2; }
[ -f "$SETTINGS" ] || { echo "no settings at $SETTINGS" >&2; exit 2; }

mkdir -p "$WS/scripts"
BP="$("$BIN" search --world "$WORLD" --drills 1-6 --furnaces 1-6 --emit "$CANDIDATE" 2>/dev/null)"
[ -n "$BP" ] || { echo "search --emit $CANDIDATE printed nothing" >&2; exit 2; }
printf '%s\n%s\n%s\n' "$CANDIDATE" "$BP" "$COAL" > "$WS/scripts/rate_block.txt"
cp "$HERE/scripts/rate_block.lua" "$WS/scripts/rate_block.lua"

LOG="$HERE/rate-$CANDIDATE-$INSTANCE.log"
echo "measuring $CANDIDATE on headless-$INSTANCE at ${SPEED}x, log $LOG"
set +e
timeout 1500 "$BIN" lua rate_block.lua --headless --bots 1 --game-speed "$SPEED" \
  --seed 31337 --new --settings "$SETTINGS" > "$LOG" 2>&1
STATUS=$?
set -e
# Restore the shared mod link to the main checkout regardless of outcome.
if [ -L "$WS/mods/BotBridge" ]; then
  ln -sfn "$REPO/mods/BotBridge" "$WS/mods/BotBridge"
fi
grep -E "^RATE_BLOCK|DRILL COVERAGE|BUILD INCOMPLETE|REFUSED|pass [0-9]:" "$LOG" || true
echo "exit $STATUS"
exit $STATUS
