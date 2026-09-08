#!/usr/bin/env python3
"""Decode every blueprint in scripts/rcontest.lua and print its shape.

WHY THIS EXISTS. CLAUDE.md already says "The fixture is the authority, not this
prose. Decode the blueprint before quoting its shape; the tests do." Two
sessions violated that rule on the same afternoon anyway -- one quoting an
entity count corrected in its own commit message, the other attaching a lamp
correction to the block that shared the number rather than the block with the
lamps. Knowing the rule is not the same as re-deriving at the point of use, and
the difference between them is friction. This is the friction removed: one pass
over all the fixtures, cheaper than the two round trips those corrections cost.

WHAT IT DELIBERATELY DOES NOT REPORT: power draw. That would need consumer_kw,
which lives in crates/planner/src/state.rs and reads the prototype table. A
copy here would be a second copy of exactly the kind of table this project has
already been bitten by -- a pole reach table that read 30 where the game said
32, and a consumer table missing 17 electric machines. Ask the planner for kW;
ask this for shape.

    tools/fixture_shape.py             every fixture
    tools/fixture_shape.py SmeltRow24  just one
"""

import base64
import collections
import json
import pathlib
import re
import sys
import zlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "scripts" / "rcontest.lua"


def decode(text):
    """Version byte, then base64, then zlib, then JSON -- Factorio's own format."""
    return json.loads(zlib.decompress(base64.b64decode(text[1:])))


def fixtures(src):
    """Every `Name = "0..."` assignment, matched by SHAPE rather than by an
    allowlist of names, so a fixture added tomorrow is covered without anyone
    remembering to extend this."""
    for m in re.finditer(r'^\s*(?:local\s+)?([A-Za-z0-9_]+)\s*=\s*"(0[^"]{40,})"', src, re.M):
        yield m.group(1), m.group(2)


def report(name, text):
    try:
        body = decode(text).get("blueprint", {})
    except Exception as exc:  # a corrupt fixture is a finding, not a crash
        print(f"{name}\n    DOES NOT DECODE: {exc}")
        return
    ents = body.get("entities", [])
    counts = collections.Counter(e["name"] for e in ents)

    print(f"{name}")
    print(f"    entities   {len(ents)}")
    for n, c in sorted(counts.items()):
        print(f"                 {c:4d}  {n}")

    # The declared PERIOD, which cannot be derived from the entities: MinerLine
    # spans 4 tiles of x between its outermost entity centres and declares a
    # pitch of 7. The difference is room the author left on purpose.
    grid = body.get("snap-to-grid")
    if grid:
        extras = []
        if body.get("absolute-snapping"):
            extras.append("absolute")
        if body.get("position-relative-to-grid"):
            extras.append(f"offset {body['position-relative-to-grid']}")
        print(f"    pitch      {grid['x']} x {grid['y']}"
              + (f"   ({', '.join(extras)})" if extras else ""))
    else:
        print("    pitch      none declared")

    if ents:
        xs = [e["position"]["x"] for e in ents]
        ys = [e["position"]["y"] for e in ents]
        print(f"    extent     {max(xs) - min(xs):g} x {max(ys) - min(ys):g}"
              "   (entity CENTRES; not the pitch, and not a footprint)")


def main():
    src = FIXTURES.read_text()
    wanted = sys.argv[1:]
    seen = 0
    for name, text in fixtures(src):
        if wanted and name not in wanted:
            continue
        seen += 1
        report(name, text)
        print()
    if seen == 0:
        print(f"no fixture matched {wanted} in {FIXTURES}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
