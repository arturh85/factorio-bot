#!/usr/bin/env python3
"""Decode a world dump's resource patches: extent, components, inradius.

Run this instead of typing a patch's shape. It answers the question
`produce::cell_search_radius` is derived from -- **how far from a patch's
deepest ore tile is the nearest non-ore ground** -- which is the quantity that
decides whether a cell siting search can reach a rim from an interior anchor.

    tools/patch_shape.py workspace/scripts/map.json iron-ore

Reads only the `resources` map out of the dump (a `{name: [[[x, y], amount]]}`
table), so it does not need the 800 MB of entity tree around it.

Three quantities, deliberately named apart, because two of them were being
quoted at each other:

* **bbox**       width and height of the bounding box.
* **inradius**   Chebyshev distance from the deepest tile to non-ore ground.
                 The ring walk in `plan_cell` is Chebyshev (a square ring per
                 radius), so this is the radius that search needs.
* **components** connected tiles, 8-neighbour. A "patch" in `EntityGraph` is a
                 flood fill, so a name with two blobs has two of them and the
                 bounding box of either is not the bounding box of both.

Measured 2026-09-09 on the seed-31337 t=0 dump (`workspace/scripts/map.json`,
fingerprint `c161fa3f437221d0`):

    iron-ore    940 tiles  ONE component  38 x 36  inradius 12
"""

import json
import sys


def resources(path):
    """The dump's `resources` table, decoded without parsing the whole file."""
    with open(path) as handle:
        text = handle.read()
    key = '"resources"'
    at = text.index(key)
    start = text.index("{", at + len(key))
    return json.JSONDecoder().raw_decode(text, start)[0]


def components(tiles):
    """Connected tile sets, 8-neighbour, largest first."""
    remaining = set(tiles)
    out = []
    while remaining:
        seed = remaining.pop()
        stack, comp = [seed], [seed]
        while stack:
            x, y = stack.pop()
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    n = (x + dx, y + dy)
                    if n in remaining:
                        remaining.discard(n)
                        stack.append(n)
                        comp.append(n)
        out.append(comp)
    out.sort(key=len, reverse=True)
    return out


def inradius(comp):
    """Chebyshev distance from the deepest tile of `comp` to ground off it."""
    inside = set(comp)
    depth = {}
    front = [
        t
        for t in inside
        if any(
            (t[0] + dx, t[1] + dy) not in inside
            for dx in (-1, 0, 1)
            for dy in (-1, 0, 1)
        )
    ]
    for tile in front:
        depth[tile] = 1
    d = 1
    while front:
        d += 1
        nxt = []
        for x, y in front:
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    n = (x + dx, y + dy)
                    if n in inside and n not in depth:
                        depth[n] = d
                        nxt.append(n)
        front = nxt
    return max(depth.values()) if depth else 0


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    table = resources(argv[1])
    wanted = argv[2:] or sorted(table)
    for name in wanted:
        if name not in table:
            print(f"{name}: not in this dump ({', '.join(sorted(table))})")
            continue
        tiles = [(entry[0][0], entry[0][1]) for entry in table[name]]
        comps = components(tiles)
        print(f"{name}: {len(tiles)} tiles in {len(comps)} component(s)")
        for comp in comps:
            xs = [t[0] for t in comp]
            ys = [t[1] for t in comp]
            print(
                f"  {len(comp):5d} tiles  "
                f"x[{min(xs)},{max(xs)}] {max(xs) - min(xs) + 1} wide  "
                f"y[{min(ys)},{max(ys)}] {max(ys) - min(ys) + 1} tall  "
                f"inradius {inradius(comp)}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
