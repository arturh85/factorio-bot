#!/usr/bin/env bash
# apply.sh <linker: ld|mold> <lto: fat|thin> <cu: N> <opt: s|3>
# Rewrites .cargo/config.toml [build] rustflags and Cargo.toml [profile.release].
set -euo pipefail
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

LINKER="$1"; LTO="$2"; CU="$3"; OPT="$4"

# --- .cargo/config.toml rustflags -------------------------------------------
# `build.rustflags` is EXTENDED, never replaced by a
# `[target.x86_64-unknown-linux-gnu]` block: cargo does not merge the two, the
# target one wins outright, and `--cfg tokio_unstable` would vanish silently.
if [ "$LINKER" = mold ]; then
  FLAGS='rustflags = ["--cfg", "tokio_unstable", "-C", "link-arg=-fuse-ld=mold"]'
else
  FLAGS='rustflags = ["--cfg", "tokio_unstable"]'
fi
python3 - "$WT/.cargo/config.toml" "$FLAGS" <<'PY'
import re,sys
p,flags=sys.argv[1],sys.argv[2]
s=open(p).read()
s=re.sub(r'^rustflags = \[.*\].*$', flags+' # for tokio-console (+linker)', s, count=1, flags=re.M)
open(p,'w').write(s)
PY

# --- Cargo.toml [profile.release] -------------------------------------------
case "$LTO" in fat) LTOV='true';; thin) LTOV='"thin"';; esac
case "$OPT" in s) OPTV='"s"';; *) OPTV="$OPT";; esac
python3 - "$WT/Cargo.toml" "$LTOV" "$CU" "$OPTV" <<'PY'
import sys
p,lto,cu,opt=sys.argv[1:5]
want={'lto':lto,'codegen-units':cu,'opt-level':opt}
# Cargo.toml is CRLF in this repo -- preserve it, or every line reads as
# changed and a diff says nothing.
raw=open(p,newline='').read()
eol='\r\n' if '\r\n' in raw else '\n'
lines=raw.split(eol)
inblock=False; seen=set()
for i,l in enumerate(lines):
    if l.startswith('['):
        inblock = (l.strip()=='[profile.release]')
        continue
    if not inblock: continue
    k=l.split('=')[0].strip()
    if k in want:
        lines[i]=f'{k} = {want[k]}'; seen.add(k)
assert seen==set(want), f'missed {set(want)-seen}'
open(p,'w',newline='').write(eol.join(lines))
PY

echo "== applied: linker=$LINKER lto=$LTO cu=$CU opt=$OPT"
grep -n 'rustflags' "$WT/.cargo/config.toml"
sed -n '/\[profile.release\]/,/^$/p' "$WT/Cargo.toml"
