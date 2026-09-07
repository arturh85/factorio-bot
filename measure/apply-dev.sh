#!/usr/bin/env bash
# apply-dev.sh <our-opt> <deps-opt> <debug>
#
# Rewrites [profile.dev] opt-level, [profile.dev.package."*"] opt-level, and
# [profile.dev] debug. The release profile is left alone.
#
# The asymmetry is the whole point of the experiment: OUR crates are recompiled
# on every edit, so their opt-level is where build cost lives; DEPENDENCIES are
# built once and served by sccache thereafter (sccache refuses incremental
# compilation, and cargo builds registry deps non-incrementally, so deps are
# exactly the half that caches), which makes optimising them close to free and
# buys runtime.
#
# `debug` is in here because debug info is a first-class build cost that the
# opt-level discussion tends to hide -- it is written by rustc, grows the object
# files, and is what the linker then has to chew through. A dev profile that is
# fast to build but useless in a debugger is a different trade from a slow one,
# so it is measured rather than assumed either way.
set -euo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

OURS="$1"; DEPS="$2"; DBG="$3"

q() { case "$1" in s|z) echo "\"$1\"";; *) echo "$1";; esac; }

python3 - "$WT/Cargo.toml" "$(q "$OURS")" "$(q "$DEPS")" "$DBG" <<'PY'
import sys
p, ours, deps, dbg = sys.argv[1:5]
# Cargo.toml is CRLF in this repo -- preserve it, or every line reads as
# changed and a diff says nothing.
raw = open(p, newline='').read()
eol = '\r\n' if '\r\n' in raw else '\n'
lines = raw.split(eol)

section = None
seen = set()
out = []
for l in lines:
    s = l.strip()
    if s.startswith('['):
        # closing out [profile.dev]: make sure `debug` exists even if the file
        # never had the key. A missing key is not "debug off" -- cargo's dev
        # default is debug=true -- so it has to be written, not assumed.
        if section == 'dev' and 'debug' not in seen:
            out.append(f'debug = {dbg}')
            seen.add('debug')
        section = ('dev' if s == '[profile.dev]'
                   else 'deps' if s == '[profile.dev.package."*"]'
                   else None)
        out.append(l)
        continue
    k = s.split('=')[0].strip()
    if section == 'dev' and k == 'opt-level':
        out.append(f'opt-level = {ours}'); seen.add('opt'); continue
    if section == 'dev' and k == 'debug':
        out.append(f'debug = {dbg}'); seen.add('debug'); continue
    if section == 'deps' and k == 'opt-level':
        out.append(f'opt-level = {deps}'); seen.add('deps'); continue
    if section == 'deps' and k == 'debug':
        # the file ships `debug = false` for deps; leave it, but note it
        out.append(l); continue
    out.append(l)
if section == 'dev' and 'debug' not in seen:
    out.append(f'debug = {dbg}'); seen.add('debug')

need = {'opt', 'deps', 'debug'}
assert need <= seen, f'missed {need - seen}'
open(p, 'w', newline='').write(eol.join(out))
PY

echo "== applied dev: ours=$OURS deps=$DEPS debug=$DBG"
sed -n '/\[profile\.dev\]/,/git-cliff/p' "$WT/Cargo.toml" | tr -d '\r' | head -14
