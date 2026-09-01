# BotBridge / RCON audit against the Factorio 2.1.17 runtime API

Date: 2026-09-02
Scope: `mods/BotBridge/control.lua`, `mods/BotBridge/types.lua` (unavoidably —
`control.lua` delegates every serialiser to it), `crates/core/src/factorio/rcon.rs`,
`crates/executor/src/rcon_actuator.rs`.
Authority: `workspace/factorio-api-docs/runtime-api.json`
(`application_version: 2.1.17`, `api_version: 6`, 157 classes) and the rendered
HTML beside it.
Nothing was changed. This is a findings document only.

**Line numbers are against commit `75a098a8`.** `control.lua` grew from 2804 to
2903 lines *during* this audit — the `early eof` agent landed
`fix(mod): …`/`75a098a8` while I was reading, adding `pcall` guards and
`record_sample_failure` around the samplers. Every line number below was
re-derived against the current file after that landed; nothing here is cited
from the pre-change offsets. That change fixed a `LuaEntity.mining_target` read
on a character in `sample_bots`, which is their finding, not one of mine.

## How to read the evidence markers

Every claim below carries one of:

- **[V]** — verified against `runtime-api.json`. The API entry is named.
- **[L]** — verified against an artefact already in this checkout
  (`workspace/server-log.txt`, 8.4 MB, from the 2026-08-31 run).
- **[I]** — inferred from code reading plus documented API semantics, but not
  proven by either of the above. Treat as a hypothesis to test, not a fact.

Two things the docs simply do not state, and which I therefore did **not**
assert anywhere: whether an omitted `quality` in an `ItemFilter` means "any
quality" (the docs give a default only for `ItemStackDefinition`), and the exact
byte size at which Factorio's RCON server splits a reply across packets.

One thing I set out to report and then disproved, recorded here so nobody
re-derives it: `rcon_async_request_path` uses `game.forces[1]` where every other
call site uses `game.forces["player"]`, which looks like it pathfinds for the
wrong force. It does not. `workspace/server-log.txt` carries
`§0§force§{"name":"player","force_id":1,…}` alongside `"enemy",force_id:2` and
`"neutral",force_id:3` **[L]**, so index 1 *is* the player force on this map.
That call has a different, real problem — see A4.

---

## Part 1 — The teleport question

### 1.1 What actually happens

`player.walking_state` at `control.lua:685` is real walking, and it is the normal
path. The mod is not a teleporter. But there are three `player.teleport` sites,
and the first one is not the rare safety valve its log message implies.

**The stuck-teleport: `control.lua:674-682`**

```lua
674  if w.idx_tick ~= nil and event.tick - w.idx_tick > 60 then
675      if w.idx > #w.waypoints - 1 then -- if last waypoint just abort
676          print("Player is stuck while moving to last waypoint, just stop moving")
677          w.waypoints[w.idx] = nil
678      else
679          print("Player is stuck while moving, teleporting to next waypoint")
680          player.teleport(w.waypoints[w.idx])
681      end
682  end
```

`w.idx_tick` is set in exactly one place: `control.lua:630`, when a waypoint is
*reached*. It is not set when the walk starts — `control.lua:2139` builds
`{idx=1, waypoints=tmp, action_id=action_id}` with no `idx_tick` — and it is not
reset by the teleport.

So the trigger condition is **not** "the bot has not moved". It is "more than 60
ticks have elapsed since the bot reached the *previous* waypoint". A bot walking
perfectly, at full speed, in a straight line, along a leg longer than one second
of travel satisfies it. Character running speed is 0.15 tiles/tick
(`crates/planner/src/schedule.rs:13`, `WALK_TILES_PER_TICK`), so **any path leg
longer than about 9.2 tiles is finished by teleport rather than by walking**, and
that is the common case, not the exceptional one: the mod requests
`prefer_straight_paths = true` (`control.lua:2788`), which is precisely the flag
that produces long collinear legs.

The one exemption is the first leg, because `idx_tick` is nil until a waypoint
has been reached. A single-waypoint path can therefore never teleport. Every
subsequent leg can.

### 1.2 How far one jump can move a player

`player.teleport(w.waypoints[w.idx])` moves the character to the *current
destination waypoint*, in one tick. The jump length is therefore

    leg_length − (61 ticks × 0.15 tiles/tick) ≈ leg_length − 9.2 tiles

and it is **unbounded above**, because nothing bounds the distance between two
pathfinder waypoints. A 200-tile straight leg is closed by a ~191-tile teleport.
`LuaControl.teleport` takes `build_check_type`, defaulting to `script` **[V]**
(`LuaControl::teleport`), and the mod passes nothing, so the destination is not
validated the way a manual move would be.

It self-limits to one teleport per waypoint: the next tick finds `|dx|,|dy| < 0.3`
and advances `idx`, stamping a fresh `idx_tick`. So a 10-waypoint path teleports
at most 9 times — but it *will* teleport on every leg over ~9 tiles.

### 1.3 The bug next door, which is worse than the teleport

