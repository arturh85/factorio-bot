# Where the time goes on rung 1 — 2026-09-03

Rung 1 is `automation researched, which needs a lab and therefore a power
plant`. Four bots take **27.1 minutes** of game time, to the tick, in four
consecutive runs. The owner does it by hand with one bot in ~9.

Every number here comes from `tools/run_analysis.py` (`just analyse`), which is
committed so these can be recomputed rather than re-derived. Nothing below was
measured any other way.

```
just analyse                                        # one line per archived run
just analyse workspace/runs/run-1788449752-46541    # the full accounting
python3 tools/run_analysis.py --json <dir>          # same, machine-readable
```

---

## The headline, and the correction to it

The session's framing was **hand-mining and idle bots**. Hand-mining is
confirmed and is the largest line item. The idle bots are confirmed and are
worse than reported. But the causal story is backwards in a way that reverses
the priority list:

> **Adding three bots makes rung 1 seven minutes slower.**
> `run-1788401146-98497` reached `automation` with a roster of `[1]` in
> **20.1 minutes**. Every four-bot run since is **27.1 minutes**. The four-bot
> run mines **2.06× the raw material** to produce a byte-identical bill of goods.

So the first thing to fix is not "use the other three bots". It is "stop the
four-bot expansion buying twice the ore".

---

## Rung 1 across the archive

`m1` span, `roster ready` → `SATISFIED`, from `milestone_started` /
`milestone_satisfied`:

| run | start | roster | m1 | fleet util | mine ticks | stone furnaces placed |
|---|---|---|---|---|---|---|
| `run-1788401146-98497` | 04:05 | **[1]** | **20.1m** | **64.8%** | **22,119** | **9** |
| `run-1788396958-07935` | 02:55 | [1,2,3,4] | 24.7m | 23.6% | 51,809 | 24 |
| `run-1788399150-53956` | 03:32 | [1,2,3,4] | 25.6m *stuck* | 23.8% | 58,037 | 27 |
| `run-1788405365-21697` | 05:16 | [1,2,3,4] | 29.0m | 24.1% | 56,446 | 36 |
| `run-1788408407-02764` | 06:06 | [1,2,3,4] | 36.1m | 20.5% | 58,684 | 36 |
| `run-1788413329-43771` | 07:28 | [1,2,3,4] | 36.0m | 20.6% | 58,686 | 36 |
| `run-1788420521-55539` | 09:28 | [1,2,3,4] | 39.2m | 22.0% | 69,800 | 39 |
| `run-1788429666-94865` | 12:01 | [1,2,3,4] | 27.0m | 23.0% | 45,454 | 33 |
| `run-1788432181-42528` | 12:43 | [1,2,3,4] | 27.0m | 23.0% | 45,460 | 33 |
| `run-1788438602-66074` | 14:30 | [1,2,3,4] | 27.1m | 23.0% | 45,453 | 33 |
| `run-1788449752-46541` | 17:35 | [1,2,3,4] | 27.1m | 22.9% | 45,478 | 33 |
| `run-1788455754-92581` | 19:15 | [1,2,3,4] | live | 53.4% @2.3m | — | 1 so far |

Duration, mine ticks and furnaces placed move together across the table — not
strictly monotonically (`07935` mines more than `46541` in less time), but no
row is far off the line. Fleet utilisation is pinned near 23% for **every**
four-bot run and is 65% for the one-bot run: the fleet is not merely underused,
it is **anti-used**.

### Which runs predate which fix

- `1f498593` (walk-failure classifier) **14:14**. Runs before it record
  `failure.kind: "other"` for wordings it did not know. `run-1788432181-42528`
  (12:43) and `run-1788449752-46541` (17:35) are otherwise the *same run* —
  identical dispatch counts `{1: 563, 2: 9, 3: 9, 4: 36}`, identical failed
  destinations — and the first records 19 of its 20 walk failures as `other`
  where the second records the same 19 as `no_path`. The tool classifies the
  `error` text independently, so the older run still groups correctly, and
  flags the disagreement in place.
- `4e4d7cf8` (enclosure detection) **16:23**, `ddac41ac` (evacuation) **17:18**.
  `run-1788449752-46541` (17:35) postdates both and contains **no
  `bot_enclosed` events at all**, while two of its bots were provably frozen
  for 45 minutes (below). Either the binary predated the build or the pen was
  wider than `searched_tiles`. Absence of the event is not evidence of no
  enclosure — the record says so, and this run demonstrates it.
