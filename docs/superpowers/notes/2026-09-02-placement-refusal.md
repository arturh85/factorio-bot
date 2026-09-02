# Placement refusal: `can_place_entity said 'no'` on the first action of run 8b

Status: fixed. Commit `11c2655a` (on `master` — the checkout was no longer on
`feat/axum-server`).

## The failure

`workspace/runs/run-1788309767-54739`, milestone 4, first dispatched action:

```
{"kind":"action_dispatched","id":1,"bot":2,"action":"place stone-furnace at [-70, 42]"}
{"kind":"action_settled","id":1,"status":"failed",
 "error":"...cannot place item 'stone-furnace' because surface.can_place_entity said 'no'"}
{"kind":"milestone_stuck","index":4,"outcome":"stuck","best_steps":96}
```

11 actions dispatched, then stuck. The planner had already checked
`Condition::AreaFree` for that exact position and got `true`.

## Did `map.jsonl` answer it? No — twice over.

1. **It is empty (0 bytes) for this run.** `record.keyframe()` bounds the
   keyframe at the bounding box of everything *placed so far* and returns
   `false`, writing nothing, when nothing has been placed
   (`crates/scripting_lua/src/globals/record.rs`, `placed_bounds`). The run
   died on its first placement, so there was never a box to take a keyframe
   over. Six of the nine recorded runs have a zero-byte `map.jsonl`; only
   `run-1788307982-79011` has content (14 `placed` deltas, 1 keyframe).

2. **Even a populated keyframe could not have shown this.** `keyframe_relevant`
   (`record.rs:60`) deliberately filters `tree`, `rock-small`, `item-entity`
   and `character` out of the `game` side, so that `game` and `model` describe
   comparable populations — it mirrors `EntityGraph::add`'s entity-tree
   whitelist on purpose, and its own test
   (`keyframe_relevant_admits_only_what_the_entity_graph_models`) pins
   `!keyframe_relevant("tree", "tree-01")`. The obstacle class that causes this
   refusal is excluded by design from the divergence list.

So the divergence stream is structurally blind to terrain-versus-factory
divergence. It is built to answer "did the game keep the thing we built", not
"is the ground we plan on buildable". That is a reasonable thing for it to be,
but it means it cannot be the first place to look for a placement refusal, and
the note in the task that it "has never yet been used in anger" now has a
concrete reason attached.

## Root cause

`PlanState::is_area_clear` (`crates/planner/src/state.rs`) had three sources:
entities the plan placed, entities the base world had, and ore. The middle one
went through `EntityGraph::find_entities_in_radius`, which reads `entity_tree`.

`EntityGraph::add` (`crates/core/src/graph/entity_graph.rs`) inserts into
`entity_tree` only a **whitelist of factory entity types** — `Furnace`,
`Inserter`, `Boiler`, `Lab`, `OffshorePump`, `MiningDrill`, `StorageTank`,
`Container`, `Splitter`, `TransportBelt`, `UndergroundBelt`, `Pipe`,
`PipeToGround`, `LogisticContainer`, `AssemblingMachine`, plus `rock-big` and
`rock-huge` by name. Trees, cliffs, `rock-small`, units and tiles are not in
that list, so **a forest reads as open ground** to every placement check.

This is "nobody told the planner", not "the planner cannot know". The mod's
`writeout_entities` calls `surface.find_entities(area)` with no filter at all
and ships everything — the API audit counted **10,510 `"entity_type":"tree"`
records against 2,681 `"resource"`** in one run's writeouts — and every one of
those reaches `EntityGraph::add`. They land in `blocked_tree`, which `add`
fills with every non-resource, non-rail entity that has a collision box, and
which `add_tiles` also fills with every `player_collidable` tile (water). That
tree had exactly one reader: `draw.rs`, for rendering.

Chunk coverage is not a confound: `on_chunk_generated` emits `writeout_entities`
and `writeout_tiles` for the same chunk, so anywhere the planner knows there is
ore, it also received that chunk's trees and tiles.

### Where the position came from, and what checks it

`Smelt::expand` (`crates/planner/src/method/have.rs:345`) anchors on
`nearest_resource_tile` for the first ingredient and calls `free_area_near`
(`method/util.rs:238`), which searches integer-grid candidates in rings out to
`FREE_TILE_SEARCH_RADIUS = 12`, testing each with `PlanState::is_area_free`.
`[-70, 42]` is a few tiles off the copper patch the plan was smelting from.