The `if` branch at `control.lua:675-677` — "stuck on the *last* waypoint" — does
`w.waypoints[w.idx] = nil`, shortening the array. On the next tick,
`control.lua:620` reads `dest = w.waypoints[w.idx]` as nil and falls into:

```lua
622  if dest == nil then
623      action_completed(event.tick, w.action_id)
624  else
```

That branch does **not** clear `storage.p[idx].walking` and does **not** set
`player.walking_state = {walking=false}`. Compare the normal completion at
`control.lua:632-634`, which does both. Consequences, all permanent for the rest
of the session:

1. `action_completed ok <id>` is written to stdout **every tick, forever**.
2. The character keeps the last `walking_state` it was given at
   `control.lua:685` and **walks off in a straight line indefinitely**, because
   nothing ever stops it.
3. The walk is reported to Rust as *successful* at the moment it timed out, at a
   position that is not the goal.

Point 3 is only partially caught: `move_player_timed` re-checks the path endpoint
before dispatch (`crates/core/src/factorio/rcon.rs:1161`) and `player_mine_timed`
re-measures where the bot landed (`rcon.rs:1258`). Neither helps, because both
check the *plan*, not the *outcome*; the bot is reported arrived and then keeps
walking away from where it was reported.

This branch is reachable **only** through the stuck-abort at 675-677 — the normal
last-waypoint case exits through 631-634 — so it has been latent exactly as long
as the stuck path has.

### 1.4 The other two teleports

`control.lua:2556-2557` (in `rcon_revive_ghost`) and `control.lua:2699-2700` (in
`rcon_place_blueprint`) both do:

```lua
player.teleport({x = bb.right_bottom.x + 1, y = bb.right_bottom.y + 1})
```

where `bb` is the ghost's collision box, floor/ceil-expanded to half-tiles and
translated to the ghost's position (`add_to_bounding_box` /
`expand_rect_floor_ceil`, `control.lua:2208-2227`; the `bb` is built at
`control.lua:2553` and `control.lua:2696`).

**Can they change what an action costs? Yes, three ways.**

1. **Distance.** The displacement is half the entity's footprint plus one tile,
   diagonally, away from the ghost centre. For a stone furnace (2×2) that is
   ~2.8 tiles; for a rocket silo (9×9) ~7.8 tiles. Every subsequent
   `travel_ticks` (`crates/planner/src/schedule.rs:16`) is computed from the
   position the *world model* holds, which is updated by
   `on_player_changed_position` — so the model does follow the jump, and the
   *next* walk is planned honestly. What is wrong is the *record*: the bot
   covered that distance for free.

2. **Repetition.** The 2699 site is **inside the per-ghost loop** of
   `rcon_place_blueprint` (`control.lua:2657` onward). A blueprint whose
   footprint covers the bot produces one teleport per overlapping ghost, each to
   a different corner. There is no accumulation guard.

3. **Reach.** Neither site checks that the destination is reachable, empty, or
   inside the bot's build range. `ghost.revive()` ignores reach entirely **[V]**
   (`LuaEntity::revive` takes only `raise_revive` and `overflow`), so the revive
   still succeeds — but a *later* action planned against `player.build_distance`
   starts from wherever the teleport dumped the bot.

There is an API-native replacement for exactly this shape:
`LuaSurface.create_entity` takes `move_stuck_players` — "If true, any characters
that are in the way of the entity are teleported out of the way" **[V]**. It is
still a teleport, so it does not solve observability, but it removes the
hand-rolled bounding-box arithmetic at `control.lua:2187`, `2553` and `2696`,
which is the part most likely to be subtly wrong. It applies to `create_entity`,
**not** to `ghost.revive()`, so it covers `rcon_place_entity` and not the two
sites in question.

### 1.5 Is a teleport observable to the Rust side?

**No.** Verified by exhaustion:

- `grep -rn teleport crates app/src scripts` returns four hits, none about this:
  a Lua test using `"teleportation"` as a fake technology name, and a `types.rs`
  fixture for a `"teleport-a-biter"` trigger.
- The three sites use bare `print(...)`, not `writeout(...)`.
  `crates/core/src/process/output_reader.rs:72` only parses lines beginning with
  `§`, and `writeout` (`control.lua:1791`) is the only thing that emits that
  framing. So the line does land in `workspace/server-log.txt` verbatim — it is
  not invisible — but it carries **no tick, no player id, no source position, no
  destination and no distance**, and nothing downstream reads it.
- `crates/core/src/process/output_parser.rs` has no `teleport` arm, and
  `crates/core/src/record/mod.rs`'s `EventKind` has no teleport variant.
- The stdout `on_player_changed_position` event *does* fire on a teleport **[V]**
  (documented "Called when the tile position a player is located at changes"), so
  the world model's position is correct after the jump. It arrives as an ordinary
  position update, indistinguishable from one produced by walking.

So the situation is exactly as the brief framed it: a run whose bots teleported
repeatedly reports ordinary walking durations, and the only trace is an
untimestamped English sentence in a log nothing parses.

### 1.6 The smallest honest fix

In increasing order of cost. Tier 1 alone converts "no trace" into "a trace a
person can find"; Tier 2 is what makes a run record self-describing.

