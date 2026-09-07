# A machine says what it is doing: `FactorioEntity.status`, and the distribution it measures

2026-09-07. Session `entity-status`, branch `a-machine-says-what-it-is-doing`,
off `b5b1e072` (the `a-machine-standing-still` merge). Handover taken from §5 of
`2026-09-07-a-machine-standing-still.md`, which specified the field and said
why the modelling could not proceed without it.

**One field on the entity record, and a fresh dump of the world-record save to
turn it into evidence.**

---

## 1. `status` is an ATTRIBUTE, and that was checked rather than assumed

`LuaEntity.status` in this install's `runtime-api.json` (2.1.17):

```
read_type: defines.entity_status   optional: true   subclasses: (none)
```

So it is a plain read and it is safe on **any** entity -- a tree, a chest, a
belt, a furnace. It is not a method.

That distinction is the reason the check was made rather than skipped. Reading
a *method* as an attribute does not raise usefully: it hands back a function,
`helpers.table_to_json` drops it, the mod's `pcall` swallows whatever else
happens, and the field is simply **absent with nothing anywhere to say it
should not be**. That is how `crafting_speed` arrived nil for 1,028
prototypes and what `get_supply_area_distance()` nearly repeated.

The same check found the opposite answer for the second field this session
landed -- see §5.

## 2. The name crosses the wire, never the number

`ENTITY_STATUS_NAMES` already existed in `control.lua`, built by inverting
`defines.entity_status` for the sample stream. It has **moved to `types.lua`**
as `entity_status_name(status)` and `control.lua`'s `machine_row` now calls it:
two inversions of one enum in one mod is two things to keep in step, and the
sample stream and the entity record must never disagree about what a status is
called.

A value this build cannot name is written `unmapped_<n>` rather than dropped --
the rule `transport_line_name` already followed. A raw enum integer would be a
number whose meaning lives in a table nobody joins, and it could silently change
meaning across Factorio versions.

## 3. Absent stays distinguishable from every named state

`#[serde(default)] pub status: Option<String>` on `FactorioEntity`.

- A tree, a chest and a belt have **no status concept**: the mod sends no key,
  which reads as `None` -- *the sender did not say*.
- A machine that is **stopped** has a name for being stopped:
  `no_ingredients`, `no_power`, `full_output`,
  `waiting_for_space_in_destination`.
- Every archived record and world dump written before today also reads `None`.

Nothing may default the absent case to `working`; that would invent a duty
cycle out of an archive that never measured one. This is the same distinction
the input inventory needed a day earlier, where "holds ore and is not smelting"
had to stay separate from "no ore ever arrived", and it is pinned from both
ends: `an_entity_with_no_status_says_nothing_rather_than_working` (Rust, on the
real `types.lua`) and `reads a missing status as null, which is not the same as
working` (TypeScript, on `parseFactorioEntity`).

**It is written OUTSIDE the `omit_inventories` guard, deliberately.** That guard
is the `writeout_entities` bulk path -- every entity of every chunk, contents
withheld -- and it is the *only* path that fills the world model a dumped world
is built from. A status gated behind it would be unmeasurable in exactly the
artefact the question is asked of. One short string is not an item-by-item
inventory.

## 4. The measurement: the world-record base, re-dumped

`wr-census.json` was taken on 2026-09-06, before `input_inventory`,
`transport_lines` or `status` existed. **A field cannot be backfilled into an
archive**, so the base was re-dumped: `workspace/wrload` (its own workspace,
ports 4390/34290), the 6:39:53 Space Age rocket-launch save, read-only, script
`scripts/wr_status_census.lua`, written to `wr-census-status.json` and
deliberately **not** over the old census, which is what every number in
`2026-09-06-what-the-record-base-knows.md` was computed from.

`game.tick` 1,443,169. Model 39,237 entities, identical to the previous census.
Dump 2.94 GB, scanned as a stream.

**Furnaces -- 1,222, every one reporting:**

| status | count | share |
|---|---:|---:|
| `working` | 1,072 | **87.7%** |
| `no_ingredients` | 72 | 5.9% |
| `full_output` | 72 | 5.9% |
| `no_fuel` | 6 | 0.5% |
| (no status key) | 0 | 0% |

**Mining drills -- 1,544, every one reporting:**

| status | count | share |
|---|---:|---:|
| `working` | 1,292 | **83.7%** |
| `waiting_for_space_in_destination` | 173 | **11.2%** |
| `no_minable_resources` | 71 | 4.6% |
| `missing_required_fluid` | 6 | 0.4% |
| `no_fuel` | 2 | 0.1% |
| (no status key) | 0 | 0% |

By prototype: `steel-furnace` 1,196 (1,048 working, 72 `no_ingredients`, 70
`full_output`, 6 `no_fuel`), `stone-furnace` 26; `electric-mining-drill` 1,492
(1,275 working, 140 `waiting_for_space_in_destination`, 71
`no_minable_resources`, 6 `missing_required_fluid`), `burner-mining-drill` 28
(26 of them `waiting_for_space_in_destination`).

Three things this says that the previous session could only assert:

- **87.7% and 83.7% confirm the 87% / 82% quoted from live RCON queries**, and
  now they are in the record rather than in a transcript.
