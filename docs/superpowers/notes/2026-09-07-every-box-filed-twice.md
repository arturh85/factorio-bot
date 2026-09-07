# Every box filed twice

2026-09-07. Branch `every-box-filed-twice`, off `ecd4fa08`.

## The finding it came from

`docs/superpowers/notes/2026-09-07-ask-what-is-in-the-blocked-tree.md` built
`world.blocked_boxes` and `scripts/blocked_diff.lua`, and its first clean-world
run reported **26 model boxes for 13 game entities** — 13 `BOTH`, 13
`DUPLICATE`, 0 `MODEL ONLY`. It left the fix alone on purpose. This is the fix.

Reproduced here first, on the same rectangle and the same map (seed 31337,
t=0, `--headless --bots 1`, `headless-d.toml`, release build of `ecd4fa08`):

```
model coverage: charted (384 of 384 tiles written out to the model)
model boxes: 26
game entities overlapping the same rectangle: 13
summary: 13 both, 13 duplicate, 0 model only, 0 game only
```

## Where the two filings come from — verified, not inherited

The handover named `on_chunk_generated` and the `initial_discovery` replay. That
is right, and the server log says so directly rather than by reading Lua.

In `mods/BotBridge/control.lua`, `on_chunk_generated` calls `writeout_entities`
**unguarded** and `writeout_tiles` **guarded** by the `tile_chunks` table (one
per chunk, ever). `on_whoami` snapshots `surface.get_chunks()` into
`initial_discovery`, and `on_tick` then calls `on_chunk_generated` *by hand* for
each of them. A chunk generated after the output parser attached and before the
whoami snapshot therefore reaches `EntityGraph::add` twice.

Counted in `workspace/headless-d/server-log.txt` from the run above (`-l`):

| | count |
|---|---|
| `§entities§` writeouts | 596 |
| distinct chunk headers among them | 400 |
| `§tiles§` writeouts | 400 |
| writeouts of the wooded rectangle's chunk `-32,-128;0,-96` | **2** |

596 over 400 chunks is 196 chunks written twice and 204 once; tiles are exactly
one per chunk, which is the `tile_chunks` guard doing its job and the control
that makes the entity number readable.

## Was `allow_duplicates = true` load-bearing?

**Yes, and in the crudest possible way: with `false`, the second filing aborts
the process.**

`QuadNode::insert` does not skip an item no node accepted — it ends

```rust
if !did_insert {
    panic!("didn't insert {:?} into {:?}", item_aabb, self.bounding_box());
}
```

That is a plain `panic!`, not a `debug_assert!`, and `[profile.release]` sets
`panic = "abort"`. So flipping `blocked_tree` to `allow_duplicates = false` —
the obvious one-word fix, and how `entity_tree` and `tile_tree` are built —
would have killed the run on the first replayed chunk rather than deduplicating
anything. The two trees that carry the flag survive it only by upstream
convention: `add` checks `entity_at` and `continue`s before touching
`entity_tree`, and the mod's `tile_chunks` guard keeps a tile from being written
twice at all. Neither is a property of the tree.

This was found by writing the test, not by reading the code: a test that added
one water tile twice through `add_tiles` panicked inside `tile_tree`, which is
also why there is no `add_tiles`-level test of the water arm — the public entry
point cannot reach it.

So the fix is an explicit check in `EntityGraph`, `file_blocked_box`, on both
insert sites (`add`'s entity arm and `add_tiles`' water arm). Two further
reasons it is better than the flag: the flag compares rectangles with an epsilon
and ignores the payload; and it is **serialised with the tree**, so a graph
loaded from a `world.dump` would keep whatever flag it was written with, while
this check runs on every insert regardless of provenance.

## The key is the box plus its one bit, not the position

`EntityGraph::resources` fixed the identical double-filing from the identical
two paths by keying on the floored tile
(`docs/superpowers/notes/2026-09-02-resource-double-count.md`). **That key is
wrong here.** A blocked box is sub-tile and unaligned — a tree in this rectangle
sits at `(-18.211, -99.148)..(-17.414, -98.352)` — so two different trees can
stand on one tile, and collapsing them would erase ground that really is
blocked. A resource tile is a tile; a collision box is not.

The `is_minable` bit is part of the key too, because
`blocking_boxes_within_minable` hands it to callers deciding whether an obstacle
can be chopped out of the way. One rectangle claimed both minable and not is two
different answers to that question, and both are kept.

Comparison is **exact**, no epsilon: the two filings of one entity are
byte-identical `f32`, and widening the test would collapse *neighbouring* boxes,
which is a worse bug than the one being fixed.

## Which readers depend on counts

Surveyed exhaustively. In production code, **exactly one behaviour changes** if
a rectangle is filed twice, and it is reached through one helper:

