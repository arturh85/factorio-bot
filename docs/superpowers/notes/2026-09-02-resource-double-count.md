# The model counted every resource twice

## Status

Fixed at the point of insertion in `EntityGraph::add`. One commit, three tests.

## The measurement

Not taken on trust. `workspace/runs/run-1788319014-01846/map.jsonl`, keyframe 3
(tick 21074, bounds `(-66,-17)`--`(-1,65)`), counted by name:

| ore | game | model | ratio |
|---|---|---|---|
| iron-ore | 417 | 900 | 2.16 |
| copper-ore | 248 | 488 | 1.97 |
| stone | 152 | 316 | 2.08 |
| coal | 108 | 236 | 2.19 |

Those ratios are not the number. Counting *per position* is:

* game: 925 resource records, 925 unique positions, every one with
  multiplicity 1.
* model: 1940 records, 970 unique positions, **every one with multiplicity
  exactly 2**.

So the ratio is exactly **2.000 per tile**, and the apparent 2.16 is the ore
the model holds outside the keyframe rectangle (the quad-tree query admits
boxes that merely touch the bounds) plus the ore mined during the run, which
the game no longer reports. The same per-position histogram holds in
`run-1788307982-79011` (751 unique positions, all at multiplicity 2) and
`run-1788317597-64759`. Every keyframe in every run that carries resources
shows it, so this is structural, not a race.

## Where the doubling happens

`EntityGraph` has exactly **one** insertion site for resources
(`crates/core/src/graph/entity_graph.rs`, `add`), and it wrote both stores --
the `resources: DashMap<String, Vec<Pos>>` and the `resource_tree` quad-tree --
unconditionally, once per delivered entity. `QuadTree::query` dedupes by
`ItemId`, so two entries in a query result mean two distinct `insert_with_box`
calls: the tile was genuinely stored twice, not reported twice.

The duplicate delivery is on the transport. `writeout_entities` in
`mods/BotBridge/control.lua` is called from `on_chunk_generated` (line 1093),
and `on_chunk_generated` has **two** callers: the real Factorio event
(registered line 2326) and the `initial_discovery` replay in `on_tick` (line
700), which synthesises the event for every chunk `get_chunks()` reports at
`whoami` time. The *tiles* writeout one line below is guarded against emitting
a chunk twice (`tile_chunks[chunk_id]`, line 1095); the entities writeout is
not, and never was -- the guard is in the initial import commit.

**Do not add the same guard to the entities writeout.** The Aug-31 server log
(`workspace/server-log.txt`, the last run recorded with `-l`) shows why: of its
419 chunks, 54 were emitted twice, and in every one of those the *first*
emission -- from `initial_discovery` -- was `{}`, with the real event
delivering the contents thousands of ticks later. A per-chunk guard would have
dropped the payload and kept the empty answer. The mod is right to re-send; the
graph was wrong to re-store. That is why the fix is at the insertion point.

(Aside, harmless: `control.lua` registers `on_built_entity` twice, lines 2330
and 2333. Factorio replaces a handler rather than adding one, so nothing
doubles.)

## What compensated, and what did not

`EntityGraph::resource_patches` keys its flood fill into a
`HashMap<Pos, Option<u32>>`, which collapses the copies. Every one of the four
selectors the task names -- `resource_tiles_for`, `nearest_resource_tile`,
`resource_supply_at_least`, `resource_seats` -- reads resources only through
`PlanState::resource_patches` -> `EntityGraph::resource_patches`. So **the
planner's sizing was never inflated**: `DEFAULT_RESOURCE_PER_TILE` was
multiplied by the correct tile count, and `Mine::applicable` gated on the
correct supply. `resource_contains` and `any_resource_at` answer booleans and
are likewise indifferent.

It was live in three places:

1. `snapshot_within`, hence the `model` half of every keyframe -- the visible
   symptom, and a false picture for anyone reading a run back.
2. **`remove`.** It deletes every overlapping item from `resource_tree` but
   removed a *single* `Pos` from the `resources` vector. With two copies
   stored, mining a tile out left one behind, so `resource_patches` kept
   offering the planner a tile the game had no ore on. This is the one that
   could actually cost a run, and it is the failure the divergence list could
   not see either, because the leftover copy still matches nothing in `game`
   only by absence.
3. Memory: two vector entries and two quad-tree nodes per ore tile, map-wide.

## The fix

`resources` is now `DashMap<String, BTreeSet<Pos>>`, and `add` skips the
quad-tree insert when the set already held the tile, so both stores stay in
step. `remove` drops the single entry instead of scanning for the first copy.
The wire shape is unchanged (a `BTreeSet` serialises as the same JSON array a
`Vec` did), and `resource_contains` / `any_resource_at` get O(log n) instead of
O(n) for free.

No existing test moved. That is itself the confirmation that the planner's
half was compensated: had any test encoded the doubled count, it would have
been in the patch-sizing tests, and those never saw a duplicate.

## Recommendation: teach the divergence list to see this class

`divergence_between` (`crates/core/src/record/map.rs:164`) compares with
`Vec::contains` in both directions -- pure set membership. A doubled
multiplicity of the same tile is, to it, perfect agreement. Two options,
smallest first:

1. **Multiset difference instead of set difference.** Count each side into a
   `BTreeMap<EntitySnapshot, usize>` and emit `max(0, model - game)` records
   with `only_in: "model"` (and symmetrically). No schema change, no new
   field, ~15 lines -- and it would have printed 485 extra `iron-ore`
   divergences on the run above. Downside: it prints them one per copy, which
   drowns the list exactly when it fires.
2. **A per-keyframe count summary.** Alongside `divergence`, a small
   `counts: [{name, game, model}]` block, populated only where the two
   disagree. One line per ore instead of 485, and it reads as the terrain-lane
   summary that `2026-09-02-keyframe-at-start.md` sketched. Costs a schema
   field, so it moves the openapi snapshot and the TypeScript contract.

I recommend (2), and note that neither is worth building blind: the same
keyframe also shows the model holding 23 fewer in-bounds tiles than the game,
which nobody has explained yet, and a count lane would make that visible
too.

## Test summary

Three tests in `crates/core/src/graph/entity_graph.rs`, all on tile *centres*
so the flooring `Pos` round trip cannot hide the difference:
`a_resource_delivered_twice_occupies_one_tile` (two separate `add` calls, the
shape the transport actually delivers -> one modelled tile, one patch element),
`two_adjacent_resource_tiles_both_survive` (the negative control: neighbours
whose boxes share a query rectangle must both live), and
`removing_a_resource_delivered_twice_empties_the_tile` (the live consequence:
a mined tile leaves nothing behind).

Non-vacuity checked by restoring `HEAD`'s `entity_graph.rs` under the new
tests: the first fails `2 != 1`, the third fails on `resource_contains`, and
the negative control passes on both the old and the new code.