- `11fabe43` (drill-vs-hand-mining) **18:44**, `602c6856` (walled-in share
  exclusion) **19:11**. **Only `run-1788455754-92581` (19:15) postdates them**,
  and it had reached tick 13,404 — 2.3 minutes — when it last wrote at 19:18,
  so it says nothing about rung 1's duration yet. What it does say: it has
  placed a `burner-mining-drill`, so the gate fires; its fleet utilisation is
  53.4%, which is where every run starts before the split collapses; and
  **both of its `m1` plans are still `{1: 214, 2: 10, 3: 10, 4: 10}` steps**,
  with a makespan of 82,896 against `46541`'s 86,721. **A 4.4% improvement on a
  target that needs 63%.** The drill change does not touch the split.

---

## The 27.1 minutes, itemised

`run-1788449752-46541`, milestone 1, 97,659 ticks:

| | ticks | % of milestone |
|---|---|---|
| mining (105 actions, all bots) | 45,478 | 46.6% |
| walking (96 settled walks, all bots) | 31,085 | 31.8% |
| bot 1 waiting, blocks ≥300 ticks | 19,500 | 20.0% |
| crafting (54 actions, all bots) | 7,034 | 7.2% |
| `automation` in the lab | 5,999 | 6.1% |
| bot 1 per-action overhead (318 gaps, mean 5 ticks) | 1,729 | 1.8% |

**These rows are not disjoint and deliberately sum past 100%.** The first four
are summed across bots, so two bots mining at once bill 2 ticks per tick of
milestone; the last two are bot 1 alone, and the lab's 5,999 ticks are inside
bot 1's waiting. What the rows are for is the *ranking*, not a pie chart.

Mining splits: **stone 19,953**, iron-ore 14,376, coal 5,897, copper-ore 5,252.
Hand-mining runs at a flat ~121 ticks per unit for every resource.

Per bot:

| bot | dispatches | action ticks | walk ticks | walk failures | busy |
|---|---|---|---|---|---|
| 1 | 256 | 50,176 | 26,254 | 0 | **78.3%** |
| 2 | 9 | 2,658 | 1,052 | 2 | 3.8% |
| 3 | 9 | 2,658 | 1,609 | 2 | 4.4% |
| 4 | 12 | 3,019 | 2,170 | 0 | 5.3% |

### Two qualifications the "88% is hand-mining" number needs

1. **It is 88% of the *timed* verbs.** `place`, `insert`, `take`, `fuel`,
   `top`, `set` and `charge` settle in the tick they dispatch and contribute
   exactly zero. That is 126 of milestone 1's 286 actions costing nothing in
   the record — while each one still costs a walk. The tool prints the list of
   untimed verbs under every verb histogram for this reason.
2. **Walking is 31,085 ticks and is invisible in a verb histogram.** It is the
   second-largest line item and it is not an action.

---

## Why bots 2–4 do nothing: the planner, not the executor

This is decided before the scheduler ever runs. Every `plan_created` for
milestone 1 reads:

```
tick    2323  m1  steps=244  makespan=86721
    steps/bot         {1: 214, 2: 10, 3: 10, 4: 10}
    planned ticks/bot {1: 52340, 2: 2690, 3: 2690, 4: 2450}
tick   52875  m1  steps=119  makespan=38529
    steps/bot         {1: 113, 2: 2, 3: 2, 4: 2}
tick   79541  m1  steps=22   makespan=16823
    steps/bot         {1: 22}
```

The mechanism, read out of the code:

- Rung 1's top-level goal is `Goal::Researched`, and **`SplitAcrossBots` — the
  only method that hands one goal to several bots — cannot claim it**:
  `crates/planner/src/method/have.rs:2071-2073` requires
  `Goal::Have { whose: Holder::Anyone }`.
- `Researched::expand` states its entire subtree as
  `Holder::Share(ctx.chain_actor)` (`have.rs:1708`, `1786`, `1877`), and
  `chain_actor` is `bots.first()` (`crates/scripting_lua/src/globals/goal/mod.rs:891`).
- `crates/planner/src/method/mod.rs:605-615` stamps that bot as the chain's
  **owner**, and `crates/planner/src/schedule.rs:350` makes an owner a
  single-candidate hard constraint:
  `None if owner.is_some() => vec![vec![owner.expect("just checked")]]`.

The scheduler *is* load-aware — it is an earliest-completion-first list
scheduler with a `free_at` ranking and an explicit spread preference
(`schedule.rs:194-196`, `355-380`). None of it fires, because ranking a list of
one is a no-op. The only work reaching bots 2–4 is `SharedSmelt`'s ore
handovers (`have.rs:2523`), which fired five times because `worth_converging`'s
break-even is ~14 ore and most of rung 1's smelts are 2–10.

**Corollary: no amount of executor work fixes this.** The idleness is in the
plan.

### The old ladder proves the mechanism, and it was faster

`run-1788381727-24882` (09-02 22:42) and its two replicas ran a hand-decomposed
ladder — `gather iron ore x20`, `smelt iron plates x50`, … , `research
automation` — and reached `automation` in **21.2 minutes total**. Fleet
utilisation by milestone:

| milestone | shape | fleet util |
|---|---|---|
| m1 gather iron ore x20 | `Have{Anyone}` | **71.9%** |
| m2 gather copper ore x20 | `Have{Anyone}` | **104.7%** \* |
| m3 smelt iron plates x50 | `Have{Anyone}` | **50.3%** |
| m4 smelt copper plates x20 | `Have{Anyone}` | **62.7%** |
| m5 craft iron gear wheels x20 | craft chain | 22.4% |
| m6 craft automation science packs x10 | craft chain | 19.4% |
| m7 research automation | `Researched` | 20.8% |

\* Over 100% because an action whose measured duration began before the window
opened is billed whole to the window it settled in. It means "saturated", not
"impossible".

Exactly the predicted split: `Have{Anyone}` at top level parallelises, anything
else collapses to bot 1. Stating the goal as one `Researched` bought
declarativeness and cost 6 minutes.

---

## Why four bots mine twice as much

Same goal, same output, different roster:

| | 1 bot (`98497`) | 4 bots (`46541`) |
|---|---|---|
| first m1 plan | **104 steps** | **244 steps** |
| stone mined | **45** | **165** |
| iron ore mined | 93 | 119 |
| copper ore mined | 26 | 41 |
| coal mined | 19 | 49 |
| stone furnaces built | **9** | **33** |
| mine ticks | **22,119** | **45,478** |
| *output*: science packs / gears / iron plate | *10 / 32 / 93* | *10 / 32 / 93* |
| m1 | **20.1 min** | **27.1 min** |

The output columns are identical. The input columns are not.

The cause is `reserve_chain_produce`
(`crates/planner/src/method/mod.rs:700-733`): when a chain closes, everything it
produced is reserved against `chain_actor`, because the scheduler may put a
sibling chain on a different bot and a sibling that sizes itself against stock
another bot is carrying plans work it cannot do. That is sound. Its cost is
that **every subsequent chain re-mines and re-smelts** what an earlier chain
already made.

A roster of one is explicitly exempt, and the comment at `mod.rs:678-686` states
the measurement:

> `researched("automation")` at one bot goes from 58 steps to 101 with no defect
> to show for it.

The archive shows the same effect from the other side: 104 steps at one bot,
244 at four. Subtracting the 30 steps that `SharedSmelt` gave bots 2–4 leaves
**214 steps on bot 1 where a one-bot plan needs 104** — a 2.06× duplication on
the *same bot*, which is `reserve_chain_produce` and nothing else.

**Caveat, stated plainly.** `run-1788401146-98497` ran at 04:05 on a build
predating `b405dc5a`, `1f498593`, `0e4e3bff`, `4e4d7cf8`, `ddac41ac` and
`11fabe43`, and it was interrupted (no `manifest.json`, no `splits.json`). This
is not a controlled A/B. What is not in doubt is the production bill on disk
and the plan step counts, which agree with what the code comment predicts.

---

## Thirty-three furnaces, and the bots they wall in

Milestone 1 places **33 stone furnaces at 33 distinct sites**. Milestone 1's 85
insert/fuel/take actions land on 35 distinct targets, 33 of which are those
furnace sites — and **19 of the 35 receive exactly one operation** for the rest
of the milestone. The planner builds a fresh furnace per smelt goal and never
returns to it. That is 165 stone (19,953 mine ticks, 5.5 minutes, 20% of the
milestone), 33 crafts and 33 walks to fresh sites.

It is also a correctness defect, not only a cost. The tool's frozen-bot pass
cross-references each stalled bot against what the run built within 4 tiles of
it:

```
bot 2  ticks 47100 -> 211080  = 163980  45.5m at [-56.24, 14.28]  (still frozen at the last sample)
    already there when it stopped: {'stone-furnace': 2}
    built around it afterwards:     {'stone-furnace': 7} (first at tick 49352)
bot 3  ticks 47100 -> 211080  = 163980  45.5m at [-56.27, 14.68]  (still frozen at the last sample)
    already there when it stopped: {'stone-furnace': 2}
    built around it afterwards:     {'stone-furnace': 6} (first at tick 76209)
```

Both bots stop dead at tick 47,100 — inside milestone 1 — and never move again.
**Every long freeze in the archive sits inside a stone-furnace cluster the run
built itself.** The walk failures agree: all 20 in the run are `no_path`, and
the same destinations are re-selected 3–4 times each:

```
bot 2 -> [-46.5, -9.5]  x4
bot 3 -> [-54.5, -12.5] x4
bot 4 -> [-50.5, -12.5] x3
```

