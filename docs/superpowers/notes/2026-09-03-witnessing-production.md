# Witnessing production: the half of "done" that watches instead of planning

**Date:** 2026-09-03. `supervisor.witness` exists. It is the durative half of
`docs/superpowers/specs/2026-09-03-starter-factory-design.md` §9.4, the thing
`2026-09-03-producing-stage-1.md` §5 named as the reason stage 1's own "done
when" was not met, and the answer to the objection that a `Producing` goal can
be satisfied by machines that stand and make nothing.

**Nothing here has been run against a game.** Every claim below is source,
tests, the mod's own serialiser and the vanilla prototype numbers.

---

## 1. What it is, in one sentence

A milestone that **dispatches no actions at all**, waits a stated number of game
ticks, and asserts a machine's *output* inventory rose. Because no bot acted
during the window, an item that appeared can only have been made by a machine.

That is the property `goal.holds` cannot have, and it is why the two halves are
not redundant: the planner answers *structure* — a burner drill stands on ore
and drops into a fuelled stone furnace — and it answers yes for a cell whose fuel
has run out, whose output has backed up, or whose patch is exhausted. Rows 1, 3
and 4 of the spec's failure table. `PlanState` reads no fuel level and no
container contents, so it cannot see any of the three, ever.

```lua
supervisor.witness {
    item = "iron-plate",
    from = "burner-mining-drill", into = "stone-furnace",
    near = { x = 0, y = 0 }, radius = 300,
    at_least = 1, within_ticks = 2400,
}
```

It is a **rung in the ladder**, handed to `supervisor.list` beside the goals, and
`Sup:step()` recognises it in the `planning` state and never calls `goal.plan`
for it.

---

## 2. The four decisions, with the reasons rather than the defaults

### 2.1 How long it waits, and what a timeout means

**Stage 1 waits 2400 ticks, and only a dead cell pays for them.**

The planner's own arithmetic for the cell (`method/produce.rs`) is

```
drill:   ceil(60 * ore.mining_time / drill.mining_speed) = 240 ticks/ore
furnace: smelting_ticks(recipe, stone-furnace) / yield   = 192 ticks/plate
```

Those are two stages of one pipeline, so the **first** plate is `240 + 192 = 432`
ticks away — the fill, not the period — and every plate after it is 240, because
the drill is the bottleneck. 2400 is 5.5× the fill.

The margin is **not** for wall-clock slowness. The witness counts *game ticks*,
so a server running below 60 UPS costs it seconds and not ticks; that is exactly
what `FactorioRcon::game_tick`'s own doc comment was written for, after
`run-1788320177-77989` waited "4032 ticks" in wall clock, got ~3599 real ones,
and found 18 plates where it had modelled 20. The margin is for the two numbers
above being a **model**: `mining_speed`, `mining_time` and the smelting time are
read off prototypes, and the entire reason to witness a cell is that the model
can be optimistic.

The cost is asymmetric on purpose. With `at_least = 1`, a working cell stops the
wait the moment it has proved itself — about 500 ticks, eight seconds — and only
a cell that produces nothing runs the clock out. `a_cell_whose_output_rises_is_
witnessed_and_the_wait_stops_early` pins the short circuit, and
`a_cell_that_stands_and_produces_nothing_halts_with_its_own_code` pins that the
dead verdict is only given after the full window.

**`within_ticks` has no default**, and that is a decision. It is the number that
decides what a failure *means*; a library that guessed it would hand back a
verdict nobody derived. `supervisor.witness` raises at construction without it —
twenty minutes before the run, not twenty minutes into it.

**A timeout is a verdict, not a fault.** It closes the milestone `stuck`,
carrying its reason on `t.refusal` — the same channel and the same family as a
planner refusal (`7de7c6a0`) and `supervisor::unanswerable` (`ee623717`). No
driver has to learn anything: `scripts/factory_stage1.lua`'s existing `halted`
branch already reads `t.refusal.message` and hands it to
`record.milestone_stuck`.

