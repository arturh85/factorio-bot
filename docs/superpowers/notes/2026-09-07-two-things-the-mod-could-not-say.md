# Two things the mod could not say

**2026-09-07**, branch `two-things-the-mod-cannot-say`, off `9da3447a`.

Two fields on the same seam, both of the same shape: **a fact the game has, the
mod never read, and nothing downstream could distinguish from absence.**

* `LuaEntityPrototype.crafting_categories` — *what* a machine crafts. Sequel to
  `2026-09-07-a-recipe-the-planner-cannot-run.md`, which located the oil
  blocker here and said what to send.
* A census of `game.surfaces` — *what surfaces exist*, as opposed to what this
  bridge observes.

**Neither opens a gate.** The planner is untouched (`crates/planner/` was out
of bounds and stays that way); the mod's Nauvis guard is untouched;
`FactorioWorld` still refuses a second surface by name. The deliverable is that
the fact arrives.

> **Mid-task, `crates/core/src/factorio/` was taken back**: the owner approved
> the surface separation (moving the game- and force-global fields off
> `FactorioSurface`), and an agent started on `world.rs`. The Rust *storage*
> for the census had been written there and was **reverted**; what ships is the
> mod side, the type in `types.rs`, and a handover. `crafting_categories` is
> unaffected — it is a `FactorioEntityPrototype` field.

## 1. `crafting_categories`

### Attribute or method — re-verified, not recalled

**Attribute.** Read out of `workspace/client1/doc-html/runtime-api.json`
(2.1.17) by this session rather than taken from the brief:

```
LuaEntityPrototype
  ATTR crafting_categories  read = dictionary string -> literal true  optional
  (no `get_crafting_categories` method anywhere on the class)
```

That is the opposite way round from its three neighbours —
`get_crafting_speed()`, `get_supply_area_distance()` and
`get_max_wire_distance()` are methods with no attribute at all — and getting it
backwards is not a compile error but a **silence**: reading a method as an
attribute raises, the mod's `pcall` swallows it, and the field arrives absent
for every prototype in the game. That is exactly how `crafting_speed` was nil
for 1,028 live prototypes.

`the_crafting_categories_come_from_the_attribute_and_not_a_method` holds it
shut with a **decoy method** returning different values, so reading the wrong
one cannot look like success. Break B (read through the method when one exists)
kills that test and only that test.

### The premise, asserted rather than recited

The reason the machine could not be derived is that `entity_type` says the same
thing about all of them. That is now a test rather than a paragraph:
`two_machines_with_one_entity_type_are_separated_by_their_categories` builds
`oil-refinery` and `assembling-machine-1`, asserts their `entity_type` is
*equal*, and asserts their categories are disjoint.

### `None` is not `Some(vec![])` — and this is where it differs from its sibling

`resource_categories`, the mining half of the identical rule, sends `nil` for an
empty set: its Lua explicitly says "nil rather than `{}` so an empty set does
not arrive as an empty *map*". **So on that field the split is documented in
Rust and unreachable from the mod** — an empty set and an absent one arrive
identically. That is a pre-existing inconsistency, noticed here and
deliberately not changed (it is another agent's field and another agent's call).

`crafting_categories` sends the empty table. `helpers.table_to_json` renders it
as the JSON object `{}`, and `FactorioEntityPrototype`'s existing
`option_vec_or_empty_map` deserializer already accepts exactly that shape as
`Some(vec![])`. So:

| wire | Rust | meaning |
|---|---|---|
| key absent | `None` | the sender did not say — every archived dump, and every prototype that is not a crafting machine (the attribute is optional and reads nil) |
| `"crafting_categories":{}` | `Some(vec![])` | the game says this machine runs no category |

Break A (make it send `nil` for empty, like its sibling) kills exactly
`a_machine_that_crafts_nothing_is_distinguishable_from_one_that_never_said`.

## 2. The surface census

### What was actually missing

`mods/BotBridge/control.lua`'s `on_chunk_generated` drops every non-Nauvis
chunk and **writes out the surface it dropped**. That is honest per chunk, and
it is left exactly as it was. What it cannot do is sum: *N honest refusals* and
*we never looked* produce the same world model. The world-record base was
loaded and its census read 39,237 entities, every one on `nauvis` — a number
that is equally consistent with both readings, because the entities that would
have said otherwise were dropped upstream.

