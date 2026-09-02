# Water is solid, and the planner can find it — 2026-09-02

**Files:** `mods/BotBridge/control.lua`, `crates/core/src/graph/entity_graph.rs`,
`crates/core/src/types.rs`, `crates/core/tests/botbridge_writeout_tiles.rs`.
**Follows:** `2026-09-02-building-power.md` §8 items 1 and 2, which stopped here
on purpose rather than building stage 2b on sand.
**Status:** both defects fixed and committed separately (`fa8dabf3`,
`9ca7229a`). Stage 2b — the plant method itself — is now unblocked and is not
started.

A third, unrelated fix arrived mid-task and is recorded in §6:
`dba715da`, the crash-site cutscene.

---

## 1. The collision API, and what the TODO got wrong

The standing TODO in `writeout_tiles` said Factorio 2.0 had changed the
collision-layer API and that tiles should be assumed walkable until somebody
worked out the new one. The premise is false in an instructive way.

**`LuaTile::collides_with(layer: CollisionLayerID) -> boolean`**, straight out
of `workspace/factorio-api-docs/runtime-api.json` for 2.1.17:

```
METHOD collides_with [{"name":"layer","type":"CollisionLayerID","optional":false}]
   -> [{"type":"boolean","optional":false}]
   doc: What type of things can collide with this tile?
```

`CollisionLayerID` is "the name of a `LuaCollisionLayerPrototype`" — a plain
string. The method did not change. **What 2.0 changed was the layer's name**:
`player-layer` became `player`, and the old spelling does not degrade to
`false`, it raises (`Unknown collision-layer name: player-layer`).
`mods/BotBridge/types.lua` records having been bitten by exactly that on the
RCON path, where it took `find_tiles_filtered` down entirely, and it has called
`tile.collides_with('player')` correctly ever since.

So the two transports were asking two different questions about the same lake,
and the one that fills `EntityGraph` was the one answering `0`.

