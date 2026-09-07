# A pump that knows its own speed — and two keys that did not know their surface

2026-09-07. Session `pump-speed`, branch `a-pump-that-knows-its-own-speed`, off
`910e36d9`. Oracle: the 6:39:53 Space Age rocket-launch save, dumped as
`workspace/wrload/scripts/wr-census-status.json` (2.94 GB).

Predecessor: `2026-09-07-a-well-knows-its-own-yield.md`, which measured water as
the largest wrong number in the model — 600/min modelled against 68,250/min
eaten, a 114× gap — and **handed it over rather than guessing**, because the
number lives behind a runtime method the mod did not call.

**Headline: 1.066 → 0.959, and every digit of it is one prototype field that
was not on the wire.** The `OffshorePump` arm had emitted a flat `1.` fluid per
second since the file was written. The game says **20 per tick**.

---

## 1. `get_pumping_speed()` is a METHOD, and the unit is per TICK

Both facts measured on a live 2.1.17 server rather than recalled. One RCON
command, one reply:

```
offshore get_pumping_speed=20   pump get=20
attribute_ok=false
attribute_val=LuaEntityPrototype doesn't contain key pumping_speed.
```

**The method.** `LuaEntityPrototype::get_pumping_speed(quality)`, subclasses
`OffshorePump` and `Pump`, returns `double`. There is no `pumping_speed`
*attribute* at all — the read raises, and that error is precisely what the mod's
`pcall` around every prototype read swallows. Read as an attribute the field
would have arrived `None` for every prototype in the game with nothing anywhere
saying it should not have, which is how `crafting_speed` was nil for 1,028 live
prototypes. Five accessor pairs this week went attribute/method one each way.

**The unit, and how it was checked.** Three sources, the third of which is the
one that would have caught an error in the other two:

1. The prototype stage documents `pumping_speed` as *"How many units of fluid
   are produced per tick"*.
2. `base/prototypes/entity/entities.lua` writes `20` for `offshore-pump` and
   `20` for `pump`; the runtime method answers the same `20`, **not 1,200**, so
   it did not convert on the way out.
3. **Measured against the running game.** An offshore pump piped into a storage
   tank on seed 31337:

   ```
   tick=17424  water= 9624.02
   tick=17627  water=13588.87     +3964.84 over 203 ticks = 19.53 /tick
   tick=17830  water=17553.71     +3964.84 over 203 ticks = 19.53 /tick
   ```

   **19.53 per tick, 97.7% of 20**, the pipe run taking the rest. Read as per
   *second* the same `20` is 60× wrong and every doc string above would still
   have agreed. That is exactly the shape of the units bug the predecessor
   fixed: `mining_speed / mining_time` counts mining *operations*, and every
   solid ore yielding one item per operation is what hid it for months.

The unit is named in the mod's own comment, in `FactorioEntityPrototype`'s doc,
and in the ×60 helper `pumping_speed_per_second`, which is the single place the
conversion happens so no call site can pick a different one.

## 2. Not `1200.` in the arm — the field, plus a vanilla fallback for old worlds

`FactorioEntityPrototype::pumping_speed: Option<f64>` is fluid per **tick**, as
the game gives it. `FlowGraph::pumping_rate_per_second` reads it and falls back
to a table keyed by **vanilla prototype name** when it is `None`.

**The fallback is not the hard-coding this project rules against, and the
distinction is worth stating.** It is the same shape `crates/planner/src/state.rs`
already uses for `maximum_wire_distance`: a `Some` from the wire always wins, the
table fires only when *the sender did not say*, and **a pump vanilla has never
heard of gets `None`, no flow edge, and a warning naming it** — rather than
inheriting an offshore pump's throughput. Writing `1200.` into `update` with no
field behind it would be the defect; writing `1.` into it, which is what the file
did, was a 1,200-fold one.

The fallback is not academic: **every world dumped before today carries no
`pumping_speed`, the 2.94 GB oracle included**, so it is the path the whole
table below runs through.

## 3. The table

Every column from `production_rates_of_a_dumped_world`, **one binary, one dump**,
`entity_graph.connect()` re-run after loading. The "before" column is this branch
with the fix mutated out (`Some(1.)` in the vanilla table), not quoted from the
predecessor — and **it reproduces the predecessor's published figures to the last
digit** in all four aggregates and all fourteen rows, which is what makes the
delta readable.

