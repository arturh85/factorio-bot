# There is no mall — and machines are the wrong target

2026-09-08. Offline only, `target/release/factorio-bot` (built 19:35), map
`workspace/scripts/map.json` (seed 31337, t=0, fingerprint `c161fa3f437221d0`).
Action counts are deterministic and load-independent; the box ranged 5–36
during these measurements. Baseline reproduced CLAUDE.md exactly:
`researched:automation` = **176 / 21,784**.

## 1. The premise is right, the target is wrong

**"There is no mall" — CONFIRMED**, in one line:

```
plan --goal producing:assembling-machine-1:1
  → the goal did not plan: no method can satisfy goal
```

**"A large share of the plan is hand-crafting its own machines" — FALSIFIED.**
Composition of `producing:logistic-science-pack:6`, 4 bots (441 actions, 584
steps, 135,573 planned ticks):

| verb | ticks | % |
|---|---:|---:|
| **craft** | **39,420** | **29.1%** |
| walk | 37,280 | 27.5% |
| mine | 33,120 | 24.4% |
| research | 17,400 | 12.8% |

Crafting is the largest verb — but of its 39,420 ticks, **machines are 1,920
(4.9% of crafts, 1.4% of the plan)**. The cost is elsewhere:

| item | qty | ticks | % of crafts | cell-capable today? |
|---|---:|---:|---:|---|
| automation-science-pack | 85 | 25,500 | 64.7% | **YES** |
| iron-gear-wheel | 191 | 5,730 | 14.5% | no (1 ingredient) |
| copper-cable | 102 | 3,060 | 7.8% | no (1 ingredient) |
| electronic-circuit | 66 | 1,980 | 5.0% | **YES** |
| inserter | 34 | 1,020 | 2.6% | no (3 ingredients) |
| *all machines* | 42 | 1,920 | 4.9% | mostly no |

**A facility that makes machines from machines addresses 1.4% of this plan.**
The compounding lives in packs and intermediates.

## 2. The capability exists and is unreachable

`BuildAssemblyCell` **can** build a red-science cell — `producing:
automation-science-pack:6` plans 316 actions. The green plan hand-crafts 85 red
packs anyway, because nothing routes there:

- `Researched` deals its pack bill as `Goal::Have{.., Holder::Share(bot)}`
  (`have.rs:5375`).
- `registry_for` (`have.rs:6644`) lets **`HandCraft` claim every `Goal::Have`
  with a crafting recipe**. No method between `Smelt` and `HandCraft` ever
  considers standing a cell up instead.
- Nothing in the tree emits `Goal::Producing` — it is constructed only by the
  CLI/Lua goal parsers and tests.

This is the `Goal::Gathered` shape again: a working method reachable only from
outside the planner. **The decision site is the registry order, not `HandCraft`'s
body.**

## 3. Why `assembly_spec` refuses almost everything

`assembly_spec` (`assemble.rs:432`) destructures **exactly two** ingredients
(`let [(a,_),(b,_)] = ingredients.as_slice() else { return None }`), requires
**exactly one** of them to be a craftable intermediate, and a tie returns `None`
(`:453`). Over the 190 crafting recipes in the dump:

| ingredients | recipes | |
|---:|---:|---|
| 2 | **58 (31%)** | only these are even considered |
| 3 | 78 | refused |
| 1 | 15 | refused |
| 4–6 | 38 | refused |

Three distinct refusals, each verified against the recipe table:

- **arity** — `assembling-machine-1` (3), `inserter` (3), `lab` (3),
  `electric-mining-drill` (3), `iron-gear-wheel` (1), `pipe` (1).
- **the tie rule** — `offshore-pump` (gear + pipe, both depth 1) → `Equal =>
  None`.
- **both-raw** — `steel-furnace` (steel-plate + stone-brick, both smelted).

`MAX_FEED = 2` is set by the *pole*, not by preference: one small pole at
`(-1,2)` cannot light a third feed row.

## 3a. The real threshold is the unlock bill, not the crossover

Verified in the dump's own technology table:

```
automation             research_unit_count = 10   (unlocks assembling-machine-1)
logistic-science-pack  research_unit_count = 75
                                          -----
                                             85   = the measured hand-crafted red packs
```

Both cost 1 red pack per unit, so the 85 is not a coincidence — it is the sum of
two research bills, and **10 of them are unavoidable**: you cannot build an
assembling machine before `automation`, and `automation` costs exactly 10 red
packs. The other **75 are waste**, ~22,500 of the 25,500 craft ticks.

> **Predicate:** hand-craft the `research_unit_count` of the technology that
> unlocks the machine that would otherwise make this item; machine-make the
> rest.

