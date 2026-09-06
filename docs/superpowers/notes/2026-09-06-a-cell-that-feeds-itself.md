# A cell that feeds itself: the mechanism, the price, and what is still a hand

2026-09-06, branch `a-cell-that-feeds-itself` (worktree `selffed`), from
master `e7de6707`, merged onto `2afe55d1` (the hand-credit balance). **Planned and tested offline; not yet run live.** Every
number below is from `factorio-bot plan` against
`workspace/scripts/map.json` (seed 31337, fingerprint `c161fa3f437221d0`)
with a **release** build, or from that dump's own recipe table. Nothing here
is a measurement of a running game, and the three claims that need one are
named at the end.

## The mechanism, and the evidence that chose it

The brief offered two shapes — belt the coal in, or go electric — and asked
for evidence rather than preference. The evidence is the `recipes` section of
seed 31337's own t=0 dump, read directly:

| recipe | `enabled` at t=0 | ingredients |
|---|---|---|
| `burner-inserter` | **true** | 1 iron-plate, 1 iron-gear-wheel |
| `transport-belt` | **true** | 1 iron-plate, 1 iron-gear-wheel |
| `iron-chest` | **true** | 8 iron-plate |
| `inserter` | false | + 1 electronic-circuit |
| `electric-mining-drill` | false | 10 iron-plate, 5 gear, 3 electronic-circuit |
| `small-electric-pole` | false | wood, copper-cable |
| `offshore-pump` / `boiler` / `steam-engine` | false | — |
| `electric-furnace` | false | 10 steel, **5 advanced-circuit**, 10 stone-brick |

**Electric is not one rung but four, and one of them is oil.** Generation
(pump, boiler, engine), the poles to carry it, and the drill itself each need
research before anything electric turns; and the electric *furnace* is behind
advanced circuits — that is, behind oil processing — however much research is
done, so the **smelting half stays a burner whatever happens to the mining
half**. The electric drill is indeed reachable where the furnace is not, as
the brief suspected, but reaching it does not remove the fuel problem: it
moves it from two machines to one.

Belting coal needs **no research at all**. Both items it costs are enabled
from the start. So the mechanism is: **belt the coal, with burner inserters.**

That last word is the part that had to change in the code. `method::connect`
hard-coded `inserter`, the electric one — so its first real caller would have
refused on the bill at t=0 before it ever reached geometry.
`connect_steps_with(.., inserter)` now names the prototype;
`connect_steps` keeps the old default and the old behaviour.

## The arrangement

```
coal patch                                  iron patch (its coal-facing edge)
  drill (16,-28)                              drill (-7,-27) -> furnace (-5,-27)
    |  drops into                                ^                  ^
  chest (16.5,-26.5)                             |                  |
    |--- arm -> belt -> arm --> back into the coal drill   (the source refuels itself)
    |--- arm -> 40 belts -> arm --> chest (-0.5,-30.5)     (the haul)
                                      |--- arm -> belt -> arm -> the cell's drill
                                      |--- arm -> belt -> arm -> the cell's furnace
```

Four `connect` runs, 8 burner inserters, 63 transport belts, 2 iron chests,
2 burner drills, 1 stone furnace. **`method::connect` has a caller.**

Three things in that picture were forced by measurement rather than chosen:

* **The chest exists because a mining drill is not a pickup target.** A drill
  *pushes* to its drop tile; an arm cannot reach into it. So the coal lands in
  a chest, and every belt run — including the one that refuels the drill —
  starts there.
* **There are two chests, not one, and the second one has to stand in the
  open.** A chest is 1×1, so it has four perimeter tiles and each belt run
  claims one plus the cell beyond it. Three runs off one chest refused, twice,
  with all four neighbours named: `no belt route, blocked by 4 tile(s)`. The
  fix that worked was a clearance test — a cell's buffer is sited where the
  **7×7 around it takes a belt** — because with only a 5×5 an earlier run's
  belt turned alongside the chest and ate the perimeter anyway. Both the 5×5
  and the one-chest shape are refusals this branch produced, not guesses.
