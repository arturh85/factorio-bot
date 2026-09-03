# Closing the Idle Gap — Implementation Plan

**Status:** planning only. **Live experiments are paused** at the owner's
instruction until these pieces are assembled. A rung-1 run costs 20+ minutes
and the current bottleneck is understood well enough that further runs buy
little.

**Target:** `researched("automation")` in **under 9 minutes**, ideally near
the single-player world record's **6:12**. Current: **21.4 minutes** with
four bots.

---

## The measurement that reframes the problem

From `workspace/runs/run-1788465258-49050` (21.4 min game time, roster
`[1,2,3,4]` confirmed — the only clean four-bot rung-1 run since the walk
fixes landed):

| | ticks | time | share |
|---|---|---|---|
| bot 1 **executing actions** | 23,397 | **6.5 min** | 34.1% |
| bot 1 **idle between actions** | 45,117 | **12.5 min** | 65.9% |

Executing time by verb, bot 1:

| verb | actions | ticks | time |
|---|---|---|---|
| `mine` | 20 | 11,155 | 3.1 min |
| `craft` | 25 | 6,243 | 1.7 min |
| `research` | 1 | 5,999 | 1.7 min (lab time — a hard floor) |
| `take` / `place` / `insert` / `fuel` | 48 | **0** | free |

Whole roster:

| bot | actions | executing |
|---|---|---|
| 1 | 94 | 6.50 min |
| 2 | 3 (2 mine, 1 insert) | 0.34 min |
| 3 | 3 | 0.34 min |
| 4 | 3 | 0.34 min |

**7.5 bot-minutes of work across 85.6 bot-minutes available — 8.8% roster
utilisation.** All 25 crafts went to bot 1; bots 2–4 crafted nothing.

**The conclusion that reorders everything below: our bot's actual working
time is 6.5 minutes against the world record's 6:12.** The work is already
roughly WR-paced. The entire gap is idleness — one bot standing still for
12.5 minutes while three others have nothing to do. This is not a
"make it faster" problem and it will not yield to incremental shaving.

Two structural facts establish *why* nothing fills the idle:

1. **The executor gives each bot exactly one action at a time.** Measured:
   **0 of 99** consecutive same-bot dispatch pairs overlap — the next action
   is never dispatched before the previous one settles.
2. **The planner cannot express work that has no consumer.** Every `Goal`
   variant (`Have`, `Researched`, `Produced`, `Producing`, `All`) is
   demand-driven. There is no way to say "and if you have nothing to do,
   do this."

Every large idle gap sits immediately before a `take … from the furnace`.
The idle is smelting lag with nothing scheduled against it.

---

## Workstreams, in value order

### 0. Offline planning on a real map — do this FIRST

**Today a plan cannot be made without launching Factorio**, so every planner
change costs a 20-minute run to evaluate. This is the single biggest tax on
iteration and it is nearly free to remove.

**`FactorioWorld` already round-trips through serde.** Both halves are
hand-written and present (`crates/core/src/factorio/world.rs:1080` and
`:1099`). What is serialized: `players`, `forces`, `recipes`,
`entity_prototypes`, `item_prototypes`, `actions`, `path_requests`, and
`entity_graph` — which itself carries `entity_graph`, `blocked_tree`,
`entity_tree`, `tile_tree`, `entity_nodes`, `resources`, `resource_tree`
and `minables` (`crates/core/src/graph/entity_graph.rs:1775-1784`).

**Nothing writes one to disk.** That is the whole gap: add a dump after
world setup, and a load that feeds `PlanState::from_world`. Then
`expand()` + `schedule()` run against a real map, offline, in
milliseconds.

**This supersedes the archive-replay design (`ec735a59`).** That design
declared three gaps it could not close from a run archive — resource
*amounts* absent from `EntitySnapshot`, trees/water/cliffs structurally
absent from keyframes, `techs_unlocked` a count rather than a list.
Dumping `FactorioWorld` has none of them, because it captures the actual
world the planner reads rather than a lossy derived snapshot: `resources`
carries amounts, `tile_tree` carries water and terrain, `forces` carries
research state. Keep `ec735a59` as the record of why the archive route was
rejected.