`LuaSurface.planet` is an **attribute** too (optional, returning a `LuaPlanet`;
`LuaPlanet.name` is the planet's own name). Verified in the same
`runtime-api.json` read.

### Three fields, and one of them has a meaningful absence

`FactorioSurfaceInfo { name, index, planet }`. `planet` is `None` for a surface
that **is not a planet** — which is what a space platform is, and the
distinction worth having, since a platform moves and a planet does not. The
serialiser fixture uses a surface whose name differs from its planet's
(`vulcanus-2` / `vulcanus`), so the obvious wrong implementation — copying
`surface.name` into both, which passes on Nauvis where they agree — fails.

**Store a census as `Option<Vec<FactorioSurfaceInfo>>`, never a bare `Vec`** —
the rule is written into the type's own doc because nothing in Rust stores one
yet. `None` is *nobody enumerated*; an empty list would be the claim that a
running game has no surfaces, which cannot happen, and folding the two together
puts the field straight back into the silence it exists to end.

### What ships, and what is handed over

**Shipped, and answerable today:**

* `remote.call('botbridge', 'surfaces')` — the census of a **running** game,
  one small reply. `rcon_world_snapshot` also carries it as a `surfaces` field,
  so the day Rust has somewhere to put it the mod needs no change.
* `FactorioSurfaceInfo` in `crates/core/src/types.rs`, with the
  `None`-versus-`Some(vec![])` rule written down.

**Handed over** — three pieces, all in `crates/core/src/factorio/` and
`crates/core/src/process/`, which the separation agent owns:

1. `WorldSnapshot.surfaces: Option<Vec<FactorioSurfaceInfo>>`, landed by
   `apply_snapshot` under the same rule as `daylight`: **a sender that says
   nothing does not erase what a newer one said**, or "this build is old"
   becomes "nobody ever looked".
2. Somewhere to store it. It is **game-global, not per-surface** — a census of
   `game.surfaces` is a fact about the save — so after the separation it
   belongs on `FactorioWorld` rather than on `FactorioSurface`, which is
   exactly why the timing worked out.
3. The stdout half: `writeout_surfaces()` in `writeout_initial_stuff` plus a
   `"surfaces"` arm in `output_parser.rs`. Two notes for whoever writes it.

   **They must land together.** `output_parser.rs` logs
   `unexpected action: <key>` as an **error** for any writeout key it has no
   arm for, so emitting the census first would put a red line in every run that
   looks like a defect and is not. `control.lua` says so where
   `writeout_surfaces` would have gone.

   **And the arm should be all-rows-or-none**, deliberately unlike
   `entity_prototypes` directly above it, which `filter_map`s a bad row away.
   That is right for a prototype table: missing one entry is *degraded*. A
   **census** missing one entry is *wrong in the exact direction the field
   exists to prevent* — it under-reports the surfaces a save has while reading
   as a complete answer. A census that cannot be parsed should stay `None`.

Meanwhile the extra `surfaces` key on the snapshot reply is **ignored, not
rejected** — `WorldSnapshot` has no `deny_unknown_fields` — and
`a_snapshot_carrying_the_census_still_loads_before_rust_has_a_field_for_it`
pins that, because otherwise every `--connect` session would break on a key
nobody reads.

## Falsification

Eight breaks against the shipped code, one at a time, each script-asserting its
substitution matched **exactly once** and restoring before the next
(`scratch/falsify.py`).

| # | break | subs | red |
|---|---|---|---|
| A | empty `crafting_categories` sent as nil | 1 | 1 |
| B | read through a method when one exists | 1 | 1 |
| C | sorted the other way | 1 | **2** |
| D | misspelt on the Rust struct | 1 | **4** |
| E | census not sorted by index | 1 | 1 |
| F | planet reported as the surface's own name | 1 | 1 |
| K | a surface with no planet is given one | 1 | 1 |
| M | `rcon_surfaces` not registered | 1 | 1 |

C and D are honest rather than isolated: two tests assert sorted output, and D
removes the field from the wire entirely, which is the whole feature.

Five further breaks (G-L) were run against the **reverted** Rust storage before
the boundary changed and each killed exactly the test it should have — the
malformed-row refusal, the snapshot's non-erasure, the dump round trip, the
parser arm, and the writeout envelope. Those tests went with the code; they are
recorded here because the same assertions are what the handover above asks for.

**M was GREEN on its first run, and that is the finding of this section.** The
test reached the handler through its Lua global, so dropping the
`surfaces=rcon_surfaces` registration line changed nothing — a handler that
exists and is not published is a handler nobody can call, and the test could
not see the difference. Fixed by capturing `remote.add_interface` in the shared
stub (`_interfaces`) and calling through it. **A test in that file had been
able to prove less than its name claimed, and only a mutation said so.**

## Baselines

Release binary built from this branch, `--bots 1,2,3,4`, seed 31337, Factorio
2.1.17, default settings.

| goal | world | expected | measured |
|---|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 | **176 / 21,784** |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | **316 / 22,457** |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 | **441 / 47,478** |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,115 / 317,283 | **2,115 / 317,283** |

`gathered:crude-oil` against `map.json` refuses, correctly, naming the charting
radius: *"charted ground covers 17 of 17 probes within 256 tiles of [0, 0], so
the uncharted ground is beyond that radius"*.

**What they can prove and what they cannot.** They prove that a
serialisation-only change moved nothing — which is what a serialisation-only
change must do. They prove **nothing about either field**, and cannot: *no
archived dump carries either one*, so the offline basis has no value to
exercise. `map.json` and `map-31337-explored.json` were both written before
these fields existed and load with `None` for both, which is the correct
reading and is also exactly the reading a broken implementation would produce.

**The tell for a plumbing failure is uniformity**: a field that is *sometimes*
absent is a fact about the world; one that is *always* absent is a bug. On the
offline basis both are uniformly absent for the honest reason, so the tell is
unavailable there. Only a live probe distinguishes them — the next section.

The binary was interrogated rather than the game, because `include_dir!` has no
`rerun-if-changed` and a rebuild after a `.lua` edit can legitimately ship the
old mod:

```
strings -a target/release/factorio-bot | grep -c crafting_categories   4
strings -a target/release/factorio-bot | grep -c rcon_surfaces         2
```

## The live probe — and both answers

An isolated instance (`workspace/wrload.toml`, RCON 4390, never the default
ports), loading the **world-record save** — a 6:39:53 Space Age base — with
this branch's mod copied into that workspace's own `mods/BotBridge` and nothing
else touched. Read-only throughout: two RCON queries and one `--connect`
attach. The workspace's mod was restored from the main checkout afterwards, and
`workspace/mods/BotBridge` still points at the main checkout.

### How many surfaces the world-record save actually has: **TEN**

```
{"name":"nauvis",    "index":1,  "planet":"nauvis"}
{"name":"platform-1","index":2}
{"name":"platform-2","index":3}
{"name":"platform-3","index":4}
{"name":"gleba",     "index":5,  "planet":"gleba"}
{"name":"vulcanus",  "index":6,  "planet":"vulcanus"}
{"name":"platform-4","index":7}
{"name":"platform-5","index":8}
{"name":"fulgora",   "index":9,  "planet":"fulgora"}
{"name":"aquilo",    "index":10, "planet":"aquilo"}
```

**Five planets and five space platforms.** The 39,237 entities we read off that
save, every one on `nauvis`, were **one surface of ten** — and until this query
there was no way to say whether that was a fact about the save or about us.
Both answers were live: it is about us.

The platforms are the first live case of a `planet` that is legitimately
absent, so the `None`-is-a-real-answer rule is not hypothetical either — half
of this save's surfaces take it.

### `crafting_categories` crosses the whole chain

Over the same running game, 1,028 entity prototypes:

```
prototypes = 1028   with crafting_categories = 18   empty = 0

stone-furnace         furnace              smelting
assembling-machine-1  assembling-machine   advanced-crafting, crafting, parameters
oil-refinery          assembling-machine   oil-processing, parameters
chemical-plant        assembling-machine   chemistry, parameters
centrifuge            assembling-machine   centrifuging, parameters
iron-chest            container            (absent)
```

**18 of 1,028 is the tell.** A field that is *sometimes* absent is a fact about
the world; one that is *always* absent is a plumbing failure. And these are the
identical numbers read two ways: first straight off `prototypes.entity` with an
ad-hoc RCON command, then out of a **dump written by `--connect`**, which is the
whole chain — mod serialiser → `world_snapshot` over RCON → `WorldSnapshot` →
`FactorioSurface` → JSON on disk. `oil-refinery` and `assembling-machine-1`
still share `entity_type`, and now they no longer share an answer.

Three corrections to the brief and to my own earlier prose, from this probe:

* **The chemical plant's category is `chemistry`, not `chemical`.**
* **`parameters` is on four of the five**, and it is not a recipe category
  anybody makes anything in — it is Factorio 2.0's blueprint-parameter
  pseudo-category. Whoever writes the third method must not treat a
  `parameters` match as evidence that a machine runs a recipe.
* **18 declare categories, not 12.** The 12 in
  `2026-09-07-a-recipe-the-planner-cannot-run.md` is a count of `crafting_speed`
  on the older t=0 capture; this is a different game and a different question.

**`empty = 0`**, so `Some(vec![])` is unreachable on vanilla + Space Age today.
The distinction is kept anyway: it costs one branch, and the alternative is
that the first mod which ships such a machine is silently indistinguishable
from a build that could not ask.

## What this does not do

* **It does not open the category gate.** Writing the third method — naming
  `oil-refinery` from `crafting_categories` rather than from a table — is its
  own task in `crates/planner/`, with its own measurements. Walls two and three
  from `2026-09-07-a-recipe-the-planner-cannot-run.md` are still standing
  behind it.
* **It does not ingest a second surface**, and nothing here is a step towards
  doing so accidentally. The Nauvis guard stays and `game.surfaces[1]` in the
  chunk replay stays. (As of this branch `FactorioWorld::insert_surface` also
  still refuses the second surface by name — the separation now under way is
  what lifts that, and this note does not.)
* **It does not land the census in Rust.** Three pieces are handed over, listed
  under "What ships, and what is handed over"; until they exist,
  `remote.call('botbridge', 'surfaces')` is the only way to ask, which is
  exactly how the ten surfaces above were counted.
* **It does not fix `resource_categories`' unreachable empty case.** Named
  above so the next person does not rediscover it.
* **It says nothing about what is ON the nine surfaces we do not read.** Ten is
  a count of surfaces, not evidence about their contents; the 39,237 entities
  remain the only surface anybody has looked at.