Per-bot walk failures read straight off `walk_settled.bot` are
**bot 1 = 0, bot 2 = 8, bot 3 = 8, bot 4 = 4** for both `run-1788432181-42528`
and `run-1788449752-46541`. The session's recorded truth was 0/7/7/4; the extra
one on each of bots 2 and 3 is the shared destination `[-29.0, -29.0]`, the only
whole-tile destination in the set, failed once by each. I cannot tell which
count was intended — the tool's comes with no join at all.

---

## What has to change for sub-10 minutes

Sub-10 means under 36,000 ticks. Budget, from the measured one-bot bill:

| | ticks | note |
|---|---|---|
| `automation` in the lab | 5,999 | hard floor; nothing makes this smaller |
| mining, one-bot bill ÷ 4 | 5,530 | 22,119 split across four bots |
| walking, ÷ 4 | 3,252 | 13,007 split across four bots |
| crafting on the critical path | ~4,000 | the 10-pack craft (3,009) is one action |
| smelting, 4 reused furnaces | ~5,700 | see below; overlaps mining |
| **serial worst case** | **24,481** | **6.8 min** — 30% headroom to the target |

The smelting row is the one line not measured from a record: 93 iron plate +
26 copper plate at 192 ticks each in a speed-1 stone furnace is 22,848
furnace-ticks, divided by four furnaces run concurrently. Everything else in
the table is a number the tool printed.

Ranked by ticks recovered:

1. **Stop the four-bot expansion duplicating the bill. ~23,000 mine ticks
   (6.4 min).** `reserve_chain_produce` reserves a closed chain's output for the
   rest of expansion whenever the roster exceeds one. The reservation is only
   *needed* when two chains actually land on different bots, and that is a fact
   the scheduler establishes afterwards. Either make it conditional on the
   assignment, or drop the duplicated production in a post-schedule pass.
   Nothing else on this list matters as much, and this one is a regression
   against a run that already exists on disk.
2. **Make gathering split for a `Researched` goal. ~4× on what is left.**
   `SplitAcrossBots` claims only top-level `Have{Anyone}`. Either let it claim
   the gathering subgoals of a `Researched` expansion, or stop
   `Researched::expand` stating them as `Holder::Share(chain_actor)` so
   `schedule.rs:350` is not handed a list of one. The scheduler's spread
   preference and `free_at` ranking already exist and do the right thing the
   moment they get more than one candidate. `mod.rs:560-590` warns that the
   pre-2026-09-02 unowned-chain behaviour crashed `run-1788405365-21697` — the
   sizing problem it removed has to be re-solved, not merely reverted.
3. **Reuse furnaces. ~15,000 mine ticks of stone, ~30 walks, and the
   enclosures.** A bank of 4–6 furnaces, fed by whichever bot is nearest, is
   both faster and what stops the run walling its own bots into a furnace pen.
   Note the direction: fewer furnaces is not the goal — *reused* furnaces is.
   Parallel smelting is what turns 22,850 furnace-ticks into ~5,700.

### What is not the problem

- **Dispatch and RCON overhead.** 1,729 ticks across 318 gaps in a
  97,659-tick milestone; mean gap 5 ticks. 1.8%.
- **Walk failures as a time cost.** Twenty of them, all `no_path`. They cost
  re-selection and they are a symptom worth reading, but deleting them entirely
  buys well under a minute.
- **The hand-mining/drill trade-off.** `11fabe43`'s gate is per-goal with a
  crossover near 43 units (`produce.rs:1021`, `2041-2056`); rung 1's goals are
  5–20 units apart from the one 50-plate `steam-power` trigger, where it does
  fire. Lowering the threshold is not where the six minutes are.
- **Bot idleness treated as an executor problem.** 214 of 244 steps are on bot
  1 before the executor is handed anything.

---

## The tool

`tools/run_analysis.py`, invoked via `just analyse`. Read-only, stdlib-only,
tolerant of a run that is still being written and of a run directory missing
any file. It reports milestone spans, action cost by verb and by subject,
per-bot utilisation, walk failures with repeated destinations, frozen bots
cross-referenced against nearby placements, plan step assignment, and the
production bill per milestone.

Four traps are encoded in it because each one produced a wrong number today:

- **`walk_dispatched` has no `id`.** Joining walks on `id` gives every walk the
  key `None`, which Python accepts, so the join silently attributes every
  failure to one bot. All per-bot walk numbers come off `walk_settled.bot` with
  no join.
- **Action ids restart at 0 with every plan.** The dispatch→settle join is
  scoped to a plan epoch, and unmatched settles are counted and reported rather
  than guessed at.
- **`failure.kind` is incomplete before `1f498593`.** The tool classifies the
  `error` text independently and prints both columns side by side, flagging the
  rows where the recorded kind is `other` but the text is recognisable.
- **Most verbs record zero duration.** Every verb histogram names the untimed
  verbs underneath it, so a "% of action time" figure cannot be read as "% of
  the run".