| item | game /min | before (sustained) | ratio | **after (sustained)** | **ratio** |
|---|---:|---:|---:|---:|---:|
| copper-cable | 22,367 | 2,992.1 | 0.13 | **4,184.7** | **0.19** |
| iron-ore | 15,247 | 16,500.0 | 1.08 | 16,500.0 | 1.08 |
| iron-plate | 15,170 | 9,230.5 | 0.61 | 9,234.0 | 0.61 |
| copper-ore | 15,157 | 15,870.0 | 1.05 | 15,870.0 | 1.05 |
| copper-plate | 15,147 | 1,973.5 | 0.13 | **2,529.9** | **0.17** |
| electronic-circuit | 6,694 | 857.2 | 0.13 | **1,270.2** | **0.19** |
| coal | 4,096 | 5,580.0 | 1.36 | 5,580.0 | 1.36 |
| plastic-bar | 2,846 | 319.7 | 0.11 | **284.5** | **0.10** |
| stone | 1,775 | 1,740.0 | 0.98 | 1,740.0 | 0.98 |
| steel-plate | 1,213 | 1,096.2 | 0.90 | **1,005.7** | **0.83** |
| advanced-circuit | 942 | 105.1 | 0.11 | **93.5** | **0.10** |
| iron-gear-wheel | 578 | 658.2 | 1.14 | **670.3** | **1.16** |
| stone-brick | 450 | 549.5 | 1.22 | **661.6** | **1.47** |
| processing-unit | 249 | 11.7 | 0.05 | **33.3** | **0.13** |
| **mean abs log error** | | | **1.066** | | **0.959** |
| *nameplate, unconstrained* | | | *0.306* | | *0.306* |
| *descent* | | | *1.253* | | *1.006* |
| *guessed bill* | | | *0.893* | | *0.812* |

**The mechanism is one, and it is not in the nameplate column.** The provenance
ledger moves in exactly one row, measured in both states on this binary:

```
                item     made BEFORE      made AFTER      known/min    unknown/min
               water           600.0        720000.0        44700.0        23550.0
           crude-oil         13800.0         13800.0        13333.3            0.0
           heavy-oil          1500.0          1500.0        13800.0            0.0
           light-oil          9900.0          9900.0        58800.0         1170.0
       petroleum-gas          9300.0          9300.0        36000.0            0.0
       sulfuric-acid          6000.0          6000.0         3600.0            0.0
           lubricant           600.0           600.0         1890.0            0.0
```

Ten offshore pumps at `1.`/s made 600/min against 68,250/min eaten — **74.5×
short**. At the game's own rate they make 720,000/min, a 10.5× surplus. Nothing
else in the ledger moves, and `nameplate` — the unconstrained column — does not
move at all, which is the check that no machine, recipe or other rate was
touched. **The whole delta is the rationing pass**: water stops being the binding
constraint on every machine that drinks it, and the allocation of everything
downstream is redistributed.

**Five rows got worse and they are in the same table as the wins.** That is not
a caveat added afterwards — it is what a shared allocator does. `stone-brick`
1.22 → 1.47 continues the same walk this file has now recorded four times: its
old value was rationing landing by luck on the right side of an honest
over-prediction (its nameplate figure is **3.33**, unmoved), and relieving
rationing walks it back towards the truth about the model. `plastic-bar`,
`steel-plate` and `advanced-circuit` lose share of contended inputs that other
lines can now take; `iron-gear-wheel` barely moves. `processing-unit` 0.05 → 0.13
and `copper-cable`/`electronic-circuit` 0.13 → 0.19 are the wins, and they are
larger.

**What is honestly left.** 0.959 is still dominated by the term nothing in this
graph represents — **idleness**, worth +16% to +23% on the record base, against
coverage at ~1.6% — and the fourteen items are not independent: ten sit
downstream of the two fluids. The pumpjack yield factor (`normal_resource_amount`
and the infinite-resource flag) is still handed over and still unmeasurable
here, because crude supply already clears crude demand.

## 4. `tile_chunks` now carries its surface

Reported by the predecessor session, not fixed there:

> `tile_chunks` is keyed `chunk_x.."/"..chunk_y` with **no surface**, so a
> Nauvis chunk (0,0) would suppress the tiles writeout for chunk (0,0) on
> **every other surface** — silently.

