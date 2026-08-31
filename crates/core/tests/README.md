# Fixture provenance

Every JSON file in this directory is a payload the BotBridge mod produced.
**Each one records which Factorio version it came from and when it was
captured**, because the alternative has already cost this project four shipped
defects.

The three `*-fixtures.json` files below were captured from **Factorio 1.1 in
June 2022** and never refreshed. Nothing said so. A reader could not tell a 1.1
capture from a current one without running `git log`, and nobody did, so four
types drifted away from the game while their tests kept passing:

* `FactorioProduct::probability` — a field Factorio 2.1 does not have.
* `FactorioRecipe::category` — 2.1 renamed it to `categories[]`.
* `FactorioPlayer::main_inventory` — `get_contents()` returns an array in 2.0+,
  not a name→count map.
* `FactorioPlayer::{item_pickup_distance, loot_pickup_distance,
  resource_reach_distance}` — declared `u64`; the game sends `double`.

None of those four could have been caught by the fixtures that existed, and the
type carrying three of them — `FactorioPlayer` — had **no fixture at all**. A
type fed only hand-written JSON is only ever tested against what its author
already believed.

So: if you add a fixture, add a row here. If you refresh one, change its row.

## Rules

1. **Capture verbatim.** Do not pretty-print, round, reorder or hand-edit.
   `resource_reach_distance` really is
   `2.70000000000000017763568394002504646778106689453125`; a rounded fixture
   would not have caught the defect that motivated the capture.
2. **Never hand-write a fixture.** A fabricated payload re-creates exactly the
   problem this directory exists to fix — it can only ever agree with the type
   it is checked against.
3. **Every fixture needs a test that could fail.** A file nothing deserialises
   is decoration. See `live_2_1_payloads.rs`.

## Current fixtures

| File | Game version | Captured | Source | Read by |
|---|---|---|---|---|
| `live-2.1.17-players.json` | 2.1.17 (build 87315, linux64, space-age) | 2026-08-30 | `remote.call('botbridge', 'players')` | `live_2_1_payloads.rs` |
| `live-2.1.17-world-snapshot.json` | 2.1.17 (build 87315, linux64, space-age) | 2026-08-30, **recaptured twice on 2026-08-31** | `remote.call('botbridge', 'world_snapshot')` | `live_2_1_payloads.rs` |
| `live-2.1.17-tiles.json` | 2.1.17 (build 87315, linux64, space-age) | 2026-08-30 | `remote.call('botbridge', 'find_tiles_filtered', {area={{-256,-288},{-248,-280}}})` | `live_2_1_payloads.rs` |
| `live-2.1.17-entities-spawn.json` | 2.1.17 (build 87315, linux64, space-age) | 2026-08-30 | `remote.call('botbridge', 'find_entities_filtered', {area={{-24,-16},{8,8}}})` | `live_2_1_payloads.rs` |
| `live-2.1.17-entities-resources.json` | 2.1.17 (build 87315, linux64, space-age) | 2026-08-30 | `remote.call('botbridge', 'find_entities_filtered', {area={{-56,-60},{-40,-44}}})` | `live_2_1_payloads.rs` |
| `live-2.1.17-inventory-contents-at.json` | 2.1.17 (build 87315, linux64, space-age) | 2026-08-30 | `remote.call('botbridge', 'inventory_contents_at', {{name='crash-site-spaceship', position={x=-5,y=-6}}})` | `live_2_1_payloads.rs` |
| `recipes-fixtures.json` | **1.1 (stale)** | 2022-06-27 (`21cebd70`) | unrecorded | `src/test_utils.rs::fixture_recipes` |
| `item-prototype-fixtures.json` | **1.1 (stale)** | 2022-06-27 (`21cebd70`) | unrecorded | `src/test_utils.rs::fixture_item_prototypes` |
| `entity-prototype-fixtures.json` | **1.1 (stale)** | 2022-06-27 (`21cebd70`) | unrecorded | `src/test_utils.rs::fixture_entity_prototypes` |

### Game state at capture

All six `live-2.1.17-*` files come from **one** session, so they are mutually
consistent — with one documented exception, below:

* A freeplay `space-age` save (`workspace/server/saves/level.zip`), mods
  `base`, `elevated-rails`, `quality`, `recycler`, `space-age`, `BotBridge`.
* A **connected graphical client** (`client1`) standing at the origin with its
  starting inventory — which is what makes `live-2.1.17-players.json` carry real
  distances and a real inventory instead of an empty roster. `players` only
  reports players that are `connected` *and* have a `character`, so an
  unattended headless server answers `{}` and proves nothing.