### 2.2 Which machine it watches, and how it finds it

**The machine something drops into, according to the game itself.**

`rcon.find_entities_in_radius` returns `FactorioEntity`, and the mod's
`serialize_entity` fills in `drop_position`, `bounding_box` **and**
`output_inventory` on every entity it sends. So one filtered call finds the
drills, a second finds the furnaces, and a furnace is watched exactly when some
drill's own reported drop point lands on one of its tiles. Two consequences
worth stating:

* **No copy of `delivery_offset`.** The planner's north-frame table is
  hand-written, and row 5 of the spec's failure list says so: a wrong table is a
  wrong answer that every test agrees with. A witness that re-derived the drop
  point from the same table would inherit the same error and agree with it. This
  reads what the game reports.
* **The hand-smelt furnaces are excluded by construction.** Stage 1's bill
  smelts nine plates by hand, in stone furnaces of its own; nothing drops into
  those, so they are not watched. The fixture carries such a decoy holding two
  plates a bot put there, and
  `a_furnace_nothing_drops_into_is_not_watched_however_full_it_is` asserts it is
  not in the baseline.

  **The mutation for this found something worth writing down.** M3 — watch every
  candidate, feed or no feed — does *not* fail
  `a_cell_that_stands_and_produces_nothing_halts_with_its_own_code`, because a
  witness measures a **delta**: a decoy's leftovers sit in `before` as well as in
  `after` and cancel. So the exclusion is not what stops a *static* decoy; the
  delta already does. What the exclusion buys is a decoy that is still **smelting
  at T0** — a bot's ore finishing mid-window, which is machine-made in the
  trivial sense and is not evidence about the cell. That case is not in the
  fixture and is therefore not tested; it is the reason the rule exists, and the
  three tests M3 *does* fail are about the watch set and the baseline rather than
  about the verdict.

**Tile containment, not box containment, and it is not a stylistic choice.** A
burner drill at an integer position facing north drops at `(-0.35, -1.3)` from
its centre; the stone furnace two tiles north has a collision box of
`±0.69921875`, so its near edge is at `33.69921875` against a drop at `33.7`.
**The drop point is outside the box by 0.00078125 of a tile, one part in 1280.**
Box containment would answer "not fed" for the one layout stage 1 exists to
build. `PlanState::delivers_into` decided this the same way for the same reason;
`the_drop_point_that_misses_the_box_by_one_part_in_1280_still_feeds_it` asserts
the miss *as a number* so nobody later tidies it into a box test.

**Scope, said plainly: this is the one-link case.** `from`/`into` name the two
ends of a single machine-to-machine link, which is the whole of stage 1's shape.
A longer chain — drill to furnace to gear assembler to pack assembler — means
choosing *which* end is the terminal, and that is the caller's decision because
only the caller knows which end the goal was about. Generalising it is stage 2's
problem, and inventing an answer now would be inventing it without the chain to
check it against.

### 2.3 How a dead cell reads differently from an unbuilt one

Three codes, because the three have three different fixes and the worst thing a
witness can do is report "your cell is dead" for a cell that was never built.

| code | means | what it does not claim |
| --- | --- | --- |
| `supervisor::no_cell` | nothing in the area is fed by anything | says **nothing** about production, and the sentence says so in those words |
| `supervisor::not_producing` | a cell stands, the full window elapsed, output did not rise | — this is the verdict the witness exists to be able to give |
| `supervisor::witness_inconclusive` | the poll budget ran out before the window did, or the game would not report a tick | nothing either way |

All three close `stuck` and carry `refusal.code` + `refusal.message`, which is
the shape a planner refusal and `supervisor::unanswerable` already take, on the
channel every driver already reads. `code` is what tells them apart without
reading the sentence, and no `PlannerError` can produce any of the three.

