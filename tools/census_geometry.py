#!/usr/bin/env python3
"""Measure the repeating construction units in a factorio-bot world dump.

    awk -f tools/census_nodes.awk dump.json > nodes.tsv
    python3 tools/census_geometry.py nodes.tsv [--map NAME]

Everything here is derived from the dump, never typed in: footprints come from
the collision boxes, ratios from counting, periods from neighbour-gap
histograms.  The one hand-written table is the inserter reach, which no
prototype field in `FactorioEntityPrototype` carries (see
`crates/planner/src/state.rs::inserter_reach`, which has the same problem).

Written for `docs/superpowers/notes/2026-09-08-how-the-record-base-is-built.md`.
"""
import sys, math, collections

BELT = {"transport-belt", "fast-transport-belt", "underground-belt",
        "fast-underground-belt", "splitter", "fast-splitter"}
REACH = {"inserter": 1, "fast-inserter": 1, "bulk-inserter": 1,
         "burner-inserter": 1, "filter-inserter": 1, "stack-inserter": 1,
         "long-handed-inserter": 2}
DIR = {"North": (0, -1), "South": (0, 1), "East": (1, 0), "West": (-1, 0)}
MACH = {"assembling-machine-1", "assembling-machine-2", "assembling-machine-3",
        "chemical-plant", "steel-furnace", "stone-furnace", "biolab",
        "centrifuge", "oil-refinery", "boiler", "electric-mining-drill"}
GLYPH = {"transport-belt": "-", "fast-transport-belt": "=", "underground-belt": "u",
         "fast-underground-belt": "U", "splitter": "S", "fast-splitter": "S",
         "inserter": "i", "long-handed-inserter": "L", "fast-inserter": "f",
         "bulk-inserter": "k", "burner-inserter": "b", "steel-furnace": "F",
         "stone-furnace": "s", "assembling-machine-1": "1", "assembling-machine-2": "A",
         "assembling-machine-3": "3", "chemical-plant": "c", "electric-mining-drill": "D",
         "burner-mining-drill": "d", "small-electric-pole": "o", "medium-electric-pole": "O",
         "big-electric-pole": "@", "boiler": "H", "steam-engine": "E", "pipe": "+",
         "pipe-to-ground": "^", "offshore-pump": "P", "pumpjack": "J",
         "oil-refinery": "R", "biolab": "B", "centrifuge": "Z", "storage-tank": "T"}


def load(path):
    out = []
    for line in open(path):
        f = line.rstrip("\n").split("\t")
        out.append(dict(n=f[0], ore=f[1], x=float(f[2]), y=float(f[3]), d=f[4],
                        lx=float(f[5]), ly=float(f[6]), rx=float(f[7]), ry=float(f[8])))
    return out


def tiles(r):
    """Exact tile footprint. Collision boxes are shrunk inside their tiles, so
    floor(left_top)..floor(right_bottom) is the footprint and `position` is not."""
    return [(x, y) for x in range(math.floor(r["lx"]), math.floor(r["rx"]) + 1)
                   for y in range(math.floor(r["ly"]), math.floor(r["ry"]) + 1)]


def occupancy(rows):
    occ = {}
    for r in rows:
        for t in tiles(r):
            occ.setdefault(t, r)
    return occ


def box(r):
    ts = tiles(r); xs = [t[0] for t in ts]; ys = [t[1] for t in ts]
    return min(xs), max(xs), min(ys), max(ys)


def faces(r):
    """(start_x, start_y, out_x, out_y) for the middle tile of each of 4 faces."""
    x0, x1, y0, y1 = box(r); mx = (x0 + x1) // 2; my = (y0 + y1) // 2
    return ((mx, y0, 0, -1), (mx, y1, 0, 1), (x0, my, -1, 0), (x1, my, 1, 0))


def composition(rows):
    print("=== composition ===")
    for n, c in collections.Counter(r["n"] for r in rows).most_common():
        print(f"  {n:26s} {c}")


