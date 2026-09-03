# Offline replan: planning a captured world with no game

**Status:** design only. Nothing below is built.

**Why it is a design and not a branch:** the harness itself is small — the
survey below puts the happy path at well under a hundred lines, and every piece
already exists somewhere in the tree. What is *not* small is making it
trustworthy. A replan tool that silently diverges from what the real planner
does would be worse than none, because it would be believed. Three fidelity
gaps are unavoidable from a run archive alone, and the design's real content is
what the tool does about them.

## The problem it solves

`crates/planner` is pure and deterministic: no I/O, no async, no wall clock,
ordered collections only, byte-identical plans for identical inputs. So most of
what was chased on 2026-09-03 by launching Factorio — furnace churn, bill
duplication, bot-1 concentration, drill-versus-hand-mining — is visible **in the
plan alone**. Each of those investigations cost a ~25 minute run and several
cost more than one. `steps/bot` is the number that has mattered throughout, and
it is a property of the plan, not of the execution.

The bar: `just replan <run-dir>` prints, in seconds, the plan a given commit
would produce for that world — steps, makespan, `steps/bot`, `planned ticks/bot`.
That makes "did my change help?" answerable without a game, and A/B across two
commits trivial.

## What already exists, and where

There is no `Goal::expand` method; expansion is a free function. The whole
pipeline is three calls plus a registry:

```rust
let state = PlanState::from_world(Arc::new(world), &bots);   // state.rs:1059
let actor = pick_chain_actor(&state, &bots)?;                // method/mod.rs:410
let net   = expand(&[goal], &state, &registry_for(&bots), actor)?; // method/mod.rs:443
let sched = schedule(&net, &state, &bots)?;                  // schedule.rs:248
```

`registry_for` (`method/have.rs:3101`) is the one to use, not `default_registry`:
it wires `SplitAcrossBots` and `SharedSmelt`, which is the difference between
planning for four bots and planning for one. The production call site to copy is
`crates/scripting_lua/src/globals/goal/mod.rs:901-919`.

`Schedule` (`schedule.rs:152`) already carries everything the report needs:
`ScheduledStep { what: StepKind, bot, start, end }` and `makespan`. So
`steps/bot` and `planned ticks/bot` are `steps_for(bot).len()` and a sum over
`end - start`. `render::mermaid_gantt` and `render::graphviz` exist for dumping.

**Assembling the world offline is a solved problem too** — `attach_world`
(`crates/core/src/factorio/snapshot.rs:132-169`) is the reference recipe, and
three of its four steps are pure data:

1. `world.apply_snapshot(snapshot)` — a `WorldSnapshot` is plain serde
   (`snapshot.rs:68`) and `crates/core/tests/live-2.1.17-world-snapshot.json` is
   a real 765 KB capture of one.
2. `world.players.insert(id, FactorioPlayer { .. })` per bot.
3. `world.update_chunk_entities(entities)`, filtering out `character`.
4. `world.entity_graph.connect()?; world.flow_graph.update()?;`

Replace step 1's RCON call with `serde_json::from_str`, step 2's with a
`samples.jsonl` `bots` row, and step 3's with a `map.jsonl` keyframe, and the
harness is done.

**Four test fixtures have already hand-rolled two-thirds of this, unowned and
duplicated.** `furnace_bank.rs:62` is the cleanest template for the pipeline;
`enclosure_prevention.rs:107-137` (`world_of`) is the prototype for rebuilding a
`FactorioWorld` from `{name, position, direction}` triples plus a prototype
table, and it even redeclares `record::map::EntitySnapshot` field-for-field
rather than importing it. `unreachable_memory.rs` copy-pastes the same
`players.insert(.. ..Default::default())` block five times. A shared harness
should absorb these, but that is a follow-up, not a precondition.

`crates/core/tests/run-1788432181-42528-frozen-bots.json` is the strongest
precedent: it is literally the last `kind: "bots"` line of a run's
`samples.jsonl` stitched to the `game` array of the last keyframe of the same
run's `map.jsonl`. **The input format has already been invented once.**

## The three fidelity gaps

These are the design. None can be closed from a run archive alone, and each
would produce a confidently wrong plan if left unstated.

### 1. Resource amounts are lost — every tile reads as 500

`EntitySnapshot` is `{name, position, direction}` and carries no amount.
`EntityGraph::add` stores `entity.amount`, and a delivery with `None` leaves
whatever is stored alone. So a keyframe reload yields `resource_amount == None`
and `resource_available` substitutes `DEFAULT_RESOURCE_PER_TILE = 500`
(`state.rs:426, 2823`). **Depleted patches become invisible**, and a replan late
in a run will happily plan mining that the real world could not support.

### 2. Trees, water and cliffs are structurally absent