* **`iron-chest`, not `wooden-chest`, for a planner reason and not a game
  one.** A wooden chest costs 2 wood and **no method in this crate can obtain
  wood**: the whole arrangement refused with
  `NoApplicableMethod { goal: "have 2 wood" }`. Trees are minable in the game
  and every bot starts with one wood; neither fact reaches the planner. That
  is a real gap and it is not fixed here.

## The price, stated as the two plans side by side

Seed 31337, four bots, release build, `master e7de6707` + this branch:

| goal | actions | makespan |
|---|---|---|
| `producing:iron-plate:15` — a cell that stands | 6 | 2,057 ticks (0:34) |
| `sustain:iron-plate:15:7200` — a cell that feeds itself | **289** | **16,206 ticks (4:30)** |

48× the actions and 7.9× the makespan. Most of it is the 63 belts and their
iron: `transport-belt` is 1 plate + 1 gear for two belts, so 63 belts is ~95
iron plates that have to be hand-smelted before the first coal moves.

**And what got worse as well as what got better**: nothing else moved. The
three offline baselines are byte-for-byte where they were —
`researched:automation` **176 / 21,784**,
`producing:automation-science-pack:6` **316 / 22,463**,
`producing:logistic-science-pack:6` **442 / 47,542**. `Goal::Producing` is
untouched and no other goal reaches the new code.

## What is still a hand, measured off this plan

The brief's rule — size the lead-in against **the longest-lasting
hand-delivered input** — needs a number, and this plan gives one. Counting the
coal `insert` actions that land inside a belted burner's own tiles:

| machine | coal by hand | ticks it buys |
|---|---|---|
| coal drill | 1 | 1,600 |
| cell's drill | 6 | 9,600 |
| cell's furnace | 5 | ~13,330 |

The **ignition** is one coal per burner, by design: a burner with an empty
fuel slot never turns over to receive the belt's first delivery. The rest is
something nobody designed and it is the finding of the offline pass: the
`Goal::Have` chain that smelts the belts' own iron hand-charges furnaces, and
`smelt_steps` queues into a furnace **that already stands** — which, once the
cell is up, is the cell's furnace. So the cell is topped up a coal at a time
while it is being built.

Against the run this replaces — 23 coal in the drill (36,800 ticks) and 14 in
the furnace (37,324) — that is a 3–4× reduction, and it is **not zero**.
`scripts/selffed_run.lua` therefore states a lead-in of **20,000 ticks**,
which outlasts the longer of the two, and says in place that it is a claim and
not a derivation: the mod reports `fuel_inventory` and not input slots, so
nothing archived can settle it. The hand-credit mass balance on
`sustain-mass-balance` is what removes the parameter, and this arrangement is
exactly the case it should be judged by.

## What now judges it: the hand-credit balance, not the lead-in

`367fdc15` landed the mass balance while this branch was being written, and
the branch was merged onto it (`2afe55d1`) before anything was measured. It
changes what this cell has to beat, and for the better:

* **The lead-in is no longer the acceptance parameter.** The balance takes
  none. `scripts/selffed_run.lua` still states one, because
  `supervisor.sustain` uses it to decide *how long the roster stays idle*
  before the window, and because the old check still prints beside the new one
  — but nothing rests on the number any more.
* **Belted material is invisible to the balance by design.** That asymmetry is
  the mechanism, and it is exactly why a belted cell can pass where a
  hand-charged one cannot. This arrangement is the first thing built to
  exploit it.

**Projection, from the plan's own hand deliveries — to be confirmed or
falsified by the run, not quoted as a result.** The balance prices each hand
delivery at the most output it could ever explain and combines stages by
maximum:

| stage | coal by hand | priced as iron-plate |
|---|---|---|
| the cell's drill | 6 | 6 x 1,600 / 240 = **40** |
| the cell's furnace | 5 | 5 x 2,666 / 192 = **~69** |
| combined (max) | | **~69 credit** |

Against the archived run's **194**. And the term that should clear it is
`spent`: the cell runs through the whole build tail and the 20,000-tick idle
lead-in before the window opens, which at 15/min is ~83 plates of machine
production — more than the credit — so `outstanding` should reach 0 and all 30
plates in the window should be `unexplained`, which is `sustained`.

**If that is what the run reports, it is the first time the balance has been
shown passing anything**: it has only ever been shown right about a refusal,
because until now no cell fed itself. So the run's report must carry the
balance's own four numbers — credit, spent, outstanding, unexplained — and not
just the verdict word.

