#!/usr/bin/env bash
# Two-point timing: run each roster at two tick spans and DIFFERENCE them, so
# server startup (~16s, and the dominant term last time) cancels exactly rather
# than being estimated. The previous attempt measured a 1-2 second tick window
# with 1-second timestamps and produced 6000/6000/3000 tps, which is resolution
# noise, not a signal.
set -u
OUT="$1"; WS="$2"
lt() { awk -v a="$1" -v b="$2" 'BEGIN{exit !(a+0 < b+0)}'; }
if ! lt 1 2 || lt 2 1; then echo "REFUSED: comparator broken"; exit 2; fi
L=$(cut -d' ' -f1 /proc/loadavg)
if ! lt "$L" 6; then echo "REFUSED: load $L too high to start"; exit 3; fi
echo "load_start=$L"
declare -A MS
declare -A TK
for N in 1 4 8; do
  for SPAN in 60000 180000; do
    echo "$SPAN" > "$WS/scripts/tickrate_span.txt"
    S=$(date +%s%N)
    timeout 600 target/release/factorio-bot lua tickrate.lua \
      --settings "$PWD/scratch/blocks.toml" \
      --headless --bots "$N" --game-speed 100000 --seed 31337 --new \
      > "$OUT/two-$N-$SPAN.log" 2>&1
    rc=$?
    E=$(date +%s%N)
    clean=$(sed 's/\x1b\[[0-9;]*m//g' "$OUT/two-$N-$SPAN.log")
    T=$(printf '%s' "$clean" | grep -o "TICKS=[0-9]*" | head -1 | cut -d= -f2)
    B=$(printf '%s' "$clean" | grep -o "BOTS_PRESENT=[-0-9]*" | head -1 | cut -d= -f2)
    # The probe stops at the first poll PAST the span, so it overshoots by a
    # few ticks (60004 for 60000). Demanding equality rejected valid runs --
    # a guard that is too strict throws away data as surely as one that is too
    # loose lets bad data through. Accept a small overshoot and difference the
    # ACTUAL tick counts below rather than the nominal ones.
    over=$(( ${T:-0} - SPAN ))
    if [ "$rc" -ne 0 ] || [ "${T:-0}" -lt "$SPAN" ] || [ "$over" -gt 100 ] || [ "${B:-x}" != "$N" ]; then
      echo "BOTS=$N SPAN=$SPAN INVALID rc=$rc ticks=${T:-none} roster=${B:-unknown}"
      MS[$N-$SPAN]=""
      continue
    fi
    MS[$N-$SPAN]=$(( (E - S) / 1000000 ))
    TK[$N-$SPAN]=$T
    echo "  BOTS=$N SPAN=$SPAN ticks=$T ms=${MS[$N-$SPAN]} roster=$B"
  done
done
echo "--- differenced (startup cancels):"
for N in 1 4 8; do
  a=${MS[$N-60000]:-}; b=${MS[$N-180000]:-}
  if [ -z "$a" ] || [ -z "$b" ]; then echo "BOTS=$N INVALID: a run was rejected"; continue; fi
  d=$(( b - a )); t=$(( ${TK[$N-180000]} - ${TK[$N-60000]} ))
  if [ "$d" -le 0 ]; then echo "BOTS=$N INVALID: longer run was not slower ($a -> $b ms)"; continue; fi
  echo "BOTS=$N delta_ms=$d for ${t} ticks -> $(( t * 1000 / d )) tps"
done
echo "load_end=$(cut -d' ' -f1-3 /proc/loadavg)"
echo DONE
