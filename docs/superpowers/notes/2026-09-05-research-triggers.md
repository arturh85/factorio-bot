# Research triggers on a headless run: what the game fires by itself, and what the mod has to

2026-09-05. Branch `research-triggers-headless`, worktree `.worktrees/triggers`,
scratch instance `workspace/headless-h` (RCON 4337, game port 34217), one
character bot, `--seed 31337 --new`, Space Age 2.1.17. Every number below was
read over `factorio-bot rcon --settings scratch/headless-h.toml -s localhost`
against that live server; none is from memory or from the prototype files.

## The 32 trigger technologies, from the running game

`for n,t in pairs(game.forces.player.technologies)` with
`t.prototype.research_trigger ~= nil`, payload via `serpent.line`. Prerequisites
are the direct ones (`t.prerequisites`).

| technology | type | payload (runtime shape) | prerequisites |
|---|---|---|---|
| agricultural-science-pack | craft-item | `{count=100, item={name="bioflux"}}` | artificial-soil, bacteria-cultivation, bioflux-processing |
| agriculture | mine-entity | `{entities={"iron-stromatolite"}}` | planet-discovery-gleba |
| artificial-soil | craft-item | `{count=500, item={name="nutrients"}}` | jellynut, yumako |
| automation-science-pack | craft-item | `{count=1, item={name="lab"}}` | electronics, steam-power |
| bacteria-cultivation | craft-item | `{count=1, item={name="bioflux"}}` | bioflux |
| big-mining-drill | craft-item | `{count=1, item={name="foundry"}}` | electric-mining-drill, foundry |
| biochamber | craft-item | `{count=10, item={name="nutrients"}}` | jellynut, yumako |
| bioflux-processing | craft-item | `{count=25, item={name="bioflux"}}` | bioflux |
| bioflux | craft-item | `{count=1, item={name="biochamber"}}` | biochamber |
| biter-egg-handling | capture-spawner | `{}` | captivity |
| calcite-processing | mine-entity | `{entities={"calcite"}}` | planet-discovery-vulcanus |
| cryogenic-plant | craft-item | `{count=1, item={name="lithium-plate"}}` | lithium-processing |
| cryogenic-science-pack | craft-item | `{count=1, item={name="cryogenic-plant"}}` | cryogenic-plant |
| electromagnetic-plant | craft-item | `{count=50, item={name="holmium-plate"}}` | holmium-processing |
| electromagnetic-science-pack | craft-item | `{count=1, item={name="supercapacitor"}}` | electromagnetic-plant |
| electronics | craft-item | `{count=10, item={name="copper-plate"}}` | -- |
| foundry | craft-item | `{count=1, item={name="tungsten-carbide"}}` | calcite-processing, tungsten-carbide |
| heating-tower | mine-entity | `{entities={"copper-stromatolite"}}` | planet-discovery-gleba |
| holmium-processing | craft-item | `{count=1, item={name="holmium-ore"}}` | recycling |
| jellynut | mine-entity | `{entities={"jellystem"}}` | agriculture |
| lithium-processing | mine-entity | `{entities={"lithium-iceberg-big","lithium-iceberg-huge"}}` | planet-discovery-aquilo |
| metallurgic-science-pack | craft-item | `{count=1, item={name="tungsten-plate"}}` | tungsten-steel |
| oil-processing | mine-entity | `{entities={"crude-oil"}}` | oil-gathering |
| recycling | mine-entity | `{entities={"fulgoran-ruin-vault"}}` | planet-discovery-fulgora |
| space-platform | create-space-platform | `{}` | rocket-silo |
| space-science-pack | **build-entity** | `{entity={name="asteroid-collector"}}` (singular) | space-platform |
| steam-power | craft-item | `{count=50, item={name="iron-plate"}}` | -- |
| steel-axe | craft-item | `{count=50, item={name="steel-plate"}}` | steel-processing |
| tungsten-carbide | mine-entity | `{entities={"big-volcanic-rock","big-volcanic-rock-hot","huge-volcanic-rock","huge-volcanic-rock-hot","small-demolisher-corpse","medium-demolisher-corpse","big-demolisher-corpse"}}` | planet-discovery-vulcanus |
| tungsten-steel | craft-item | `{count=1, item={name="big-mining-drill"}}` | big-mining-drill |
| uranium-processing | mine-entity | `{entities={"uranium-ore"}}` | uranium-mining |
| yumako | mine-entity | `{entities={"yumako-tree"}}` | agriculture |

