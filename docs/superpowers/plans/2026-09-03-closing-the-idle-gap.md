# Closing the Idle Gap — Implementation Plan

**Status:** planning only. **Live experiments are paused** at the owner's
instruction until these pieces are assembled. A rung-1 run costs 20+ minutes
and the current bottleneck is understood well enough that further runs buy
little.

**Target:** `researched("automation")` in **under 9 minutes**, ideally near
the single-player world record's **6:12**. Current: **21.4 minutes** with
four bots.

---

## Owner decisions (2026-09-03, before an unattended night)

1. **Success is game time, from run start to milestone satisfied.** Not
   roster-ready, not wall clock. The reference run is 21.34 min on that
   clock. The target is **under 9 minutes**; the single-player world record
   researches automation at **6:12**.
2. **Workstream C is approved for full implementation**, including the
   executor's per-bot concurrency change — not design-only.
3. **Savepoints are preferred over cheat hatches.** Cheating and teleporting
   are **fine during development**, to reach an untested stage quickly. The
   **final measured runs must be as cheat-free as we can get them.** Any run
   that used a cheat path must say so in provenance, and `--compare` must
   refuse to compare it against an honest run. This promotes workstream G
   (milestone saves) from "queued" to the supported way of making runs cheap.

## The measurement that reframes the problem

From `workspace/runs/run-1788465258-49050` (21.34 min game time, roster
`[1,2,3,4]` confirmed via `plan_created`).

### Counting rules — read these before quoting a number

Three quantities here are easily confused, and confusing them has already
produced two wrong versions of this section:

- **Action ids are per-plan, not global.** This run replanned twice
  (`plan_created` at ticks 4,642 / 23,092 / 81,514) and ids are reused
  across plans: id 38 is `craft 2 iron-gear-wheel` in the first plan and
  `craft 1 offshore-pump` in the third. **44 of 147 ids collide.** Keying any
  aggregate on `id` silently drops those 44 settles and understates executing
  time by 8,067 ticks (2.24 min). An earlier version of this plan did exactly
  that. **Aggregate over settle *events*, never over ids.**
- **147 dispatch events = 147 distinct actions**, across three plans. There
  are no repeated dispatches of the same action, and no wasted walks to
  already-satisfied actions — a hypothesis that looked strong until the id
  collision explained it away.
- **"Planned steps" (96) ≠ dispatched actions (147).** A replan re-emits
  remaining work.

### Bot 1, over a span of 70,616 ticks (19.62 min)

| | ticks | time | share |
|---|---|---|---|
| **executing actions** | 31,464 | **8.74 min** | 44.6% |
| **walking** (`walk_settled`) | 14,330 | **3.98 min** | 20.3% |
| **neither** — acting nor walking | 24,822 | **6.89 min** | 35.2% |

Executing time by verb:

| verb | actions | ticks | time |
|---|---|---|---|
| `mine` | 39 | 18,890 | **5.25 min** |
| `craft` | 34 | 6,575 | 1.83 min |
| `research` | 1 | 5,999 | 1.67 min (lab time — a hard floor) |
| `place` / `fuel` / `insert` / `take` | 61 | **0** | free |

**Mining is the single largest activity in the run**, larger than crafting
and research combined.

### The whole roster

| bot | executing | walking | busy |
|---|---|---|---|
| 1 | 31,464 | 14,330 | **12.72 min** |
| 2 | 1,691 | 435 | 0.59 min |
| 3 | 1,691 | 613 | 0.64 min |
| 4 | 1,693 | 591 | 0.63 min |

**14.59 bot-minutes of work across 85.4 bot-minutes available — 17.1% roster
utilisation.** All 34 crafts went to bot 1; bots 2–4 crafted nothing.

### What this means

Against a single-player world record that researches automation at **6:12**,
bot 1 is *busy* for 12.72 minutes and the whole roster does 14.59 bot-minutes
of work. So there are two distinct gaps, and they need different fixes:

1. **Bot 1 idles 6.89 minutes.** Workstream F establishes that essentially
   **all** of it is a single executor defect — the F diagnosis independently
   measured 24,583 ticks of lag-clock waiting against the 24,822 ticks of
   "neither" computed here, i.e. ~99%. This is a bug, not a capacity problem.
2. **Three bots are idle almost the entire run.** That is the
   `Holder::Share` ceiling, addressed by R3 (`c0c3bc5c`) — which landed
   **after** this run and has therefore never been measured live.

Two structural facts explain why nothing fills the idle:

- **The executor gives each bot exactly one action at a time.** Measured: 0
  of 99 consecutive same-bot dispatch pairs overlap.
- **The planner cannot express work that has no consumer.** Every `Goal`
  variant (`Have`, `Researched`, `Produced`, `Producing`, `All`) is
  demand-driven.

## The path to under 9 minutes — a budget, not a hope

Bot 1's 19.62-minute span is the run. Spending it down:

