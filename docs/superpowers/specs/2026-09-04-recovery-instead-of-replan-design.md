# Recovery instead of replan

**Status:** design only. Nothing below is built. No production code was written
for this document; every claim about behaviour is read off the source or
measured from `workspace/runs/*/events.jsonl`.

**The short version.** `crates/executor/src/recover.rs` is not an unreachable
crate-internal seam. It is fully wired to Lua as `obs:recover()`
(`crates/scripting_lua/src/globals/goal/recovery.rs`, landed `66404580`,
2026-08-31 21:57), documented, and covered by five end-to-end Lua tests.
`scripts/supervisor.lua` was written three and a half hours later and does not
call it — **deliberately**, per decision D2 / failure-class 2 of
`docs/superpowers/specs/2026-08-31-supervisor-loop-design.md`. The premise of
that decision ("planning is ~1s, cheap enough to redo constantly") is *true*,
and I have measured it again below. The conclusion drawn from it does not
follow: the replan is cheap, but **the plan's site choices are not free to
discard**, and a replan discards them all.

Recommendation, in one line: **do the observability change first (S0, ~half a
day), then tier-1-only recovery for the failure classes the record actually
shows (S1, ~2–3 days), and do not attempt plan seeding or general continuation
until a run has shown S1 working.** The large version is about a week; it is
decomposed in §7. Three prerequisite defects (§5) must be fixed *inside* S1, not
after it — one of them is an unbounded loop that today's code has never hit only
because nothing calls `recover`.

---

## 0. Four corrections to the brief

Each of these changes what the right design is, so they lead.

**0.1 — `obs:recover()` exists and is fully wired.** The brief asks whether
this is "an interface that was never wired to Lua". It was wired, on purpose,
with a doc-comment that opens with the exact loop a supervisor would write
(`recovery.rs:9-19`). The log-pairing hazard that makes recovery dangerous is
made unrepresentable at that boundary: a script never holds an `ExecutionLog`,
and `PlanValue::from_recovery` (`plan.rs:256`) decides by `match` on the variant
which log the proposal runs against. **There is no API work to do.** The gap is
entirely in `scripts/supervisor.lua` plus the defects in §5.

**0.2 — the dominant failure class is a walk, and recovery cannot see walks.**
Across all 21 archived runs there are **28 failed/lost actions and 76
failed walks**. `recover()` reads `net.actions()`, `log.status`, `log.attempts`
and `log.failed()` and nothing else; `ExecutionLog` keeps walks in a *separate*
map keyed by `(BotId, step_index)` (`log.rs:301-320`) that `recover.rs` never
touches. Of the 57 replans in the archive, **28 (49%) were triggered by a walk
failure with no action failure at all.** Any design that treats recovery as
"the tiers handle failures" is designing for a third of the problem.

**0.3 — a replan is not "cheap CPU, expensive nothing".** I first measured the
gap from a milestone's last settle to its next `plan_created` at a median of
3,464 ticks and a total of 143 game-minutes, and that number is wrong for the
purpose. Restricting to the 23 replans that have `batch_progress` heartbeats —
which beat every 30 s during a run and stop when it returns — the gap from the
**last heartbeat** to the next `plan_created` is a median of 1,396 ticks and a
maximum of 2,669, of which up to 1,800 is heartbeat granularity. So planning a
400-step milestone costs well under 30 game-seconds. **The earlier figure was
measuring the executor's deadline waits at the tail of a batch, not planning.**
D2's premise stands. I am retracting my own number here rather than leaving it
in, because it is exactly the shape of mistake `CLAUDE.md` warns about.

**0.4 — the two `plan_created` events of `run-1788481380-80843` share three
id+action pairs, not zero.** Minor; the finding is unaffected and is in fact
stronger than the brief states. The real evidence is not id churn but this:

```
succeeded before the replan, tick 13813:  place small-electric-pole at [10.5, -41.5]
the replan's power plant:                 offshore-pump [-5.5, -57.5], boiler [-7, -54.5],
                                          steam-engine [-11.5, -54.5], pole [-13.5, -56.5]
```

**A pole was built, and the replan moved the plant 65 tiles away from it.** The
pole is still standing, unpowered and unconnected, and nothing in the system
knows it is garbage. That is the cost of a replan in one line, and it is a cost
no amount of cheap planning offsets.

