# The lag clock starts too late — 2026-09-03

`run-1788465258-49050`, bot 1, ticks 44,204 → 56,467. **12,263 ticks (3.4 min,
17% of a 21.4-minute run) spent standing still next to a finished furnace.**

It is not smelting lag. **Nothing was smelting.** It is a defect in
`crates/executor/src/run.rs`, and it costs this run ~24,500 ticks in total.

---

## The one-sentence version

`wait_out_lag` measures a lag edge **from the moment the bot reaches the
action**, not from the moment the predecessor finished — so every tick the bot
usefully spends between the two is *added* to the wait instead of subtracted
from it.

```rust
// crates/executor/src/run.rs:601-611  (wait_out_lag)
let Some(started) = act.game_tick().await.ok().flatten() else { ... };
let deadline = started.saturating_add(u64::from(lag));   // started == NOW
```

against the planner's own semantics, which is the correct one:

```rust
// crates/planner/src/schedule.rs:285  (deps_ready)
.map(|(p, lag)| finished[p] + lag)                        // finished[p], not now
```

The two agree only when the bot arrives at the exact tick the predecessor
settled. They diverge by exactly the amount of parallelism the planner
achieved — and `schedule.rs:225-227` says walking across a lag "is most of
where multi-bot parallelism comes from". **The better the plan overlaps a lag
with useful work, the more the executor throws away.**

## The gap, tick by tick

| tick | what |
|---|---|
| 23,108 | `#22 place stone-furnace at [-31,-25]` — success |
| 26,829 | `#21 place burner-mining-drill at [-33,-25]` — success |
| 26,831 | `#23 fuel the burner-mining-drill with 8 coal` — success |
| **26,833** | **`#24 fuel the stone-furnace with 5 coal` — success. Every predecessor of `#25` is now done. The cell is running.** |
| 26,833 → 44,204 | bot 1 does 17,371 ticks of *other work* — mining, crafting, other furnaces, seven walks |
| 43,950 → 44,204 | walk to `[-31,-25]`. Bot 1 is standing at the cell. |
| **44,204 → 56,467** | **nothing. 12,263 ticks.** |
| 56,467 | `#25 take 50 iron-plate from the cell` — success, `elapsed_ticks: 0` |

Modelled lag for `#25` is **12,240** ticks (`produce.rs:2051`: `51 * 240`; the
drill's 240 ticks/ore is the cell's bottleneck, not the furnace's 192 —
`produce.rs:203`, `:244`). The plan's own arithmetic agrees:
`planned_start(#25) 14,934 − (planned_start(#24) 2,684 + 10) = 12,250`.

Observed wait 56,467 − 44,204 = 12,263 ≈ **12,240 from arrival**. That is
`wait_out_lag` doing exactly what it is written to do.

The correct deadline was `26,833 + 12,240 = 39,073`. Bot 1 arrived **5,131
ticks after the plates were due**. The right wait was zero.

## The plates were provably there, and the world was provably idle

`samples.jsonl`, force production, cumulative `iron-plate` made:

```
tick   39,600   75
tick   39,900   76
tick   40,200   76   <- last plate this run's furnaces ever make
tick   44,100   76   <- bot 1 arrives at the cell
tick   56,400   76   <- bot 1 has waited 12,200 ticks
tick   60,000   76
```

**Zero iron plates were produced anywhere on the map during the entire wait.**
Production stopped at ~40,000, which is where the drill's 8 coal runs out
(8 × 1,600 = 12,800 ticks from 26,831 → 39,631) — the planner sized the fuel
charge for exactly this smelt and got it right. The model predicted 39,073, the
game delivered by 40,200: **agreement within 3%.** The planner was not wrong
about anything. The executor waited a second, redundant 12,240 ticks for machine
time that had already been spent.

Bot 1's position is frozen at `(-26.2, -24.8)` in every `bots` sample from
44,400 to 56,460. It was not walking, not blocked, not retrying.

## This is not one gap — it is seven, and it is bot 1's largest single cost

Every one of bot 1's ten idle gaps > 400 ticks after the second plan epoch is a
`take` waiting out a lag edge. For each: how long it waited, against how much
of that lag had *already elapsed* when the bot arrived.

| idle window | ticks | action | already elapsed on arrival | avoidable |
|---|---|---|---|---|
| 25,930→26,702 | 772 | `take 3 iron-plate` | 0 | 0 |
| 35,781→36,553 | 772 | `take 3 copper-plate` | 2 | 2 |
| 36,553→37,325 | 772 | `take 3 copper-plate` | 772 | **772** |
| 37,389→38,353 | 964 | `take 4 copper-plate` | 1,612 | **964** |
| 38,756→42,793 | 4,037 | `take 20 iron-plate` | 5,961 | **4,037** |
| **44,204→56,467** | **12,263** | **`take 50 iron-plate from the cell`** | **17,371** | **12,263** |
| 60,050→61,206 | 1,156 | `take 5 iron-plate` | 0 | 0 |
| 61,206→62,938 | 1,732 | `take 8 iron-plate` | 1,160 | 1,160 |
| 64,944→68,213 | 3,269 | `take 16 iron-plate` | 4,896 | **3,269** |
| 69,602→71,718 | 2,116 | `take 10 copper-plate` | 5,033 | **2,116** |