Counts: 19 `craft-item`, 10 `mine-entity`, 1 `build-entity`, 1
`capture-spawner`, 1 `create-space-platform`. `craft-fluid`,
`send-item-to-orbit` and `scripted` are unused. Two corrections to what the
tree said before this: **`build-entity` is shipped** (`space-science-pack`),
and its runtime spelling is `entity = {name = ...}`, singular, where
`mine-entity` spells `entities = {...}` -- `trigger_names` in `types.lua`
already reads both. On a Nauvis-only run the reachable ones are the four
`craft-item` early-game gates (`electronics`, `steam-power`,
`automation-science-pack`, `steel-axe`) and the two `mine-entity`
(`oil-processing`, `uranium-processing`).

## What the game fires on its own for a server-side character

Each row is one act on the scratch server, the technology read before and
after. The mod's sweep at the time handled `craft-item` only, so every
`mine-entity` and `build-entity` result is the game's alone.

| act | prerequisite | before | after | verdict |
|---|---|---|---|---|
| character mines `big-volcanic-rock` via `action_start_mining` (tick 8122 -> `action_completed` 8363; 6 tungsten-ore in inventory) | `planet-discovery-vulcanus` open | tungsten-carbide false | **false** | not while the prerequisite is open |
| same, prerequisite set researched first, fresh rock (tick 10527 -> 11592) | met | false | **true** | **the game fires `mine-entity` for a character with no player** |
| fuelled `burner-mining-drill` on a created calcite patch (15079 -> 15829, 2 calcite in statistics) | `planet-discovery-vulcanus` met | calcite-processing false | **true** | a drill fires it |
| `pumpjack` on a created `crude-oil` well, powered by an `electric-energy-interface` (14926 -> 15829, 150 crude in the fluid statistics) | `oil-gathering` set | oil-processing false | **true** | a pumpjack fires it |
| character mines `copper-stromatolite` (18035 -> 18083) **then** `planet-discovery-gleba` is set at 19311 | open at the act | heating-tower false at 19311 | **true** by 21104, no further act | **the act is counted before the prerequisite and fires when it closes** |
| `surface.create_entity{name="asteroid-collector", force=player}` | `space-platform` open, then set | space-science-pack false | false, and **false** | `create_entity` fires nothing ... |
| same with `raise_built = true` | met | false | **false** | ... with or without the event |
| stone furnace smelts 20 copper ore, first world (17861 on; 10th plate before 19800) | none | electronics false | true at 19800, **by the mod's sweep** (`research_trigger_emulated`, `produced: 10`) | inconclusive: the sweep raced the game |
| same, second world, **sweep switched off** (`set_research_trigger_emulation(false)` at 1669; 9 plates at 3587, 13 at 4194) | none | electronics false | **true** by 4194, no sweep line in the log | **`craft-item` from a machine fires by itself**, within ~400 ticks of the tenth plate |
| character hand-crafts a lab through `action_start_crafting` (6035 -> `action_completed` 6156; lab in inventory, `lab` absent from the statistics), **sweep off**, `electronics` and `steam-power` researched | met | automation-science-pack false | **false** through 10240 | **a hand craft does not fire `craft-item`** |
| sweep switched back on at 11096 | met | false | **true at 11100**, `research_trigger_emulated {item: lab, needed: 1, produced: 1}` | the hand-craft tally is the emulation that matters |
| character asked to place an `asteroid-collector` through `place_entity` (space-platform set researched) | met | space-science-pack false | refused: `surface.can_place_entity said 'no' (nothing in the footprint; tile: dirt-6)` -- a ghost of it says the same | the one `build-entity` act cannot happen on Nauvis, honestly or otherwise |

