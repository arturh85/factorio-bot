# Standing goals: `Sustain`, and the check that a hand-feeding roster fails

2026-09-06, branch `standing-goals-design` (worktree `sustain`), from master
`bda147a8`. **Design only — nothing is implemented.** The artefacts are this
note and two failing test files:

* `crates/planner/tests/standing_goals.rs` — four `#[ignore]`d tests pinning
  the goal's shape and its `holds()` semantics.
* `tools/test_run_analysis_sustain.py` — seven tests, failing (not ignored,
  not skipped) pinning the verification.

## Why the vocabulary needs a fifth kind

`Have`, `Researched`, `Produced`, `Producing`, `Extracted`, `Built`. Every one
of them is **one-shot or structural**, so no plan ever expands capacity and
every run's output stops at exactly the bill its plan was written for. Runs 13,
14 and 15 all plateau at 670 iron plates and 85 red packs around minute 15 and
never move again (`docs/superpowers/plans/2026-09-03-closing-the-idle-gap.md`,
"THE RATE VIEW"); every plateau classifies as *input ran out*; and every
interval before it attributes as **roster-fed** — bots hand-carrying ore and
coal into stone furnaces.

`Producing` is the near miss, and reading it carefully is what this design is
built on. It names a rate and is **satisfied by structure**: enough drills
stand on the right ore, each delivering into a furnace (`method::produce::
holds_producing`, `Condition::Feeds`). Its own doc says what it cannot see — a
drill whose fuel ran out, a furnace whose output backed up, a patch mined out
from under a drill. It is a claim about *capacity*, and capacity has never been
the thing that failed here. **Supply is.**

## 1. The goal, and its exact semantics

```rust
Sustain {
    item: ItemId,
    per_minute: u32,
    window_ticks: Ticks,   // u32, like every duration in this crate
}
```

Read as: *`item` comes out of machines at `per_minute` or better, continuously,
for `window_ticks`, with nothing a bot carried able to explain it.*

* **What is measured.** Items **produced by machines**, from the per-machine
  lifetime counters (`produced` / `produced_source`, `13d45c6b`), summed over
  `machines` samples. Explicitly **not** `force.production.made`, the series
  every rate table in this project is drawn from: hand crafting and hand
  mining pass through no machine and land in that series indistinguishably.
  Explicitly not an output-chest count either, which conflates *made* with
  *moved* and cannot tell a machine finishing a queue from one being resupplied.
* **Over what window.** A single contiguous trailing window of `window_ticks`
  of **game time**, ending at the observation tick — not an average over the
  run, which a burst can carry, and not a mark-to-mark interval, which is fixed
  by the analyser rather than by the goal.
* **Tolerance.** `machine_made >= ceil(per_minute * window_ticks / 3600)`.
  Integer, no smoothing, no allowance. `per_minute` is an integer for the
  reason `Producing`'s already is: this crate's defining constraint is
  byte-identical plans for identical inputs, so no float enters the arithmetic.
* **Plus a provenance test**, which is the half that makes it worth having:
  **zero feeding-verb dispatches** (`FEEDING_VERBS` in `tools/run_analysis.py`
  — `insert`, `stock`, `charge`, `fuel`, `take`, `mine`) inside the window
  **and inside a lead-in before it** (§3).
* **When it is evaluated.** Never by the planner. By the run, at the end of a
  *sustain milestone* that dispatches no actions, and by `just analyse`
  afterwards from the archived record.

### How it differs from the two goals it sits between

| | witnessed by | satisfied by a hand-feeding roster? |
|---|---|---|
| `Produced { item, count }` | an event: N came into existence, by any means including a bot's hands | yes, by design — that is what it means |
| `Producing { item, per_minute }` | structure: machines stand and deliver into one another | yes — the structure can be dead |
| **`Sustain { item, per_minute, window_ticks }`** | a window of history: machine counters plus an idle roster | **no. That is the entire point.** |

### Alternatives rejected

* **`Producing` with a `window` field bolted on.** It would change the meaning
  of a goal that four methods, `holds`, the Lua bridge, the CLI and
  `executor::recover` already agree about, and — worse — it would make one goal
  answer `Some(true)` structurally *and* be measured over a window, which is
  exactly the ambiguity the plateau came from. A fifth kind, as the record
  already called it.
* **`Sustain` as a *modifier* on any goal (`Maintain(Box<Goal>)`).** Attractive
  and premature: `Maintain(Researched(..))` is meaningless (research is
  monotone), `Maintain(Built(..))` is a repair loop nobody has designed, and
  the only member with content is the rate one. Generality with one inhabitant.