One thing to watch that has never been exercised by a real run:
`ActionDispatched.delivery` (item, count, entity, slot) has only ever been
written through a Lua test. If it comes out empty or wrong, the balance falls
back to reading the prose label, and **that is a finding to report rather than
something to work around** — a `credit read from: label` line where `delivery`
was expected means the new field did not survive its first contact with a run.

## The one refusal that is not a halt

`Goal::Sustain` builds an arrangement and then, on the re-plan that finds all
of it standing, **refuses** — `planner::sustain_supply_not_standing` — rather
than returning an empty plan. That is the design's own rule
(`2026-09-06-standing-goals.md` §2, and `standing_goals.rs`'s fourth test): an
empty plan is this planner's word for *done*, `goal.holds` answers `nil` for a
sustain, and whether the rate held is a fact about a window of history.

But with the supervisor as it stood, that made a `Goal::Sustain` milestone
**always** end `stuck`, whichever way it went — refusal or empty plan (which
`goal.holds` turns into `supervisor::unanswerable`). A ladder could never get
past it to the measurement. So `supervisor.lua` now recognises that one code
and closes the milestone `satisfied`, borrowing the reason word
`already_satisfied` — **which is about the buildable half only**. The record
carries `t.sustain_built` with the refusal's own sentence, and the driver
prints `ARRANGEMENT STANDS (the rate is NOT claimed here)`, because a reason
word that a reader takes for a claim about the rate is the
confidently-wrong-object failure this project has paid for twice.

## Three claims that need a running game, and none of them is checked

Stated plainly because the branch is otherwise green and a green suite here
proves less than usual — **this task wrote both the code and its fixtures**,
and the planner fixture it added (a coal patch 14 tiles from the iron, since
the stock one is 44.7 tiles apart and every belt run refuses on it) inherits
`spawn_ore`'s known distortion: ore at integer positions, the one input for
which `EntityGraph`'s flooring round-trip is lossless.

1. **A burner inserter takes its own fuel out of the coal it is moving.**
   The whole arrangement rests on it: eight arms on a coal belt, none with a
   supply of its own. It is well-attested vanilla behaviour and it is
   **not measured here**. If it is false, the cell dies when the arms' 0 coal
   runs out and the run reads `short`, not `sustained`.
2. **An arm delivering coal into a burner machine fills its fuel slot.**
   Same class; assumed from the game's own inserter rules.
3. **`method::connect`'s belts actually move items.** This is its first caller
   ever. Its geometry defect survived four reviews precisely because nothing
   but its own fixtures had run it, and `2026-09-05-belt-routing-first-run.md`
   says to treat "complete and tested" as scoped to those fixtures until a
   belt has moved something in a real game.

The run that answers all three:

```bash
# 5-minute load below 6 first, and tell the owner before starting it.
cp scripts/selffed_run.lua scripts/supervisor.lua <workspace>/scripts/
factorio-bot lua selffed_run.lua --headless --bots 4 --game-speed 5 \
    --seed 31337 --new --settings scratch/headless-v.toml
tools/run_analysis.py --sustain iron-plate:15:7200:20000 <run-dir>
# reports BOTH: the lead-in check and the hand-credit balance beneath it.
```

Quote the `Using mods directory` and `Using scripts directory` lines from it.
**`workspace/scripts` is a copy, not a symlink** — an edit in the checkout does
not reach a run until it is copied over, and a populated directory is left
alone.

## What the next rung is

* **The furnace's output is not taken away.** A stone furnace holds a stack,
  which is far more than a two-minute window at 15/min needs, so this
  arrangement sustains a *window* and not a factory. A belt off the furnace is
  the next rung.
* **Resource depletion is still unmodelled.** `Effect::ConsumeResource` is
  emitted by hand mining only; nothing checks that the coal or the ore under
  the drills outlasts the window.
* **The `Have` chain's reuse of the cell's furnace is the residual hand
  credit.** Reserving a sustained cell's machines from later hand smelting
  would take it to the ignition charge alone, and the design already names
  reservation as the obligation a satisfied `Sustain` puts on later
  milestones.
* **No method can obtain wood**, which is why the buffer is an iron chest.
