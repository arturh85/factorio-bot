# The world-record save did not fail on payload. It failed on a handshake.

2026-09-06. Branch `identity-in-bulk-contents-on-demand`. Everything below is
measured against files in this checkout; where a number comes from reading code
rather than from a file, it says so.

## The headline, and it is not the one the task expected

**`workspace/any-wr-6-39-53.zip` never reached the ingest at all.** The 900-second
timeout was not a slow chunk replay. **Not one chunk was ever requested**, and no
RCON command was ever sent, because the mod's readiness beacon cannot fire on a
save the mod has never seen.

`crates/core/src/process/output_reader.rs` blocks on `rx1.recv()` until a stdout
line `contains("my_client_id")`. Only afterwards does it open RCON and call
`initialize_server` → `whoami("server")` → `on_whoami`, and `on_whoami` is what
builds `client_local_data.initial_discovery`, the replay that carries a loaded
save's already-generated chunks into the world model.

The one line that can print that substring is the tick-120 beat in `on_tick`
(`mods/BotBridge/control.lua`), and it was guarded on `my_client_id ~= nil`.
`my_client_id` is a module local assigned in exactly one place: `on_load`.

**And `on_load` does not run.** From the shipped 2.1.17 API docs in this
workspace (`workspace/factorio-api-docs/runtime-api.json`,
`LuaBootstrap::on_load`):

> This is **only** called for mods that have been part of the save previously,
> or for players connecting to a running multiplayer session.

A save created without BotBridge gets `on_init` instead — "only called when a new
save game is created or when a save file is loaded that previously didn't contain
the mod" — and `on_init` never touched `my_client_id`. So on any foreign save the
local stays `nil` for the whole session and the beacon is silent for ever.

`workspace/wrload/server/factorio-current.log` corroborates every part of it:
Factorio migrated 2.0.66 → 2.1.17, hosted at 4.425 s, opened RCON, autosaved at
17.9 s and 618.1 s, and quit cleanly on SIGTERM at 899.986 s. Nothing was wrong
with the game. **We never spoke to it.**

**Why nobody found this before.** Our own runs create the map in a separate
`--create` invocation (`instance_setup.rs`) and then start a *second* process
with `--start-server level.zip`. That second process loads a save the mod is
already part of, so it takes the `on_load` path and the beacon fires. The bug is
reachable only by the one thing this project had never done: hand Factorio a save
somebody else made.

**The shape is the one CLAUDE.md already enumerates.** The mod's own comment above
`rcon_session_reset` describes this print as a "debug `print`" whose value
"nothing reads". Nothing in the *mod* reads it. The **host** reads it, and it is
the only thing the host waits for. A signal misfiled as debug output went silent
in exactly the state nobody expected, and cost 900 seconds with no message.

Fixed in two halves, in `mods/BotBridge/control.lua`:

* `on_init` now sets `my_client_id` as well, restoring the invariant the rest of
  the file assumes.
* the beacon prints `tostring(my_client_id)` with no `~= nil` guard, so an
  unexpected state produces a line rather than silence.

`crates/core/tests/botbridge_ready_beacon.rs`, five tests. Falsified three ways,
each producing **exactly** the predicted failures:

| break | failures | which |
|---|---|---|
| A — `on_init` assignment removed | 1 | `the_beacon_carries_the_client_count_on_init` |
| B — `~= nil` guard restored | 1 | `the_beacon_speaks_even_when_it_has_no_number_to_report` |
| C — both (i.e. `master`) | 4 of 5 | only the `on_load` control survives |

**This is not yet proven live.** Nothing here has been run against Factorio. What
is proven is the mechanism, in the mod's own source, under the engine's
documented lifecycle. The acceptance test remains a real load of that save.

## The payload measurement, which stands on its own

**Source: `workspace/server-log.txt`, a real run on seed 31337 — the exact bytes
`writeout_entities` printed and `output_parser.rs` read back.** Not a
re-serialisation of a dump, and not an estimate. `tools/measure_entities_payload.py`
and `tools/measure_discarded_payload.py` reproduce it, both taking a server log
as their one argument.

```
entities lines            1,424        10,561,952 bytes   (40% of the log)
tiles    lines            1,024        10,133,451 bytes
entity records           50,256           209 bytes each
```

Bytes by field, compact JSON:

| field | bytes | share |
|---|---|---|
| `bounding_box` | 5,612,528 | **55.3%** |
| `position` | 1,772,662 | 17.5% |
| `entity_type` | 1,063,453 | 10.5% |
| `name` | 886,649 | 8.7% |
| `direction` | 653,328 | 6.4% |
| record envelope | 313,903 | 3.1% |
| `amount` (resources) | 156,900 | 1.5% |
| **`output_inventory` + `fuel_inventory`** | **0** | **0.00%** |

Bytes by entity type:

| type | bytes | share | n |
|---|---|---|---|
| `tree` | 7,119,560 | **68.1%** | 34,515 |
| `resource` | 2,718,192 | 26.0% | 12,367 |
| `fish` | 266,503 | 2.5% | 1,552 |
| `simple-entity` (rocks) | 170,119 | 1.6% | 839 |
| `cliff` | 159,073 | 1.5% | 860 |
| everything else | 26,000 | 0.2% | 123 |

### Three findings, in order of how much they change the work

**1. Inventories cost nothing on our maps, and that is a fact about our maps,
not about the mechanism.** Zero bytes across 50,256 records — a map of trees and
ore has no machine to own an inventory. `serialize_entity` calls
`get_output_inventory()` and `get_fuel_inventory()` on every entity, and both
return nil for a tree, a rock, a belt and an ore tile. **So the offline
measurement cannot size the endgame cost at all**, and nothing in this repo can:
it needs a base. That was the task's central question and the honest answer is
that the dump and the log both answer it "zero", for a reason that does not
generalise.

**2. `bounding_box` is 55.3% of the payload — the single largest term by far,
and larger than every identity field combined.** It is *mostly* derivable:
`crates/core/src/factorio/util.rs` already builds rects as
`add_to_rect(&prototype.collision_box, &entity.position)`, and this was verified
numerically against a live capture (`crash-site-spaceship` at (-5,-6): instance
box = position + prototype collision box, exactly). **But not for cliffs**, whose
real box depends on `cliff_orientation`, which `serialize_entity` does not send
and which appears nowhere in `crates/` or `mods/`. Removing it also means the
Rust side must synthesise it, which touches `crates/core/src/types.rs` and moves
the OpenAPI snapshot. **Designed, not implemented** — see below.

**3. Trees are 68.1% and they must stay.** `EntityGraph::add` puts every tree,
rock and cliff into `blocked_tree` and every tree and rock into `minables`;
`PlanState::walkable_obstacles_within`, `occupant_of`, `resource_tile_blocked`,
`method::connect` and `method::gather` all read the former, and
`minables_yielding` / `minable_sources` read the latter. `3c05e0a3` (removing an
entity takes its own box out of the blocked tree) and `36c04f12` ("a belt is not
a wall") both depend on that data being complete. **A smaller payload that breaks
siting is a worse outcome than a slow one.**

## What was implemented, and why exactly this much

### Do not send what the ingest throws away

`EntityGraph::add` opens its loop with

```rust
if entity.entity_type == EntityType::FlyingText.to_string()
    || entity.entity_type == EntityType::Fish.to_string()
    || entity.bounding_box.width() == 0.
{ continue; }
```

Those records reach no quad tree, no `minables`, no `threats`, no petgraph node.
`writeout_entities` now applies the same predicate at the source.

**Measured share of the wire: 1,574 records, 270,683 bytes, 2.59%** — almost all
of it fish. Small on a fresh map; a finished base adds every remnant, corpse and
particle source to the same category. **This is the only "should not be sent"
category that can be stated as a fact rather than a preference.**

The two lists are kept in step by a test that names both types and the zero-width
rule, not by care.

### Contents on demand

`serialize_entity` takes an `opts.omit_inventories` flag and `writeout_entities`
is its only caller. Every RCON path — `rcon_find_entities_filtered`,
`find_entities_in_radius`, `world_snapshot`, `rcon_place_entity`'s reply — passes
nothing and gets the full record, unchanged; scripts genuinely count plates that
way (`scripts/furnace_run.lua`, `two_row_smelter_live.lua`).

**Nothing reads an inventory off a bulk-ingested entity.** `add` clones the whole
entity into `entity_tree`, so the fields *are* stored — but `add` refuses to
re-add over an occupied position, so the stored copy is a snapshot from the
moment the chunk was generated and is permanently stale. `crates/planner/src/state.rs`
states the design rule in as many words: contents live *beside* the graph. The
planner's buffer model reads `FactorioSurface::inventories`, filled only by
`observe_inventories` from the RCON reply to `inventory_contents_at`. **The
on-demand path already exists and is the one everything real uses.**

So this change is justified by the **consumer census**, not by a byte count —
its measured saving on our maps is exactly zero, which is also why it cannot move
the offline planning baselines.

### Belts: a deliberate non-goal, now pinned

**We send no belt contents today** — verified: nothing in `serialize_entity`
reads a transport line, and a belt goes out as name, position, direction,
bounding box (and surface). **Keep it that way.** A yellow belt holds 8 items per
tile and a base has thousands of belt tiles, so per-tile item positions are the
most expensive thing that could be added here and the least useful: the planner
reasons about connectivity, not about which item is on which lane.

`a_belt_is_direction_and_geometry_and_nothing_else` asserts a belt's **exact key
set** and fails if anything is added. The owner's shape for the day flows are
needed, recorded here so the next person does not invent a different one:

> "Same for conveyor belts — we probably only need the direction, and for a whole
> chain maybe what types of items are on the belts. Having all items individually
> would be expensive on a large base."

Direction and connectivity per belt; item **types** per chain; never per-tile
positions.

`crates/core/tests/botbridge_bulk_entities.rs`, four tests, falsified six ways,
each producing **exactly one** failure and exactly the predicted one:

| break | failing test |
|---|---|
| D — named-type filter removed | `the_bulk_writeout_sends_exactly_what_the_ingest_can_use` |
| E — zero-width rule removed | same |
| F — `omit_inventories` ignored (contents ride along in bulk) | `a_machine_is_sent_without_its_inventories` |
| G — contents dropped for every caller | `an_rcon_query_still_answers_with_the_contents` |
| H — trees swept up with the fish | `the_bulk_writeout_sends_exactly_what_the_ingest_can_use` |
| I — a `transport_line` field added to belts | `a_belt_is_direction_and_geometry_and_nothing_else` |

H is the important one: it is the "smaller payload that breaks siting" mistake,
and the control test refuses it.

## Designed and NOT implemented

### The chunk replay is one chunk per tick, and that is the ingest's real ceiling

`on_tick` in `control.lua`:

```lua
local maxi = id.idx + 1 - 1
```

**Exactly one chunk per tick, hard-coded, with a `- 1` that makes the intent
unreadable.** On our explored map that is 1,424 chunks in 1,424 ticks — 23.7
seconds of game time, invisible. It is a rate limit and it is the term that
dominates on a real save.

Arithmetic, not measurement: at 60 UPS a Nauvis of 10,000 chunks costs 167 s and
one of 50,000 costs 833 s **before** any per-chunk work, and the per-chunk work
on a base chunk is far heavier than on grass, so the effective rate is lower. Our
own level data is 667 KB for 1,424 chunks; the world-record save's is 50,395,154
bytes uncompressed across five surfaces. That brackets the 900-second budget, but
it is an estimate and should not be quoted as more.

**Not touched here, deliberately.** With the handshake broken it has never
executed once, so tuning it would be tuning code that has never run. The right
sequence is: land the beacon fix, load the save, and *measure* the replay — the
budget should then be a time budget per tick (drain until N ms of work), not a
chunk count, because chunks differ by two orders of magnitude in cost.

### `bounding_box` — 55.3%, and out of bounds for this task

Sending the prototype's `collision_box` once (it already travels in
`entity_prototypes`) and reconstructing the instance box from position and
direction would remove the largest single term. Blocked on two things, neither
of them hard, both of them outside this task's boundaries:

* `cliff_orientation` must be sent, or cliffs must keep their explicit box —
  `direction` alone cannot reconstruct a cliff.
* `FactorioEntity::bounding_box` is not an `Option`, so making it derivable
  changes `crates/core/src/types.rs` and moves the OpenAPI snapshot. **Reported
  rather than done**, as instructed.

The subordinate wins in the same family, also unimplemented: `direction` is
6.4% and is read by nothing for a tree, fish or cliff; `entity_type` is 10.5% of
bytes spent re-spelling `"tree"` 34,515 times.

### `tiles` is the same size as `entities` and nobody has looked at it

10,133,451 bytes over 1,024 chunks — 9.9 KB per chunk, one comma-separated
`name:0`/`name:1` per tile, 1,024 tiles per chunk. It is not JSON and it is not
per-entity, so none of the above touches it, and on a save with every chunk
already generated it is paid in full alongside `entities`. Un-analysed.

## Verification

* The three offline baselines are **unchanged**, on the same release binary:
  `researched:automation` 176 / 21,784 · `producing:automation-science-pack:6`
  316 / 22,463 · `producing:logistic-science-pack:6` 442 / 47,542. They could not
  have moved: `git diff --stat` touches only `mods/BotBridge/*.lua`, and no Rust
  the planner compiles.
* Old dumps still load: `map-t0-baseline.json` planned (231 actions / 21,645
  ticks — a different map, so a different number is expected).
  `map-31337-explored.json` loads and then refuses in the planner for a domain
  reason ("a power plant needs water"), which is pre-existing on `master` and
  unrelated.
* `cargo test -p factorio-bot-core` green: 573 unit tests plus every integration
  target, 0 failed. `cargo clippy -p factorio-bot-core --all-targets --deny
  warnings` clean.
* All nine new tests confirmed by name in the full run output
  (`grep -cE "^test (a_save_the_mod|the_beacon|the_bulk_writeout|a_machine_is_sent|an_rcon_query|a_belt_is_direction).* \.\.\. ok"` → 9).

## What is still open

1. **Load the save.** The beacon fix is unproven live. That run is the
   acceptance test and it is now cheap to attempt.
2. **Then measure the replay**, and only then change its budget.
3. **`bounding_box`**, which needs `types.rs` and a decision about cliffs.
4. **Nothing here says what an endgame base's inventories cost**, because
   nothing in this repo contains one. Step 1 produces that number.