**Nothing re-checks it before dispatch.** `run_action`
(`crates/executor/src/run.rs:495`) maps `ActionKind::Place` straight to
`act.place(...)`. The precondition was evaluated once, at expansion time,
against `PlanState`; the game's refusal was the only other check in the system.

## The fix

- `EntityGraph::blocking_boxes_within(&Rect) -> Vec<Rect>` — the blocked tree,
  exposed as world-space rectangles. Ordered by the quad-tree's own item ids,
  so deterministic. Edges snapped to Factorio's 1/256-tile position grid,
  because the quad-tree stores `f32` and a caller keying a recovered box by the
  tile its centre falls in would floor a centre a few ulps below an integer
  into the wrong tile — which is exactly how `removed` is matched below, the
  blocked tree carrying no name or position to match on.
- `PlanState::is_area_clear` gains it as a fourth source. A blocked box whose
  centre tile is in `removed` is skipped, mirroring the entity loop above it.

A **minable** obstacle still blocks: no method emits an action to mine a tree
out of the way, so treating one as clear would be the same wrong answer as not
seeing it.

The planner stays pure — this is a read of the same `FactorioWorld` snapshot it
already reads, no I/O, no clock, and the result is a bare boolean so query
order cannot leak in. `expansion_is_deterministic` passes.

## Tests

Five new, all of which fail with the fourth source removed:

- `state::a_furnace_does_not_fit_on_a_tree_or_in_water` — `fixture_world`
  already spawns 100 trees around `(-20,-20)` and a 4x4 `player_collidable`
  water patch around `(40,40)`; both read as open ground before, and open
  ground a few tiles away still takes a furnace so it cannot pass on an
  always-false check.
- `state::removing_a_tree_frees_the_ground_under_it` — pins the `removed`
  key, which is silent when wrong (the placement is simply never planned).
- `method::util::the_free_tile_search_walks_out_of_a_forest` — the end-to-end
  shape: start the search inside the forest, get back a site a furnace fits.
- `entity_graph::a_tree_is_a_blocking_box_even_though_the_entity_tree_never_sees_it`
  — asserts both halves: absent from `find_entities_in_radius`, present here.
- `entity_graph::a_player_collidable_tile_is_a_blocking_box_on_exact_tile_bounds`
  and `..._outside_the_bounds_is_not_reported`.

No existing expectation moved. The fixture's trees and water were already
there; no test had ever asked whether anything fits on them.

Gates: `cargo fmt --check`, `cargo clippy --workspace --all-features
--all-targets -- --deny warnings`, `cargo test --workspace` — all clean.

## Reported, not fixed

- **`map.jsonl` cannot report this class of divergence.** Fixing it properly
  means the *model* side would have to carry terrain too — `snapshot_within`
  reads `entity_tree` + `resource_tree`, so widening `keyframe_relevant` alone
  would make every tree a permanent spurious "only in game". A terrain lane
  (bounds + blocked boxes, compared against `find_tiles_filtered` and an
  unfiltered entity query) would be a separate line kind, not a widened filter.
- **No keyframe is written before the first placement.** A run that dies on
  action 1 leaves no map record at all. A keyframe bounded on the *bots'*
  positions rather than on placed entities would have caught this one.
- **No pre-dispatch placeability check.** The executor takes the planner's word.
  Given the fix, a re-check would be belt-and-braces — but it is also the only
  thing that would catch a world that changed between planning and dispatch
  (another bot, a biter walking in). If it is added it should *replan*, not
  retry: a retry loop is the "planner that relies on refusal" the task warns
  against.
- **Over-blocking is possible and cheap.** `blocked_tree` also holds units
  (19 in the audited run) and items on the ground, and neither is removed when
  it moves or is picked up, so a stale box can hold one tile. The cost is that
  `free_area_near` steps one ring further out; the cost of the alternative was
  a stuck run. If it ever matters, the fix is a type-aware blocked tree, not a
  relaxation of the check.
- **`FREE_TILE_SEARCH_RADIUS = 12` is now load-bearing.** With terrain visible,
  a site deep inside a forest can exhaust the search and fail expansion with
  `NoApplicableMethod`. That is a clean planning failure rather than a stuck
  run, and it is honest, but it is a new way for milestone 4 to end.