The third one matters more than it looks. There is no `sleep` in the sandbox, so
the wait is a poll loop, and a game whose clock is **not** advancing — paused,
saving, gone — would spin forever without a cap. Hitting the cap must not read
as "produced nothing": that is the same mistake as recording an unanswerable
goal as a satisfied one, which this loop spent `ee623717` removing.
`a_clock_that_does_not_advance_is_inconclusive_and_not_a_dead_cell` asserts the
code *and* asserts the message does not contain "produces nothing".

A witness with no `rcon` at all is neither — it **raises**. Nothing about the
world was established and the script was built wrong; halting quietly would
record a condition of the world for a defect in the run, which is the rule
`refusal_of` already follows for an unclassifiable raise.

### 2.4 Always, or on request

**On request: it is a rung the ladder writes.** Three reasons, and the third is
the one that decides it.

* A witness costs real game time, and charging every milestone for one would be
  paying to watch a `have` goal that has no machine in it.
* An automatic witness cannot know `at_least`, `within_ticks`, or which two
  entity kinds form the link — it would have to invent all three, and §2.1 is
  the argument against inventing the first.
* The spec asks for it this way and says why: that every `Producing` milestone
  is followed by its witness is a *fact about what a run proves*, and belongs in
  the run rather than as a code invariant.

The prompt's objection to this — "a witness nobody runs is theatre" — is real
and is answered with a test rather than a promise.
`factory_stage1_lib::the_ladder_witnesses_the_cell_it_just_built` drives the
**shipped** `scripts/factory_stage1.lua` and asserts it has two rungs, that the
second one is the witness, and that the build rung planned once while the witness
rung planned not at all. `a_cell_that_produces_nothing_ends_the_run_stuck_rather_
than_done` is the same script over a world where the cell stands and makes
nothing: it now finishes `stuck` with the witness's own sentence on the record,
where before this change it finished `done`.

That file had no Rust test at all before today.

---

## 3. What had to change outside `supervisor.lua`, and why the spec was wrong
   about one thing

**The spec's "no Rust change, no planner change, no executor change, no mod
change" is false, in exactly one place, and it is a load-bearing place.**

Two of its three supporting claims check out:

* `rcon.inventory_contents_at` *is* already exposed to Lua
  (`globals/rcon.rs`), and the mod's `rcon_inventory_contents_at` *does* return
  `output_inventory` and `fuel_inventory` and not input slots
  (`control.lua:3931`). Verified rather than taken. In the end the witness uses
  `find_entities_in_radius` instead, because that one call carries the inventory
  *and* the `drop_position`/`bounding_box` the watch set is derived from — one
  round trip where the spec's shape needs two, and no position for the caller to
  supply from knowledge it does not have.
* The supervisor does already take a keyframe at every milestone boundary, and a
  witness milestone gets one like any other
  (`a_witness_milestone_is_closed_and_gets_its_keyframe`).

The third does not. **There was no way to read the game's clock from Lua without
dispatching an action.** `rcon.last_tick` reports the stamp on the last *timed*
call, and the timed calls are exactly the ones that act — `mine`, `craft`,
`place_entity`, `insert_to_inventory`, `remove_from_inventory`, `move`,
`add_research`, `frame_capture_start`. Its own doc string says the query calls
never advance it. A loop that dispatches nothing therefore reads a frozen number
and concludes that no time passed, which is the exact opposite of what a witness
needs to be able to say.

So `rcon.game_tick()` is new, in `crates/scripting_lua/src/globals/rcon.rs`. It
wraps `FactorioRcon::game_tick`, which already existed, is already tested, is a
plain `/silent-command` needing no mod, and whose doc comment was written for
precisely this: *"Anything that means 'wait for the machine to do N ticks of
work' has to read this clock; converting ticks to seconds and sleeping is a
different, weaker claim."* It dispatches nothing.

**Files touched outside the stated boundary, reported rather than assumed:**

* `crates/scripting_lua/src/globals/rcon.rs` — the new binding and its
  `__doc_entry_game_tick`. `doc_guard`'s
  `every_doc_entry_names_a_binding_and_every_binding_a_doc_entry` is
  bidirectional, so one without the other fails the build.