Three facts fall out of that table:

1. **`mine-entity` needs no emulation.** The game counts a character's
   swings, a drill's output and a pumpjack's flow with nobody at a keyboard.
   Emulating it would complete the technology a second time at best and, on a
   counter that disagrees with the game's, early at worst. The sweep leaves it
   alone, and a stub test pins that.
2. **Prerequisites gate every trigger, and the game remembers the act.** The
   rock mined with `planet-discovery-vulcanus` open earned nothing; the
   stromatolite mined before `planet-discovery-gleba` earned `heating-tower`
   a few ticks after the prerequisite closed. The sweep now does the same:
   it skips a technology with an open prerequisite and reads counters that
   persist. Before this it ignored prerequisites entirely, so
   `automation-science-pack` could complete in the same 60-tick sweep as --
   or, by `pairs` order, before -- `electronics` and `steam-power`, which the
   game would not have allowed.
3. **`build-entity` needs emulation, and `craft-item` needs it for hand
   crafts only.** `create_entity` with the force set, which is how every
   placement the mod makes lands, fires nothing, and `raise_built` does not
   change that. A furnace's plates fire the trigger on their own; a
   character's hand-crafted lab never does. The first attempt at this table
   read the furnace row as "does not fire" because the sweep, on a 60-tick
   beat, had beaten the game's own check (~400 ticks) to it -- the reason the
   switch exists, and the reason the row was re-measured with it off.

## What was emulated, and how it stays honest

`emulate_research_triggers` (`mods/BotBridge/control.lua`), every 60 ticks,
only while character bots exist:

- **`craft-item`**, as before: the larger of the force's item production
  statistics and `storage.crafted_tally` (hand crafts, which never reach the
  statistics), never the sum. Now also **only once every prerequisite is
  researched**. The statistics half is redundant with the game (measured) and
  can only complete a technology the game was about to complete; the tally
  half is the one a headless run cannot do without.
- **`build-entity`**, new: `storage.built_tally[name]`, incremented in
  `on_some_entity_created` for every entity whose force is `player` -- the
  one point `rcon_place_entity` (after the item is paid for), the game's
  `on_built_entity` and `on_robot_built_entity` all pass through.
  `on_biter_base_built` reaches the same handler and is excluded by the force
  check. Any of the trigger's names counts; the event records which one and
  how many.
- **`mine-entity`**: not touched. The game does it.
- **`capture-spawner`, `create-space-platform`**: nothing in the mod or the
  executor can capture a spawner or launch a platform, so there is no act to
  count. Refused by the planner, now with the act in the refusal
  (`PlannerError::UnsupportedResearchTrigger { act }`: "capture a spawner",
  "create a space platform", "build 1 asteroid-collector").

Every completion writes `research_trigger_emulated` with the counts that
earned it (`produced` or `built`, against `needed`). A new remote function,
`set_research_trigger_emulation(false)`, switches the sweep off for a
measurement of what the game does alone and writes
`research_trigger_emulation {"enabled": false}` so a run made that way cannot
be mistaken for one in which the game fired everything.

## What the planner does with each kind

Unchanged in shape: `trigger_requirement` (`crates/planner/src/method/util.rs`)
turns `craft-item` into `Produced { unlocks }` and `mine-entity` into
`Produced` (hand-minable) or `Goal::Extracted` (a well), and refuses the rest.
The refusal now names the act. The one shipped `build-entity` technology wants
an asteroid collector, which stands only on a space platform, so a `Have this
entity placed` goal would be a goal no bot on Nauvis can meet; it stays
refused rather than planned as unreachable.