* `enclosure::drop_walkable` (`crates/core/src/graph/enclosure.rs`), used by
  `enclosure::grid_for` and by `PlanState::walkable_obstacles_within`. It is a
  **multiset** subtraction, deliberately — it removes one occurrence per
  walkable entity so that a coincidence of geometry cannot delete a real
  blocker. `entity_tree` deduplicates and `blocked_tree` did not, so a
  double-filed belt offers two boxes against one walkable entity and **one
  survives, standing in the enclosure grid as a wall a character walks over**.
* `crates/scripting_lua/src/blocked.rs` returns the box list to Lua verbatim, so
  the duplicate was user-visible — that is how it was found.

Everything else is duplicate-proof: `occupant_of`, `resource_tile_blocked`,
`standing_verdict` and the walk-target lookup are `find`/`any`; `connect` and
`gather`'s route grids and `enclosure::rasterize` `fill(true)` a cell; three
call sites read only `blocked_tree().bounding_box()`. `EntityGraph::remove`
sweeps every box whose centre lies inside the removed entity's bounds, so it
took both twins with it — the note's guess about that was right.

## What it actually cost, measured

`what_the_blocked_tree_costs_at_world_record_scale` (ignored, gated on
`FACTORIO_BOT_WORLD_DUMP`) loads a dump written by the old code and re-files
every box through `file_blocked_box`:

| dump | boxes as written | after | duplicates |
|---|---:|---:|---:|
| `wr-census-status.json` (2.94 GB, world record base) | 180,012 | 180,012 | **0** |
| `map-31337-explored.json` | 65,642 | 60,804 | 4,838 |
| `map.json` (seed 31337, t=0) | 15,173 | 10,335 | 4,838 |

**Read the first row before the others.** At world-record scale the saving is
**zero bytes**, and the reason is the mechanism above: that base was reached by
loading a save, so no chunk was ever *generated* while the parser was attached
and `initial_discovery` filed everything exactly once. The duplicates come from
the startup window and nowhere else — which is why the count is **identical**
(4,838) on the fresh map and on the explored one, even though the explored map
holds four times as many boxes. Exploration adds chunks; it does not add
duplicates.

So the honest size of this is **≈207 KiB, bounded, not scaling** — 32% of the
boxes on a t=0 map, 7.4% after exploration, 0% on a loaded base.

**And the `drop_walkable` defect is latent, not demonstrated.** The duplicated
window is map generation at server start, which precedes every entity a run
builds, so the 4,838 duplicated boxes are trees, rocks, cliffs and water — none
of them walkable, so `drop_walkable` never had a walkable twin to under-cancel.
`a_belt_written_out_twice_does_not_read_as_a_wall` is therefore a guard against
a reachable-in-principle failure, not a reproduction of one that happened.
Saying otherwise would be inflating the finding, and the instrument's whole
value is that it does not.

What is unambiguously fixed is the instrument itself: the diff now reads the
model correctly.

## Verification

* **The acceptance test, end to end.** Same rectangle, same map, same flags,
  release build of this branch:

  ```
  model boxes: 13
  game entities overlapping the same rectangle: 13
  summary: 13 both, 0 duplicate, 0 model only, 0 game only, coverage charted
  ```

  `BOTH` unchanged at 13, `DUPLICATE` to zero. The server log of *that* run
  still shows the chunk written out twice (596 writeouts, 400 chunks), so the
  duplicate is being **deduplicated on the way in**, not eliminated upstream by
  accident.

* **Four offline baselines byte-identical**, re-measured on two release binaries
  built from the same tree with the same flags (`--no-default-features
  --features cli,lua`) — one from clean `ecd4fa08`, one with the change:

  | goal | before | after |
  |---|---|---|
  | `researched:automation` | 176 / 21,784 | 176 / 21,784 |
  | `producing:automation-science-pack:6` | 316 / 22,457 | 316 / 22,457 |
  | `producing:logistic-science-pack:6` | 441 / 47,478 | 441 / 47,478 |
  | `gathered:crude-oil` (`map-31337-explored.json`) | 2,115 / 317,283 | 2,115 / 317,283 |

  Nothing moved, which is the expected result given the reader survey: no
  planner reader counts boxes.

* `nix develop -c cargo test --workspace`, redirected to a file with the exit
  code taken from the command itself: **exit 0**, 103 suites, no failures.
  `cargo clippy -p factorio-bot-core --all-targets -- --deny warnings`: clean
  (it caught one `cloned_ref_to_slice_refs` in a new test).

* **All six new tests falsified**, one mutation at a time, each mutation
  asserted to match exactly once in the file it edited:

  | mutation | tests that must fail |
  |---|---|
  | drop the `if !already_filed` guard | the chunk-replay, the direct-helper and the belt test — and only those |
  | compare the rectangle but ignore the `minable` bit | the two-minable-bits test only |
  | compare the floored tile instead of the rectangle (the `resources` key) | the two-boxes-on-one-tile test only |
  | make `QuadNode::insert` not panic when nothing accepted the item | the `should_panic` test only |

  The third is the one worth keeping: it is a working implementation of the fix
  that `EntityGraph::resources` uses, and it breaks exactly one test — which is
  the whole argument for not reusing that key here.
