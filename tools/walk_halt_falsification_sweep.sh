#!/usr/bin/env bash
# Falsification sweep for `walk_halted_on_a_stall`.
#
# Back up by FILE COPY and restore by copy plus `touch` -- `cp -p` preserves
# mtime, so cargo would re-run the mutated binary against restored source and
# report a false red. `git checkout --` is not used: it restores the last
# COMMITTED state, not the mutation, and has already destroyed uncommitted work
# in this repo.
set -u
F=crates/executor/src/recover.rs
BAK=.walk-halt-sweep.orig  # repo-local, removed at the end; never /tmp
cp "$F" "$BAK"

restore() { cp "$BAK" "$F"; touch "$F"; }

mutate() {  # name, from, to
  local name="$1" from="$2" to="$3"
  restore
  local n
  n=$(grep -F -c -- "$from" "$F")
  if [ "$n" -ne 1 ]; then
    echo "SKIP  $name -- substitution matched $n times, not exactly once"
    return
  fi
  python3 - "$F" "$from" "$to" <<'PY'
import sys
p, a, b = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p).read()
assert s.count(a) == 1
open(p, 'w').write(s.replace(a, b))
PY
  touch "$F"
  out=$(nix develop -c cargo test -p factorio-bot-executor --lib recover:: 2>&1)
  code=$?
  # `error: test failed` is cargo reporting a RED, not a compile failure, and
  # matching `^error` swallowed all five kills into "BROKEN" on the first run.
  if echo "$out" | grep -qE '^error\[E|could not compile'; then
    echo "BROKEN  $name -- did not compile (a green would have been meaningless)"
  elif [ $code -ne 0 ]; then
    echo "KILLED  $name -- $(echo "$out" | grep -E '^test .* FAILED|^---- ' | head -3 | tr '\n' ' ')"
  else
    echo "SURVIVED $name -- A GREEN MUTATION IS A FINDING"
  fi
}

mutate "drop the whole check"          '&& !walk_halted_on_a_stall(log)' '&& true'
mutate "halt is not required"          'walk.abandoned.is_some() &&' 'true &&'
mutate "any halt counts, not a stall"  'walk.error.as_deref().is_some_and(walk_reports_stalled_leg)' 'walk.error.is_some()'
mutate "no verdict reads as a stall"   'walk.error.as_deref().is_some_and(walk_reports_stalled_leg)' 'walk.error.as_deref().is_none_or(walk_reports_stalled_leg)'
mutate "every walk must have stalled"  'log.walks().any(|(_bot, _index, walk)| {' 'log.walks().all(|(_bot, _index, walk)| {'

restore
echo "restored; diff against HEAD:"
git diff --no-ext-diff --stat -- "$F"
rm -f "$BAK"