## Tests

`crates/core/tests/botbridge_research_triggers.rs` -- eleven stub-game tests on
the real `control.lua`: ten smelted plates earn `electronics`, nine do not; a
hand-crafted lab earns `automation-science-pack` once its prerequisites are
researched, waits while one is open, and still counts after it closes; a
built asteroid collector earns `space-science-pack`, nothing built does not,
and an enemy-force build does not count; `oil-processing` is left alone with
150 crude oil in the statistics; the switch stops the sweep and is recorded;
no character bots, no sweep. `an_inexpressible_trigger_is_refused_by_name` in
`crates/planner` now checks the act too.

## The event reaches `events.jsonl` now, and did not before

`research_trigger_emulated` was a `writeout` the mod had been making since the
first headless run, and `OutputParser` answered it with `unexpected action`:
it lived in the server log only, and `events.jsonl` showed
`on_research_finished` for `automation-science-pack` with no research ever
started. The first proof run here (`run-1788619699-45661`) had exactly that
shape -- 166 actions, `automation` done at tick 39368, and not one line
saying how the trigger technologies had been earned.

It now travels the same road a death does: `ResearchTriggerEvent`
(`crates/core/src/factorio/world.rs`) is parsed in
`crates/core/src/process/output_parser.rs`, queued on `FactorioWorld`, and
drained by `record.research_triggers()`
(`crates/scripting_lua/src/globals/record.rs`) into
`EventKind::ResearchTriggerEmulated { technology, trigger, item, entity,
needed, count }`, mirrored in `app/src/api/types.ts` and the OpenAPI
snapshot. `scripts/research_run.lua` calls it beside `record.deaths()`.
**The tick on the record line is the flush tick** (`recorder.not_before`),
exactly as for a death: the mod's own tick is in the server log, and the
record says only that it happened before the batch that flushed it.

## The live run

`run-1788620523-34289`, `workspace/headless-h/runs/`, four character bots at
5x, `--seed 31337 --new`, `research_run.lua`, `plan_created.bots ==
[1, 2, 3, 4]`, `RUN FINISHED state=done`, 40,856 game ticks. The machine's
load was 10-22 for most of it (other agents building), so it is a proof,
not a timing.

`events.jsonl`:

```
{"tick":23530,"kind":"research_trigger_emulated","technology":"steam-power","trigger":"craft-item","item":"iron-plate","entity":null,"needed":50,"count":50}
{"tick":23530,"kind":"research_trigger_emulated","technology":"automation-science-pack","trigger":"craft-item","item":"lab","entity":null,"needed":1,"count":1}
```

The server log, in the mod's own ticks:

```
§6409§on_research_finished§                       electronics -- no emulation line: the game fired it from furnace output
§17340§research_trigger_emulated§{"technology":"steam-power", ... "produced":50}
§22620§research_trigger_emulated§{"technology":"automation-science-pack", ... "item":"lab", "produced":1}
§41144§on_research_finished§                      automation
```

Three things to read off that: `electronics` came from the game alone, which
is the furnace finding above happening in a real run; `steam-power` came from
the sweep, which read 50 in the statistics before the game's own check did
(the redundancy, and its bound); and `automation-science-pack` completed
5,280 ticks after `steam-power`, its last prerequisite, from the hand-craft
tally -- the ordering the prerequisite gate exists to enforce, and the one
act in this run the game would never have counted.

## What remains out of reach for a headless run, and why

- `capture-spawner` (`biter-egg-handling`) and `create-space-platform`
  (`space-platform`): no action performs the act. An emulation would be a
  grant.
- `build-entity` (`space-science-pack`): emulated, but the only entity it
  names cannot be placed on Nauvis, so the emulation is exercised by the stub
  tests and by a live RCON check, not by a plan.
- Every off-planet `mine-entity` and `craft-item`: the game fires the former
  itself and the sweep handles the latter, but the acts need the planet.