* **A rate as `f64` items/second.** Refused for `Producing` already, and the
  same boundary bites harder here: a target that is exactly one machine's
  output sits on a representation edge, and this crate forbids float
  arithmetic in plan decisions.
* **`window` in seconds or minutes.** Ticks are the only clock the record, the
  planner and the mod share; a seconds field would be converted at three sites
  and be wrong at one of them.
* **A default window.** Refused, deliberately, exactly as
  `supervisor.witness`'s `within_ticks` is: the window decides what a failure
  means, and a library that guessed it would hand back a verdict nobody derived.

## 2. What satisfies it, in terms the planner can reason about

A `Produced` goal expands to a bill. A `Sustain` goal expands to **capacity
plus supply**, and the supply half is what does not exist:

```
Sustain{item, rate, window}
├── capacity   Producing{item, rate}            — reuse, unchanged
├── supply     for each input of each machine: a STANDING deliverer
├── power      Condition::Powered{pos, entity, kw} for every electric machine
└── source     enough resource under the drills to last `window`
```

**`holds()` answers `None`.** This is the semantic decision, and
`crates/planner/tests/standing_goals.rs` pins it. `Some(true)` is the lie the
design exists to stop — `AlreadySatisfied` is registered ahead of every method,
so `Some(true)` means an empty plan and a goal that claims itself.
`Some(false)` is its own lie: it asserts the goal is unmet, which nothing in
`PlanState` can know either. `None` already means precisely the right thing in
this crate (`Produced`, `Extracted`): *no reading of the world settles this*.
For `Produced` the unsettleable thing is an event; for `Sustain` it is a
**window of history**, which a pure planner with no clock and no I/O is
constitutionally unable to observe.

Two consequences follow and both are load-bearing:

* Nothing can report a sustain goal already satisfied, so **every expansion,
  including every replan, must yield a plan or a named refusal — never an empty
  network** (test 4).
* And therefore the expansion must be **idempotent against a world that already
  has the arrangement**, re-deriving what is not yet standing the way
  `Goal::Built` does. `tests/standing_site_reuse.rs` is both the precedent and
  the warning: a run whose plans each treated the world as empty ended with 3
  offshore pumps, 2 boilers and 6 assemblers for a goal wanting one of each.

### What existing conditions already cover

* `Condition::Feeds { from, to }` — the right primitive, and already the exact
  shape of a standing relationship: *this machine's drop point lands inside
  that machine's box*. It is a fact about geometry with no duration, which is
  why it can be a precondition at all.
* `Condition::Powered { pos, entity, kw }` — already a *network budget*
  (uncommitted supply, not per-consumer coverage), which is the right shape.
* `Condition::EntityAt`, `AreaFree`, `HasItem` — the construction half needs
  nothing new.
* `method::connect` (`connect_steps`, `route_belt`, `inserter_facing`) — a belt
  run between two machines with an inserter at each end, refusing before it
  places anything. **It has no caller anywhere in the tree.** `Sustain` is the
  caller it has been waiting for, and that is a real risk to state: nothing but
  its own fixtures has ever exercised it, and that is how a geometry defect
  survived four reviews in it.

### What is genuinely new

1. **A recursion with a termination rule.** Supply is `Sustain` applied to the
   inputs: a furnace sustaining 15 plates/min needs 15 ore/min *delivered*,
   which is another `Sustain` on `iron-ore` whose deliverer is a drill. The
   chain must bottom out at something that needs no delivery — a drill on an
   ore patch, a pump on water, a solar panel on daylight. Nothing in the
   planner expresses "this is a source"; today every chain bottoms out at a
   **bot's hands**, which is the defect stated as a fixed point.
2. **Fuel is an input.** A burner machine's coal is a standing requirement like
   its ore. **The blunt consequence: a stage-1 burner cell can satisfy no
   `Sustain` whose window exceeds one fuel stack's drain, unless coal is belted
   in.** One coal is ~1,600 ticks in a drill, ~2,666 in a furnace
   (`DRILL_BURN_TICKS`, `COAL_BURN_TICKS`) — and both constants are admitted
   approximations, because the mod does not send `energy_usage`. Sending it is
   a prerequisite for reasoning about fuel honestly at any other machine.
3. **Resource depletion.** `Effect::ConsumeResource` is emitted by *hand*
   mining only; a drill's draw on its patch is not modelled at all
   (`produce.rs` says so in place). `per_minute * window / 3600` ore must
   remain under the drills, and nothing tracks that.
