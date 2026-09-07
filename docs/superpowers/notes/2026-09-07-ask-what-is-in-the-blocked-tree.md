# Ask what is in the blocked tree

2026-09-07. Branch `ask-what-is-in-the-tree`, off `e391a6c2`.

## What this is

An instrument, not an explanation. `world.blocked_boxes(left_top, right_bottom)`
enumerates the boxes in `EntityGraph`'s `blocked_tree` over a rectangle, and
`scripts/blocked_diff.lua` puts that beside `rcon.find_entities_in_radius` over
the same ground.

It exists because two sessions were blinded by the same gap. A build refused
with *"cannot build burner-mining-drill at tile (-16,-14): occupied by a tree,
cliff, rock or unit"*; the game was asked and the model was asked and neither
showed a tree at any point, because **neither was being asked the question the
refusal was decided by**. `world.find_entities_in_radius` reads `entity_tree`, a
whitelist of factory entity types that trees, small rocks, cliffs, units and
water never enter. `blocked_tree` is the tree the refusal reads, and nothing in
Lua could enumerate it. Two attempts to explain the mechanism were written from
that blind spot and both were retracted (`a8460ee5`).

**Nothing here tries to solve the (-16,-14) case.** It makes it askable.

## It needed no mod change, and no change to `crates/core` either