def inserter_rule(rows, occ):
    """The whole grammar: where an inserter stands relative to a machine face,
    and which belt row that lets it reach."""
    print("\n=== inserter offset rule  (machine | kind | inserter tile offset "
          "from face | belt row offset from face) ===")
    tab = collections.Counter()
    for r in rows:
        if r["n"] not in REACH:
            continue
        ux, uy = DIR[r["d"]]; k = REACH[r["n"]]
        cx, cy = math.floor(r["x"]), math.floor(r["y"])
        a = occ.get((cx + ux * k, cy + uy * k)); b = occ.get((cx - ux * k, cy - uy * k))
        m = belt = None
        for e, o in ((a, b), (b, a)):
            if e and e["n"] in MACH and o and o["n"] in BELT:
                m, belt = e, o
        if m is None:
            continue
        x0, x1, y0, y1 = box(m)
        if ux:
            face = x0 - 1 if cx < x0 else x1 + 1
            io = abs(cx - face) + 1
            bo = max(abs(cx - ux * k - face), abs(cx + ux * k - face)) + 1
        else:
            face = y0 - 1 if cy < y0 else y1 + 1
            io = abs(cy - face) + 1
            bo = max(abs(cy - uy * k - face), abs(cy + uy * k - face)) + 1
        tab[("furnace" if "furnace" in m["n"] else m["n"], r["n"], io, bo)] += 1
    for (mn, ins, io, bo), c in sorted(tab.items(), key=lambda kv: -kv[1]):
        if c < 20:
            continue
        print(f"  {mn:22s} {ins:22s} inserter@{io}  belt@{bo}   {c:5d}")
    L = sum(1 for r in rows if r["n"] == "long-handed-inserter")
    T = sum(1 for r in rows if r["n"] in REACH)
    print(f"\n  long-handed share of all inserters: {L} / {T} = {100*L/T:.1f}%")
    n = 0
    for r in rows:
        if r["n"] != "long-handed-inserter":
            continue
        ux, uy = DIR[r["d"]]; cx, cy = math.floor(r["x"]), math.floor(r["y"])
        for s in (2, -2):
            e = occ.get((cx + ux * s, cy + uy * s))
            if e and "furnace" in e["n"]:
                n += 1
    print(f"  long-handed inserters touching a furnace at either end: {n} of {L}")


def ends(rows, occ):
    print("\n=== what each inserter class moves, pickup -> drop ===")
    print("  (direction names the side it PICKS UP from -- repo convention, and")
    print("   the sensible pairings below are what confirm it)")
    tot = collections.Counter(); pair = collections.defaultdict(collections.Counter)
    for r in rows:
        if r["n"] not in REACH:
            continue
        ux, uy = DIR[r["d"]]; k = REACH[r["n"]]
        cx, cy = math.floor(r["x"]), math.floor(r["y"])
        def kind(t):
            e = occ.get(t)
            if e is None: return "empty"
            if e["n"] in BELT: return "belt"
            if "chest" in e["n"]: return "chest"
            return e["n"]
        tot[r["n"]] += 1
        pair[r["n"]][(kind((cx + ux * k, cy + uy * k)),
                      kind((cx - ux * k, cy - uy * k)))] += 1
    for n, c in tot.most_common():
        print(f"\n  {n}  reach {REACH[n]}  n={c}")
        for (a, b), k in pair[n].most_common(6):
            print(f"     {a:22s} -> {b:22s} {k:5d}  {100*k/c:4.1f}%")


def per_face(rows, occ, mach):
    """How many inserters of which kind stand on each served face, split by the
    two lanes a machine face can be served from."""
    combo = collections.Counter()
    for m in (r for r in rows if r["n"] == mach):
        x0, x1, y0, y1 = box(m)
        for ux, uy in ((0, -1), (0, 1), (-1, 0), (1, 0)):
            lane = {1: [], 2: []}
            span = range(x0, x1 + 1) if uy else range(y0, y1 + 1)
            for v in span:
                for d in (1, 2):
                    t = (v, (y0 if uy < 0 else y1) + uy * d) if uy \
                        else ((x0 if ux < 0 else x1) + ux * d, v)
                    e = occ.get(t)
                    if e and e["n"] in REACH:
                        lane[d].append("L" if e["n"] == "long-handed-inserter" else "n")
            if lane[1] or lane[2]:
                combo[("".join(sorted(lane[1])) or "-", "".join(sorted(lane[2])) or "-")] += 1
    t = sum(combo.values())
    print(f"\n=== {mach}: inserters per served face "
          f"(lane1 = adjacent to face, lane2 = one further out) ===")
    for (a, b), k in combo.most_common(8):
        print(f"  lane1={a:5s} lane2={b:5s}  {k:5d}  {100*k/t:4.1f}%")