| step | what it removes | bot 1 span |
|---|---|---|
| baseline | — | **19.62 min** |
| **F** — lag clock starts at the predecessor's finish | 6.89 min of idle (F accounts for ~99% of it) | **12.72 min** |
| **B + R3** — split the mining across four bots | `mine` is 5.25 min, the largest single activity; four ways is ~1.31 | **~8.8 min** |
| **C** — overlap `research` (1.67 min) and `craft` (1.83) with walking | up to ~2 min more, minus what cannot overlap | **~7 min** |
| **0b** — a seed with ore near spawn | some of the 3.98 min of walking | **lower** |

**F plus splitting the mining is what reaches the target.** Everything else is
margin. That is a materially different conclusion from this plan's first
draft, which ranked "fill idle time" first on a mis-measured 12.5-minute idle
figure.

### The floor, and why it is not zero

Two costs are irreducible without more machines:

- **Research is 5,999 ticks (1.67 min) of lab time.** A hard floor for the
  milestone itself.
- **Machine time.** The cell's modelled lag alone is 12,240 ticks (3.4 min) —
  `51 * 240`, the drill's 240 ticks/ore. The milestone cannot complete before
  that elapses, no matter how many bots wait for it.

So roughly **5 minutes of the 6:12 world record is machine time and lab
time**. A human speedrunner is not working faster than us during it; they are
*doing something else* while it runs. That is exactly what F stops us
throwing away, what A fills, and what D/E shorten by adding machines.

**This reorders the plan's own premise.** After F, bot 1 has little idle left
to fill — so **A's value is not bot 1 at all**, it is bots 2, 3 and 4, which
are idle for 48,504 ticks apiece while doing 0.6 minutes of work each. A is
about the roster, not about the busiest bot.

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

### 0b. A reasonable starting seed — resources close to spawn

**Walking is 21% of bot 1's run.** Against a 6:12 target, 4 minutes of
walking would consume two thirds of the entire budget. No amount of
planner improvement compensates for a spawn whose ore is far away, so a
comparison against WR times is meaningless until the map is a fair one.

**Scope: deliberately modest.** Not a search for a global optimum — a
reasonable seed where everything rung 1 needs is close to spawn. Required
within a short radius: **iron ore, copper ore, coal, stone**, and **water**
(the run places a boiler and steam engine to power the lab). Trees nearby
are a bonus, for the wood the owner wants gathered during idle time.

**Workstream 0 makes this cheap, and removes the blocker that gated the
existing tool.** `roll-seed` exists as a CLI surface but is deliberately
disabled (`app/src-tauri/src/cli/roll_seed.rs`): its old fitness function
(`-shortest_path()`, minus 10,000 per resource type not found within 3,000
tiles) died with the task-graph planner, and its doc says resurrection
needs "a fitness function for the current planner — `Schedule::makespan` is
the plausible candidate — and the cross-boundary plumbing to read a
`Schedule` back out of the handle-based Lua runtime."

**That plumbing is exactly what a world dump removes.** Generate a map →
dump `FactorioWorld` → `expand()` + `schedule()` → read `makespan`. A
direct function call on a deserialized world; no Lua runtime involved.

Two tiers, and the cheap one is probably enough:
- **Distance scoring** — nearest patch of each required resource from
  spawn, read straight off `EntityGraph::resources` on a dumped world. No
  planner needed at all.
- **Makespan scoring** — free once workstream 0 lands, and strictly better,
  since it prices the actual plan rather than a proxy.

**Record the chosen seed and freeze it.** Note that `--seed` was silently
ignored until `61ec7364`, so every earlier run used an uncontrolled map;
`20260903` is documented as the benchmark seed but has never been run and
has not been scored by any of the above. Scoring it is part of this
workstream — it may well not be a good map.

### A. Fill idle time — for bots 2-4, not bot 1 (see the budget above)

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

### C. Overlap crafting AND research with other work — ~2 min, approved for full implementation

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

### F. The lag clock starts too late — DIAGNOSED, and it is the biggest single item

**This was "the unexplained 3.6-minute gap". It is a defect, and it is worth
~32% of the run.** Full diagnosis:
`docs/superpowers/notes/2026-09-03-the-lag-clock-starts-too-late.md`
(`6cee0143`).

**The mechanism.** `wait_out_lag` (`crates/executor/src/run.rs:601`) starts
the lag countdown at the tick the bot **arrives** at the action:

```rust
let deadline = started.saturating_add(u64::from(lag));   // started == NOW
```

The planner's semantics (`crates/planner/src/schedule.rs:285`) is
`finished[predecessor] + lag`. The two agree **only** if the bot arrives on
the exact tick its predecessor settled. They diverge by exactly the amount of
parallelism achieved — and `schedule.rs:225-227` states that walking across a
lag is *where multi-bot parallelism comes from*.

**So the better the plan overlaps a lag, the more the executor throws away.**
Parallelism is not merely unrewarded here; it is actively punished.