It is `ground_chunk_first_seen(surface, chunk_x, chunk_y)` now, keyed
`surface.name.."/"..chunk_x.."/"..chunk_y`, named and separate from
`on_chunk_generated` so the scoping can be tested across two surfaces **without
lifting the Nauvis guard**, which no test here does.

Two tests, and the pairing is the point. `ground_is_written_once_per_chunk_per_surface`
asserts Vulcanus (0,0) is first-seen *after* Nauvis (0,0) — but pairs it with
the suppressions that must still happen, from the same table, because **a key
that suppresses nothing passes the Vulcanus row on its own**. That is the
vacuity `ground_names_its_own_surface.rs` was built to avoid, and it is the same
failure that let an aliasing test pass because the other surface was never
occupied at that tile. `ground_from_a_second_surface_survives_the_first_surfaces_chunk`
does it end-to-end through the real `writeout_tiles`, re-pointing
`game.surfaces['nauvis']` at the second surface — which is precisely the world
the guard's removal will create — and checks the emitted header reads
`0,0;32,32;vulcanus:`, so the parser routes it somewhere other than Nauvis.

This is a class this project has catalogued: **a key that does not carry its own
scope.** `POLE_WIRE_REACH_TILES` was documented as "a small pole's" and used as
every pole's; `0.84` was "the accumulator ratio" and used as the average.

## 5. `storage.map_area` is per surface — and NOTHING READS IT

Also reported, also fixed: it was one `{x1,y1,x2,y2}` across every surface, and a
platform's coordinates are its own frame, so the union describes no region of
anything. `map_area_of(surface)` keys it by surface name and **migrates itself**:
`storage` survives save/load, an `x1` at the top level is the tell of a
pre-today save, and the old union is discarded rather than split, because there
is no honest way to attribute a union to a surface.

**One premise in the handover does not survive contact with the code, and the
correction is small.** The ±512 clamps are applied **per chunk**, not to the
union — they `return` before the box is touched at all. The shared box was the
whole defect.

**The bigger finding is that the field has no reader.** Not the mod, not
`crates/`, not `scripts/`, not the frontend — checked by grep, which finds only
the one write, the `on_init` initialiser, and four test stubs. **The mutation
that reverted this fix came back GREEN**, and that was the useful result rather
than a nuisance: an unobservable field is exactly the no-caller shape that let
`flow_graph`'s own hard-coded rates go unvalidated for months, and it is the same
reason `method::connect`'s geometry defect survived four reviews. Two shape
tests now exist and kill the mutation; **a shape test is not a reader and does
not pretend to be one.**

## 6. What the Nauvis guard is now waiting for

The guard's own comment listed four reasons it still stands, two of which fail
silently. **Both silent ones are closed** (this session). What remains, and
neither is silent:

3. The initial-discovery replay is hard-coded to `game.surfaces[1]` at both
   ends. On a *loaded* save this has never once run — a loaded save generates no
   chunks, and `surface_chunk_dropped` read 0 across a whole ten-surface run.
4. `OutputParser::on_init` connects only the default surface's graph, and
   `FactorioWorld::only_surface()` refuses on a multi-surface world, so
   `factorio-bot lua` cannot start against one.

**The guard was not lifted and must not be** until 3 and 4 are done. That is a
separate task and an owner decision.

## 7. Verification

- **`nix develop -c cargo test --workspace` → exit 0**, taken from the command
  and not from a pipeline. **108** `test result: ok` blocks, 0 failed — the same
  count as master, because these are new tests in existing files rather than new
  files. `cargo clippy -p factorio-bot-core --all-targets -- --deny warnings`
  clean. `rustfmt --edition 2024` on each touched file, never `cargo fmt`.
- **`cd app && pnpm lint` → exit 0** and `pnpm run test:coverage` → 53 files,
  942 tests, exit 0. The API surface *is* touched: `FactorioEntityPrototype` is a
  published schema, so `app/src/api/openapi.snapshot.json` is regenerated (+8
  lines). `types.ts` declares no `FactorioEntityPrototype`, so the contract
  spec has nothing to mirror.
- **The binary was interrogated, not trusted**: `include_dir!` has no
  `rerun-if-changed`, so a rebuild can ship the *old* mod and the new field then
  reads uniformly absent exactly as if the game never sent it.
  `strings -a target/release/factorio-bot | grep get_pumping_speed` → 4 hits;
  `ground_chunk_first_seen` → 3; `map_area_of` → 4.