**Tier 1 — make the mod say it, in the framing that already exists (3 edits, one
file).** Replace the `print` at `control.lua:679` (and the two ghost sites at
2556 and 2699) with:

```lua
writeout(event.tick, "teleport", helpers.table_to_json({
    player_id = idx, reason = "walk_stuck",
    from = pos, to = w.waypoints[w.idx],
    distance = distance(pos, w.waypoints[w.idx]),
    action_id = w.action_id,
}))
```

This costs nothing at runtime, and the line is already tick-stamped and
machine-readable by construction.

**What this breaks if done alone:** `output_parser.rs:398` has a `_ =>` arm that
logs `error!("<red>unexpected action</>: …")`. An unrecognised key therefore
produces a red error line *per teleport*. The `early eof` agent hit exactly this
today and handled it correctly — `output_parser.rs:387-397` now carries a
`"sample_error"` arm with a comment explaining that a dropped failure "would just
trade one invisible bug for another". Follow that precedent: Tier 1 must ship
with the parser arm below, or it converts a silent problem into a noisy one.

**Tier 2 — record it (3 more edits, 3 files).**
1. `crates/core/src/process/output_parser.rs`: a `"teleport" => { … }` arm.
2. `crates/core/src/record/mod.rs`: an `EventKind::Teleport { bot, reason, from,
   to, distance, action_id }`. `EventKind` already has `#[serde(other)] Unknown`,
   so an older viewer skips the new kind rather than refusing the file — the
   forward-compat cost is zero.
3. The `ActionSettled` for a walk that contained a teleport should be flagged, or
   the per-action duration still reads as honest. That is the part that actually
   protects run-to-run comparison, and it is why the `action_id` belongs in the
   payload above.

**What Tier 2 breaks:** nothing in the wire contract — `EventKind` is
`#[serde(tag = "kind")]` with an `Unknown` fallback, and `SAMPLE_SCHEMA` is not
touched. But the OpenAPI snapshot seam *will* fail
(`app/src/api/openapi.snapshot.json`, then `openapi.contract.spec.ts`), by
design, and both halves must be regenerated. That is the seam working.

**Explicitly not recommended: inferring teleports from `samples.jsonl`.** The
data is there — `BotSample.position` at 60-tick intervals
(`crates/core/src/record/samples.rs:79`) — and a >9.2-tile jump in one second is
detectable in principle. But: samples only exist when a capture run is active
(`sample_bots` returns early at `control.lua:1439` when `storage.frame_capture`
is nil); a 1 Hz sample cannot distinguish one 60-tile teleport from six seconds
of ordinary running sampled badly; and it would be a *reconstruction* where the
mod has the fact firsthand. Detecting is not reporting, and the mod knowing
something and not saying it is the actual defect.

**One more thing the fix should carry.** The teleport is not merely
under-reported, it is *mis-triggered* (§1.1). Making it observable without fixing
the 60-tick condition means honestly recording that nearly every long leg
teleports. Both belong in the same change, or the first run afterwards will look
like a catastrophic regression when it is only the first honest measurement.

---

## Part 2 — Findings, ranked

### Rank A — causes wrong behaviour

#### A1. The stuck-teleport fires on ordinary long walks, and its sibling branch leaves a bot walking forever

`control.lua:619-686`. Detailed in §1.1–§1.3. Two defects in one block: the
trigger measures the wrong interval (`control.lua:674` against an `idx_tick` set
only at `control.lua:630`), and the `dest == nil` path at `control.lua:622-623`
omits both `storage.p[idx].walking = nil` and `walking_state = {walking=false}`
that its sibling at `control.lua:632-634` performs.

*Minimal correct trigger:* stamp `idx_tick` at `control.lua:2139` too (so leg 1
is covered), and compare against *progress*, not elapsed time — the mod already
reads `player.character.position` every tick at `control.lua:619`. The game also
publishes `LuaControl.character_running_speed`, "the current movement speed of
this character, including effects from exoskeletons, tiles, stickers and
shooting" **[V]**, which is the honest per-tick expectation to compare against
and removes the need for a hardcoded 0.15 anywhere in the mod.

*What could break:* raising the threshold makes genuinely stuck bots hang longer
before anything intervenes, up to `ACTION_RESULT_DEADLINE` (360 s,
`crates/core/src/factorio/rcon.rs:37`). A progress-based check does not have that
problem, which is why it is the better shape.

#### A2. `build_blueprint`'s `force_build` parameter does not exist in 2.x — the flag has been dead since 2.0

`control.lua:2650` and `control.lua:2755` both pass `force_build = force_build`
into `bp_entity.stack.build_blueprint({…})` (calls at `control.lua:2641` and
`control.lua:2746`).

In 2.1.17, `build_blueprint` lives on `LuaItemCommon` (inherited by
`LuaItemStack`, not declared on it) and its parameters are `surface, force,
position, direction, build_mode, skip_fog_of_war, by_player, raise_built` **[V]**
(`LuaItemCommon::build_blueprint`). There is no `force_build`. The replacement is
`build_mode: defines.build_mode`, values `normal | forced | superforced` **[V]**
(`defines.build_mode`), documented: "If `normal`, blueprint will not be built if
any one thing can't be built. If `forced`, anything that can be built is built
and obstructing nature entities will be deconstructed."