**24,583 of 27,853 idle ticks — 88% — were waits for machine time that had
already passed.** That is 6.8 minutes of a 21.4-minute run, ~32% of the whole
run and ~36% of bot 1's span.

`just analyse` puts bot 1's idle at **40.7% of span** and names the big gap by
its action. It was already reporting this; nobody had asked what the bot was
waiting *for*.

## Why the existing analysis missed it

`notes/2026-09-03-morning-summary.md` §"Run 8" concludes from this same run that
"mining is still 65.6% of measured action time" and that `steps/bot` is the
number that matters. Both true, and both blind to this: **a verb histogram
cannot see waiting.** `take` settles in its dispatch tick and contributes 0
ticks to every histogram, so the most expensive thing bot 1 did all run appears
in the accounting as free.

## Why the test suite misses it

All four `wait_out_lag` tests (`run.rs:1966-2060`) use `cross_bot_fixture`,
where the dependent action's bot has nothing else to do and reaches it
immediately after the predecessor settles. Arrival-time and finish-time are the
same number in that fixture, so the defect is invisible to every test that
exists. The tests pin the *duration* of the wait (ticks not seconds, game speed,
stopped clock) and never its *origin*.

## The fix

`ExecutionLog::AttemptRecord` already carries `replied_tick` — "`game.tick` when
the game reported the outcome of this attempt" (`log.rs:192`) — which is exactly
the missing number. Two shapes:

1. Carry it in the completion signal: `Status::Success(Ticks)` instead of a bare
   `Status::Success`, so `await_preds` can compute
   `deadline = max over preds (finish(pred) + lag(pred))` and wait
   `deadline.saturating_sub(now)` — zero when already past.
2. Or pass the `ExecutionLog` into `await_preds` and read `replied_tick` there.

Note that `await_preds` currently collapses `max_lag` across predecessors
(`run.rs:446-468`) independently of *which* predecessor each lag belongs to.
That is harmless under arrival-based timing and wrong under finish-based
timing: the max must be taken over `finish(pred) + lag(pred)` pairs, not over
lags alone.

Also worth deciding deliberately: the wait sits in the `Act` step, *after* the
`Walk` step of the same bot has completed (`run.rs:280-322`). The planner
schedules the walk to happen *during* the lag on purpose. Charging the walk
against the lag falls out of the finish-based fix for free.

**Sanity guard for whichever fix lands**: the deadline must still be honoured
when it has *not* passed (rows 1 and 7 above needed their full wait), and it must
still be chased in game ticks rather than wall-clock seconds — that is the
`run-1788320177-77989` defect the current code exists to fix, and it must not be
undone.

## Answers to the four questions asked

1. **What was bot 1 waiting for?** Nothing. It stood at a fuel-exhausted cell
   holding 50 finished plates, waiting out a software timer.
2. **Is 12,837 consistent with smelting 50 plates?** The *duration* is — 12,240
   ticks is the correct cell time for 50 plates, drill-limited at 240
   ticks/ore. The *placement* is not: that time had already elapsed between
   ticks 26,833 and 39,073 while the bot did other work.
3. **Were bots 2/3/4 idle?** Yes, from tick 32,793 to the end of the milestone
   at 81,297 — 48,504 ticks, ~13 minutes. Not blocked: the plan gave them
   **two steps each** (`steps/bot {1: 90, 2: 2, 3: 2, 4: 2}`). They finished
   their schedules and stood still, positions unchanged in every sample, 8
   iron-plate each in inventory. That is the known `Holder::Share` single-owner
   constraint (R3 in the four-bot design), not this defect.
4. **Defect or expected cost?** **Defect. High confidence.** The arithmetic is
   exact to within 23 ticks, the code path is unambiguous, and the production
   record independently proves the world made zero plates during the wait.

---

## What is *not* claimed here

Fixing this does not straightforwardly cut 6.8 minutes off the run: the freed
time only shortens the run to the extent bot 1's remaining chain is the critical
path, and bot 1 holds 90 of 96 steps, so it very likely is — but that is a
prediction, not a measurement. Measure it on `BENCHMARK_SEED` before quoting a
number.

The four-bot design's R3 (gathering cannot split under an owned chain) remains
the larger structural ceiling. This is a separate, smaller, and much cheaper
defect that happens to be the single largest line item in this particular run.
