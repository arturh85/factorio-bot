# A belt off the furnace: what an offtake costs, and the four refusals that shaped it

2026-09-06, branch `a-belt-off-the-furnace` (worktree `offtake`), branched from
master `3d013ec9` and **merged up to `7ebe707d` before anything was measured**.
Every offline number below is a **release** build against
`workspace/scripts/map.json` (seed 31337, fingerprint `c161fa3f437221d0`), and
the before/after pair was taken on **one commit** — the "before" column is this
same tree with `crates/planner/src/method/sustain.rs` reverted to the merged
HEAD's own version and rebuilt, not a number quoted from an earlier note
against a different commit.

That re-measurement was not ceremony. The merge brought `b8c28831` (ghost site
markers) into `crates/planner/src/state.rs` and `method/have.rs`, so the tree
the first pass measured no longer existed. Both columns and all three baselines
were taken again on the merged commit; **every one of the five numbers came
back identical**, which is a fact about this change's scope and not a licence
to skip the check next time.

## What this is for

`run-1788679826-02267` is the measurement. A belted burner cell ran with **no
bot in the loop for 27,249 ticks** at **100% of nominal tick rate** — and came
back `SHORT`, 11 of 30. The furnace's own status line said why:

```
stone-furnace [-5.0, -27.0]  working 62% of samples, finished 118
  status: {working: 80, no_ingredients: 19, no_fuel: 15, full_output: 15}
```

Nothing took the plates away, so the output slot filled and the machine
throttled itself: iron-plate production decayed **166 → 72 → 8** across the run
while coal and ore held flat at ~16/min. The cell sustained a *window*, not a
rate.

## What it is

**One burner inserter on the furnace's perimeter, a chest on the tile beyond
it, and a belt branch that keeps that arm fuelled.**

```
            plate chest  <- arm  <- FURNACE <- drill (on ore)
                                     ^   ^
   coal ...belt... arm ---------------   |
        \                                |
         `- arm -> belt -> arm -> the offtake arm's own fuel slot