def periods(rows):
    print("\n=== packing period: gaps between consecutive same-kind machines ===")
    for n in sorted({r["n"] for r in rows if r["n"] in MACH or r["n"] == "steam-engine"}):
        ps = [(math.floor(r["x"]), math.floor(r["y"])) for r in rows if r["n"] == n]
        if len(ps) < 20:
            continue
        line = f"  {n:24s} n={len(ps):5d}"
        for ax, lab in ((0, "x"), (1, "y")):
            g = collections.defaultdict(set)
            for p in ps:
                g[p[1 - ax]].add(p[ax])
            c = collections.Counter()
            for v in g.values():
                v = sorted(v)
                c.update(v[i + 1] - v[i] for i in range(len(v) - 1))
            line += f"   {lab}-gaps {dict(c.most_common(3))}"
        print(line)


def drills(rows, occ):
    print("\n=== drill output target, by ore ===")
    res = collections.Counter()
    for r in rows:
        if "drill" not in r["n"]:
            continue
        ux, uy = DIR[r["d"]]; x0, x1, y0, y1 = box(r)
        cx = (x0 + x1) // 2 if not ux else (x0 if ux < 0 else x1)
        cy = (y0 + y1) // 2 if not uy else (y0 if uy < 0 else y1)
        e = occ.get((cx + ux, cy + uy)); nm = e["n"] if e else "empty"
        res[(r["ore"], r["n"], "belt" if nm in BELT else
             ("chest" if "chest" in nm else nm))] += 1
    for (o, d, t), c in sorted(res.items(), key=lambda kv: (kv[0][0], -kv[1])):
        print(f"  {o:12s} {d:22s} -> {t:24s} {c}")


def render(rows, occ, name, w=48, h=32):
    """ASCII map of the densest w x h cell containing `name`."""
    cnt = collections.Counter()
    for r in rows:
        if r["n"] == name:
            cnt[(math.floor(r["x"]) // w, math.floor(r["y"]) // h)] += 1
    (bx, by), c = cnt.most_common(1)[0]
    x0, y0 = bx * w - 4, by * h - 4
    print(f"\n### densest {name} region ({c} in cell)  x {x0}..{x0+w+7}  y {y0}..{y0+h+7}")
    for y in range(y0, y0 + h + 8):
        row = ""
        for x in range(x0, x0 + w + 8):
            e = occ.get((x, y))
            row += " " if e is None else GLYPH.get(e["n"], "C" if "chest" in e["n"] else "?")
        print(f"{y:6d} {row}")


def main():
    rows = load(sys.argv[1]); occ = occupancy(rows)
    if "--map" in sys.argv:
        render(rows, occ, sys.argv[sys.argv.index("--map") + 1])
        return
    composition(rows)
    inserter_rule(rows, occ)
    ends(rows, occ)
    for m in ("assembling-machine-2", "assembling-machine-3"):
        per_face(rows, occ, m)
    periods(rows)
    drills(rows, occ)
    nf = touch = 0
    for r in rows:
        if r["n"] not in MACH and r["n"] != "steam-engine":
            continue
        for sx, sy, ux, uy in faces(r):
            e = occ.get((sx + ux, sy + uy)); nf += 1
            if e and (e["n"] in MACH or e["n"] == "steam-engine"):
                touch += 1
    print(f"\n=== packing density: {touch} of {nf} machine faces "
          f"({100*touch/nf:.0f}%) abut another machine ===")


if __name__ == "__main__":
    main()