The handover called this "the mod". It is not: `blocked_tree` lives in Rust,
and every accessor it needs already existed —
`EntityGraph::blocking_boxes_within_minable` (the boxes and their one bit),
`EntityGraph::tiles_within` (what ground has been written out) and
`EntityGraph::blocked_tree()` (the index's own extent). The whole change is
`crates/scripting_lua`: a new `blocked.rs`, one binding in `globals/world.rs`,
one root in `documented_type_schemas()`. `entity_graph.rs` is untouched, which
also keeps it out of the way of the agent adding edge maintenance to it.

## How an unnameable box reads

`blocked_tree`'s payload is a bare `is_minable` flag and no name. That is
defensible for a real tree and it is exactly what makes a wrong verdict
unreadable, so the binding reports the flag and refuses to invent the rest:

```
a box, minable, source unknown
a box, not minable, source unknown
```

Following `Occupant::Terrain { minable }` (`crates/planner/src/state.rs`), whose
wording was fixed the same way after the four-item recital *"a tree, cliff, rock
or unit"* named a tree at a tile the game said held nothing but ore. `minable`
is `true` for a tree or a rock (`FactorioEntity::is_minable` is exactly "type is
`tree` or `simple-entity`") and `false` for **anything else with a collision box
the entity tree does not hold** — the binding does not know which and does not
say.

`a_box_reads_as_a_box_and_never_as_a_tree` asserts this against a real tree
entity: the flag must be `true` and the rendered report must not contain the
string `tree`. Naming it correctly is still forbidden, because the next box is a
cliff and the same code would name that a tree too.

## How "no boxes" differs from "never charted"

Four values, and `coverage` must be read before `boxes`:

| coverage | meaning | an empty `boxes` means |
|---|---|---|
| `charted` | every tile written out to the model | the ground is clear |
| `partial` | some tiles written out | nothing, over the rest |
| `unknown` | **no** tile ever written out | nobody looked |
| `outside_model` | the rectangle leaves the index's own ±5120 extent | the query came back short by construction |

The evidence behind `charted` is `tile_tree`, and it is a real per-tile record
rather than a proxy: the mod's `on_chunk_generated` calls `writeout_tiles` for
every tile of every chunk the engine generates, exactly once (`tile_chunks` in
`control.lua`), and `EntityGraph::add_tiles` files all of them, collidable or
not. So a tile in `tile_tree` is ground the model has been shown.

`outside_model` is kept separate from `unknown` deliberately — one is ground
nobody charted, the other is ground the index cannot hold — and reuses the check
`PlanState::enclosure_grid` and `enclosure::Escape::Unknown` already make.

The house rule is `EntityGraph::resource_fingerprint`'s and `runMatch.ts`':
equal means equal, different means unknown. Folding `unknown` into "no boxes" is
the exact mistake that produced the retracted tree hypothesis, and
`empty_over_charted_ground_is_not_empty_over_uncharted_ground` asserts the two
answers in one test so a coverage that collapsed them has to break a line.

`Option::None` never reaches Lua from here: every field is always present and
`boxes` is always a list, because mlua's null sentinel is light userdata and
therefore truthy, so `r.boxes or {}` does not substitute a default.

## What the diff says on a clean world

Seed 31337, t=0, `--headless --bots 1`, isolated instance (`headless-d.toml`),
release build. Three rectangles:

| rectangle | coverage | model boxes | game entities | verdict |
|---|---|---|---|---|
| `[-24,-22] .. [-8,-6]` | charted, 256/256 | 0 | 110, all `resource` | agree — ore is deliberately not in the tree |
| `[8,-16] .. [40,16]` | charted, 1024/1024 | 0 | 0 | agree, open ground |
| `[-24,-112] .. [0,-96]` | charted, 384/384 | **26** | 13 | see below |

The first two are the boring answers the instrument has to be able to give:
`charted` with nothing in it, distinguishable from `unknown`. The third is not.

## The finding: every tree is in the tree twice

Over the wooded rectangle the model holds **26 boxes for 13 game entities**, and
the 26 are the 13 rectangles each appearing exactly twice — 13 `BOTH`, 13
`DUPLICATE`, **0 `MODEL ONLY`, 0 `GAME ONLY`**.

```
BOTH       -18.211,-99.148,-17.414,-98.352  model: a box, minable, source unknown  game: tree-08
DUPLICATE  -18.211,-99.148,-17.414,-98.352  ... (already matched; the model holds this box 2 times)
summary: 13 both, 13 duplicate, 0 model only, 0 game only, 0 expected-absent, coverage charted
```

This is the same double-filing that `EntityGraph::resources` was made a
`Pos`-keyed map to fix — a chunk's entities reach `EntityGraph::add` from both
`on_chunk_generated` and the mod's `initial_discovery` replay of chunks that
already exist. `resources` was fixed; `blocked_tree` was not, and
`insert_with_box` on it has no dedupe (`entity_tree` is built with
`allow_duplicates = false`, `blocked_tree` with `true`).

**It is left, deliberately.** This task's scope is a read-only instrument, and
changing what goes *into* the tree is out of it. Three things for whoever picks
it up:

- **It does not explain (-16,-14).** A duplicate box sits exactly on top of a
  real one, so it cannot conjure an obstacle where the game has none. It is a
  bookkeeping fault, not a phantom — and the script labels it `DUPLICATE` rather
  than `MODEL ONLY` for precisely that reason. Reading these 13 rows as 13
  phantoms would be a third wrong story.
- **The count is not inert.** `EntityGraph::remove` clears blocked boxes by
  querying the removed entity's own bounding box, so a duplicate is probably
  cleared with its twin; that is a guess and is not measured here.
- **The binding does not deduplicate, on purpose.** Collapsing repeats is how
  this stayed invisible; a diagnostic that tidied it would have shown 13 boxes
  for 13 trees and reported perfect agreement.

## Two honest limits of the script

- `rcon.find_entities_in_radius` is the only game-side query the Lua bindings
  expose and it returns **entities**. Water is a tile, so a water box will read
  `MODEL ONLY` for a completely ordinary reason.
- `EntityGraph::add` keeps resources, rails and ghosts out of the blocked tree
  by design. The script counts those as *expected-absent* and summarises them
  rather than listing them — the first rectangle above is 110 ore tiles, which
  would otherwise bury every other row.

## Verification

- `nix develop -c cargo test --workspace`, redirected to a file with `$?`
  captured from the command itself: **exit 0**, 103 suites, no failures.
- `cargo clippy -p factorio-bot-scripting-lua --all-targets -- --deny warnings`:
  clean.
- **All nine new tests falsified**, one mutation at a time, each pattern
  asserted to occur exactly once in the source and each mutation required to
  fail exactly the one named test. One survived the first pass and the test was
  wrong, not the code: `a_box_touching_the_edge_is_not_inside_the_rectangle`
  used a box abutting the query's **right** edge, which the quad tree's
  half-open predicate already rejects, so deleting the strict re-test changed
  nothing. It now uses both edges — the asymmetry is the whole reason the
  re-test exists.
- **Three offline baselines byte-identical** on this branch's own release
  binary (`--no-default-features --features cli,lua`), `workspace/scripts/map.json`,
  bots 1,2,3,4:

  | goal | actions / ticks |
  |---|---|
  | `researched:automation` | 176 / 21,784 |
  | `producing:automation-science-pack:6` | 316 / 22,457 |
  | `producing:logistic-science-pack:6` | 441 / 47,478 |

  A read-only binding must move nothing, and it moved nothing.
- `app/` untouched, so `pnpm lint` was not required.