So the caller-facing `force_build` flag is silently ignored, `build_mode`
defaults to `normal`, and **the semantics are the opposite of what the caller
asked for**: `force_build = true` currently gets all-or-nothing behaviour. The
comment at `control.lua:2649` still quotes the 1.1 documentation verbatim, which
is why nobody noticed.

*Fix:* `build_mode = force_build and defines.build_mode.forced or
defines.build_mode.normal`.

*What could break:* `forced` also deconstructs obstructing nature entities, which
1.1's `force_build = true` did not. A blueprint placed over trees will now clear
them. Given this project's interest in honest material accounting, that is a
behaviour change to decide on deliberately rather than inherit. Also: `by_player`
already causes `on_built_entity` to fire **[V]**, and the mod passes
`by_player = player`, so its own `on_some_entity_created` handler already sees
these — do **not** add `raise_built` as well or every blueprint entity is
reported twice.

#### A3. `rcon_place_entity` removes the item before creating the entity, and checks neither result

`control.lua:2196-2201`:

```lua
2196  player.remove_item({name=item_name,count=1})
2197  result = surface.create_entity{name=entproto.name, …}
2198
2199  if result == nil then
2200      complain("placing item '"..item_name.."' failed, …")
```

`LuaControl.remove_item` returns the number of items removed **[V]**; the return
is discarded. `create_entity` may return nil; when it does, the item has already
been taken and nothing gives it back. So a failed placement **destroys the
material**, and a placement where `remove_item` took nothing **builds for free**.

This is precisely the bug already found and fixed one function away:
`charge_item_to` (`control.lua:2616-2622`) exists because "`remove` finds nothing
to take, returns 0, and that discarded return value is what kept it silent". The
same reasoning was never applied here.

*Fix:* create first, then charge, and destroy the entity if the charge fails —
the exact pattern `rcon_place_blueprint` already uses at `control.lua:2680-2690`.

*What could break:* the ordering change means `on_some_entity_created` fires for
an entity that may then be destroyed. `on_some_entity_deleted` would need to fire
too, or the Rust `EntityGraph` keeps a phantom.

#### A4. `rcon_async_request_path` omits two *required* parameters

`control.lua:2795-2809` calls `game.surfaces[1].request_path{start, goal, force,
radius, pathfind_flags}`. `LuaSurface.request_path` is declared `takes_table:
true, table_optional: false`, and `bounding_box` (order 0) and `collision_mask`
(order 1) both have `optional: false` **[V]**. The sibling
`rcon_async_request_player_path` at `control.lua:2774-2793` passes both correctly.

So this call raises inside the remote call. Its only Rust consumer,
`FactorioRcon::path` (`rcon.rs:1954`), has no callers in the tree — `plan_path`
(`rcon.rs:2054`) does its own thing with `find_entities_filtered` — so this is
latent rather than actively breaking anything. It is ranked here because it is a
`pub` method that cannot work, and the failure mode when someone does call it is
a 60-second timeout (`rcon.rs:1093`) rather than an error naming the cause.

*Fix:* copy the two arguments from the player variant, or delete
`rcon_async_request_path` and `FactorioRcon::path` together.

#### A5. Mined quantities are recorded as *expected* values, not actual ones

`control.lua:804`, inside the `on_player_mined_entity` handler:

```lua
804  local mining_results = products_to_dict(proto.mineable_properties.products)
```

`products_to_dict` (`control.lua:197-209`) takes `product.amount`, or
`product.amount_min` when `amount` is absent, and **ignores probability
entirely**. The file's own `simplify_amount` (`control.lua:536-552`) does this
correctly, and `control.lua:196` literally reads `-- TODO: use simplify_amount`.
So a randomised drop is recorded at its floor and a probabilistic one at full
weight.

The API hands over the exact answer: `on_player_mined_entity` carries
`buffer: LuaInventory` **[V]**, documented "Called after the results of an entity
being mined are collected just before the entity is destroyed. After this event
any items in the buffer will be transferred into the player." The mod already has
`inventory_counts` (`control.lua:1973-1979`) to read it in the 2.0 shape.

The same `products_to_dict` also feeds `serialize_entity_prototype`'s
`mine_result` (`types.lua:330`) and `on_player_crafted_item`
(`control.lua:1929`). For a *prototype* an expected value is defensible; for an
*observed mine* it is not, and those are the same function today.

*What could break:* `recent_item_additions` consumers expect a name→count dict;
`inventory_counts(event.buffer)` returns exactly that, so the change is local.
`buffer` is valid only during the event and cannot be stashed.

#### A6. Path waypoints that require destroying something are walked as if the way were clear

`control.lua:1995-2006`, `on_script_path_request_finished`:

```lua
1999  for k,v in pairs(event.path) do
2000      table.insert(positions, v.position)
```