* Two entities placed by hand before the entity capture, to reach branches of
  `serialize_entity` that a pristine spawn does not have: a `stone-furnace` at
  `(2, 2)` (an entity whose inventories are **empty**) and an `inserter` at
  `(4.5, 2.5)` (the only entity type that reports `pickup_position`).

### The world-snapshot recapture

`live-2.1.17-world-snapshot.json` was recaptured on 2026-08-31, from a headless
`-c 0` server rather than the original session, because the mod changed what the
call returns: `collect_recipes` stopped filtering on `enabled` and
`serialize_technology` grew `unlocked_recipes`, without which the planner cannot
plan through a recipe a technology has yet to unlock.

Recapturing rather than hand-editing is the point of these files, but it does
break the one-session guarantee, so the delta was checked rather than assumed.
Against the previous capture: entity prototypes (1028) and item prototypes (342)
are **byte-identical**, all 277 technologies are unchanged but for the added
`unlocked_recipes` key, and all 23 previously-sent recipes are present and
unchanged. The only additions are 639 disabled recipes. `world_snapshot` carries
no player data, so capturing without a connected client changes nothing in it —
`live-2.1.17-players.json` is still the original session's.

The reply grew from 393 kB to 744 kB. See `FactorioRcon::world_snapshot` for the
single-packet headroom that makes that safe.

### The second world-snapshot recapture

Recaptured again later on 2026-08-31, after a sweep of every `pcall`-wrapped
read in `types.lua` fixed four fields the mod had been failing to collect. Same
method as the first recapture: a headless server started directly from
`workspace/server/bin/x64/factorio`, the reply taken verbatim off the RCON
socket.

The delta against the previous capture is **exactly** those four mod changes
and nothing else, which is what a recapture has to be able to show:

| what changed | records | why |
|---|---|---|
| `collision_mask` now holds layer names | 1028 | `serialize_entity_prototype` iterated the 2.0 `CollisionMask` table instead of its `.layers`, so every prototype reported `["layers"]` and friends. 579 of the 1028 now carry a non-empty mask; the other 449 collide with nothing and correctly report none. |
| `connection_type` on pipe connections | 28 prototypes / 95 connections | read as `type` renamed, which is the 1.1 spelling; `PipeConnectionDefinition` has no `type` in 2.1. Was 0 of 95, now 95 of 95. |
| `crafting_speed` added | 18 | `get_crafting_speed()` (5e4be5a5). Exactly the crafting machines plus `character`. |
| `manual_mining_speed_modifier` added | the one force | `serialize_force` grew it in 0d93dc3b, after the previous capture. |
| `speed` removed from `repair-pack` | 1 | 1c75b866 stopped sending two item fields no Rust type declares. |

`recipes` (662) and all 277 `technologies` are **byte-identical** to the
previous capture, and both prototype lists still name the same 1028 and 342
prototypes. So the one-session caveat above is unchanged: nothing in this file
depends on player state, and `live-2.1.17-players.json` is still the original
session's.

### Why these areas

* Tiles `(-256,-288)..(-248,-280)` is a **shoreline**: 62 collidable water tiles
  and 2 walkable grass tiles. `player_collidable` is the one field the mod
  computes rather than copies, so an all-water or all-land capture could not
  tell a working `collides_with` from one returning a constant.
* Entities `(-24,-16)..(8,8)` is the vanilla crash site: containers with
  non-empty inventories, the player's `character`, plus the placed furnace and
  inserter.
* Entities `(-56,-60)..(-40,-44)` is a plain iron-ore patch — 39 `resource`
  entities, the only ones that carry `amount`.

### Not captured

* **`FactorioGraphic`** — the `graphics` stdout record is parsed by hand from a
  colon-separated string, not from JSON, so there is no payload to capture.
* **`PlayerChangedDistanceEvent` / `PlayerChangedPositionEvent` /
  `PlayerChangedMainInventoryEvent`** — these arrive on the server's *stdout*
  rather than over RCON, so capturing them needs a running `OutputParser`,
  which currently panics on the first placed entity (see
  `an_empty_entity_inventory_from_the_live_game_does_not_yet_deserialise`).
  Left for after that is fixed rather than hand-written.
* **A researched technology, a launched rocket** — `FactorioForce` was captured
  with `current_research: null` and every technology unresearched, which is the
  state a fresh game is in. The researched branch is a documented gap, not a
  covered one.
