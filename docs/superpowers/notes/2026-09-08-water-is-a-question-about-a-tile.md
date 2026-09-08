# Water is a question about a tile, and `get_fluid_source_fluid` cannot answer it

2026-09-08. Branch `water-is-a-question-about-a-tile`.

## What was asked, and what the shipped API actually says

The brief carried a lead: after the fluidbox-filter work measured that an
offshore pump's output box is `Any` rather than `Only(water)`, the next
candidate crossing was **`get_fluid_source_fluid`** — explicitly flagged as a
lead, not a fact.

It exists. It is also **the wrong end of the problem**, and
`workspace/factorio-api-docs/runtime-api.json` (`application_version` 2.1.17)
says so in one line:

```json
{ "name": "get_fluid_source_fluid",
  "subclasses": ["OffshorePump"],
  "description": "Checks what is expected fluid to be produced from the
                  offshore pump's source tile. It accounts for visible tile,
                  hidden tile and double hidden tile.",
  "return_values": [{ "type": "string", "optional": true }] }
```

It is a **method on `LuaEntity`, restricted to `OffshorePump`**. It can only be
asked of a pump that already stands. A planner deciding *where to put one* has
no such entity, so the call it was reaching for could never have answered the
question that blocks it.

**The prototype side answers the same question with no entity and no runtime
call at all**, which is strictly better and is what shipped here:

| what | where | shape |
|---|---|---|
| which fluid this tile gives | `LuaTilePrototype::fluid` | optional `LuaFluidPrototype` — *"The fluid offshore pump produces on this tile, if any"* |
| which tile a pump reads | `LuaEntityPrototype::fluid_source_offset` | optional `Vector`, `subclasses: ["OffshorePump"]`; `{0, -1}` for the vanilla pump |

Neither is a runtime observation, so both are available offline, before
anything is built, on a map nobody has walked.

**The transferable part**: this doc's own `sources_of` comment has now named a
sufficient condition for closing water **twice** and been wrong both times —
first *"the day the fluidbox's accepted fluid crosses"*, then *"ask
`get_fluid_source_fluid`"*. Both times it wrote down where the answer ought to
live instead of grepping the shipped schema for where it does. The grep costs
about ten seconds. Both wrong guesses cost more.

## Three states, and the trap they exist to avoid

`TileFluid::{Yields{fluid}, Dry, Unknown}` — `Unknown` is the `Default` and is
**inert**: it neither attributes nor refuses, and falls through to exactly the
inferences that ran before the field existed.

- `Yields` — *"a `water` tile gives water"*. Definite.
- `Dry` — *"a `grass-1` tile gives nothing"*. **Also definite, and a different
  fact.** It is what makes "there is no water here" sayable.
- `Unknown` — an archived dump, an older mod, unexplored ground, a read that
  raised. Not a fact about the game.

An `Option<String>` has two slots for three states and would have merged the
middle one into the last — the `absent-is-not-a-value` collapse, now at eight
instances. The wire keeps them apart deliberately:

```
water:1:water      yields water
grass-1:0:         yields nothing, definitely
grass-1:0:?        we could not tell
grass-1:0          an older sender — two fields — which is EVERY archived
                   server log and both world dumps (4,440,064 tile records)
```

Reading those 4.4 million records as `Dry` would have asserted that no map this
project has ever run on has water on it.

`fluid_at` on an uncharted tile also answers `Unknown`, never `Dry`. That
collapse is deliberate and one-directional: both genuinely mean *we could not
tell*, and reporting unexplored ground as dry would let a planner refuse a lake
it has simply never walked to. A `Dry` from `fluid_at` is always a charted tile.

## Why it is not a `bool` called `is_water`

`FactorioTile::is_water` tests the name against `["deepwater", "water"]`, read
off a **vanilla** capture. **This install runs Space Age**, where
`ammoniacal-ocean` yields ammonia and Vulcanus' lava yields lava, and neither
name is in that pair. Naming the *fluid* rather than the *tile* is what makes
the rule survive a mod — the same argument that replaced the hard-coded
smelting rate the world-record base falsified.

## What `sources_of` can now answer, and what it refuses

`attributable_to` gains a second **observation**, ranked with the box filter
and above the three inferences:

- **it attributes**: a ground-drawing entity whose source tile yields the fluid
  is a source. This is water, which no other rule could ever reach — water is
  not a charted resource, a pump carries no recipe, and a pump is not a buffer;
- **and it refuses**: a pump facing charted dry land is *not* a source, even
  where the footprint inference would have accepted it. A ground-drawing entity
  has no other source, so the inferences must not talk it back up.

**No name is written anywhere.** The discriminator is the *absence* of
`fluid_source_offset`, which the runtime API restricts to `OffshorePump` — so a
boiler on a shoreline is declined without anybody excluding boilers, and a
modded ground-drawing machine works with no code change. The offset is applied
in the entity's own frame, so a pump's `direction` decides which tile is read; a
non-cardinal direction declines rather than guessing.

## Cost: one call per chunk, not 1024

`writeout_tiles` is marked SLOW in its own comment and runs over a 32×32 chunk,
so the fluid is resolved from a **by-name prototype cache** (the same shape as
`player_collides_with_tile`) — one engine crossing per distinct tile name.

The hidden-tile walk is the interesting part. A lake under landfill still pumps
water, so the visible name alone would be *confidently wrong* there — worse than
`?`. Rather than 1024 `hidden_tile` reads per chunk, one
`count_tiles_filtered{has_hidden_tile = true, limit = 1}` answers for the whole
area; only when it is non-zero does the per-tile walk run. On ground the game
has just generated, it is zero. The RCON path (`serialize_tile`) is per-tile and
low volume, so it always does the full walk.

## Filed and NOT handled — the two callers still reading water by name

Saying so in the same breath, because two of the eight `absent-is-not-a-value`
instances were limitations already recorded in a doc comment and never acted on:

- `EntityGraph::nearest_water_tile` and `water_tiles_within` filter on
  `FactorioTile::is_water`, i.e. on the vanilla name pair. They site the steam
  plant (`method::power`).
- `EntityGraph::is_water_at` does the same, and `PlanState` reads it to label an
  obstacle.

**They cannot be swapped for `fluid_at` directly**, and that is the whole reason
they are still here: every archived dump reads `Unknown`, so a straight swap
would find no water on any of them and refuse every plant. The correct form is
the widening one — `fluid.yields("water") || (fluid is Unknown && is_water_by_name())`
— which is a behaviour change to power siting and wants its own measurement, not
a drive-by. Named here so the next session does not have to rediscover it.

## Verification

- Live headless run on an isolated instance, debug build (release embeds the mod
  via `include_dir!`, which has no `rerun-if-changed`) — see the run record in
  the commit message.
- The four offline baselines were re-measured on the same binary before and
  after. The rule is **inert against `map.json`**: the dump's 409,600 tiles
  carry no `fluid` key and every prototype's `fluid_source_offset` is absent, so
  every reading is `Unknown`/`None` and nothing changes. That was the prediction
  and it is what the numbers say.