`PathfinderWaypoint` is `{needs_destroy_to_reach: boolean, position:
MapPosition}` **[V]**, where `needs_destroy_to_reach` is "`true` if the path from
the previous waypoint to this one goes through an entity that must be destroyed".
The mod drops that field and sends only positions, so the Rust side never learns
a leg is blocked.

This is the direct upstream cause of A1's firing: the bot walks into the
obstruction, fails to progress, the 60-tick timer expires, and it **teleports
through the thing the pathfinder said had to be destroyed**. The two findings are
one story.

Compounding it: the mod does not set `cache` in `pathfind_flags`
(`control.lua:2786-2791`), and it defaults to `true` — "This can be more
efficient, but might fail to respond to changes in the environment" **[V]**
(`PathfinderFlags`). A bot that has just built a furnace can be handed a cached
path straight through it. **[I]** that this is a meaningful contributor to
observed stuck-walks; the mechanism is documented, the frequency is not measured.

*Fix:* forward `needs_destroy_to_reach` and refuse (or clear) rather than walk;
set `cache = false` for player paths, where correctness matters more than
pathfinder throughput.

---

### Rank B — wastes time or bytes

#### B1. Entity payloads are ~79% trees, and the one code path that can be truncated is the one with no truncation check

Measured from `workspace/server-log.txt` **[L]**: across all `§…§entities§`
writeouts, **10,510 `"entity_type":"tree"` records against 2,681 `"resource"`**,
plus 281 `simple-entity`, 19 `fish`, 13 `unit`. The largest single chunk payload
is **136,864 bytes** for one 32×32 chunk; the total across 473 chunks is
2,809,653 bytes.

`writeout_entities` (`control.lua:1751-1760`) uses `surface.find_entities(area)`
at `control.lua:1755` — "Find entities in a given area. If no area is given all
entities on the surface are returned" **[V]** — with no filter at all.

That path is stdout and merely wasteful. The same shape over RCON is not:

- `crates/core/src/factorio/snapshot.rs:154-157` (`attach_world`) calls
  `find_entities_filtered` over a box of `DEFAULT_ATTACH_RADIUS = 200`
  (`snapshot.rs:53`), i.e. 400×400 tiles = 156 chunks, unfiltered.
- `crates/scripting_lua/src/globals/record.rs:710` fetches every entity in the
  placed-bounds box at each keyframe and then **discards everything not
  `keyframe_relevant`** (`record.rs:60`) — i.e. filters in Rust, after the whole
  payload has crossed the wire.

Critically, `FactorioRcon::find_entities_filtered` (`rcon.rs:1750`) and
`find_tiles_filtered` (`rcon.rs:1810`) go through `remote_call`, **not**
`remote_call_json`. `remote_call_json` (`rcon.rs:567`) is the only path that
checks the reply is a complete JSON document and calls `conn.mark_desynced()` on
a short read; its own doc comment explains that without that, "the connection
goes back into the `bb8` pool still holding that tail, and the *next* command on
it reads someone else's reply — permanent cross-talk from one oversized read".
The two find methods have no such check: on a truncated reply they fail at
`serde_json::from_str` and return the connection to the pool intact.

I am not diagnosing the `early eof` failure — that is another agent's. The
API-shape statements are:

1. `EntitySearchFilters` accepts `type?: string | string[]` and `limit?: uint32`
   **[V]**. `record.rs`'s `keyframe_relevant` list could be sent as a `type`
   array and the filtering done by the game, cutting roughly 4× off the reply.
   `FactorioRcon::find_entities_filtered` takes `search_type: Option<String>` — a
   single type — so this needs the signature widened to a list.
2. `TileSearchFilters` accepts `collision_mask` and `limit` **[V]**.
   `is_area_empty` (`rcon.rs:1718`) fetches every entity *and* every tile in the
   area and then only asks whether the lists are empty.
   `LuaSurface.count_entities_filtered` — "As it doesn't construct all the
   wrapper objects, this is more efficient if one is only interested in the
   number of entities" **[V]** — plus
   `find_tiles_filtered{collision_mask = "player", limit = 1}` answers the same
   question in two integers instead of two megabytes.
3. Neither Rust find method passes `limit`, so no reply is bounded by
   construction.

Sizes of the *other* RCON replies, for calibration **[L]**: `world_snapshot` is
`entity_prototypes` 217,417 B + `item_prototypes` 50,797 B + `recipes`
354,894 B + one force 143,715 B ≈ **767 kB in one reply**. That one *does* go
through `remote_call_json` (`rcon.rs:1383`), so a short read there is caught and
the connection is dropped. The comment at `control.lua:2400` claiming "this
payload is well under a megabyte" is true, but only just.

#### B2. Four polls that have a matching event

**(a) Researched-technology count, every 300 ticks.** `control.lua:1453` iterates
the force's entire `technologies` table (≈250 entries on vanilla 2.1) to compute
`unlocked`. `on_research_finished` is *already registered*
(`control.lua:2113`) and carries `research: LuaTechnology` **[V]**. A counter
incremented there is exact and free. The current loop is O(all technologies)
every five seconds for a number that changes a few dozen times per run.