```

Three things in that picture were forced by a refusal, not chosen.

### 1. One arm and a chest, not a `connect` run

A belt run costs **two** arms, and every arm on this offtake needs coal
delivered to it (see §2). Two fuel branches do not fit; one does. So the
offtake is placed directly — three collinear tile centres off one face of the
furnace — rather than through `method::connect`. The facing still comes from
`connect::inserter_facing`, and both deliveries are then asked of a **fork with
all three entities standing**, so the check is `PlanState::delivers_into`'s own
answer and not a restatement of the offset table that sited it.

### 2. The offtake arm cannot fuel itself, and that is a property of its cargo

The whole self-feeding cell rests on *a burner inserter takes its own fuel out
of the coal it is moving*. That is a property of the **cargo**. This arm moves
iron plates and can never take a plate as fuel. Given only an ignition charge
it would stop the moment that coal burned through, and the cell would go
quietly back to filling its output slot — the failure this rung exists to
remove, returning by the back door.

It gets **no ignition charge at all**, and needs none: an arm being *filled*
does not have to swing to receive, and `run-1788679826-02267` placed eight arms
with no charge and every one of them started.

### 3. Its coal is branched off a BELT, because a 1x1 chest cannot carry four runs

The obvious answer — a fourth run leaving the cell's coal buffer — was tried
first and refused. **A `wooden-`/`iron-chest` is 1x1: four perimeter tiles,
each run claiming one plus the cell beyond it, and the earlier runs' *routes*
hug what is left.** Three (haul in, drill, furnace) is the budget the previous
rung proved. The fourth did not fail on itself: it pushed the failure onto the
**furnace's** run, which is the one that matters most, with the chest's own four
neighbours named.

A branch off an existing belt costs no chest perimeter. An inserter takes items
off a belt exactly as it takes them out of a chest, and the arm doing it is on
coal, so it self-fuels like every other arm here.

**Every belt this method places carries coal**, which is what makes "the
nearest belt" a safe tap. That is a property of *this method*, not of the map:
a belt somebody else built inside the cell would be picked up by the same
search and nothing here could tell.

## The four refusals, in the order they arrived

Each one is a measurement. Three of them came from the **offline** plan against
the real dump, in seconds, and would each have cost a run.

| # | refusal | what it meant | what changed |
|---|---|---|---|
| 1 | `no belt route, blocked by 4 tile(s)` at the arm `[-35.5, 32.5]` | the site had one free side and it was the tile the offtake's **own chest** was about to stand on | `room_to_fuel` takes the arrangement's own tiles as `taken`, and does not read them as free |
| 2 | all **eight** furnace perimeter tiles `occupied` | sited last, the three coal runs had wrapped the cell first | the offtake is sited and placed **first**; a coal run needs one face and a route, the offtake needs two collinear tiles |
| 3 | the coal run to the furnace refused at the buffer's four neighbours | on a fresh plan the cell adopted **its own plate chest** as its coal buffer — both are an `iron-chest`, and the plate chest is nearer the furnace | the exclusion is by the *resolved* offtake's position, not only by a chest found already standing |
| 4 | the fuel branch refused with the tap belt **diagonal** to the arm | laid last, the drill's and furnace's belts had taken the arm's remaining sides | the branch is laid **between** the haul and the two cell runs |

Two constants moved with them, and both are load-bearing rather than
decorative:

* **`room_to_route` 7x7 → 9x9.** Reverting it alone, with everything else in
  place, turns **five of this module's nine tests red**. It is a siting
  heuristic, not a reservation — nothing stops a later route from taking a side
  — which is exactly why the number had to be measured. The real fix reserves
  the perimeter inside `method::connect`, and is not this rung.
* **The test fixture's coal patch, `(-40, 26)` → `(-40, 18)`.** Said plainly
  because it is the trap `2026-09-06-fixtures-agree-with-their-code.md` warns
  about: *the fixture was changed while the code under it was failing.* What
  justifies it is a number measured off the real map and independent of this
  change. Both patches are 8×8, so at `(-40, 26)` their **closest tiles were 6
  apart** against the **19.6** seed 31337 really has — three times tighter than
  the map it stands for, with a whole cell plus four belt runs to fit in the
  gap. The fixture was refusing arrangements the target map accepts, which is
  the wrong way round. `(-40, 18)` is 14 apart: still the harder case, no longer
  harder than anything this method is meant to build on. **The real-map plan
  succeeded at both distances**, before and after the fixture moved.

## The price, before and after, on one commit

Seed 31337, four bots, release, `7ebe707d`:

| goal | actions | makespan |
|---|---:|---:|
| `sustain:iron-plate:15:7200` — **before** (`sustain.rs` at `7ebe707d`) | 289 | 16,206 (4:30) |
| `sustain:iron-plate:15:7200` — **with the offtake** | **376** | **20,128 (5:35)** |

+87 actions (+30%) and +3,922 ticks (+24%), for one arm, one chest and a fuel
branch — most of it the branch's belts and the iron they cost.

**And what did not move.** The three standard goals are byte-for-byte where
they were, with the same binary that produced the numbers above:

| goal | actions | makespan |
|---|---:|---:|
| `researched:automation` | 176 | 21,784 |
| `producing:automation-science-pack:6` | 316 | 22,463 |
| `producing:logistic-science-pack:6` | 442 | 47,542 |

Nothing else reaches `Goal::Sustain`.

## The witness had to change, and a working offtake is why

**This is the finding most worth carrying off this branch**, and it is a
sibling of the credit fix that landed the same night: *an instrument that
inverts at exactly the success it is meant to detect.*

`supervisor.count_item` reads `output_inventory`, which for a furnace is what
it has made and **not yet given away**. That was the right thing to watch while
nothing took the plates. It is the wrong thing now: a working offtake keeps the
furnace's output near zero, so the old witness would read `+0` and report a
**dead cell precisely when the cell works best** — a confident answer about the
wrong object, which is the failure shape this project has paid for most.

`scripts/selffed_run.lua` therefore witnesses `burner-inserter -> iron-chest`
instead, which is both safe and stronger: `get_output_inventory()` on a
container is its contents, so the same counter works unchanged, and a plate
sitting in a chest with no bot acting was **smelted by a machine and carried
there by an inserter** — the whole chain, not one link of it. The window goes
2,400 → 7,200 ticks because the chain being witnessed is three belt runs long
and starts cold.

The same file's third milestone name said `lead-in 9600` while its constant
said 20,000, so every record it wrote named a milestone by a parameter it was
not measured with. Fixed.

The general shape, stated because two instances of it landed within an hour:
**an instrument built while a mechanism was absent can encode the absence.**
`count_item` reading `output_inventory` assumed nothing empties a machine;
hand credit pooled by prototype assumed one machine of a kind. Each became
*more* wrong as the thing it measured got *better* — the witness would have
read the best cell as dead, and the balance made a cell harder to prove the
better its plan got. A metric that degrades with success is worse than a
missing one, because it answers.

## What the tests assert, and what they were seen to do when it is taken away

Three new tests, each **watched to fail for the right reason** with the
substitution asserted to have matched before the result was read:

| test | substitution | result |
|---|---|---|
| `the_furnaces_output_is_taken_into_a_chest` | the whole per-cell offtake section deleted (4,567 chars, asserted to contain `plan_offtake` exactly once) | red: *"the stone-furnace at [-37, 33] has nothing taking its iron-plate away"* |
| `the_offtake_arm_is_belted_its_own_coal` | the fuel branch deleted | red **alone**: *"nothing delivers coal into the offtake arm at [-38.5, 32.5]"*, the other eight green |
| `a_side_the_offtake_itself_will_occupy_is_not_room_for_its_coal` | the `spoken_for` guard removed from `room_to_fuel` | red, **and six of nine** with it, reproducing refusal #1 verbatim |

Each also carries an in-test falsification with its own matched-substitution
assertion: the offtake arm turned around (`standing_offtake` must stop
recognising it — an arm pointing the other way feeds the furnace from the
chest, which places perfectly and moves nothing), and the feeding arms removed
from the built world (`fed_by_machine` must go false, and the experiment
refuses to draw a conclusion if it matched no arm).

**One thing worth stating because it is the trap and not the cure.**
`a_replan_over_the_arrangement_it_just_built_adds_nothing` — the previous
rung's regression test — stayed **green** under the full deletion of the
offtake. That is correct (it is about idempotence) and it is exactly the shape
`2026-09-06-fixtures-agree-with-their-code.md` names: a suite that stays green
under a substitution says nothing about the removed thing.

**This task wrote both the code and its fixtures**, and the fixture it changed
is named above with the independent number that justifies it.

## THE RUN HAS NOT HAPPENED, and nothing here claims the arrangement stood

**No live run was made for this branch.** The box carried a five-minute load of
42–49 for the whole session against a ceiling of 6 — three sessions compiling,
two release builds and a viewer — and a headless run at 5x would have competed
with all of it. The owner's instruction was to hand over the offline result and
say so plainly rather than start one; the run is theirs to schedule on a quiet
floor.

So every claim above is **a claim about a plan**, not about a game:

* the offtake is *planned* on seed 31337 and on the crate's fixture. Nothing has
  been placed, nothing has moved a plate, and `full_output` has **not** been
  observed to fall;
* the mechanism the whole thing rests on — that an arm delivering coal fills
  another burner inserter's fuel slot, and that an inserter takes items off a
  belt — is **assumed from the game's own rules and measured nowhere here**.
  Its sibling claims were answered by `run-1788679826-02267`; these two are
  the same class and are still open;
* the previous rung's own record is what says an offtake is needed at all, and
  that record is trustworthy. Whether *this* offtake fixes it is not yet
  evidence.

What the run must report when it happens: the `sustain` verdict, the machine
status histogram (specifically whether `full_output` is still among the
dominant non-working statuses), the delivered tick rate, and the hand-credit
balance's own four numbers. **The balance's arithmetic changed under this branch
while it was being written** — `828466d3` keys credit by machine *instance*
rather than by prototype, which is the fix `2026-09-06-a-cell-that-feeds-itself`
asked for, and it makes the previous cell read `short` rather than `roster-fed`.
So the balance's answer for this arrangement has no precedent to be read
against, and the first one it gives is evidence about the balance as much as
about the cell.

Quote the `Using mods directory` and `Using scripts directory` lines. Since
`dbfc6a97` a bridge mod that does not resolve refuses the run instead of hanging,
so the `readlink` habit is enforced by the code now — but the lines still say
*which* bytes loaded, which no pre-flight can.

## What the next rung is

* **The chest is the new ceiling.** 32 stacks is far beyond a two-minute window
  at 15/min, but a factory would want it emptied too.
* **`method::connect` reserves nothing.** Four runs converging on one cell are
  routed greedily one after another against a shared grid, and three of this
  branch's four refusals are that. `room_to_route` at 9x9 and the ordering
  above are heuristics that happen to fit *this* arrangement on *this* map; the
  real fix is a perimeter reservation inside `connect`, and it is what a fifth
  run would need.
* **Resource depletion is still unmodelled**, unchanged from the last rung.
* **`Have`'s reuse of the cell's furnace is still the residual hand credit**,
  unchanged from the last rung.