- **Mutations**, applied one at a time and reverted, each **refusing unless its
  substitution matched exactly once** (`scratch-mutate.py`):

  | mutation | kills |
  |---|---|
  | the offshore pump arm goes back to a flat `1.`/s | `a_boiler_carries_its_water_through_to_the_steam_engine`, `a_pump_the_world_said_nothing_about_falls_back_only_when_vanilla_knows_it` |
  | the per-tick value is emitted as if it were per second | `a_pumping_speed_crosses_from_per_tick_to_per_second`, `a_pump_produces_what_its_own_prototype_says_per_tick` |
  | the wire is ignored and vanilla is always used | `a_pump_produces_what_its_own_prototype_says_per_tick` (alone) |
  | an unknown pump inherits an offshore pump instead of nothing | `a_pump_the_world_said_nothing_about_falls_back_only_when_vanilla_knows_it` (alone) |
  | the mod reads the attribute instead of the method | `a_serialised_pump_carries_its_speed_per_tick_from_the_method` (alone) |
  | the ground chunk key drops its surface | `ground_is_written_once_per_chunk_per_surface`, `ground_from_a_second_surface_survives_the_first_surfaces_chunk` |
  | the ground chunk key drops its coordinates | `ground_is_written_once_per_chunk_per_surface` (alone) |
  | `map_area` goes back to one box across all surfaces | `each_surface_charts_its_own_bounding_box`, `a_pre_surface_map_area_migrates_instead_of_raising` |
  | the pre-surface flat box is left in place | `a_pre_surface_map_area_migrates_instead_of_raising` (alone) |
  | the box is reset on every call instead of kept | both `map_area` tests |

  **The `map_area` mutation came back GREEN the first time** — see §5. Three
  tests were written for it and the three mutations now kill them.

  Every accidental-value assertion is paired with a non-accidental one from the
  same computation: vanilla's `20 → 1200` against a modded `30 → 1800` no
  vanilla table holds and against `0.5 → 30`, which a `/60` cannot produce; the
  vanilla fallback against an unknown pump that must get `None`; the surface key
  against the suppressions it must still perform.

- **A real test-fixture defect fell out of it.** `botbridge_pre_tick_handlers.rs`
  stubbed a surface with no `name`, which `LuaSurface` always has, and both new
  keys raised `table index is nil` on it. Fixed in the stub, where the error was.

- **Four offline baselines, byte-identical, on this branch's release binary**:
  `researched:automation` 176 / 21,784, `producing:automation-science-pack:6`
  316 / 22,457, `producing:logistic-science-pack:6` 441 / 47,478 on `map.json`,
  and `gathered:crude-oil` 2,115 / 317,283 on `map-31337-explored.json`, with
  `map.json` refusing that goal because no oil is charted there — correct, not a
  regression.

  **They prove nothing about this change and are reported for that reason.** The
  flow graph still has no planner caller; nothing here touched a root predicate,
  an entity-graph rule or a recipe. What they rule out is the thing worth ruling
  out: that a change inside `update` leaked into a plan.

## 8. Handed over, not taken

Unchanged from the predecessor and untouched here:

- **A resource's infinite-yield triple** — `normal_resource_amount`,
  `infinite_resource`, `minimum_resource_amount`, all three *attributes* on
  2.1.17 — so a pumpjack's rate can carry the well's yield. The numerator
  (`FactorioEntity::amount`) is already sent. Worth roughly 8.83× on crude oil
  on this base, which the fourteen-item table cannot see because crude supply
  already clears crude demand.
- **`LuaFluidBoxPrototype::filter`**, so a boiler's output can be named `steam`
  rather than carried through as `water`. The `Boiler` arm's misnomer is pinned
  by a test rather than hidden — and note that **the boiler test's magnitude is
  now 1,200, not 1**, because the pump upstream of it finally reports its rate.
- `PipeConnectionDefinition::direction`; a furnace's recipe (all 1,215 report
  `null`); off-map supply needing a second surface.

New from this session:

- **`storage.map_area` needs a reader or it needs deleting.** It is written on
  every chunk and read by nothing, which makes its correctness unfalsifiable by
  anything but a shape test.
