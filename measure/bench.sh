#!/usr/bin/env bash
# Build-speed measurement harness. Run INSIDE `nix develop -c`.
#
# Every timing records the load average before and after, because these are
# wall-clock measurements on a box several agents build on. A timing without
# its load is not data.
set -uo pipefail

WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$WT/measure/results.tsv"
LEAF="$WT/crates/planner/src/lib.rs"

load() { awk '{print $1"/"$2}' /proc/loadavg; }

# wait until the 1-minute load is under $1, up to $2 seconds
wait_quiet() {
  local want="$1" max="${2:-1800}" waited=0
  while :; do
    local l1
    l1=$(awk '{print $1}' /proc/loadavg)
    if awk -v a="$l1" -v b="$want" 'BEGIN{exit !(a<b)}'; then return 0; fi
    if [ "$waited" -ge "$max" ]; then
      echo "WARN: still load $l1 after ${waited}s" >&2
      return 1
    fi
    sleep 30; waited=$((waited+30))
  done
}

# record CONFIG PHASE SECONDS LOAD_BEFORE LOAD_AFTER NOTE
record() {
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" "${6:-}" >>"$OUT"
  printf '%-12s %-16s %8ss  load %s -> %s  %s\n' "$1" "$2" "$3" "$4" "$5" "${6:-}"
}

# Wall AND cpu seconds. Wall is what the brief asks for; cpu (user+sys, whole
# process tree) is what survives a loaded box. This repo's own rule is that a
# measurement a wall clock can move is a broken instrument and should be bound
# in something invariant -- for a build, total CPU work is that invariant.
# It is not perfectly immune either (cache and memory-bandwidth contention
# still show up), but it moves by percent where wall moves by multiples.
timeit() { # CONFIG PHASE NOTE -- cmd...
  local cfg="$1" phase="$2" note="$3"; shift 3
  [ "$1" = "--" ] && shift
  local lb la t0 t1
  lb=$(load); t0=$(date +%s.%N)
  # bash's BUILTIN time -- NixOS has no /usr/bin/time, and reaching for it
  # cost a whole round of this harness (rc=127, four configs, zero seconds).
  local rc
  TIMEFORMAT='%R %U %S'
  { time "$@" >"$WT/measure/last-$cfg-$phase.log" 2>&1; rc=$?; } \
    2>"$WT/measure/t-$cfg-$phase.txt"
  t1=$(date +%s.%N); la=$(load)
  local secs; secs=$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.1f", b-a}')
  local cpu; cpu=$(awk 'NF==3{printf "%.1f", $2+$3}' "$WT/measure/t-$cfg-$phase.txt" 2>/dev/null)
  [ $rc -ne 0 ] && note="$note FAILED(rc=$rc)"
  record "$cfg" "$phase" "$secs" "$lb" "$la" "cpu=${cpu:-?} $note"
}