* `crates/scripting_lua/src/lua_docs.rs` — one line in the `EXPECTED` list for
  `rcon.lua`, which pins the generated documentation.
* `crates/scripting_lua/src/lib.rs` — one line, `pub mod factory_stage1_lib;`.
* `crates/scripting_lua/src/factory_stage1_lib.rs` — new, mirroring
  `research_run_lib.rs` exactly. §2.4.

Nothing in `crates/planner`, `crates/core`, `crates/executor`, `mods/` or
`app/src/` was touched.

### 3.1 One thing that is imprecise, and is not hidden

A satisfied witness reports `reason = "already_satisfied"`.
`record.milestone_satisfied` takes one of exactly two `SatisfiedReason` strings
and refuses anything else *by name* (`parse_satisfied_reason`, deliberately, so
no future reader has to wonder whether the value ever meant what it says). A
third variant lives in `crates/core/src/record/mod.rs`, is mirrored into
`app/src/api/types.ts` and the OpenAPI snapshot, and is a cross-boundary change
this task did not make.

Of the two available, `already_satisfied` is the one that is not a lie: no
planning was attempted and the world met the milestone without being asked to do
anything. What was actually observed rides on `t.witness` — `before`, `after`,
`gained`, `at_least`, `elapsed_ticks`, `within_ticks`, `polls`, `watched`,
`missing` — and `factory_stage1.lua` prints those numbers rather than the reason
word. The milestone's recorded *name* says "witness" out loud, which is the only
thing in the record that does.
`a_satisfied_witness_carries_a_reason_the_record_will_accept` pins the string,
because getting it wrong raises inside the recorder and takes the run with it.

---

## 4. Determinism

The ladder's byte-identical plans are untouched, and the mechanism is one line:
the witness is recognised in `Sup:step()`'s `planning` state and returns before
`goal.plan` is reached, so a witness milestone costs the planner **zero
expansions**. `a_witness_dispatches_nothing_at_all` asserts `__plan_calls`,
`__run_calls` and `__holds_calls` are all zero for a witness rung, and
`a_witness_beside_a_goal_leaves_the_goals_own_milestone_alone` asserts a ladder
of `{goal, witness}` still plans exactly once.

The witness reads no order it does not control: `fed_machines` preserves the
candidate order the game returned and the caller sums over the result, so the
sum does not depend on it; the watch set is keyed by `name@x,y` at 1/100000 of a
tile, finer than the 1/256 the game stores positions at.

`PlanState` is never consulted and never mutated. A witness cannot change what
the next plan looks like.

---

## 5. Evidence

### 5.1 Red-first, and where it was not

**This was not written test-first, and saying otherwise would be a decoration.**
The pure helpers (`delivers_into`, `fed_machines`, `count_item`) and
`Sup:_witness` were written before their tests, and the whole `supervisor_lib`
suite — 51 tests with the 15 new ones in it — was green on its first run. The stage-1 note made the same
admission for `method/produce.rs`; this is the same shape and deserves the same
sentence.

What the tests were held to instead is the battery in §5.2, applied one mutation
at a time to the fixed tree with the whole `--lib` suite run and the tree
restored between each. The one that stands in for the missing red is **M1**,
which removes the two-line dispatch in `Sup:step()` — the feature, absent — and
is quoted verbatim below because it is the failure a genuine red-first would have
started from.

```
---- supervisor_lib::tests::a_witness_dispatches_nothing_at_all stdout ----

thread '...' panicked at crates/scripting_lua/src/supervisor_lib.rs:1307:9:
assertion `left == right` failed: a witness must not cost the planner an
expansion, or a ladder with one in it would plan differently from a ladder
without
  left: 1
 right: 0
```

Without the dispatch, the witness table falls through to `goal.plan`, the stub
returns an empty plan, `goal.holds` says `true`, and the milestone closes
**satisfied** — a factory reported witnessed on no observation at all. Eleven
tests go red on it, including both of `factory_stage1_lib`'s.

### 5.2 The battery

