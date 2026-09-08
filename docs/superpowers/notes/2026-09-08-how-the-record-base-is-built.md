# How the record base is actually built

Geometry extracted from `workspace/wrload/scripts/wr-census.json`. Apparatus:
`tools/census_nodes.awk` (streams the 2.9 GB dump to TSV in ~100 s) and
`tools/census_geometry.py`. Full output in
`docs/superpowers/notes/data/2026-09-08-wr-census-geometry.txt`, ASCII maps in
`…-wr-census-maps.txt`. Everything below is derived from the dump; nothing is
typed in. Load 9-18 throughout; no timing here depends on it.

## What the data is, and is not

- **Both census files are the same world at the same moment** — identical
  `research_progress` (0.8597424445688568) and current research. There is only
  one base in this data.
- It is the **6:39:53** save: 1,195 steel + 26 stone furnaces against the 1,222
  in `2026-09-06-what-the-record-base-knows.md`, 550 iron drills against 559.
  **`RSNG_3_17_06.zip` has not been censused.**
- Space Age confirmed from the dump: `calcite`, `biosulfur`, `biolab`,
  `foundry`, `metallurgic-science-pack` all in the prototype table. 165 of 277
  technologies researched.
- **Nauvis only** (the mod's chunk guard), 39,184 entities, and the
  `EntityGraph` whitelist applies — **all 249 beacons are absent**, so nothing
  here can speak to beacon lanes.
- **END STATE ONLY. Nothing carries a build tick.** The 73 burner drills of
  minute 10 are 28 here. Treat every unit below as minute-400 geometry unless
  the note says otherwise.

## 1. The inserter offset rule — the whole grammar

Measured over 4,300 machine-serving inserters. **Zero exceptions above n=20.**

| inserter tile offset from machine face | kind | belt row it reaches (offset from face) |
|---|---|---|
| 1 | `inserter` / `fast-inserter` / `burner-inserter` | 2 |
| 1 | `long-handed-inserter` | 3 |
| 2 | `long-handed-inserter` | 4 |

So **one machine face reaches three belt rows using two tile-rows of inserter
space**. `normal@1` and `long@1` stand *side by side* in the same lane; `long@2`
sits behind them. There is no `normal@2` — it cannot reach the machine.

This is the owner's description, measured: *"two ingredient belts below a
crafter and two above… normal and long inserters to pull/push."*

**Long-handed inserters are 20.7% of all inserters (1,172 of 5,673)** and are an
assembly tool exclusively:

| pickup → drop | n | share of long |
|---|---|---|
| belt → assembling-machine-2 | 623 | 53.2% |
| assembling-machine-2 → belt | 166 | 14.2% |
| belt → assembling-machine-3 | 107 | 9.1% |
| assembling-machine-2 → assembling-machine-3 | 72 | 6.1% |
| assembling-machine-2 → assembling-machine-2 | 30 | 2.6% |

**0 of 1,172 long-handed inserters touch a furnace at either end.** Smelting is
100% reach-1.

Inserters per served face:

| lane 1 (adjacent) | lane 2 | AM2 | AM3 |
|---|---|---:|---:|
| `n` | – | 46.3% | 25.6% |
| `n` + `L` | – | 16.4% | 8.9% |
| `L` | `L` | 10.3% | 12.8% |
| `n` + `L` | `L` | 2.4% | **21.7%** |

The higher-tier machine, with more ingredients, uses the deeper stack. That is
the scaling axis: **k ingredients cost k inserters on one face, not k chests.**

## 2. Smelting — period 11 across, 2 along

```
x:  [F F][i][B B][i][F F][i][B][i]  = 11
     ^^^^      ^^^        ^^^^   ^
   furnace  INPUT lane  furnace  OUTPUT lane
   column   (2 belts)   column   (1 belt)
```

Confirmed by the column histogram: furnace tile columns at 35, 40, 46, 51, 57,
62, 68, 73, … — deltas alternating 5, 6, so period 11 with two furnace columns.

Lane roles measured from inserter direction, not assumed:

| | lane width 1 | lane width 2 |
|---|---:|---:|
| furnace → belt (OUT) | **1,158** | 12 |
| belt → furnace (IN) | 306 | **902** |

- **The 1-wide lane is plate output, shared by the two furnace columns flanking
  it. The 2-wide lane is ore/coal input**, one belt per flanking column — each
  column's inserter reaches only its own near belt, and that belt carries ore on
  one lane and coal on the other. That is our own measured *"an inserter picks
  from both lanes, preferring the far one"* being relied on.
- Along the row, furnace pitch is **2** (packed solid; y-gaps
  `{2: 668, 5: 234, 6: 210}`, and the 5 and 6 are the two lane widths), with
  **exactly one inserter per furnace per side**. Because a 2×2 furnace's
  inserter sits on its near tile, consecutive furnaces alternate which of their
  two tiles is used — and **the spare tiles that alternation creates are where
  the poles go** (417 small poles sit horizontally adjacent to a furnace).
- Belt is always at face-offset 2.

## 3. Ore extraction — period 7

```
x:  [B][D D D][D D D][B][D D D][D D D]   = 1 + 3 + 3
     ^   drill column  ^
   belt serving BOTH flanking columns
```

Drill x-gaps `{3: 883, 4: 323}` — 3 = drills touching, 4 = one belt lane between.
y-gap identical, packed solid at 3.

- **1,473 of 1,484 electric drills output onto a belt. Zero into a furnace, zero
  into a chest.** The only exception is **7-8 coal drills dropping straight into
  a boiler** — power fed with no belt and no inserter at all.
- **The arrangement is identical for every ore.** coal / copper / iron / stone /
  uranium all show x-gaps `{3,4}` and y-gaps `{3,4}`. Nothing is special-cased
  for coal.
- Poles stand *inside* the belt lane, and the belt passes them with an
  underground pair (777 of 1,823 underground belts are within 2 tiles of a pole).

**The one plausibly-early artefact**, and it is only plausible: 28 burner drills
survive at x -48..-28, y 56..68, ~72 tiles from spawn — the closest structures to
spawn in the base. **26 of them mine coal and output into another burner drill**
(the self-fuelling chain), and **2 mine stone directly into the base's single
`assembling-machine-1`**. Beside them sits a 2-wide column of the 26 stone
furnaces. Burner technology and proximity to spawn are *consistent with* this
being the starter base; **the census carries no build tick and I cannot date it.**

## 4. Power — period 15, exactly

`boiler` y-gaps are `{15: 380}` — 380 of 380, no other value.

```
y+0 …  1   boiler row     3x2, x-pitch 3, packed solid, all North
y+2        inserter row   coal -> boiler, reach 1, belt@2
y+3        coal belt
y+4 …  8   steam engine   3x5, x-pitch 3, all North
y+9        pipe / pole row
y+10 … 14  steam engine   3x5
y+15       next boiler row
```

- **461 boilers : 895 steam engines = 1 : 1.94.** Ten tiles of engine per boiler
  row is that ratio laid out. **Do not copy the number — derive it** from
  `boiler.electric_energy_usage / steam-engine.max_energy_production`. Both
  fields are on `FactorioEntityPrototype` today, and **neither is in this
  census**, which predates them landing (2026-09-07): the dump's `boiler` and
  `steam-engine` entries carry only `collision_box`, `mine_result`,
  `mining_time` and `fluidbox_prototypes`. So 1.94 is a *measurement of one
  base*, not a derivation, and a fresh dump is what turns it into one.
- **All 461 boilers and all 895 engines face North.** One canonical orientation
  across the whole farm.
- 10 offshore pumps and 12 storage tanks for 461 boilers.
- 423 of 461 boilers are coal-fed by a normal `inserter` off a belt; 30 by a
  `burner-inserter`; 7-8 by a drill directly.

## 5. What recurs that we have no concept for

1. **`long-handed-inserter` is modelled and never placed.**
   `crates/planner/src/state.rs:719` returns reach `2.0` for it — the only place
   in the tree that knows it exists — while `method::assemble.rs:144` and
   `method::connect.rs:24` both hard-code `const INSERTER = "inserter"`. **The
   reach is there and no method can use it.** This is exactly tonight's wall:
   the impossibility proof was about a shape built from reach-1 inserters and
   one chest per ingredient, and the record base uses neither.
2. **The flanking belt bus.** We have no concept of belt rows at face-offsets
   2/3/4 reached by inserter class. `method::assemble` gives one chest per
   ingredient and runs out of mouths at three; the record gives **one belt row
   per ingredient** and pays for it in inserter *kind*, not in perimeter.
3. **Machine → machine long inserter, no belt** — 102 instances (AM2→AM3 72,
   AM2→AM2 30). Two machine rows separated by a lane, bridged directly by a
   `long@2` on each side. We have no notion of a machine handing to a machine.
4. **A shared output lane serving two machine rows from both sides** — the
   1-wide plate lane, and the drill field's shared belt. Our blocks are
   one-sided; halving the belt count is a real density win.
5. **Poles placed in the slots the inserter alternation leaves free**, and
   **underground belt used to pass a pole standing in a belt lane.** We treat
   power routing and belt routing as independent problems; here they interleave
   in the same tiles.
6. **56% of machine faces abut another machine.** The base has essentially no
   walkable interior. Our enclosure guard and walk router assume aisles; a block
   this dense is built from outside in, and the aisles we leave are cost.

## Forced vs style

| | |
|---|---|
| **Forced** (encode) | The offset rule (inserter reach). Belt at offsets 2/3/4. Furnace pitch 2, drill pitch 3, assembler pitch 3 — each is the machine's own width. Boiler:engine 1:2 from prototypes. The 15-tile power period, which is the sum of forced parts. One inserter per 2×2 furnace face. |
| **Style** (do not encode) | Which lane is input and which output. All-North power orientation. Choosing a 2-wide input lane over two 1-wide ones. |
| **Foreknowledge** | **Nothing in these units.** Only the *siting* of the fields depends on knowing where ore is. The layouts transfer intact — which is the useful half. |

## Limits

- End-state only; no build ticks. §3's burner-drill chain is the sole
  early-game *candidate* and is undated.
- One save. The 3:17:06 run is not in this data and may build differently.
- Beacons and 763 other prototypes are outside the graph whitelist, so no claim
  here covers beacon lanes — which the standing "leave room for beacons" rule
  needs and this measurement cannot supply.
- The dump carries no `recipe` per machine, so nothing here says *what* an
  assembler makes — only how it is fed.