**(b) The full recipe set, re-serialised on every research completion.**
`control.lua:2074-2079` — `on_research_finished` calls `writeout_recipes()`
(`control.lua:2075`), which is 354,894 bytes **[L]**, and `writeout_forces()`
(`control.lua:2078`), which emits **all three forces** (`control.lua:560`
iterates `game.forces`) at ~143.7 kB each **[L]**, including `enemy` and
`neutral` — the exact waste `collect_player_force` was introduced to avoid on the
RCON side, whose own comment at `control.lua:507-509` says "`game.forces` also
holds `enemy` and `neutral`, whose technology tables cost ~120kB each and
describe nobody the planner plans for". Total ≈ **786 kB of stdout per research
completion.** The event's `research.prototype.effects` names exactly which
recipes were unlocked **[V]** — `types.lua:238` already reads that field for
`unlocked_recipes`, so the machinery exists.

**(c) Per-tick re-selection while mining.** `control.lua:713-714` calls
`player.update_selected_entity(ent.position)` and re-reads `player.selected`
every tick for the whole duration of a mine, then compares name and position.
`LuaEntity.mining_target` — "The mining target, if any" **[V]** — answers "am I
mining the right thing" directly, and completion already arrives via
`on_player_mined_entity`. The `mining_state` write must stay per-tick; the
select-and-compare need not. **Caveat, and it is a live one:** `mining_target` is
on `LuaEntity`, not `LuaControl`, and reading it off a character is what crashed
the 2026-09-01 run (see `control.lua:1340-1356` and the fix in `75a098a8`). Any
use here must go through the character's own entity and be `pcall`-guarded the
same way.

**(d) Half-evented player lifecycle.** `on_player_left_game` writes out
(`control.lua:1856`); `on_player_joined_game` (`control.lua:1807`) writes out
nothing. So `crates/core/src/process/process_control.rs:218` polls
`rcon.connected_player_count()` once a second for up to 90 seconds waiting for
clients — up to 90 RCON round trips for an event the mod is already handling. One
`writeout(event.tick, "on_player_joined_game", player_idx)` would let the wait be
driven off the stdout stream, symmetrically with leaving.

*What could break for all of B2:* (b) is the risky one. The planner needs
recipes' `enabled` flags to be current, and the current sledgehammer guarantees
that. Sending only newly-unlocked recipes is correct only if nothing else about a
recipe can change at research time — `research_unit_ingredients` and
prerequisites do not, but I did not verify that no modifier can alter a recipe's
`energy` **[I]**. Measure before trusting.

#### B3. `power_totals` scans every electric pole on every surface, every 300 ticks

`control.lua:1262-1300` calls `find_entities_filtered{type = "electric-pole",
force = force}` (`control.lua:1266-1268`) per surface and walks the result to
find distinct networks. This is O(poles) every five seconds, and pole count grows
monotonically with the factory — this sample gets slower the longer a run goes.

The block's own comment is a model of the care this audit is asking for (it
verifies `LuaElectricSubNetwork` vs `LuaElectricNetwork` explicitly), so the
*correctness* here is fine. Only the cost is the finding.

Two API options, and I want to be precise about what each does and does not give:

- `LuaSurface.global_electric_network_statistics : LuaFlowStatistics` **[V]**.
  This is a `LuaFlowStatistics` — `input_counts`/`output_counts` over time
  windows — **not** the `flow_last_tick` struct the current code sums. It is a
  different measurement, not a drop-in. Do not swap it in expecting the same
  numbers.
- Cache the pole set via filtered event registration.
  `LuaBootstrap.on_event(event, handler, filters)` takes an `EventFilter`, "Used
  to filter out irrelevant event callbacks in a performant way" **[V]**, so
  `on_built_entity` / `on_entity_died` filtered to `type = "electric-pole"` would
  maintain the set incrementally. **Trap, straight from the docs:** "Each mod can
  only register once for every event, as any additional registration will
  overwrite the previous one. This holds true even if different filters are used"
  **[V]** — and `on_built_entity` is *already* registered, twice
  (`control.lua:2102` and `control.lua:2105`, same handler, harmless today).
  Adding a filtered registration would silently replace the unfiltered one and
  break `on_some_entity_created` for every non-pole entity. The pole set would
  have to be maintained inside the existing handler.

---

### Rank C — latent, or correct-but-fragile

#### C1. Quality: the mod counts across all qualities and spends only normal

`inventory_counts` (`control.lua:1973-1979`) deliberately sums across qualities —
"a normal and an uncommon inserter both answer yes" — and its result gates
`rcon_revive_ghost` (`control.lua:2531`) and `rcon_place_blueprint`
(`control.lua:2668`). The corresponding spends are
`main_inventory.remove({name=name, count=1})` (`control.lua:2549` and
`control.lua:2560`) and `charge_item_to`'s `remove({name=item, count=1})`
(`control.lua:2621`).

