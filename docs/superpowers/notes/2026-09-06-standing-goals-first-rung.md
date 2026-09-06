# The first standing goal, and the pass it should not have got

2026-09-06, branch `standing-goals` (worktree `sustain2`), from master
`3b130eb5`. Implements the design in
`docs/superpowers/notes/2026-09-06-standing-goals.md`. **Read §"The run" first
if you read nothing else: the acceptance sentence PASSED, and the pass is
unearned.**

## What landed

`Goal::Sustain { item, per_minute, window_ticks }` — the fifth kind, and the
first in this vocabulary that means *keep this true*.

| site | what it does |
|---|---|
| `crates/planner/src/goal.rs` | the variant, and `Display` — `sustain 15 iron-plate/min over 7200 ticks` |
| `crates/planner/src/method/have.rs` — `holds()` | **`None`**, always. Satisfaction is a fact about a window of history and this crate has no clock. |
| `crates/planner/src/method/sustain.rs` | the method: plans the capacity half (a `Goal::Producing` subgoal, unchanged) and refuses the supply half **by name** |
| `crates/planner/src/error.rs` | `SustainSupplyNotStanding { item, per_minute, window_ticks, inputs }` |
| `crates/scripting_lua/.../goal/value.rs` | `goal.sustain(item, per_minute, window_ticks)` — window required, no default, on both the constructor and the hand-built-table path |
| `crates/scripting_lua/.../goal/mod.rs` | `__doc_entry_sustain`, and the refusal classified as a **verdict** (a script can act on it) |
| `crates/scripting_lua/.../goal/plan.rs` | progress reports **capacity** and says the word out loud before the numbers |
| `app/src-tauri/src/cli/plan.rs` | `sustain:<item>:<rate>:<window-ticks>` |
| `scripts/supervisor.lua` | `supervisor.sustain{}` — a rung that dispatches nothing for `lead_in_ticks + window_ticks` |
| `tools/run_analysis.py` | `sustained_rate()` and `--sustain <item>:<rate>:<window>:<lead-in>` |

`crates/server/src/game/control.rs` was named by the design as a match site and
**contains no `Goal::` match at all** on this base — checked, not trusted. The
site that actually broke was one nobody had listed: `goal/mod.rs`'s exhaustive
match over **`PlannerError`**, which the new refusal variant made
non-exhaustive. It was caught by `cargo build --workspace --all-features` and
by nothing else — exactly the sibling failure in
`2026-09-06-fixtures-agree-with-their-code.md`, one enum along.

The planner's answer is deliberately narrow. Capacity that does not stand is
planned; capacity that stands is **refused**, naming the inputs nothing
delivers, because `method::connect` still has no caller and a burner cell's ore
and coal arrive as `insert` actions. Neither branch is ever an empty network —
an empty network is this planner's word for *done*, and a standing rate is
exactly what it cannot know is done.

## The run

```
factorio-bot lua sustain_run.lua --settings scratch/headless-t.toml \
    --headless --bots 4 --game-speed 5 --seed 31337 --new
```

`run-1788674059-90744`, seed 31337 (`c161fa3f437221d0`), commit `3b130eb5`
**dirty**, **debug** profile, four character bots at 5x, ticks 2,237 → 21,875
(5.5 min game time, 1.1 min wall). Delivered **4.74x realtime against a nominal
5.0 — 94.8%, ~284 tps of 300**, well clear of the 80% flag; the cell made
exactly its nominal 15/min through the window, which is itself evidence the box
was not starving it.

```
Using mods directory "/home/arturh/projects/private/factorio-bot/workspace/headless-t/mods"
  (debug build; BotBridge is a symlink to
   "/home/arturh/projects/private/factorio-bot/.worktrees/sustain2/mods/BotBridge",
   so an edit there is what the game loads)
Using scripts directory ".../workspace/headless-t/scripts"
  (debug build; seeded by copying ".../.worktrees/sustain2/scripts" ...)
```

Three rungs, all satisfied: build the cell, witness it (1 plate in 420 of 2,400
ticks), then hold 16,800 ticks idle. And then:

```
iron-plate 15/min over 7200 ticks (lead-in 9600): SUSTAINED
  window 14675 -> 21875; needed 30, machines made 30, force made 30
  feeding dispatches: 0 in window, 0 in lead-in   (source: counters)
```

**We expected `roster-fed` or `short`, for fuel. We got `sustained`. It is
wrong, and the reason is a number in the design note.**

## Why the pass is unearned

Every dispatch the run ever made, in full:

```
2430 place stone-furnace at [-13, -12]
2463 place burner-mining-drill at [-13, -14]
2926 chop huge-rock for 24 coal + 24 stone
3756 chop huge-rock for 24 coal + 24 stone
4520 fuel the burner-mining-drill with 23 coal (36800 ticks, 153 iron-plate, then it stops)
4521 fuel the stone-furnace with 14 coal
```

The window opens at 14,675 — **10,154 ticks after the last feeding action** —
and the lead-in of 9,600 reaches back only to 5,075. The machine samples close
it:

| tick | drill coal | furnace coal | drill produced | furnace produced |
|---:|---:|---:|---:|---:|
| 4,800 | 22 | 13 | 1 | 0 |
| 9,600 | 19 | 12 | 21 | 20 |
| 14,700 | 16 | 11 | 42 | 41 |
| 21,900 | 12 | 8 | 72 | 71 |

Coal falls monotonically and nothing tops it up. **The entire window ran off
one hand charge**, and the planner's own action label said so as it made it:
*36,800 ticks, 153 iron-plate, then it stops.*

The error is not in the check. It is in the **lead-in**, and specifically in
which input it was sized against. 9,600 ticks is a stone furnace's *ore* input
stack (50 ore x 192 ticks). But **the ore never needs a hand**: a burner drill
mines it and its drop point lands inside the furnace, which is a standing
supply — the one this arrangement already has. The only hand-delivered input is
**coal**, and a drill's fuel slot holds 50 of them at ~1,600 ticks each:
**80,000 ticks**, eight times the lead-in that was named.

Re-read from the same archive, changing nothing but that parameter:

| lead-in | reaches back to | verdict | feeding in lead-in |
|---:|---:|---|---:|
| 9,600 | 5,075 | **sustained** | 0 |
| 10,154 | 4,521 | sustained | 0 (half-open interval; 4,521 is excluded) |
| **10,155** | 4,520 | **roster-fed** | 1 |
| 36,800 | −22,125 | **roster-fed** | 2 |

The honest verdict for this cell is `roster-fed`, and it takes a lead-in one
tick longer than the gap to the last `fuel` to see it.

## The rule this produces

**A lead-in must exceed the drain of the LONGEST-LASTING hand-delivered input,
not the first one that comes to mind.** For a stage-1 burner cell that is the
drill's coal (up to 80,000 ticks), not the furnace's ore (9,600) — and the ore
is not hand-delivered at all. The design note derived its number from the wrong
half of its own example, and the run is what found it: nothing in the code,
the tests or the prose could have.

Two consequences, both larger than a parameter:

* **The acceptance sentence in the design note is satisfiable with no standing
  supply whatsoever.** A hand-charged burner cell passes it. As written, the
  first rung does not test the thing it was written to test, and quoting its
  pass would have been the confidently-wrong-object failure this project has
  now paid for three times.
* **The lead-in is the wrong instrument, and the design already knows it.**
  §3's deferred *hand-credit mass balance* — convert every hand delivery into
  the maximum output it could ever explain, through the recipe and through fuel
  energy, and require the window's machine output to exceed the unspent credit
  — removes the parameter entirely and would have refused this run without
  anybody choosing a number. It is now the next rung, not the second one.

What the run does establish, and it is worth having: the capacity half works
end to end, a cell built by the planner ran **flat out at its nominal 15/min
for 7,200 uninterrupted ticks with every bot idle**, and the machine-counter
instrument agrees with the force's own statistics to the item (30 and 30). The
gap between a cell that stands and a cell that feeds itself is now measurable;
this run measures it at **one fuel charge**.

## What was written alongside its own tests

Stated because the house rule asks for it. This task wrote both the
implementation and every new test in it, so each fixture is a statement its own
author had an interest in. What they assume:

* `crates/planner/src/method/sustain.rs`'s refusal test reaches the
  capacity-stands branch by asking for a **rate of zero** (zero cells needed,
  zero standing) rather than fabricating a standing cell in the overlay — the
  refusal path is the same one, and no hand-typed geometry is involved.
* The supervisor tests use the existing `witness_harness`, whose world fixture
  is not this task's.
* Every new test was watched failing under a substitution of the value it
  forbids, **with the substitution count asserted first** (`scratch/falsify.py`
  and a second round by hand): `holds -> Some(true)` reddens three of the four
  planner tests; an empty plan where the method should refuse reddens the
  refusal test; dropping the lead-in reddens the lead-in test; reading
  `production.made` instead of machine counters reddens the hand-made test;
  a claimed verdict instead of `deferred` reddens the supervisor test; a
  defaulted window reddens the no-default test. No case came back green.

## One fixture was wrong, and it is corrected in the open

`tools/test_run_analysis_sustain.py`'s first test expected `machine_made == 75`
for a furnace counting 5 an ORE-beat over a 7,200-tick window. The window is 24
beats of 300 ticks, so it is **120**, and the sibling test at 1 a beat expects
24 — the same 24 beats. The two expectations contradicted each other and only
one could be right. Corrected to 120, with the arithmetic written into the
docstring rather than fixed silently in `sustained_rate`.

## Also found, not fixed

`crates/scripting_lua/.../goal/value.rs`'s `KINDS` omits `"charted"`. Only
`goal.all` consults that list, so a charted goal works everywhere except inside
a bundle, where it is rejected as an unknown kind. `"sustain"` was added;
`"charted"` was left, with a comment in place, for whoever owns exploration to
fix with a test of its own.