---

## 1. What the tiers actually offer

`recover(goal, net, state, bots, log) -> Recovery` is pure: no I/O, no async, no
clock. It is called with a **freshly read** `PlanState::from_world` at the moment
of the ask (`recovery.rs:93`), not the state the failing plan was made from.

| Tier | Predicate | What it proposes | The failure it is for |
|---|---|---|---|
| 0 | `unfinished(net, log)` is empty | `Complete` | Nothing failed. Distinct from an empty schedule so a loop can terminate. |
| 1 | not `exhausted_tier_one` and not `refused_by_the_game`, and `schedule` succeeds | `Rescheduled { net, sched }` — the same actions minus the succeeded ones, re-scheduled against the current world | *Circumstance.* A bot stood in the footprint; a chest was briefly full; the ore is further away; a bot died. The approach still fits. |
| 2 | tier 1 declined or could not schedule; `expand` + `schedule` succeed | `Reexpanded { net, sched }` — a **new plan** for the same goal, numbered from zero, run against a **fresh log** | *The world no longer affords the approach.* The ore patch is gone, not merely further away. |
| 3 | neither | `Surfaced(log.failed())` | Nothing mechanical is left. The seam for a human or an LLM. |

Three properties are worth stating because they are stronger than the brief
assumes:

- **Tier 1 re-validates every retained action's preconditions against the
  current world.** `schedule()` (`schedule.rs:259`) does `state.fork()` on the
  fresh `PlanState` and simulates forward. A pair whose preconditions cannot
  hold is never offered; if no bot can run an action, `schedule` returns
  `PreconditionUnsatisfied` and tier 1 is skipped in favour of tier 2. Tier 1 is
  **not** "replay the old plan blindly".
- **Tier 1 keeps succeeded actions in its network for their lag edges only**
  (`keep_with_lag_bearing_preds`, `recover.rs:252`). `link(smelt, collect, 6000)`
  says the plate is not out of the furnace for another 100 seconds no matter who
  is standing there; dropping the succeeded `smelt` node would drop the lag and
  the retried `collect` would reach into a still-smelting furnace. The whole lag
  is re-waited, not the remainder — deliberately, because over-waiting is the
  safe direction and there is no game clock in that crate.
- **`refused_by_the_game`** (`recover.rs:223`) is the one escalation that skips
  tier 1 outright: a `Place` whose footprint the *game itself* turned down
  (`PlanState::is_site_refused`, fed by `FactorioWorld::placement_refusals`).
  A character standing in the footprint is explicitly *not* that — it is an
  ordinary transient and exactly what tier 1 is for.

And the escalation budget: `MAX_TIER_ONE_ATTEMPTS = 3`, counting the original
run, checked as `status == Failed && attempts >= 3` (`exhausted_tier_one`,
`recover.rs:200`). Tier 2 has **no** budget and cannot have one — it is pure —
so `recover.rs:128-134` states in as many words that the caller inherits the
duty to cap re-expansions.

---

## 2. What the record says the tiers would have handled

21 archived runs. Note before quoting anything from this table: `run-1788432181`,
`run-1788438602` and `run-1788449752` are near-identical re-runs of one scenario
(identical step counts for the first six iterations), so these are **not 21
independent samples**. The shape is reliable; the exact counts are not.

Every segment between one `plan_created` and the next `plan_created` for the
**same milestone** is a replan. There are 57. Classified by what happened in the
segment that was thrown away:

| n | trigger | what `recover()` would answer | worth it? |
|---:|---|---|---|
| 28 | **walk failed, no action failed** | `Rescheduled` — and `schedule` demotes the refused `(bot, destination)` pair to the last tier (`schedule.rs:421-438`), so another bot usually takes the work | **Yes, and this is the case the tiers were extended for.** But see §5.1: nothing bounds the loop. |
| 10 | `cannot place item 'stone-furnace' because a character is standing in the footprint` | `Rescheduled` (`is_site_refused` is false — the game blamed a character, not the ground) | **Yes.** This is the canonical tier-1 transient, named as such in `recover.rs:216-223`. In `run-1788481380-80843` the blocker cleared 53 ticks later. |
| 8 | **no failure at all** — the plan ran to completion and the goal still did not hold | `Complete` → `nil` | **No.** The supervisor must replan. Recovery buys nothing. |
| 7 | `tried to remove N X but removed M` / `cannot insert Nx X, because player only has 0` | `Rescheduled`, three times, failing identically each time, then tier 2 | **Costs 2 wasted dispatches.** See §5.2 — there is no escalation predicate for this class. |
| 3 | action `lost` (`no action result received in time`) | `Rescheduled`, **and the lost action is re-dispatched** | **Dangerous.** See §4.3. |
| 2 | `could not start mining for 301 ticks: another character is standing` | `Rescheduled` | Yes, same class as the placement transient. |

So: **38 of 57 replans (67%) had a trigger tier 1 is designed for**, 8 needed a
replan and would have got one, 7 would have cost two extra dispatches before
escalating correctly, and 3 carry a real double-execution risk.

**What a replan costs, where it can be measured.** The clean instance is
`run-1788481380-80843` (single milestone, single replan, complete record):

- first plan at tick 3,797: 194 steps, makespan 30,268
- 154 succeeded, one `place stone-furnace` failed on a standing character
- replan at tick 15,763 — **11,966 ticks of execution whose plan was discarded**
- the replan's 102 steps re-site the entire power plant; the pole built at
  tick 13,813 is stranded
- run finished at 41,381: **10.52 minutes**, against a first-plan makespan of
  30,554 ticks ≈ 8.49 minutes

The 8.49 is a *plan* number and should be quoted as one. It is defensible as a
projection — the run had done 79% of the plan's steps in 40% of its makespan, so
it was running ahead — but it is not an achieved time and nothing in the archive
proves recovery would have delivered it.

**What I deliberately do not claim.** Across all runs, 1,245,312 ticks (346
game-minutes) elapsed inside iterations that were later replanned — 80–96% of
several runs. That is **not** 346 minutes of waste. The effects of succeeded
actions persist in the world and the replan re-derives from that world, so most
of it is credited. The honest statement is: *the archive is dominated by
iterations whose plan was discarded, and there is exactly one instance in it
where I can point at physical work the replan stranded.* Measuring the general
case needs a semantic diff of consecutive plans that does not exist. See §8.

---

## 3. Why the supervisor does not use them

Not an oversight and not an unwired interface. `2026-08-31-supervisor-loop-design.md`
decides it twice:

> **D2 — Replan on completion or failure only.** No mid-plan abandonment. This
> is what keeps preemption out of scope.

> **Class 2 — `obs.failed > 0` or `obs.lost > 0`.** A world condition, and the
> normal replan trigger. Record `first_error`, return to planning. The next plan
> is computed against the world as it now is, so a mine that failed on exhausted
> ore replans onto different ore without anyone encoding that rule.

and justifies it from cost: *"Planning is ~1s, cheap enough to redo constantly."*

Both halves of that reasoning are sound as far as they go, and §0.3 confirms the
cost claim empirically. What the argument misses is that a replan is not only a
recomputation, it is a **re-decision**. `expand` re-picks sites
(`free_area_near`, the ore selectors), re-sizes bills, and re-binds chains to
bots. Those choices have physical consequences that have already been paid for
in the world, and the world model has no way to say "this pole belongs to a plan
that no longer exists". The design treated "the plan" as a pure artefact and it
is not: **half of it is already built.**

Two smaller reinforcements of the same point:

- The design's own example — *"a mine that failed on exhausted ore replans onto
  different ore"* — is a **tier-2** case. `recover()` reaches it in one step and
  says so by name. D2 chose the tier-2 answer for every failure, including the
  38 that were tier-1 cases.
- The one-line loop that D2 replaced was already written, in the doc-comment of
  the module the supervisor does not call.

---

## 4. What "continue rather than replan" would require

### 4.1 What is already safe, and why

The brief frames this as "`PlanState` is an overlay computed at expansion time
and the world has moved since". That is true of `expand`'s overlay, and it is
**not** what a tier-1 proposal rests on:

- `propose()` reads `PlanState::from_world(origin.world, origin.roster)` at the
  moment of the ask. `origin.world` is an `Arc<FactorioWorld>` — a **live
  handle**, all `DashMap`s, updated by the RCON output parser — not a snapshot.
  So the state tier 1 schedules against is current.
- `schedule()` forks that state and re-simulates. Preconditions are re-checked
  per action per bot; an infeasible action gets no candidate and the whole tier
  is declined.
- Succeeded actions are excluded from the scheduling network, so their effects
  are not double-applied in the simulation — and they are already in the real
  world, which the fork started from.

So continuation does **not** inherit a stale world. It inherits exactly three
things:

1. **The site choices.** Which tile the furnace goes on, where the plant is.
2. **The quantities.** How much ore the expansion decided to mine.
3. **The chain-to-bot bindings** carried in the network's `owner_of`.

Every one of those is a *decision*, and continuing means standing by a decision
made against an older world. That is the actual trade, and — this is the
correction to question 5 — **it is symmetric, not one-sided**:

- Continuing risks building the second half of a layout the world has outgrown.
- Replanning risks stranding the first half of a layout that already stands, and
  it does so *every single time*, unconditionally, whether or not anything about
  the world made it necessary.

The brief calls the replan "robust because it re-derives everything from the
world". It re-derives *feasibility* from the world; so does tier 1. What it
additionally re-derives is *layout*, and the `cells_standing` incident is the
proof that re-deriving layout from a world model that is wrong is not robustness
— the supervisor built more cells beside three working ones. **Neither path is
safer than the other with respect to world-model accuracy. They fail
differently.**

### 4.2 What makes it unsafe: double execution

`unfinished()` retires an action only on `Success`, so `Running` and `Lost` come
back in the proposal. `recover.rs:44-70` names the consequence: `Insert`,
`Remove` and `Place` are not idempotent and will visibly double. `Mine`, `Craft`
and `Research` merely over-produce.

This is not theoretical here. **Three of the 57 replans were triggered by a
`lost` action** — all of them `mine`/`take`, i.e. the benign half — but
`run-1788517971-48257` lost a `take 13 coal from the wooden-chest`, which is a
`Remove`, and `run-1788479942-45523` failed four `take ... from the furnace` in
one segment. A tier-1 retry of a `Remove` that actually landed empties the slot
twice; of an `Insert` that landed, doubles the contents.

`recover.rs:62-70` says explicitly that this is *"a default, not a guarantee"*
and that a caller who cannot tolerate double execution **must inspect the log
for `Running`/`Lost` before dispatching**. Today there is no such caller. Any
supervisor that starts calling `recover` becomes that caller and inherits that
duty on day one.

### 4.3 Where the mitigation belongs

`Lost` means "dispatched, no verdict". It is not the same as `Failed`, and
retrying it is a coin flip. Three options, cheapest first:

- **(a) Refuse to recover a run with any `Lost` action; replan instead.**
  Zero new machinery, uses `obs.lost` which the supervisor already reads. Costs
  us 3 of 57 replans' worth of benefit. **Recommended for S1.**