`ItemStackDefinition.quality` is documented "Defaults to `normal`" **[V]**. So
the check says yes for an uncommon item and the spend takes zero. `charge_item_to`
checks its return and the blueprint path undoes the build — that path is safe.
`rcon_revive_ghost` at `control.lua:2549` and `2560` does **not** check the
return, so it can revive for free.

The parallel asymmetry in `rcon_place_entity`: `player.get_item_count(item_name)`
(`control.lua:2177`) takes an `ItemFilter`, whose `quality` is optional with a
`comparator` **[V]**, against `remove_item`'s `ItemStackDefinition` default of
normal. Whether omitting `quality` in an `ItemFilter` means "any quality" is
**[I]** — the docs give a default for the stack definition and not for the filter.

*Why Rank C and not Rank A:* quality as a mechanic requires Space Age, and a
vanilla 2.1 run produces only normal items, so the two spellings agree in
practice today. It is ranked at all because the mod is already internally
inconsistent about it, and because the failure mode — building for free — is the
one this project has explicitly decided it cares about.

#### C2. `rcon_place_entity` checks collision but not reach

`control.lua:2186` uses `surface.can_place_entity{… build_check_type =
defines.build_check_type.manual}`. `LuaSurface.can_place_entity` is described as
"Check for collisions with terrain or other entities" **[V]** — it takes a
`force`, not a character, and there is no reach in its parameter list.

`LuaControl.can_place_entity(name, position, direction)` — "Checks if this
**character or player** can build the given entity at the given location on the
surface the character or player is on" **[V]** — is the character-relative
counterpart. The docs do not spell out that it enforces `build_distance`, so "it
adds the reach check" is **[I]**, not verified. What *is* verified is that the
current call has no character in it at all, so `player.build_distance` (which the
mod reads and ships to Rust at `control.lua:2058`) is enforced nowhere in the mod.

Similarly `distance(player.position, ent.position) > player.resource_reach_distance`
at `control.lua:705` is a centre-to-centre Euclidean test, where
`LuaControl.can_reach_entity(entity)` **[V]** is the game's own bounding-box-aware
answer. The existing comment above that line documents the real pain this caused
(a flat `> 6` against an actual ~2.7). Swapping in `can_reach_entity` would make
the guard match what the game enforces rather than approximate it.

*What could break:* `rcon.rs` mirrors the current Euclidean rule in
`within_resource_reach` (`rcon.rs:248`) and sizes `approach_radius`
(`rcon.rs:270`) against it, with a comment citing a live run where the bot landed
3.345 tiles out against a reach of 3. Changing the mod-side predicate without
changing the Rust-side one puts two different rules on the two sides of the same
decision, which is worse than one approximate rule in both. These move together
or not at all.

#### C3. `inventory_type_name` is dead code that would raise if called

`control.lua:66-189` builds table constructors keyed on
`defines.inventory.furnace_source` (`control.lua:77`), `.furnace_result`,
`.furnace_modules`, `.assembling_machine_input`, `.assembling_machine_output`,
`.assembling_machine_modules`, `.rocket`, `.rocket_silo_result`. All eight are
absent from `defines.inventory` in 2.1.17 **[V]** (checked programmatically
against the `defines` tree; 2.0 folded them into `crafter_input` /
`crafter_output` / `crafter_modules`). A Lua table constructor with a nil key
raises `table index is nil`, so calling this function is an immediate error.

It has no callers — verified by grep across `mods/`, `crates/`, `app/src/`,
`scripts/` — and `crates/executor/src/rcon_actuator.rs:16` and
`crates/planner/src/action.rs:205` both already document it as dead 1.1-era code
that must not be used as a reference. The finding is only that it is a loaded gun
in a file people edit; deleting it removes the last in-tree copy of the 1.1
inventory names.

*What could break:* nothing. It is unreferenced. The only cost is that
`defines.inventory` names are then documented solely by
`FACTORIO_2_1_INVENTORY_DEFINES` in `rcon_actuator.rs:48`, which is the correct
place and is snapshot-tested against the installed game.

#### C4. Things I checked and found correct

Recorded so the next audit does not re-derive them.

- Every `game.*`, `helpers.*`, `prototypes.*`, `script.*`, `rcon.*`, `settings.*`
  member used anywhere in `control.lua` or `types.lua` exists on the
  corresponding class in 2.1.17 **[V]** (checked programmatically). No unknown
  members on `LuaPlayer`, `LuaSurface`, `LuaForce`, `LuaInventory`, `LuaEntity`,
  `LuaEntityPrototype`, `LuaRecipe`, `LuaTechnology`, `LuaTile`, `LuaItemStack`
  either.
- **Zero** members are flagged `deprecated` in 2.1.17's `runtime-api.json`
  **[V]**. Factorio removes rather than deprecates, so "still used the old way"
  can only be found by parameter and shape mismatch, which is how A2, A4 and C3
  were found.
- `on_some_entity_created` reads `event.entity or event.created_entity`.
  `created_entity` is gone from both `on_built_entity` and
  `on_robot_built_entity` in 2.1 **[V]** — the fallback is correct and the
  ordering (entity first) is the right way round.