- **Back-pressure is 11.2% of the drill fleet and is now readable**, which is
  the half §3 of the previous note said nothing derivable from the graph could
  see. The count is 173 here against the 194 quoted from a live query a day
  earlier: a different instant of a running base, not a discrepancy.
- **The two halves of furnace idleness are the SAME SIZE.** 72
  `no_ingredients` and 72 `full_output` -- shortage and back-pressure, exactly
  balanced. The supply balance that landed yesterday models the first and not
  the second, so on furnaces it can close at most half the gap.

And two rows nobody asked for that are worth more than the ones that were:

| | working | full_output / waiting for space | short of input |
|---|---:|---:|---:|
| assembling-machine (1,093) | 69.1% | 21.7% | 8.9% |
| inserter (5,673) | 25.6% | **49.3%** | 24.9% |

**The demand side named in §4 of the previous note is now measurable**: 30.9% of
this base's assemblers are not running, which is precisely the fictional
nameplate demand that drags `processing-unit` to 0.54 and `advanced-circuit` to
0.70. And half the base's inserters are waiting on the thing downstream -- this
base is back-pressure-limited far more than it is supply-limited. 76.2% of its
labs read `missing_science_packs`.

**What this is not.** A dump is one instant. These are instantaneous statuses at
tick 1,443,169, not a duty cycle integrated over time; using them as one assumes
the base is in steady state, which is plausible for a finished base and is an
assumption, not a measurement. Bounding a real duty cycle needs samples over a
tick window.

## 5. `volume`, taken in the same seam -- and it is a METHOD

Asked for mid-session, for `Goal::Stored { fluid, amount, where_ }`: without a
capacity the only way to size storage is a hard-coded table, which is the same
mod-compatibility defect as `pole_supply_half_extent` and the copied `1/3.2`
smelt rate the world-record base falsified (exactly 2.0x low on every plate,
because 1,196 of 1,222 furnaces are steel).

`LuaFluidBoxPrototype` in 2.1.17 has **no `volume` attribute at all**. It has
`get_volume(quality?)`, a method -- the opposite answer to §1, from the same
check. So `FactorioFluidBoxPrototype.volume: Option<f64>` is filled by *calling*
it, and a prototype that cannot answer sends no key: "this game cannot say" and
"this box holds nothing" are different claims, and a planner sizing storage must
not read the first as the second.

## 6. What is in the tree, and what was verified

- `mods/BotBridge/types.lua`: `entity_status_name`, `record.status`,
  `record.volume`. `mods/BotBridge/control.lua`: `machine_row` now calls the
  shared resolver.
- `crates/core/src/types.rs`: `FactorioEntity::status`,
  `FactorioFluidBoxPrototype::volume`.
- `app/src/api/{types.ts,game.ts}` and the regenerated
  `openapi.snapshot.json` -- the seam failed from both ends, as designed.
- `crates/planner/src/method/gather.rs`: two `volume: None` in test fixtures,
  the mechanical cost of a new struct field.
- `app/src/components/map/MapEntities.spec.ts`: **`pnpm lint` was already red on
  `master`** -- that helper never got `input_inventory` or `transport_lines`
  when they landed, and `vitest` does not type-check, so `tsc --noEmit` was
  failing while every test passed. Fixed in passing, with the three fields.
- `scripts/wr_status_census.lua`: the read-only re-dump.

Verification: `cargo test --workspace` green (2,737 passed, 0 failed, exit code
taken from the command and not from a pipeline); `cargo clippy --workspace
--all-features --all-targets -- --deny warnings` clean; `pnpm run test:coverage`
942 passed; `pnpm lint` clean.

**Ten falsifications, each confirmed to break exactly one test**: the nil guard
dropped (absent becomes `unmapped_nil`), the `unmapped_` fallback dropped, the
inversion forgetting `working`, the status moved inside `omit_inventories`, the
guard branch losing its status, `volume` read as an attribute instead of called,
a missing `get_volume` defaulting to zero, and on the TypeScript side a missing
status defaulting to `working`, a coerced instead of validated status, and a
renamed state.

**The three offline baselines are byte-identical on the same binary**, re-run
after the change: `researched:automation` 176 / 21,784,
`producing:automation-science-pack:6` 316 / 22,457,
`producing:logistic-science-pack:6` 441 / 47,478. A serialisation field must not
move a plan, and it did not.

## 7. What this unblocks, and what it does not

The three things §5 of the previous note listed are now *measurable* and none of
them is *done*: the observed duty cycle, the back-pressure term, and the
demand-side fix. `sustained_production_rates()` still throttles on shortage only
and `production_rates()` is still the honest upper bound. The modelling on top
of this reading is the next task and it is somebody else's.

One caveat for whoever takes it: **the flow graph is built from the world model,
and the world model's entities are a snapshot taken when the chunk was
ingested.** `EntityGraph::add` refuses to re-add over an occupied position, so a
status stored there is as stale as the inventories beside it. For a loaded save
read once that is exactly right and is what the numbers above are. For a live
run it is not, and "refresh on entity add/remove" is the same piece of work the
flow graph already needed.