Each mutation applied alone to the fixed tree, the whole
`-p factorio-bot-scripting-lua --lib` suite run (289 tests, one ignored), the tree restored
between each.

| Mutation | Failed |
|---|---|
| M1 the `is_witness` dispatch removed from `Sup:step()` (the feature, absent) | **11** — every witness test plus both ladder tests |
| M2 `delivers_into` tests the collision **box** instead of the tile | **10**, incl. `the_drop_point_that_misses_the_box_by_one_part_in_1280_still_feeds_it` — this is the 0.00078125 |
| M3 `fed_machines` watches every candidate, fed or not | 3 — and *not* the dead-cell verdict; see §2.2 |
| M4 `count_item` reads the **fuel** inventory instead of the output | 5 |
| M5 an inconclusive wait is reported as `not_producing` | 2 |
| M6 the wait ends after one poll (`over = true`) | 6 |
| M7 no short circuit: look only once the window is over | 1 — `..._and_the_wait_stops_early` |
| M8 `within_ticks` gets a default | 1 — `a_witness_with_no_deadline_is_refused_at_construction` |
| M9 `machine_key` forgets the position and keys by name alone | 5 — the decoy re-enters the watch set on re-read |
| M10 the "no game to witness in" guard removed | 1 |
| M11 a witnessed milestone reports `plan_empty` | 1 |
| M12 `at_least` compared with `>` instead of `>=` | 2 |
| M13 a witness halt does not `_close` its milestone | 2 — incl. the keyframe |

### 5.3 The negative controls, stated as such

Three mutations that fail **nothing**, run for the same reason the stage-1 note
ran its `fit` trial-check control: a battery with no negatives does not know
whether it is measuring anything.

| Mutation | Failed | Why that is the right answer |
|---|---|---|
| `probe_ticks` default 60 → 30 | nothing | no test pins the probe cadence, and none should: it trades round trips against how fast a working cell stops the wait, and both values are correct |
| `TOUCH_SLACK` 1/512 → 0 | nothing | the slack keeps float noise off a box edge, and the fixture's numbers are exact binary fractions nowhere near one. It is inherited from `PlanState` for consistency, and **this measures that it buys nothing here** |
| `delivers_into` drops the `source ~= target` self-feed guard | nothing | no entity in the fixture drops into its own box, and a stone furnace reports no drop position at all. The guard is for a machine kind that does not exist in stage 1 |

---

## 6. What this still cannot tell us

* **It catches a dead cell once.** Fuel runs out 22 minutes after a full slot;
  the witness sees that at the next witness milestone and nothing sees it in
  between. Row 1 of the spec's table, unchanged and unchangeable by anything
  that is not a periodic check.
* **It does not say *why* a cell is dead.** Fuel, backed-up output and an
  exhausted patch all read as `not_producing`. Distinguishing them needs the
  fuel inventory (which the mod already sends and this does not read) and the
  input slots (which the mod does not send at all). Reading fuel would be a
  cheap next step and would turn one verdict into two.
* **`at_least = 1` proves production, not *rate*.** The goal is
  `Producing{iron-plate, 15}` — fifteen a minute — and one plate in 2400 ticks is
  evidence of a working machine, not of 15/min. A witness that checked the rate
  would have to wait long enough to measure one, which is minutes, and would
  then be measuring the model's own arithmetic back at itself. The honest claim
  is the one made: the cell produces.
* **Nothing here has met a game.** The two things a live run would settle first
  are whether a burner drill's output really lands in a stone furnace two tiles
  away (§15.1 of the spec, still open) and whether ~2400 rcon round trips over
  40 seconds contend with four graphical clients badly enough to matter. The
  poll loop is paced by Factorio processing rcon once per tick, so it should be
  about one call per tick; that is an inference from how the game works, not a
  measurement.
* **`from`/`into` are names.** A witness pointed at the wrong two entity kinds
  finds no cell and reports `no_cell` — correctly, and for a reason the reader
  will have to work out. There is no check that the pair is the pair the goal was
  about, because the goal value is opaque to Lua.