- (b) Ask the world. `FactorioWorld::actions` is a `DashMap<u32, ActionOutcome>`
  keyed by the dispatch's action id, and `ACTION_RESULT_DEADLINE` is the only
  reason the executor gave up. A late reply may well be sitting there. This is a
  genuine improvement and belongs to a different piece of work ("close the lost
  action out of band"), not to this one.
- (c) Per-`ActionKind` filtering inside `recover`. Rejected: it makes a pure
  function take a policy, and the policy differs per caller. The filter belongs
  in the supervisor.

---

## 5. Three defects that must be fixed *inside* the change, not after it

These are latent today because nothing calls `recover`. The first is the one
that would bite on the very first run.

### 5.1 — Tier 1 has no loop breaker for a walk-only failure

Follow the code for the archive's most common trigger: a walk fails.

1. `run_walk` returns `Halt` → `halt()` → `abandon_rest()`
   (`run.rs:723`) publishes `Status::Failed` **on the watch channels** for the
   rest of that bot's slice.
2. `abandon_rest` writes **nothing to the log** — deliberately, and the comment
   in `run_into` says why: *"we attempted nothing in this run, so a
   never-attempted action stays `Pending`."*
3. So after a walk-only failure the log holds: `Success` for what ran,
   `Pending` for everything abandoned, and **no `Failed` action at all.**
4. `unfinished()` is non-empty → not `Complete`.
   `exhausted_tier_one()` scans for `Failed` with `attempts >= 3` → **false, and
   it will be false forever**, because no action ever gets dispatched, so no
   action's attempt count ever rises.
   `refused_by_the_game()` scans for a `Failed` `Place` → false.
5. Tier 1 fires. `schedule` succeeds — a walk refusal is a **preference, never
   an exclusion** (`schedule.rs:415-438` argues this at length and is right to).
6. Run it. The same walk fails. Go to 4.

**`recover()` will propose `Rescheduled` forever, and `MAX_TIER_ONE_ATTEMPTS`
cannot stop it, because the budget is denominated in a unit the failure never
produces.** With ≥2 bots the reschedule usually reassigns the work to another
bot and this resolves — which is exactly why it is worth doing — but two cases
do not resolve:

- a `walk_lost` (the executor's deadline on a spinning stuck-walk recovery).
  `note_walk_refusal` requires `pathfinder_found_nothing`
  (`walk_memory.rs:57-62`), and a deadline is *not* that, so **nothing is
  remembered and the next schedule has no reason to choose differently.**
  `run-1788344167-58471` produced four of these 87,766 ticks apart.
- a chain whose `owner_of` binds it to the refused bot: an owned chain gets no
  fallback tier (`schedule.rs:346-350`).

**Fix, and it must ship with S1:** the supervisor owns the budget for
recoveries, not just re-expansions. Two rules, both cheap:

- a hard cap of *k* recoveries per plan lineage (k=2 proposed: at most three
  executions of one plan's descendants);
- **"a recovery that made no new progress is not a recovery"** — track
  `obs.success` across the chain and abandon to a replan when it does not
  strictly increase. See §5.3 for why this must be a delta.

The second rule is the one that catches the walk loop on the first repetition,
and it costs three lines.

### 5.2 — No escalation predicate for world-model divergence

`refused_by_the_game` covers exactly one class: a `Place` the game refused.
There is no analogue for the archive's second-largest action-failure family:

```
["tried to remove 4 iron-plate but removed 0"]
["cannot insert 4x copper-ore, because player #2 only has 0. clamping..."]
["tried to insert 17x coal but inserted 3"]
```

7 of 28 action failures. These are not transients. The plan believed a chest or
a furnace or a bot's inventory held something it does not; retrying issues the
identical command to the identical container and gets the identical answer.
Tier 1 will burn all three attempts and then escalate correctly to tier 2 — so
the *outcome* is right, at a cost of two wasted round trips and the walks to get
there.

This is the same finding `docs/superpowers/notes/2026-09-03-buffers-are-visible.md:515`
records as *"`recover.rs`'s tiers have never seen a withdrawal."*

**Option, not a requirement for S1:** a `divergence_observed` predicate beside
`refused_by_the_game`, testing whether a failed `Remove`/`Insert`'s
`ActionFailure` classified as `MissingItem` or `PartialTransfer` — both already
exist in `EventKind::ActionSettled`'s `failure` field
(`crates/core/src/record/mod.rs`, `ActionFailure`). That would escalate the class
on the first failure instead of the third. It is a ~30-line change to
`recover.rs` with tests, and it is independent of everything else here. I would
do it, but *after* S1 has run, so that the measurement of S1 is not confounded.

### 5.3 — A carried-forward log makes three of `obs`'s counters cumulative

`build_observation` (`goal/run.rs:179-341`) computes its counters by iterating
`net.actions()` and `log.walks()` against the log it was handed. On a tier-1
rerun that log is the **previous run's**, so:

- `obs.success` counts the *whole chain's* successes, not this run's. So
  "did the recovery make progress" must be a **delta** — this is why §5.1's
  second rule is phrased as "strictly increases".
- `obs.walks_failed` / `obs.walks_lost` **re-count the previous run's failed
  walks**, because `ExecutionLog::start_walk` overwrites per `(bot, step_index)`
  and the narrower recovery schedule does not reach every index the old one did
  (`log.rs:659-664` documents the overwrite; the *survivors* are the problem).
  The supervisor's `trouble` sum would therefore report a clean recovery as
  having failed walks it did not make.
- `obs.walks` carries those survivors too, so `record.walks(t.walks)` would
  write duplicate `walk_dispatched` / `walk_settled` lines **at ticks earlier
  than the file's current position**, breaking the record's monotonicity and
  double-counting in `just analyse`.

`obs.failed` and `obs.lost` are safe: they read the *current* status of each
action, and a retried-and-succeeded action flips to `Success`.

`record.actions(t.steps, t.actions)` is also safe, and this is worth stating
because it is the one place the plumbing already does the right thing:
`PlanValue.steps` reads `this.schedule.steps` (`plan.rs:1233`), which for a
tier-1 proposal is the **narrow** schedule — only the retried actions. The
succeeded actions live in the wider `net`, which `steps` never touches. **No
duplicate action records.**

**Fix:** the supervisor filters `t.walks` on `(bot, step_index, dispatched_tick)`
against what it has already recorded for this milestone, and computes `trouble`
from deltas. Both are pure Lua, both are in the supervisor where the chain state
already lives. `obs.walks` entries carry all three fields
(`goal/run.rs:298-306`), so this is directly implementable.

---

## 6. Options, cheapest first

### S0 — Record what a replan discards (no behaviour change)

Nothing today distinguishes a first plan from a replan. `PlanCreated` carries
`milestone_index, steps, makespan, bots, plan` and no cause, no ordinal, no
flag; `just analyse` can only infer re-planning from two `plan_created` lines
sharing a `milestone_index`.

Add to `EventKind::PlanCreated` an optional `cause: Option<String>` —
`"initial" | "replan" | "rescheduled" | "reexpanded"` — plus, for a replan, the
count of actions that had succeeded under the discarded plan. Then
`tools/run_analysis.py` can report "milestone 3 discarded 352 completed steps
across 5 replans" without anyone hand-writing a Python one-liner. That number is
the whole justification for S1 and **we cannot currently measure it**.

*Cost:* ~half a day. *Touches:* `crates/core/src/record/mod.rs`,
`crates/scripting_lua/src/globals/record.rs`, six drivers,
`tools/run_analysis.py`, `app/src/lib/runDiff.ts` (additive, optional field).
**Conflict note:** two of those crates are occupied by other agents right now.

*Do this even if S1 is never built.* An unmeasurable improvement is not one.

### S1 — The smallest change that captures most of the value

**Tier-1-only recovery, one attempt, for named failure classes.**

In `Sup:step()`'s `running` branch, after `goal.run(plan)` and before
`self.state = "planning"`:

```lua
-- Recovery is attempted at most `recovery_limit` times per plan lineage, and
-- only when this run made progress the last one did not. A recovery that adds
-- no successes is the walk-refusal loop (see the design's 5.1), and the tier-1
-- budget cannot see it: it is denominated in action attempts, and a failed walk
-- dispatches no action, so no attempt count ever rises.
if self:_may_recover(obs) then
    local ok, next_plan, why = pcall(obs.recover, obs)
    if ok and next_plan ~= nil and why == "rescheduled" then
        self.plan = next_plan
        self.recoveries = self.recoveries + 1
        self.state = "running"
        t.action, t.recovery = "planned", why
        return t                       -- drivers record it as a plan, unchanged
    end
end
```

with `_may_recover` refusing when:

- `obs.lost > 0` — §4.3(a), the double-execution mitigation;
- `self.recoveries >= self.recovery_limit` (2);
- `obs.success <= self.chain_success` — §5.1's progress rule, as a delta (§5.3);
- the run is a witness, or the milestone is already closing.

and refusing `why == "reexpanded"` outright: **tier 2 is a replan by another
name**, it needs its own budget, and letting the supervisor's existing
`iterations`/`stall_limit` machinery handle it is both simpler and already
correct.

Three supporting rules, all in `supervisor.lua`:

- **A recovery does not touch `tracker`.** A tier-1 proposal's step count is the
  remainder — 39 steps where the plan had 194 — and feeding that to
  `tracker.observe` reads as a huge improvement and resets `stall`, hiding a
  genuine stall. Same for `self.iterations` and `self.step_counts`.
- **`trouble` is computed from deltas** across the chain (§5.3).
- **`t.walks` is filtered** before the driver records it (§5.3).

Plus, at the record layer, the `cause` field from S0 so a run can be read back.

*What S1 buys, from the archive:* the 10 standing-character placements and the 2
mining blocks resolve on the first retry (in `run-1788481380-80843` the blocker
cleared 53 ticks later); the 28 walk-only segments get one reschedule that
demotes the refused `(bot, destination)` pair, which is precisely the fix
`schedule.rs:421-438` was built for and has never been exercised. That is
**40 of 57 replans** with a cheaper first response. It buys nothing for the 8
no-failure segments (correct — `recover` says `Complete`), costs nothing for the
3 lost-action segments (refused), and leaves the 7 divergence segments to burn
one extra dispatch before the existing `stall_limit` closes them.

*Cost:* 2–3 days including the §5 fixes and Rust-side Lua tests in the
`recovery.rs` style (a stub `goal` table driving the shipped `supervisor.lua`
via `include_str!`, which `2026-08-31-supervisor-loop-design.md` §D5 already
established).

*Risk:* one new state transition, one new counter, no Rust behaviour change.
The failure mode if it is wrong is "the supervisor replans anyway", which is
today's behaviour.

### S2 — Seed the replan with what already stands

Not "continue the plan" but "make the replan stop re-siting things that exist".
The pole in `run-1788481380-80843` is stranded because `expand` had no reason to
prefer the half-built plant, not because it lacked information.

This is a **planner** change, not an executor or supervisor one: a site-affinity
term in `free_area_near` and the cell selectors, biasing toward existing
same-purpose entities. It is the right long-term answer and it fixes the tier-2
path too. It is also where most of the week goes, it lands in
`crates/planner` (currently occupied), and it interacts with material
convergence and `cells_standing`. **Not now.**

### S3 — Full continuation with resumable batches

Rejected for now, and I want to be clear that this is a scope judgement rather
than an impossibility. The pieces exist — the log carries forward, `run_into`
takes a seeded log, the ids stay meaningful across `Rescheduled` — so
"continuation" in the strict sense is *already what tier 1 does*. What S3 would
add is mid-plan continuation across a supervisor restart, which needs the
`ActionNetwork` and `ExecutionLog` to be serialisable and the savepoint to
carry them. Today `--resume-from` restores **the Factorio world only**: a new
`run-<id>` directory, milestone ladder from 1, no supervisor state, no plan, no
log (`crates/core/src/record/savepoint.rs`,
`crates/core/src/process/process_control.rs:409-426`). Bridging that is a
separate design.

The brief suggests savepoints "may change the calculus". I do not think they do
for this decision: a savepoint is a *coarse* recovery (rewind to the last
satisfied milestone, lose everything since) and recovery is a *fine* one. They
compose — a run that exhausts its recovery and replan budgets is exactly the
run whose next attempt should start from `--resume-from <run>:<k>` — but neither
substitutes for the other.

---

## 7. Recommendation, and the decomposition if the answer is "do the big one"

**Do S0 then S1.** Roughly a day and then two to three, and S1 is only worth
starting once S0 can measure it. Do not start S2 until a run has shown S1
working, because S2's benefit is measured in the same units and confounding them
would leave neither established.

If the large version is wanted, the decomposition is:

1. **S0 — plan provenance in the record** (0.5 d). `cause` on `PlanCreated`,
   discarded-step count, `just analyse` reporting. Blocked on the agents
   currently in `crates/core` and `crates/scripting_lua`.
2. **§5.3 — the carried-forward-log artefacts** (0.5 d). Pure Lua, plus one
   Rust test asserting `obs.success` is cumulative so the delta rule is pinned
   against a regression. Independent of everything else.
3. **S1 — tier-1 recovery in the supervisor, with the §5.1 budget** (1.5 d).
   Lua plus Rust-driven Lua tests. **The budget is not optional and not a
   follow-up** — without it the first walk-only failure loops forever.
4. **One measured run** (`just analyse`, a fixed `--seed`, the same script) to
   establish whether the discarded-step count actually falls. Half a day of
   wall clock, and the result decides whether 5–7 happen at all.
5. **§5.2 — `divergence_observed`** (0.5 d). `recover.rs` plus tests. Escalates
   the withdrawal class on the first failure instead of the third.
6. **Close a `Lost` action out of band** (1 d). `FactorioWorld::actions` already
   holds late replies; reading one back turns 3 of 57 replans from "refuse to
   recover" into "recover safely", and is useful entirely on its own.
7. **S2 — site affinity in expansion** (2–3 d, `crates/planner`). The real fix
   for the stranded pole, and the only item here that also improves tier 2 and
   the plain replan.

Total ≈ 6–7 days. Items 1, 2, 5 and 6 are each independently useful and can be
landed in any order; 3 depends on 2; 4 gates 5–7.

**If only one thing is done:** item 3 with item 2 folded in. It is the whole
behavioural change, and every other item is either measurement for it or an
improvement to something it does not touch.

---

## 8. What I could not determine without a run

Stated plainly, because several of the numbers above are bounded by exactly
these gaps:

1. **How much physical work a replan actually strands, in general.** I have one
   proven instance (the pole). The general answer needs a semantic diff of
   consecutive plans — "which entities did plan *n* build that plan *n+1* does
   not use" — which needs S0's provenance plus a comparison the archive cannot
   support today, because `plan_created` carries labels rather than structured
   placements for anything but the `place` verb.
2. **Whether a tier-1 reschedule after a walk refusal actually reassigns the
   work.** `schedule.rs:421-438` demotes the refused pair to a later tier, and
   the demotion has **never executed in a live run** — every walk refusal so far
   was followed by a full replan, which re-derives the assignment for unrelated
   reasons. Whether the demotion produces a *different* bot in practice, rather
   than the same one because a chain owner pins it, is unknown.
3. **Whether the 53-tick blocker in `run-1788481380-80843` is typical.** One
   observation. If standing characters typically clear in tens of ticks, a
   tier-1 retry is nearly free; if they park for 13,000 ticks (which
   `docs/superpowers/notes/2026-09-02-morning-report.md` documents for run 27,
   before the parked-bot position fix), the retry is a wasted round trip and the
   real fix is the furnace grid geometry. **Both have been observed. I cannot
   say which dominates now**, because the position fix (`c99e2ce2`) postdates
   most of the archive.
4. **Whether a `Lost` action's reply is usually sitting in
   `FactorioWorld::actions`.** This decides whether item 6 is worth a day. It is
   answerable from a single instrumented run and not at all from the archive.
5. **Whether `obs.success` really is cumulative across a tier-1 chain.** I read
   this off `build_observation` iterating `net.actions()` against a carried
   log, and I did not compile or run anything to confirm it. The delta rule in
   §5.1 is correct either way — it degrades to "success must increase", which is
   true of a fresh log too — but a test should pin it before anything depends
   on the reasoning.

---

## Appendix — files a change would touch

| File | Why |
|---|---|
| `scripts/supervisor.lua` | the whole of S1: `_may_recover`, the chain counters, the delta `trouble`, the walk filter |
| `scripts/{automation_speedrun,factory_stage1,factory_stage2,factory_stage3,factory_starter,research_run}.lua` | six drivers; each records `plan_created` on `t.action == "planned"`. Returning the recovery as `"planned"` with an extra `t.recovery` field means **an un-updated driver still records the plan correctly** — the reason to shape it that way |
| `crates/core/src/record/mod.rs` | `PlanCreated.cause` (S0). **Occupied.** |
| `crates/scripting_lua/src/globals/record.rs` | `record.plan_created`'s optional 4th argument. **Occupied.** |
| `crates/scripting_lua/src/globals/goal/mod.rs` (tests) | the Rust-driven Lua tests for `supervisor.lua`, in the `recovery.rs` style. **Occupied.** |
| `crates/executor/src/recover.rs` | only for §5.2's `divergence_observed`, which is deferred. **S1 needs no Rust behaviour change at all.** |
| `tools/run_analysis.py` | report the discarded-step count and the recovery cause per plan |
| `app/src/lib/runDiff.ts` | already narrates recovery actions ("a recovery action, dispatched outside the plan by the executor's recovery tiers", `runDiff.ts:73-74`) — a comment written for a path that has never run. Additive `cause` only. |