4. **Demand over time.** `Powered` is checked once, at plan time. Sustaining a
   rate needs generation ≥ demand *for the whole window*, which for steam is
   another fuel supply and for solar is a time-of-day function nothing models.
   This is roadmap item 3 arriving as a dependency rather than a neighbour.

### The hard part, stated plainly

**Every action this planner has is an event with a completion; a supply
relationship has neither.** A plan is a DAG of actions with preconditions,
effects and durations, scheduled to a makespan. "The belt keeps carrying ore"
is not an action, has no duration, and can never be a node. So the planner can
only ever build the arrangement — place belt, place inserter, place drill — and
then assert, as a *condition*, that the arrangement exists. What the
arrangement then *does* is machine behaviour the planner has no model of, and
cannot acquire one without ceasing to be pure.

Hence: satisfaction of a standing goal is not decidable inside the planner, by
construction, and the goal kind is a **contract between two halves**. The
planner owes a structure with no bot in the loop — every input of every counted
machine has a standing deliverer, checkable at plan time and refusable by name.
The record owes a measurement over a window — machine counters, idle roster.
*Neither half is the goal.* A structure with no observation is `Producing`,
which has already been shown standing and dead; an observation with no
structural requirement is the green witness, which has already passed on a
hand-charged cell.

## 3. How a run proves it

The check, written so it can be implemented — and `tools/
test_run_analysis_sustain.py` is its executable form:

```python
sustained_rate(samples, events, item, per_minute,
               window_ticks, lead_in_ticks, at_tick) -> dict
```

1. `required = ceil(per_minute * window_ticks / 3600)`.
2. `machine_made = machine_production(samples, at_tick - window_ticks,
   at_tick)["by_item"].get(item, 0)` — the existing function, unchanged. If
   `available` is false (a run archived before the counters), the verdict is
   **`unknown`**, never `sustained` and never `short`.
3. `feeding_in_window` = `FEEDING_VERBS` dispatches in
   `(at_tick - window_ticks, at_tick]`, from `action_dispatched` events.
4. `feeding_in_lead_in` = the same count over
   `(at_tick - window_ticks - lead_in_ticks, at_tick - window_ticks]`.
5. Verdict:
   * `machine_made == 0 and force delta > 0` → **`hand-made`**;
   * `feeding_in_window or feeding_in_lead_in` → **`roster-fed`**;
   * `machine_made < required` → **`short`** (name the number missed; do not
     blame a cause nothing observed);
   * otherwise → **`sustained`**.

**The lead-in is the part that is new, and the reason the green witness is not
sufficient as a general rule.** That witness — *5 packs reach the output chest
in 90 s with every bot idle* — is the best evidence this project has, and it
still cannot distinguish a fed factory from a charged one:

* 90 s is 5,400 ticks, and the runs that passed it did so in **2,220-2,280
  ticks** — the window closes as soon as the fifth pack lands. A stone
  furnace's input slot holds one stack, 50 ore, at 3.2 s a plate: **9,600 ticks
  of hand-fed running**. A burner drill's fuel
  slot holds 50 coal at ~1,600 ticks each. Every bot can be idle for the whole
  window and every item in it can come from a charge made before it began.
* It counts a *rise in one machine's output inventory*, which is production but
  not throughput: it is equally consistent with a machine draining a queue.
* It requires the roster idle, so it can only ever be a dedicated milestone —
  it cannot check a rate while a run does anything else.

`lead_in_ticks` therefore has **no default and no derivation from the record**:
the mod reports `output_inventory` and `fuel_inventory` and **not input slots**
(`serialize_entity`), so nothing archived can say how much hand-delivered
material was still in a machine when the window opened. It is stated by the
caller, derived from the machines' input capacity and consumption rate — 9,600
ticks for a hand-charged stone furnace on iron. Saying so is better than
computing it wrongly, and it is the same refusal-to-guess as `within_ticks`.

*The stronger check, deferred:* a **hand-credit mass balance** — convert every
hand delivery over the whole run into the maximum output it could ever explain
(through the recipe, and through fuel energy for burners) and require the
window's machine output to exceed the unspent credit. It removes the lead-in
parameter *and* the idle-roster requirement, so a rate could be verified while
the run keeps working. It needs item counts on insert dispatch events attributed
to a specific machine, and it rests on `COAL_BURN_TICKS`-style constants that
are admitted approximations. Second rung, not first.

## 4. What it forces elsewhere — and what it does not

**The implementation surface is the exhaustive matches over `Goal`, not the
enum.** A fifth variant makes these fail to compile, which is the good kind of
failure:

| site | what a `Sustain` arm must answer |
|---|---|
| `crates/planner/src/method/have.rs` — `holds()` | `None`. The interesting one: "is this already true" for a standing rate is a question about a window of history, not a state of the world, and the crate has no clock. |
| `crates/scripting_lua/src/globals/goal/value.rs` — `goal_from_lua` | a `sustain` constructor taking `item`, `per_minute`, `window_ticks`; `window_ticks` required, no default. |
| `crates/scripting_lua/src/globals/goal/plan.rs:837` — `goal.progress` | what "how far along" means when satisfaction is unobservable: report the *capacity* fraction (`Producing`'s answer) and say in the row that it is capacity, not output. |
| `app/src-tauri/src/cli/plan.rs:162` — the goal-string parser | `sustain:<item>:<rate>:<window_ticks>`, beside `producing:<item>:<rate>`. |
| `crates/server/src/game/control.rs` | named by the block-siting session as a match site; **on this base it contains no `Goal::` match at all** — check it on the post-`Site` tree rather than trusting either statement. |
| `crates/executor/src/recover.rs:558` | already special-cases a `Producing` goal no cell can make; a `Sustain` refusal wants the same treatment. |

Written against `Built { blueprint, site: Site }` (with `enum Site { At, Near,
Anywhere }`), the shape landing from the block-siting branch — not the
`anchor: Position` in this worktree.

**Does it need the executor to keep running after a milestone?** No — and this
is the useful answer, because "make runs continuous and event-driven" is
explicitly deferred on the roadmap. What it needs already exists in shape:
`supervisor.witness` is a milestone that dispatches nothing, waits, and asserts
something happened anyway. A `supervisor.sustain{}` is that rung with three
changes: it waits `lead_in_ticks + window_ticks` instead of a few thousand; it
reads *machine counters* rather than one machine's output inventory; and it
asserts a rate rather than a count. What it does need from the executor is that
such a wait is not cut short by an action deadline, and — practically — that
these runs are **headless at 5×**, where a 16,800-tick check costs under a
minute of wall time instead of five.

**Does the goal need to stay true during later milestones?** Yes, in one narrow
and checkable way: later milestones must not *take from* the arrangement — a
`take` emptying its output, a `mine` on the patch under its drills, a re-site
placing something on it. `PlanState` already has the machinery
(`is_resource_claimed`, `covers_claimed_resource`, the buffer ledger); the new
obligation is that a satisfied `Sustain`'s machines and its patch stay reserved
for the rest of the run. It does **not** need continuous re-verification: the
goal is checked once, over its window, and a later milestone that breaks it is
a bug in reservation, not a reason to re-run the check.

**What it does not need:** no change to `Produced` or `Producing` (both keep
their meanings and their tests); no new executor verb; no mod change (the
counters landed in `13d45c6b`); nothing in the video, replay or frontend paths;
no belt or inserter primitive invented — `method::connect` exists and merely
lacks a caller.

## 5. The smallest first rung

**Acceptance sentence** — checkable, and expected to *fail* the first time:

> On seed 31337, a headless run whose last milestone is
> `sustain iron-plate 15/min over 7,200 ticks (lead-in 9,600)` ends with
> `just analyse` reporting, from per-machine counters alone, **≥ 30 iron plates
> made by machines** in the final 7,200 ticks, with **zero** feeding-verb
> dispatches in those 7,200 ticks and in the 9,600 ticks before them — verdict
> `sustained`.

15/min of iron is one burner cell's output, so the capacity half is already
built and proven; the only standing supply it needs is **coal**. No
electricity, no assembler, no research. It is the smallest arrangement in which
the design's real content — an input arriving without a bot carrying it — is
the only thing under test.

What it takes, in order:

1. The goal kind, plus the six match arms above, plus `holds → None` — the
   `#[ignore]`s come off `crates/planner/tests/standing_goals.rs`.
2. A `Sustain` method expanding to `Producing{iron-plate, 15}` **and** a coal
   supply: a second cell on coal, belted into the first's fuel slots via
   `method::connect` — its first caller in the tree, and the first live test of
   its geometry.
3. `sustained_rate()` in `tools/run_analysis.py` and a `--sustain
   <item>:<rate>:<window>:<lead-in>` flag — `tools/test_run_analysis_sustain.py`
   goes green.
4. `supervisor.sustain{}`: the witness rung, on counters, over the window.
5. The run. **Expect `roster-fed` or `short` on the first attempt**, and expect
   the reason to be fuel: that is the finding the rung exists to produce, and
   the first honest measurement of the gap between a cell that stands and a
   cell that feeds itself.