**The arithmetic, exact to 23 ticks.** `#24 fuel the stone-furnace` settled at
26,833, every predecessor done. Modelled lag 12,240 (`produce.rs:2051`,
`51 * 240` — the drill's 240 ticks/ore is the cell bottleneck, not the
furnace's 192). Correct deadline: **39,073**. Bot 1 did 17,371 ticks of other
useful work, walked to the cell, and arrived at **44,204** — already 5,131
ticks *after* the plates were due. `wait_out_lag` then set the deadline to
44,204 + 12,240, dispatching at **56,467**. The take succeeded with
`elapsed_ticks: 0`.

**Independent proof the wait was empty.** `samples.jsonl` cumulative
`iron-plate` production reads **76 at tick 40,200 and 76 at tick 56,400** —
zero plates produced anywhere on the map during the entire wait. Production
stopped near 40,000 when the drill's 8 coal ran out at 39,631, exactly as the
planner sized it (predicted 39,073, delivered by 40,200 — within 3%). Bot 1's
position is frozen at `(-26.2, -24.8)` in every sample from 44,400 to 56,460.

**It is seven gaps, not one.** All ten of bot 1's idle gaps over 400 ticks are
`take`s waiting out lag edges. **24,583 of 27,853 idle ticks — 88%, about 6.8
minutes, roughly 32% of the run — were spent waiting for machine time that had
already been spent.**

**Why nothing caught it.** Invisible to a verb histogram, because `take`
settles in its dispatch tick. Invisible to the test suite, because all four
`wait_out_lag` tests use `cross_bot_fixture`, where arrival time and finish
time are the same number — they pin the wait's *duration* and never its
*origin*.

**The fix.** `ExecutionLog::AttemptRecord.replied_tick`
(`crates/executor/src/log.rs:192`) already holds the missing number. Either
carry it in the completion signal (`Status::Success(Ticks)`) or pass the log
into `await_preds`, then compute
`deadline = max over preds (finish(pred) + lag(pred))` and wait
`deadline.saturating_sub(now)`. Two cautions from the diagnosis:
`await_preds` currently collapses `max_lag` across predecessors independently
of which predecessor each lag belongs to (`run.rs:446-468`) — harmless today,
wrong under finish-based timing; and the change must not undo the
tick-versus-wall-clock fix (`run-1788320177-77989`). The walk-before-wait
ordering (`run.rs:280-322`) is corrected for free by the same change.

**Not claimed:** that fixing this cuts 6.8 minutes off the run. Bot 1 holds 90
of 96 steps so it is probably close to the critical path, but that is a
prediction and must be measured on the benchmark seed.

**The other three bots are idle from 32,793 to the end (48,504 ticks) but are
not blocked by this** — the plan gave them two steps each and they finished.
That is the separate `Holder::Share` ceiling.

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
  failed once under a full `--workspace` run, reported by the provenance
  agent. **Not reproduced in 18 subsequent attempts** — 12 isolated runs and
  6 full workspace passes (70 `ok` result lines each, zero failures) at
  `20323fd0`.

  **This is unresolved, not resolved.** No error text was captured from the
  original failure, so there is nothing to diagnose from, and the test body
  contains no wall-clock, no randomness and no shared state — reading it
  suggests nothing that *could* vary. In a crate whose whole contract is
  byte-identical determinism, "it stopped happening" is not an answer. If it
  recurs, **capture the failure output before doing anything else**; that is
  the missing input.

  One hypothesis worth recording rather than testing blindly: the failure
  was seen under full-workspace parallelism, and the only load-sensitive
  failure mode available to a pure test is stack depth in `expand()`'s
  recursion, since test threads get a smaller default stack than main. That
  would abort the process rather than fail one test, which does not match —
  so it is a weak hypothesis, offered only so the next reader does not start
  from zero.
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

**F first — it is a bug fix, not a capability, and it is the largest single
item.** It is confined to `crates/executor` and blocks nothing else, so it
can land immediately and in parallel with the rest.

**0 before everything else, then 0b.** Offline planning is what makes the rest
cheap to evaluate; without it each workstream below costs a 20-minute run
to judge. Seed selection (0b) comes straight after, because every timing
below is measured against a map, and comparisons across different maps are
worthless — that error was already made once this session.

**B before A before E.** The chest (B) is what makes work movable; filler
work (A) is what consumes the freed capacity; drills (E) only pay once a
bot has something else to do. C is independent and can land any time. D is
a re-measurement that only becomes meaningful after A and B. F should be
diagnosed early — it is 17% of the run and might be a defect, in which case
it changes the arithmetic above.

**No live runs until F, 0, 0b, B, A and C are in.** The next run should be the
first honest before/after: on the seed chosen and scored in 0b, with
provenance recorded and `--compare` guarding the comparison.

Note that this plan's reference run (`run-1788465258-49050`) was itself made
on an **uncontrolled map**, since `--seed` did nothing before `61ec7364`.
Its *proportions* — 34% acting, 21% walking, 45% waiting — are what the plan
rests on, and those are structural. Its absolute 21.4 minutes is not a
baseline any later run can be compared against, and `--compare` will refuse
to try.