**Two fields are not in the serialized set and both are read by
`PlanState::from_world`:**
- `inventories` — observed container contents (`DashMap<Pos,
  ObservedInventory>`)
- `placement_refusals` — sites the game has refused a build at

At t=0 on a fresh map both are empty, so a setup-time dump is already
faithful. Dumping mid-run — which is what makes replanning from a
milestone useful — needs both added to the impl. Do that as part of this
workstream rather than discovering it later.

**Payoff:** planner iteration drops from ~20 minutes to seconds, and every
workstream below can be evaluated before a run is spent on it. It also
makes the paused experiments cheap to resume.

### A. Fill idle time — worth up to 12.5 min

The largest single lever, and the one with no existing mechanism.

Needs a way to express opportunistic work: things always worth doing that no
goal demands. The owner's list: keep furnaces and miners stocked with coal,
collect wood, clear areas for later building, handcraft ahead.

Design questions to settle before implementation:
- A new `Goal` variant, or a separate "filler" pass that runs after
  `schedule()` and packs idle windows? The latter keeps `expand()` purely
  demand-driven and leaves the determinism contract untouched.
- Filler work must never delay demanded work. It needs a preemption or
  window-fitting rule: only schedule filler into a gap whose end is already
  known.
- Filler output must not confuse the planner's inventory reasoning — an
  extra 20 coal in a bot's pocket changes later `available()` sizing.

**Determinism is a hard constraint**: `crates/planner` is pure, no I/O, no
wall-clock, ordered collections only, floats via `total_cmp`. Filler
selection must be a deterministic function of plan state.

### B. Shared chest as a material buffer — unblocks A, C and R3's residual

The R3 agent could hand a furnace's stone, craft, placement and coal to
another bot, but **could not hand over the ore**: ore is
*inventory-convergent* — a downstream action reads the producing bot's
inventory, so it must land in one specific bot's hands. Placements move
freely because their precondition names a *position*, a world fact any bot
can satisfy.

**A shared chest converts inventory-convergent work into world-convergent
work.** "50 plates in bot 1's inventory" becomes "50 plates in the chest at
P" — any bot can fill it, any bot can draw from it.

This is the enabler for parallel mining and parallel crafting. It is worth
more than the crafting arithmetic alone suggests, because it removes the
structural reason work cannot move between bots.

Needs: a `Condition` naming chest contents; planner methods that route
through it; executor support for take/insert against a buffer chest;
a placement rule for where the buffer lives.

### C. Overlap crafting with other work — worth up to ~1.7 min directly, more indirectly

In Factorio hand-crafting runs in a **background queue**: a player queues
crafts and keeps walking and mining. The mod already uses `begin_crafting`
and settles on `on_player_crafted_item`, so the game side is right. The
**executor** is what serialises it — one action per bot, waiting for settle.

Crafts settle at exactly nominal duration (stone-furnace 30 ticks,
burner-mining-drill 120), so the *cost model* is accurate; the
*exclusivity assumption* is what's wrong.

Needs: a notion of which actions are exclusive (mine, walk) versus
background (craft), and per-bot concurrency for the latter. Note this
interacts with A — a bot with a filler task and a queued craft is doing two
things, which is correct and currently impossible.

### D. Stop pricing an idle bot's time as scarce

`bdbde0c9` measured building extra furnaces to cover smelting lag and it
**lost**: 126 actions / 49,743 ticks against 110 / 46,164. That verdict is
why only *reuse* of standing furnaces was kept.

**That measurement is not wrong, but it answers a different question than
the one that matters.** It prices ~630 ticks of craft-and-place against a
bot whose time is assumed scarce. The bot has 45,117 idle ticks. Once A and
B give bots slack, this must be re-measured — the owner's instinct ("while
the bot is waiting, why not keep building more miners and furnaces") is
consistent with the idle data and inconsistent with the cost model.

### E. Burner drills and miner+furnace pairs — R2

`PlaceDrill` exists in `crates/planner/src/method/produce.rs` with a cost
model. Exactly **one** burner mining drill was crafted in the reference run,
at tick 26,704. A drill is *slower per ore than hands* (240 vs 120 ticks) —
the win is freed bot-time, which only pays once there is something else for
the bot to do. **So E depends on A.**