The fix is `tile.collides_with('player')`, memoised by tile name. Collision is a
property of the tile *prototype* and a tile's name names its prototype exactly,
so one engine call per distinct name is the same answer; `writeout_tiles` runs
over a whole 32×32 chunk and its own comment already calls it SLOW ("beastie can
do ~2.8 per tick"), so 1024 extra boundary crossings per chunk would land on the
function least able to afford them.

### Ground truth, not assumption

`crates/core/tests/live-2.1.17-tiles.json` came through the RCON path, i.e.
through `collides_with('player')`, and says what the game reports:

| tile | count | `player_collidable` |
| --- | --- | --- |
| `water` | 48 | **true** |
| `deepwater` | 14 | **true** |
| `grass-1` | 2 | false |

---

## 2. Was water ever solid in an archived run? **No. Not one tile.**

Asked of the archive rather than reasoned from the code, and the archive is
unambiguous. `workspace/*-log.txt` holds the raw stdout of the server and four
clients from run 30/31:

```
$ grep -h "tiles§" *-log.txt | grep -oE "[a-z0-9-]+:[0-9]+" | awk -F: '{print $2}' | sort | uniq -c
4440064 0
```

**4,440,064 tile records, every single one flagged `0`.** Of those, 79,717 are
`water` and 330,346 are `deepwater` — 410,063 water tiles, all walkable, plus 18
`out-of-map`, which is also collidable in a real game and also came out `0`.

The code path says the same thing and says why it is *every* run, not these two:
the only route into `EntityGraph`'s tile and blocked trees is
`writeout_tiles → output_parser.rs "tiles" → update_chunk_tiles → add_tiles`,
and `add_tiles` inserts into `blocked_tree` only when `player_collidable` is
true. `FactorioRcon::find_tiles_filtered` — the transport that got the flag
right — returns tiles to its caller and never touches the graph; its callers are
`crates/server`'s query endpoint and the dead
`find_offshore_pump_placement_options`. And `attach_world`
(`crates/core/src/factorio/snapshot.rs`) fetches no tiles at all, by design and
with the reason written down.

So: **no water tile has ever entered `blocked_tree` in the history of this
project.** `PlanState::is_area_clear` would have approved a boiler standing in a
lake, and nothing else in the planner could have told it otherwise.

Two consequences worth stating rather than discovering later:

* **Nothing regressed when this landed.** No plan has ever depended on water
  being walkable, because no method has ever sited anything near water on
  purpose; the archived runs mine and smelt inland.
* **The runs are now different runs.** Every future run has ~410,000 more
  blocking boxes in a fully-charted map than every archived one. `blocked_tree`
  is a quad tree with a 1024 item cap per node and 8 levels; a fully generated
  lake is dense. If placement queries slow down noticeably, this is the first
  place to look — it was not measurable in the test suite, which is not the same
  as not measurable in a run.

---

## 3. The query: shape, and why this shape

Three methods on `EntityGraph`, and **`blocking_boxes_within` is untouched**.
That was a deliberate constraint: `1b2b2149` narrowed only the keyframe query
and left `attach_world`, `is_area_empty` and the placement obstruction checks
wide, because narrowing them reintroduced a forest-siting bug. Nothing here
narrows anything that existed.

```rust
pub fn tiles_within(&self, bounds: &Rect) -> Vec<FactorioTile>
pub fn is_water_at(&self, position: &Position) -> bool
pub fn nearest_water_tile(&self, from: &Position, max_radius: f64) -> Option<FactorioTile>
```

plus, on the type itself:

```rust
impl FactorioTile {
    pub const WATER_NAMES: [&'static str; 2] = ["deepwater", "water"];
    pub fn is_water(&self) -> bool
}
```

### Why not a richer `blocking_boxes_within`

The obvious alternative is to give `blocked_tree` a payload that carries the
name, so the boxes arrive labelled. Rejected for two reasons, the second
decisive:

1. `BlockedQuadTree` is `QuadTree<bool, …>` and is part of `EntityGraph`'s
   hand-written `Serialize`/`Deserialize`. Changing the payload changes the wire
   shape of every serialised graph.
2. `blocking_boxes_within`'s only non-test caller is `crates/planner/src/state.rs`,
   **owned by another agent right now**. Changing its signature would put my
   change inside their file. A sibling query does not.

### Why `is_water_at` takes a point and not a box

Because that is how the caller already holds the thing it wants to classify.
`is_area_clear` iterates `blocking_boxes_within(area)` and keys each box by
`Pos::from(&blocked.center())` — the floored tile of the box's centre. A tile's
blocking box is exactly the 1×1 square of the tile it came from, so that same
centre identifies the tile, and `is_water_at(&blocked.center())` slots into the
loop that exists rather than asking for a second one.

`is_water` asks the **name**, not `player_collidable`. `out-of-map` is
collidable and so are cliffs; a pump may stand in water and in neither of those.
"Blocked" and "water" are different questions and one bit cannot carry both —
which is the whole defect being fixed.

### Why nearest-water is a query and not a shoreline predicate

The stage-2 note's item 3 asks for the shoreline rule from the dead
`find_offshore_pump_placement_options` to be ported. It is deliberately **not**
ported here. That rule ("keep a water tile whose neighbour ahead along the pump
direction is not water while both lateral neighbours are") takes a
`pump_direction` and is therefore about *placing a pump*, which is a planner
method's job and lives in `crates/planner` — currently another agent's file. What
it needs from `crates/core` is the ability to read named tiles in a region and
to find the water at all, and that is what landed. The predicate is four lines
on top of `tiles_within` and belongs beside the method that chooses the
direction.

`WATER_NAMES` carries both names because that dead function asks only for
`"water"`, and in the archive `deepwater` outnumbers `water` **four to one**. A
shoreline search that misses `deepwater` misses the edge of every lake deep
enough to have a middle.

### Determinism

`crates/planner` must produce byte-identical plans for identical inputs, so:

* **Order is never the quad tree's.** `QuadTree::query` returns items in item-id
  order, i.e. the order chunks arrived in, which differs between two runs of the
  same map. `tiles_within` sorts on `(x, y, name)` with `total_cmp` — the same
  comparator and the same key `PlanState::entities_within` already uses.
* **Ties break by position, not by iteration.** `nearest_water_tile` orders
  candidates by `(distance, x, y)` with `total_cmp` and takes the first, the same
  shape as `PlanState::nearest_supply_anchor`. Two lakes exactly equidistant
  resolve to the lower `x` on every run and from either insertion order —
  pinned by a test that builds the same two tiles into two graphs in opposite
  orders and asks twenty times.
* **No floating-point comparison operators.** `total_cmp` throughout; the only
  bare comparison is `distance <= max_radius`, which is a bound and not an
  ordering.
* **Distance is Euclidean**, via `calculate_distance`. `Position::distance` is
  Manhattan despite its name (`|dx| + |dy|`) and `find_entities_in_radius` uses
  it, so two things called "radius" in this file do not mean the same thing.
* **Distance is measured to the tile's centre**, `position + (0.5, 0.5)`,
  because `FactorioTile::position` is the tile's top-left corner — what the game
  reports and what `add_tiles` assumes when it builds the 1×1 box. The tile
  handed back keeps its corner, so the half tile appears in exactly one place.
  This is the same half tile that, got wrong for resources, made mining fail
  with "no entity to mine" for every ore on every map.

### The clip, and why it is not the narrowing that was forbidden

`tiles_within` re-checks its results with `overlaps_bounds`. The quad tree's own
predicate is half-open, so a box abutting the query's **left or top** edge comes
back while its mirror image on the right or bottom does not; a tile box is a full
1×1, so unclipped, "the tiles in this rect" includes the row immediately outside
it on two of four sides. That asymmetry was worth 691 spurious keyframe
divergences when it went unclipped there.

This is not the same act as narrowing the obstruction queries. A *negative*
question — "is anything in the way?" — is safe when it over-reports and
dangerous when it under-reports, which is why `is_area_empty` and the placement
checks are deliberately wide. This is a *positive* question — "where is the
water?" — where over-reporting is the unsafe direction: it would put a lake one
tile outside every rectangle anybody asks about.

### Two things this does not do

* **`attach_world` still fetches no tiles.** A snapshot-attached world therefore
  still has no water and `nearest_water_tile` returns `None` for it. That is
  unchanged and now *matters*, where before it did not: a plant method that runs
  against an attached world will refuse for want of water rather than site badly,
  which is the right failure but is a failure. Whoever lands stage 2b decides
  whether `attach_world` should start paying for tiles.
* **The mirror lists are untouched, correctly.** `keyframe_relevant` and
  `keyframe_relevant_types` in `crates/scripting_lua/src/globals/record.rs` mirror
  the entity types `EntityGraph::add` tracks. Nothing here adds an entity type —
  tiles go through `add_tiles`, `snapshot_within` reads `entity_tree` and
  `resource_tree` and not `tile_tree` — so there is nothing to mirror and adding
  anything would have been the divergence, not the fix.

---

## 4. Red first, with the failure output

### The mod

Four tests against the real `control.lua` in Lua 5.4, before the fix:

```
---- water_and_deepwater_are_written_out_as_solid ----
assertion `left == right` failed: the flag after each tile name is what
output_parser.rs turns into FactorioTile::player_collidable, and
EntityGraph::add_tiles inserts a blocking box only when it is true.
  left: "0,0;2,2: water:0,deepwater:0,grass-1:0,grass-1:0"
 right: "0,0;2,2: water:1,deepwater:1,grass-1:0,grass-1:0"

---- the_collision_layer_is_asked_for_by_its_2_0_name ----
assertion `left == right` failed: types.lua's serialize_tile already asks
`collides_with('player')`; the two transports have to agree
  left: []
 right: [("player", "water")]

---- the_mods_own_tiles_line_makes_water_block_and_leaves_grass_open ----
assertion `left == right` failed: a water tile has to reach blocked_tree, or
PlanState::is_area_clear approves a boiler standing in the lake. Got []
  left: 0
 right: 1
```

The stub's `collides_with` **raises** on any layer name that is not a real 2.1
collision layer, exactly as the engine does. A stub that returned `false` for a
wrong name would have let the pre-2.0 spelling pass as "nothing collides", which
is the failure the whole file exists to make impossible.

### The query

The first red here was a **compile failure** — `tiles_within`, `is_water_at` and
`nearest_water_tile` did not exist — which is the honest shape of "add a query
that is missing" and is weak evidence on its own. Every behavioural claim is
therefore pinned by a separate mutation below, and two tests were added
specifically because the first draft could not distinguish two implementations
(§5, mutations 6 and 7).

---

## 5. Mutations, one at a time, each reverted after

### `writeout_tiles`

| # | mutation | caught by |
| --- | --- | --- |
| 1 | flag hardcoded back to `:0` | `water_and_deepwater…`, `the_collision_layer…`, `the_collision_flag_is_asked_once…`, `the_mods_own_tiles_line…` — **not** `walkable_ground…` |
| 2 | flag hardcoded to `:1` | **all five**, including `walkable_ground…` |
| 3 | layer name `'player-layer'` | all five, each with `Unknown collision-layer name: player-layer` from the stub |
| 4 | memoisation removed | `the_collision_flag_is_asked_once_per_tile_prototype…` **only** |

1 and 2 separate cleanly, which is what says `walkable_ground_is_still_written_out_as_walkable`
is a real negative control and not decoration: it is the only test that fails
when the flag is stuck *on*, and it is the only one that passes when the flag is
stuck *off*. It cannot go red against today's code, and that is the point of it.

4 fails alone, which says the memoisation is pinned independently of the
correctness of what it memoises.

### The query

| # | mutation | caught by |
| --- | --- | --- |
| 5 | `WATER_NAMES` forgets `deepwater` | `the_water_search_knows_both_names_for_water` |
| 6 | `is_water_at` reads `player_collidable` instead of the name | `a_collidable_tile_that_is_not_water_is_not_water` |
| 7 | distance measured from the tile corner, not the centre | `distance_to_water_is_measured_to_the_tile_centre` |
| 8 | `nearest_water_tile` takes the first candidate, unsorted | `the_nearest_water_tile_is_the_nearest_one` |
| 9 | `tiles_within` returns quad-tree (arrival) order | `tiles_within_reports_the_terrain_by_name`, `tiles_within_comes_back_in_a_fixed_order` |
| 10 | **neither** query sorts | those two, plus `the_nearest_water_tile…`, `distance_to_water…`, and `two_lakes_equally_far_away_resolve_by_position_not_by_arrival` |
| 11 | `overlaps_bounds` clip dropped | `a_tile_merely_abutting_the_bounds_is_not_inside_them` |
| 12 | `max_radius` treated as a hint | `no_water_within_the_radius_is_no_water` |

Mutations 6 and 7 are the two that a first draft of the test set could not
catch, and both were found by trying to break the implementation rather than by
reading it:

* Nothing distinguished "is water" from "is collidable" until a test put a
  collidable **non-water** tile (`out-of-map`) on the map. Without it, mutation 6
  passes every test, and the doc comment's claim that the two are different
  questions is unpinned.
* Nothing distinguished corner from centre until a case was constructed where
  the two rank differently: from the origin, the corner of `(2, 2)` is 2.83 away
  against `(-3, 0)`'s 3.0, while the *centres* are 3.54 against 2.55. Every
  earlier arrangement happened to agree.

### What could not be made to go red, and what it pins instead

* **`two_lakes_equally_far_away…` does not catch mutation 8** (dropping only
  `nearest_water_tile`'s sort), and it took mutation 10 to see it. The reason is
  that `tiles_within` already sorts by `(x, y)`, so the candidate list arrives
  in the tie-breaking order and "take the first" gets the same answer by
  accident. The tie-break in `nearest_water_tile` is therefore **defence in
  depth, not the load-bearing sort** — it becomes load-bearing the moment
  anything changes `tiles_within`'s order, which is precisely when it is wanted.
  Mutation 10 is the one that proves it exists.
* **`a_player_in_a_cutscene_is_invisible_to_rcon_players`** (§6) cannot go red:
  nothing in that change proposes to alter the `player.connected and
  player.character` filter. It pins the *mechanism* — why 750 ticks of cutscene
  is a roster defect at all — rather than the fix.

### Verification

* `nix develop -c cargo test -p factorio-bot-core` — **388 lib + 143 integration,
  0 failed**, at the last moment the crate compiled (see below).
* `nix develop -c cargo clippy -p factorio-bot-core --all-features --all-targets
  -- --deny warnings` — clean.
* `nix develop -c rustfmt --edition 2024` on the four files I own. **`cargo fmt
  --all` was not run**, and `cargo fmt -p` is also not safe here: it rewrites a
  whole crate, and four other agents are editing `crates/core`. See §7.
* **No workspace-wide test or clippy, and no Factorio process launched**, under
  the standing hold. Both are named as residuals in §7.

---

## 6. The crash-site cutscene (`dba715da`)

Routed here mid-task; separate defect, separate commit, same file. Full
evidence is in `2026-09-02-bot-one-idle.md`; what this note adds is the choice
between the two candidate fixes, which the brief asked to be argued.

**Both, because they fix different things.**

`remote.call("freeplay", "set_disable_crashsite", true)` from `on_init`
*prevents* the cutscene. It also prevents the two lines above it in freeplay's
`on_player_created`:

```lua
util.remove_safe(player, storage.crashed_ship_items)   -- firearm-magazine 8
util.remove_safe(player, storage.crashed_debris_items) -- iron-plate 8
```

and that is the half worth more than the twelve seconds. A first player that has
had its iron plates taken sits at `(0, 0)` holding
`{burner-mining-drill, stone-furnace, wood}` — *exactly* what
`initiate_missing_players_with_default_inventory` seeds a phantom with. Leaving
the plates in place makes bot 1 look like bots 2–4 and ends the ambiguity
permanently. `exit_cutscene()` alone does **not** do this: the crash site is
already created and the plates already gone by the time it runs.

`player.exit_cutscene()` from `on_player_joined_game` *ends* a cutscene that is
already running, which is the case the other cannot reach: a save that already
existed when this landed, since `on_init` runs only for a save the mod is new
to.

Timing is why neither can be swapped for the other. `set_disable_crashsite` must
be set before the first `on_player_created`, and freeplay invites exactly this in
place: *"This is so that other mods and scripts have a chance to do remote calls
before we do things like charting the starting area, creating the crash site."*
`on_init` runs at map creation, tens of seconds before any client connects.
`exit_cutscene` must run after the cutscene starts, and `on_player_joined_game`
fires after `on_player_created`.

Two guards, both load-bearing rather than defensive:

* `remote.interfaces["freeplay"]` is checked first. `remote.call` on a missing
  interface **raises**, this crate builds `panic = "abort"`, and a non-freeplay
  scenario would take the whole process down at map creation. The outcome is
  printed either way, so a run says which happened instead of leaving it to be
  inferred.
* `player.controller_type == defines.controllers.cutscene` is checked before
  `exit_cutscene()`, because the 2.1.17 API says "**Errors if not in a
  cutscene**". Unguarded, it would raise on every join of every bot on every
  run. Freeplay's own `skip_crash_site_cutscene` guards it the same way.

Red first, against the real `control.lua`:

```
---- joining_the_game_ends_the_crash_site_cutscene ----
assertion `left == right` failed: the joining player is in a cutscene and has
to be taken out of it
  left: []
 right: [1]

---- the_crash_site_is_disabled_before_any_player_exists ----
assertion `left == right` failed: without this, freeplay takes player 1's eight
iron plates into the debris and leaves it at (0, 0) holding exactly what the
invented phantom bot holds
  left: []
 right: [("freeplay", "set_disable_crashsite", true)]
```

| # | mutation | caught by |
| --- | --- | --- |
| N1 | no `exit_cutscene_if_any` on join | `joining_the_game_ends_the_crash_site_cutscene` |
| N2 | `exit_cutscene()` called unguarded | `a_player_who_is_not_in_a_cutscene_is_not_asked_to_leave_one` |
| N3 | no `disable_crashsite()` in `on_init` | `the_crash_site_is_disabled_before_any_player_exists` |
| N4 | `remote.call` without checking the interface exists | `a_game_without_the_freeplay_interface_still_initialises` |
| N5 | `set_disable_crashsite(false)` | `the_crash_site_is_disabled_before_any_player_exists` |

Five mutations, five distinct tests, one each.

**Not confirmed in a live run.** The brief for the water work forbids launching
Factorio, so `FACTORIO_BOT_REFRESH_MODS=1` was not exercised. Two things are
therefore unverified against a real game and are the residual: that
`remote.interfaces["freeplay"]` is already populated when this mod's `on_init`
runs, and that `exit_cutscene()` from `on_player_joined_game` restores the
character before the first `rcon.players()` of the run. Both fail *visibly* if
wrong — the first prints "no freeplay interface", the second leaves the existing
symptom exactly as it is — and neither can make a run worse than it is today.

---

## 7. Residuals, and one thing I may have broken

**`cargo fmt -p factorio-bot-core` was run once, early, before I understood the
hazard.** It formats the whole crate, and `crates/core/src/factorio/rcon.rs` and
`crates/core/src/record/mod.rs` were dirty with another agent's in-flight work at
that moment; `rcon.rs`'s mtime moved within a second of my command. I cannot tell
after the fact whether rustfmt rewrote their file or whether they simply wrote to
it themselves at that instant. Everything after that point used
`rustfmt --edition 2024 <file>` on named files only. **Flagging it so it can be
checked rather than discovered.**

**Two failures in `crates/core` that are not mine**, both seen and both
transient, both in files listed as read-only for this work:

* `crates/core/src/record/lanes.rs:281` — `assert_eq!(lanes[0].id, None)`, "can't
  compare `u32` with `Option<_>`". Appeared, then compiled again.
* `crates/core/src/record/mod.rs:1136` and `:1218` — `bots: vec![1, 2, 3, 4]`
  where the field is now `Option<Vec<u32>>`. This is item 4 of the bot-1 note
  (`plan_created.bots` over-reports) landing in another agent's hands, with the
  tests not yet updated.

Both break `cargo test -p factorio-bot-core --lib`. The integration tests link
the library rather than its test build and were green throughout; the 388-lib
figure above is from a window in which their tree compiled. **A workspace-wide
`cargo test` was not run and would not have been informative while this is in
flight.**

**Still open, in order, for stage 2b:**

1. The shoreline predicate, ported from the dead
   `find_offshore_pump_placement_options` onto `tiles_within` — including
   `deepwater`, which that function misses. It belongs beside the method that
   picks the pump direction.
2. Whether `attach_world` should start fetching tiles. Its "nothing reads that
   tree" justification is now false.
3. Everything in `2026-09-02-building-power.md` §5: directions read from
   `fluidbox_prototypes` and validated in a live run, fuel as a standing
   obligation, and the four-wood cap on electric poles.