Prototype-derived, no tuned constant, mod-surviving. `N*` (§4) stays as the
fallback for items with **no unlock boundary** — gears and cable, which are
gated by `assembly_spec` arity rather than by research. The two are
complementary: the unlock bill answers *how many must be hand-made*, `N*`
answers *is a cell worth building at all*.

## 4. The economics invert the intuition

From prototypes, not tables. `character.crafting_speed = 1.0`;
**`assembling-machine-1.crafting_speed = 0.5`.**

> **Every assembling-machine-1 craft is exactly 2× slower per item than a bot's
> hands.** A mall never pays on throughput. It pays only because machine time is
> free and *bot time is the scarce serial resource*.

One cell costs **~2,100 bot-ticks (35 s of one bot's hands)**: 810 to craft 2
AM1, 690 for 5 inserters, 120 for 4 chests, 60 for a pole, 360 placing, 60
charging. Crossover `N* = 2100 / hand_ticks_per_item`:

| item | hand t/item | N* items before a cell pays | plan needs |
|---|---:|---:|---:|
| automation-science-pack | 300 | **7** | **85** ✅ |
| logistic-science-pack | 360 | **6** | — |
| lab / electric-mining-drill | 120 | 18 | 2 |
| gear / circuit / inserter / AM1 | 30 | 70 | 191 / 66 / 34 / 4 |
| copper-cable / transport-belt | 15 | 140 | 102 / 4 |

**Red science clears its crossover by 12×. Machines do not clear it at all.**
Gears (191) and cable (102) are near theirs but are 1-ingredient recipes that
`assembly_spec` cannot express.

## 5. Blockers found live

- **The power invariant refuses cells at scale.** `producing:automation-science-
  pack` plans at 6/min (316 actions) and 12/min (317), and **refuses at 30/min**:
  `the steam-engine this plan places at [40.5,-5.5] generates 900 kW and no pole
  reaches it`. Any mall draws more than one engine's worth, so it hits this
  immediately.
  **It is not a mis-sited second engine.** The engine named in the refusal,
  `[40.5, -5.5]`, is placed at the *identical tile* in the 6/min and 12/min
  plans, which pass. There is only ever **one** engine (the error itself says
  900 kW, one engine's worth). So the failing entity is the *first* engine, and
  what changes with rate is the **poles**: at 6/min the plant's pole stands at
  `[38.5, -7.5]`, whose 5×5 supply area `(36,-10)–(41,-5)` overlaps the engine's
  collision area `(39.25,-7.85)–(41.75,-3.15)`. At 30/min that coverage is gone.
  Two candidates remain and I did not separate them (the file is not mine):
  `pole_chain` siting the plant pole differently once more cells crowd the
  ground, or `audit`'s `a_pole_reaches` not seeing a pole the plan does place.
  **Note the dump has `supply_area_distance = None`** — the field postdates it,
  so the planner is using its vanilla fallback here; a re-dump is worth doing
  before concluding anything about coverage arithmetic.

- **t=0 enablement** (verified in the dump): `inserter`, `assembling-machine-1`,
  `small-electric-pole`, `electronic-circuit` are all `enabled=False`; only
  `burner-inserter`, `iron-chest`, `transport-belt` are enabled. `gate_pre`
  already handles this by emitting `Goal::Researched`, which is why the 6/min
  cell plans. A new module must not re-hard-code its way around it.

## 6. `search.rs` is the wrong foundation

Confirmed no planner consumer — only `app/src-tauri/src/cli/search.rs`. It
cannot express a mall on four independent counts: `ore_to_plate_with` hard-codes
`burner-mining-drill`/`stone-furnace` and the ore→plate topology; a `Layout`
entity **cannot carry a recipe**; `FlowGraph` roots its walk only at drills on
ore, so a plate-fed cell scores `reached = false`; and `rank` orders on
`iron-plate` only. It is a drills×furnaces blueprint generator, not a design
deriver. Deriving designs is the right ambition — this is not its skeleton.

## What would falsify this

- **§1** — a plan whose machine crafts exceed ~5% of craft ticks. Re-run the
  composition script on a charted dump.
- **§2** — find any non-test site emitting `Goal::Producing`.
- **§4** — a mod or a research bonus changing `crafting_speed`; the arithmetic is
  re-derived from the dump, so re-run it rather than quoting these numbers.
- **§5** — the 30/min refusal not reproducing on a re-dump.

**Not verified here:** the "oil rig = 2,825 actions" figure. It needs a charted
dump — on this t=0 map both `producing:petroleum-gas:30` and
`have:oil-refinery:1` refuse (`no crude-oil is charted`). The composition claim
rests on the green plan, which does reproduce.
