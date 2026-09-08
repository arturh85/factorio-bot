#!/usr/bin/env python3
"""Report which BELT LANE every inserter and every belt junction feeds.

WHY. Two rules decide whether a mixed-commodity smelter works, and CLAUDE.md
records that both fail silently:

  * an inserter drops on the belt's FAR lane (the one away from itself);
  * a belt running into the SIDE of another sideloads onto its NEAR lane.

Get either backwards and the block places 100% correctly, every arm still
swings, and one commodity crowds the other out entirely -- measured once as a
chest whose coal drained 50 -> 17 while its ore never moved off 99, for one
plate produced. Placement and function are separate concerns here, so no
placement count and no `done=true` can see it.

Deriving this by hand takes careful reasoning per arm and nobody repeats it.
This does it from the blueprint alone.

WHAT IT CANNOT DO, said plainly: a blueprint holds no items, so this reports
the STRUCTURE (which lane each source feeds) and never "ore and coal collide".
Reading the verdict is one step of human judgement: the sources feeding one
belt must feed DIFFERENT lanes. When they do not, that is the defect above.

    tools/belt_lanes.py ElectricSmelter
    tools/belt_lanes.py                  # every fixture with belts
"""

import base64
import json
import pathlib
import re
import sys
import zlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "scripts" / "rcontest.lua"

# Factorio 2.0 uses 16 directions; belts and inserters use the 4 cardinals.
UNIT = {0: (0.0, -1.0), 4: (1.0, 0.0), 8: (0.0, 1.0), 12: (-1.0, 0.0)}
BLUEPRINT_VERSION_2_0 = 2 << 48


def migrate(direction, version):
    """What the PLANNER sees, not what the JSON says.

    `crates/core/src/types.rs::blueprint_direction`: 1.x's eight-value
    `defines.direction` sits on 2.x's even values, so a pre-2.0 blueprint has
    every direction doubled on decode. Reading the raw number instead reports
    diagonals for belts that are perfectly straight -- which is exactly what
    this tool did to FurnaceLine before this function existed.
    """
    if version >= BLUEPRINT_VERSION_2_0:
        return direction % 16
    return (direction % 8) * 2
NAME = {0: "N", 4: "E", 8: "S", 12: "W"}
BELTS = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
ARMS = {"inserter", "burner-inserter", "long-handed-inserter",
        "fast-inserter", "bulk-inserter"}


def decode(text):
    return json.loads(zlib.decompress(base64.b64decode(text[1:])))


def side_of(other, belt, travel):
    """Which side of `belt` does `other` sit on, named for the lane there.

    A belt's two lanes lie perpendicular to its travel, so an east-west belt
    has a north and a south lane and a north-south belt has east and west.
    """
    dx = other[0] - belt[0]
    dy = other[1] - belt[1]
    if travel in (4, 12):                      # travelling E or W
        return "north" if dy < 0 else "south" if dy > 0 else None
    if travel in (0, 8):                       # travelling N or S
        return "west" if dx < 0 else "east" if dx > 0 else None
    return None


def opposite(side):
    return {"north": "south", "south": "north",
            "east": "west", "west": "east"}.get(side)


def analyse(name, text):
    try:
        body = decode(text).get("blueprint", {})
    except Exception as exc:
        print(f"{name}\n    DOES NOT DECODE: {exc}")
        return
    ents = body.get("entities", [])
    version = body.get("version", 0)
    belts, arms, others = {}, [], {}
    for e in ents:
        p = (e["position"]["x"], e["position"]["y"])
        d = migrate(e.get("direction", 0), version)
        if e["name"] in BELTS:
            belts[p] = d
        elif e["name"] in ARMS:
            arms.append((p, d, e["name"]))
        else:
            others[p] = e["name"]
    if not belts:
        return

    print(f"{name}")
    findings = []

    # --- arms: an inserter picks up in the direction it FACES ---------------
    # Established empirically in this repo: direction 12 ("west") moves items
    # west to east, so the facing names the PICKUP side, not the drop side.
    for (p, d, nm) in sorted(arms):
        u = UNIT.get(d)
        if u is None:
            print(f"    {nm} at {p} has non-cardinal direction {d}")
            continue
        pick = (p[0] + u[0], p[1] + u[1])
        drop = (p[0] - u[0], p[1] - u[1])
        for label, tile in (("picks from", pick), ("drops onto", drop)):
            if tile in belts:
                travel = belts[tile]
                side = side_of(p, tile, travel)
                if side is None:
                    continue
                if label == "drops onto":
                    lane, why = opposite(side), "far lane, away from the arm"
                else:
                    lane, why = opposite(side), "prefers the far lane"
                findings.append((tile, lane, f"{nm} at {p} facing {NAME.get(d,d)} "
                                              f"{label} belt {tile} -> {lane} lane "
                                              f"({why})"))
            elif tile in others:
                pass  # a chest or machine: not a lane question

    # --- belt into the SIDE of another belt: sideload onto the NEAR lane ----
    for p, d in sorted(belts.items()):
        u = UNIT.get(d)
        if u is None:
            continue
        ahead = (p[0] + u[0], p[1] + u[1])
        if ahead in belts:
            target = belts[ahead]
            # Perpendicular travel means this runs into the target's side.
            if (d in (0, 8)) != (target in (0, 8)):
                side = side_of(p, ahead, target)
                findings.append((ahead, side,
                                 f"belt {p} travelling {NAME.get(d,d)} sideloads into "
                                 f"belt {ahead} ({NAME.get(target,target)}) -> {side} "
                                 f"lane (near lane, the side it came from)"))

    by_belt = {}
    for tile, lane, text_ in findings:
        by_belt.setdefault(tile, []).append((lane, text_))
    for tile in sorted(by_belt):
        for lane, text_ in by_belt[tile]:
            print(f"    {text_}")
    # The judgement step, stated rather than guessed at.
    feeders = {}
    for tile, lane, text_ in findings:
        if "drops onto" in text_ or "sideloads" in text_:
            feeders.setdefault(tile, []).append(lane)
    for tile, lanes in sorted(feeders.items()):
        if len(lanes) > 1 and len(set(lanes)) == 1:
            print(f"    ** {len(lanes)} sources feed belt {tile} and ALL land on "
                  f"the {lanes[0]} lane -- if they carry different commodities, "
                  f"one will crowd the other out")
    print()


def fixtures(src):
    for m in re.finditer(r'^\s*(?:local\s+)?([A-Za-z0-9_]+)\s*=\s*"(0[^"]{40,})"',
                         src, re.M):
        yield m.group(1), m.group(2)


def main():
    src = FIXTURES.read_text()
    wanted = sys.argv[1:]
    for name, text in fixtures(src):
        if wanted and name not in wanted:
            continue
        analyse(name, text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