`EntityGraph::add` routes them to `blocked_tree` only, and `snapshot_within`
reads `entity_tree`. `crates/core/tests/README.md:157` states it flatly: *"The
obstacle that froze bots 2 and 3 is not in here, and cannot be."* Consequences:
`enclosure::check` and `find_walled_in` see open ground, and `method::power`'s
`nearest_water_tile` finds no shoreline, so an offshore pump cannot be sited at
all. Any replan involving power is therefore not comparable to a real run.
(`enclosure_prevention.rs` works around this by *synthesising* a tree ring.)

### 3. Which technologies are researched is not recorded

`samples.jsonl`'s force row carries `techs_unlocked` as a **count, not a list**.
The shipped `WorldSnapshot` capture is a fresh game with all 277 technologies
unresearched. So a replan of any milestone past the first plans against the
wrong tech state unless the operator supplies it.

Two lesser gaps: everything outside the keyframe `bounds` does not exist (bounds
are `bounds_around(placements)`, which in run 13 excluded the coal and stone
patches entirely), and `build_distance` / `reach_distance` /
`resource_reach_distance` are not in `samples.jsonl` at all — they would be
hardcoded to the vanilla values or folded in from `live-2.1.17-players.json`.

## The design

### Refuse to be trusted where it cannot be

Every gap above is **declared in the output, on every run, above the numbers** —
the same discipline `just analyse --compare` now applies to provenance. Not a
footnote and not a `--verbose` flag. Concretely the report opens with a fidelity
block:

```
  FIDELITY  (read this before any number below)
  !! resource amounts UNKNOWN -- every ore tile reads as 500; a depleted
     patch is invisible and mining plans will be optimistic
  !! obstacles ABSENT -- no trees, water or cliffs; enclosure reasoning is
     disabled and any goal needing an offshore pump is REFUSED
  ?? research state ASSUMED all-unresearched (from the shipped snapshot);
     pass --techs to override
  ok world rebuilt from keyframe @tick 211506 (1028 entities, bounds ...)
  ok bots from samples @tick 211460 (4 bots, real inventories)
```

And it **refuses outright**, rather than flagging, for the one case where the
gap is not a distortion but a guaranteed wrong answer: a goal whose expansion
reaches `method::power` and therefore `nearest_water_tile`. A plan that cannot
site a pump is not a pessimistic plan, it is a different plan.

### It calls the real planner, never a model of it

The harness imports `expand` and `schedule` and calls them. It contains no
scheduling logic, no cost model and no reimplementation of anything. That is
what keeps "silently diverges" impossible in the *logic*; the world fidelity is
then the only axis of divergence, and it is declared.

### Inputs

```
just replan <run-dir> --goal 'researched(automation)' [--at-tick N] [--world FILE]
```

- `<run-dir>` supplies the world: the last keyframe at or before `--at-tick`
  from `map.jsonl`, and the nearest `bots` sample from `samples.jsonl`.
  `--at-tick` is what makes "replan from the middle of run 8" possible.
- `--world` defaults to `crates/core/tests/live-2.1.17-world-snapshot.json` and
  supplies prototypes, recipes and the force. A fresh capture can be swapped in.
- The goal is parsed from the same string form `goal.plan` takes.
- The new `provenance.json` (landed today) supplies the run's commit and map
  fingerprint, so the report can state *which* commit's plan this is being
  compared against.

### Where the code goes

`crates/planner/src/bin/replan.rs`, with `serde_json` promoted from
`[dev-dependencies]` to `[dependencies]` (one line; it is already a transitive
dep through core, which also re-exports it as `factorio_bot_core::serde_json`).

Rationale: it builds only core + planner — no mlua, no server, no
tokio-console — which is what makes `just replan` answer in seconds rather than
in a link step. It also keeps the tool inside the crate whose purity is the
whole premise, where a change that breaks determinism breaks the tool's own
build.

The alternative, a `factorio-bot replan` subcommand in `app/src-tauri`, was
rejected on build time alone.

### What it can and cannot reproduce faithfully

**Faithful:** step count, makespan, per-bot assignment, planned ticks per bot,
the whole DAG shape, and every decomposition choice. These are exactly the
numbers the 2026-09-03 investigations wanted, and they are computed by the real
planner from a real captured world.

**Not faithful:** anything downstream of resource depletion, anything needing an
obstacle, anything needing power, and anything gated on research state the
operator did not supply. Also, by construction, **nothing about execution** — a
plan says what was intended; the 31.8% of a run that was walking and the 39.1%
that was one bot waiting on a furnace are properties of the run, not the plan,
and `just analyse` remains the tool for those.

## Follow-ups this unlocks, deliberately out of scope

- Absorbing the four duplicated test fixtures onto the shared world builder.
- A **viability screen** for candidate seeds — reject a map with no shoreline
  that fits a pump/boiler/engine, which really did refuse a run on 2026-09-03
  (`the nearest water is 66.7 tiles away`). This becomes cheap once scoring a
  world costs seconds. Note it is a *screen*, not a search: searching for a seed
  that scores well is explicitly rejected (see `just bench` and CLAUDE.md), and
  `roll-seed` stays gated off.