The owner's framing: "usually one starts with miner+furnace pairs" — a drill
feeding a furnace directly, which removes both the mining and the hauling
from a bot's critical path.

### F. The 3.6-minute gap — unexplained

The single largest wait in the run: **12,837 ticks (3.6 min, 17% of the
entire run)** between `craft 10 electronic-circuit` and
`take 50 iron-plate from the cell`. Not yet diagnosed. Large enough that it
may be a defect rather than ordinary smelting lag, and it should be
understood before it is optimised around.

### G. Milestone saves — queued, unblocked

Save on milestone satisfaction so later experiments resume instead of
re-deriving a deterministic 21-minute prelude. `provenance.json` already
carries a `resumed_from` field (pre-added), so **no schema bump is needed**.

Two constraints established:
- A resumed run is **not benchmark-comparable** — `--compare` must refuse or
  flag it (it already refuses on differing `resumed_from`).
- A save carries mod `storage.*`. Factorio only migrates on a version bump
  and `mods/BotBridge/info.json` is pinned at `0.0.1`, so a save written
  under one `control.lua` and resumed under a changed one silently keeps the
  old state. This has already poisoned runs here.

---

## Already landed (no work needed)

- **R3** (`c0c3bc5c`) — a furnace's stone, craft, placement and coal can be
  handed to another bot. Fixture: busiest bot 86% → 52% of steps, makespan
  −16.5%. Solo plan byte-identical. 563 planner tests green.
- **Provenance** (`61ec7364`, `1390840d`) — `provenance.json` at run start:
  seed, map-exchange string, game version, git commit + dirty flag, build
  profile, roster, `resumed_from`, map fingerprint.
- **`--seed` was silently ignored** (`61ec7364`, `ccaf562f`) — it only
  reaches Factorio via `--map-gen-seed` on `--create`, which only runs when
  `level.zip` is absent, which only `--new` causes. **Every run before this
  fix used an uncontrolled map.** Now warns loudly and writes
  `map-gen-seed.txt` at every creation.
- **`--compare`** (`a1c1316a`) — refuses comparisons across differing seed,
  commit, Factorio version, build profile or `resumed_from`; flags absent
  provenance, dirty tree, degraded roster, runs ending on `plan_created`.
- **Offline replan harness** (`ec735a59`) — design only, and now
  **superseded by workstream 0**. Its three gaps are artifacts of replaying
  a run *archive*; serializing `FactorioWorld` avoids all three. Keep the
  document as the record of why the archive route was rejected.

---

## Open items not on the critical path

- **Flaky test**: `crates/planner/tests/red_science.rs::every_expansion_replays_in_time_order`
  failed once under a full `--workspace` run, passes 12/12 in isolation and
  3/3 under repeated workspace runs since. In a crate whose contract is
  byte-identical determinism this must be understood, not waited out. No
  error text was captured from the original failure.
- **Benchmark seed `20260903`** is documented but **has never been run**.
  The recipe is destructive (`--new` deletes the map) and unvalidated.
- Mod-side walk re-path defect (`walk_repath_finished` quoting the original
  path's last waypoint).
- Provenance is not surfaced in `RunSummary` or the viewer — deliberate, it
  would move the OpenAPI snapshot for no current consumer.
- `ls` is aliased to `eza` in this shell, which makes `ls -t` fail with an
  `--time` error and silently breaks globs; `cat` is aliased to `bat`.
  Belongs in CLAUDE.md's alias traps.

---

## Sequencing

**0 before everything.** Offline planning is what makes the rest
cheap to evaluate; without it each workstream below costs a 20-minute run
to judge.

**B before A before E.** The chest (B) is what makes work movable; filler
work (A) is what consumes the freed capacity; drills (E) only pay once a
bot has something else to do. C is independent and can land any time. D is
a re-measurement that only becomes meaningful after A and B. F should be
diagnosed early — it is 17% of the run and might be a defect, in which case
it changes the arithmetic above.

**No live runs until B, A and C are in.** The next run should be the first
honest before/after, on seed `20260903`, with provenance recorded and
`--compare` against this plan's reference run.