- `LuaEntity.revive()` now returns `ItemWithQualityCount[], LuaEntity, LuaEntity`
  **[V]**. `local success, entity = ghost.revive()` names the first return
  "success" when it is an array, but only ever tests `entity`, so the behaviour is
  right and only the variable name lies.
- `PathfinderFlags.prefer_straight_paths` and `allow_destroy_friendly_entities`
  both still exist **[V]**; the mod's flag table is valid.
- `player.character.prototype.collision_mask` is `CollisionMask` and
  `request_path`'s `collision_mask` parameter is also `CollisionMask` **[V]** —
  the types match, and `types.lua:315` correctly iterates `.layers` rather than
  the mask itself.
- `LuaFlowStatistics.input_counts` is `dict<string, uint64|double>` **[V]**, so
  `sample_force`'s `pairs(stats.input_counts)` at `control.lua:1460` is right.
- `LuaControl.begin_crafting(count, recipe, silent)` **[V]** matches
  `control.lua:2510`.
- `game.forces[1]` is the player force on this map **[L]** — see the note at the
  top.

---

## Part 3 — What the 2.1 API now does better than the hand-rolled version

| API entry | Replaces | Note |
|---|---|---|
| `LuaControl.can_reach_entity(entity)` **[V]** | `distance(player.position, ent.position) > player.resource_reach_distance`, `control.lua:705` | Bounding-box aware; the current test is centre-to-centre. Must move together with `within_resource_reach` (`rcon.rs:248`) — see C2. |
| `LuaControl.can_place_entity(name, position, direction)` **[V]** | `surface.can_place_entity{…}` at `control.lua:2186`, which has no character in it | Character-relative. That it enforces `build_distance` is **[I]**. |
| `LuaSurface.create_entity{move_stuck_players = true}` **[V]** | the hand-rolled bounding-box teleport built at `control.lua:2553` / `2696` | Covers `create_entity` only, not `ghost.revive()`. Still a teleport, so it does not close the reporting gap. |
| `LuaSurface.count_entities_filtered` / `count_tiles_filtered` **[V]** | `is_area_empty`, `rcon.rs:1718`, which fetches two full lists to ask two boolean questions | "As it doesn't construct all the wrapper objects, this is more efficient if one is only interested in the number." |
| `EntitySearchFilters.type: string[]` + `.limit` **[V]** | Rust-side filtering at `record.rs:710`+`record.rs:60`; unbounded replies at `snapshot.rs:154` | ~4× reduction on the observed tree/resource mix **[L]**. Needs `find_entities_filtered`'s `search_type: Option<String>` widened to a list. |
| `on_player_mined_entity`'s `buffer: LuaInventory` **[V]** | `products_to_dict(proto.mineable_properties.products)`, `control.lua:804` | Actual yield instead of expected. Valid only during the event. |
| `PathfinderWaypoint.needs_destroy_to_reach` **[V]** | the field dropped at `control.lua:1999-2000` | Turns a silent stuck-then-teleport into a refusable path. |
| `PathfinderFlags.cache = false` **[V]** | the unset default of `true` at `control.lua:2786-2791` | "might fail to respond to changes in the environment" — i.e. a path through something the bot just built. |
| `LuaControl.character_running_speed` **[V]** | the implicit 0.15 the 60-tick stuck heuristic assumes | Includes exoskeletons, tiles, stickers. The planner's `WALK_TILES_PER_TICK` must stay a constant (that crate is pure), but the *mod's* stuck check should not guess. |
| `LuaBootstrap.on_event(…, filters)` **[V]** | full-table scans in `sample_force` / `power_totals` | **Trap:** one registration per event per mod, filters included; `on_built_entity` is already registered twice (`control.lua:2102`, `2105`). See B3. |
| `defines.build_mode` **[V]** | the ignored `force_build` at `control.lua:2650`, `2755` | See A2 — the current default is the *opposite* of the intent. |
| `LuaGameScript.create_inventory(size)` **[V]** | `surface.create_entity{name='item-on-ground', stack='blueprint'}` at `control.lua:2629`, `2735` | A script inventory holds the blueprint stack without spawning a real world entity that must then be `destroy()`ed (`control.lua:2635`, `2652`), and without the window in which that entity is visible to `on_some_entity_created`. Cosmetic; ranked lowest deliberately. |

---

## Summary ranking

1. **A1** — stuck-teleport mis-triggers on ordinary long walks; the sibling
   branch leaves a bot walking forever and reports the walk successful.
2. **A2** — `build_blueprint`'s `force_build` was replaced by `build_mode` in
   2.0; the flag is silently ignored and the default is the opposite of intent.
3. **A3** — `rcon_place_entity` takes the item before creating the entity and
   checks neither return: failed placements destroy material, and a no-op
   removal builds for free.
4. **B1** — unfiltered entity queries (79% trees **[L]**) on the two RCON paths
   that lack `remote_call_json`'s completeness check.
5. **A5/A6** — mined yields recorded as expected values; path waypoints that
   need destroying are walked as clear, which is what feeds A1.

Then B2 (four polls with matching events, ~786 kB of stdout per research
completion), B3 (O(poles) power sampling), and the Rank C latents.
