#!/usr/bin/env python3
"""Perturb a real world dump by charting a small crude-oil patch.

`workspace/scripts/map.json` is the seed-31337 t=0 dump and charts **no** crude
oil at all, so `researched:oil-processing` refuses at the first rung of
`method::extract`'s ladder and the siting path is unreachable. The exploration
lane has measured that one ring of surveys takes seed 31337 from 0 to 7 charted
crude-oil tiles, but nobody has dumped the world after such a ring
(`docs/superpowers/notes/2026-09-06-exploration.md` §6), so this stands in for
one until that dump exists.

**Everything else in the dump is real** -- the terrain, the entity graph, the
prototypes with `resource_category`/`resource_categories`, the recipes and the
whole technology tree. Only the seven `crude-oil` tiles are injected, and only
into `entity_graph.resources`, which is what `PlanState::resource_patches` and
`has_resource_patches` read.

Positions are **tile keys** (`Pos`, which floors); `resource_patches` restores
the half-tile centre on the way out, which is exactly the round trip the siting
code must not break.

Usage: inject_oil.py <in.json> <out.json> <centre-x> <centre-y>
"""
import sys, mmap, json

src, dst, cx, cy = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])

# Seven tiles, the count one exploration ring actually charted on seed 31337:
# a plus with three extra so the patch is not a perfect cross.
offsets = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (2, 0)]
tiles = [[[cx + dx, cy + dy], 3000] for dx, dy in offsets]
blob = '"crude-oil": ' + json.dumps(tiles) + ",\n      "

with open(src, "rb") as f:
    m = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)
    marker = b'"resources": {\n      '
    i = m.find(marker)
    assert i != -1, "the dump has no entity_graph.resources map"
    assert m.find(marker, i + 1) == -1, "more than one resources map; ambiguous"
    cut = i + len(marker)
    with open(dst, "wb") as out:
        out.write(m[:cut])
        out.write(blob.encode())
        out.write(m[cut:])
print(f"wrote {dst}: 7 crude-oil tiles around ({cx}, {cy})")
