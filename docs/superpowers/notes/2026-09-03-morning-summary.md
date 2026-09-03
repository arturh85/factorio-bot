# Morning summary — 2026-09-03

The detail is in `2026-09-02-morning-report.md`, which is 1,200 lines of
append-only log. This is the two-minute version.

## Where stage 2 actually stands (read this first)

**Stage 2 is NOT done. No witness has ever seen red science, and no cell has
ever produced anything.** Nine live runs tonight; the sections below are in
reverse order of discovery, so this is the arc.

**What genuinely advanced:**

* **Rung 1 now reproduces six consecutive times** on independent fresh worlds.
* **Run 9 passed rung 2's plan time for the first time ever** — 107 steps —
  where run 8 refused in three seconds.
* **Run 4's game accepted all eight of the cell's placements.** The geometry,
  siting and pole coverage work in a real game; "the cell can be built" is now
  observed rather than modelled.
* Three real fixes landed and are green: the stage-2 cell layout, wood from
  trees (`b0e3e12e`), and the chain owner (`60ade9de`).

**What actually cost the night, and it was not the planner:** `set_recipe` was
committed hours before it ever reached a game. In a debug build there is no
embedded mod snapshot, so `FACTORIO_BOT_REFRESH_MODS=1` is a **no-op**, a stale
`workspace/mods` copy wins, and the one log line that would have said so is
gated behind `if !silent` and never printed. Six of the nine runs died on that
and its knock-ons (a save generated without the mod poisons every later run;
orphaned servers block the next run's ports). **The only trustworthy check is
to grep the workspace mod for the symbol you just added.**

**The one engineering item that would change the most:** `min_radius` is
dropped when `Condition::AtPosition` is lowered into `StepKind::Walk`, so the
executor aims a bot at the tile centre the planner explicitly said it must not
stand on. Six live refusals, all under one tile. It is not cosmetic — the full
chain is *walk refused -> that bot's entire remaining chain abandoned -> replan
-> power plant re-sited -> refusal -> run dead*, which is exactly how run 9,
the best run of the night, ended.

**What I got wrong, since you asked for it plainly:** I misdiagnosed the mod
staleness once before getting it right; I applied a "remedy" that deleted the
bridge mod and poisoned two runs; I claimed twice that the run record carries no
inventories when `samples.jsonl` has them; I claimed the world persists between
runs when it does not; and I let two agents write in one checkout, so a run
started against a tree another agent was still editing and I nearly read its
failure as evidence against a fix it never contained.

## The headline

**The seven-rung ladder is closed, and it is repeatable and deterministic.**
Runs 33, 34 and 35 each satisfied all seven rungs with *identical* step counts:

```
8 · 8 · 37 · 40 · 16 · 52 · 111
```

`research automation` — the rung that had never once closed — settled on the
game's own `on_research_finished` at **5,999 ticks against a predicted 6,000**.
The bots built a working power plant (offshore pump, boiler, steam engine, three
pipes, a pole and a lab) generating **900 kW against the lab's 60**, at
satisfaction 1.0.

Rung 7 had failed all day for five distinct reasons, each invisible until the one
before it was fixed:

1. a walk teleport that reported *arrival* at destinations nothing could reach;
2. crafts that never reported completion, each burning a 360-second deadline;
3. actions that could not be *represented* as failed, so the record showed none;
4. a planner reading the **`enemy` force's** technology, not the bots';
5. a lab with no power, on a map whose water was not solid.

## The new milestone: stage 1 is DONE and WITNESSED

**A factory produced, and we proved it.** Run 37:

```
WITNESSED: iron-plate in 1 watched machine(s) went 0 -> 1 (+1, wanted 1)
           in 480 of 2400 ticks, 466 polls
```

An iron plate appeared in a furnace's output inventory **while every bot stood
still**. `supervisor.witness` dispatches no actions, so a plate that appears can
only have been smelted there — the first machine-made item in this project's
history, and the first evidence of *production* rather than *placement*. 480
ticks against a predicted 432 (drill 240 + furnace 192): model and game agree
within 11%.

**Stage 2 — red science by machine, ~620 kW — is part-landed.** Its two
prerequisites are in (`7a129420`), and they are exactly the two ways it could
have shipped dead:

- **`Powered` is now a network budget**, not a per-consumer test. Its red, before
  a line was written: *"twelve assemblers already draw the whole 900 kW; the
  thirteenth has nothing left to run on."* Supply and demand now share one
  network walk so they cannot disagree about what "the same network" means.
- **The inserter direction rule is checkable**, validated against the two
  measurements CLAUDE.md made *in a running game* rather than against itself —
  the right authority, since those measurements exist because a layout once
  placed perfectly and did nothing.

**What stops it: there is no `set_recipe` anywhere in this project.** Zero hits
across `crates/` and `mods/`. Stage 2 *is* an assembling machine with a recipe on
it, and the verb spans planner → executor → core → mod. That is authorised and in
progress as one task, because the executor's dispatch match is exhaustive and
splitting the chain would deadlock.

No makespan pin moved.

### How it got there, and why the intermediate step mattered

Run 36 built a real cell — a burner drill standing on iron ore, a stone furnace
exactly where the drill's drop point lands, both fuelled. Five actions, zero
failures, correct geometry.

**It produced nothing.** `production.made` was `{coal: 37}` — the coal a bot
hand-mined. The drill was fuelled at tick 8822 and the run declared `done` at
**8823**. One tick. A burner drill needs ~240 ticks for a single ore.

That failure is what the witness was built for, and run 37 closed it.

That is the fourth appearance of one failure, and the first time we caught it
*before* claiming success:

| | reported as |
|---|---|
| a lab **placed**, never powered | researching |
| pole **coverage**, no generation | powered |
| `only_ghosts` placing an overlapping blueprint | buildable |
| a factory fuelled **one tick** before the run ended | producing |

`supervisor.witness` — dispatch nothing, wait, assert a terminal machine's output
rose, **so any increase is machine-made by construction** — is being built now.

## Things I told you were working that were not

Every one found by an agent checking rather than trusting:

- **Ore-tile retirement** (`e562847a`) — I reported it landed. It had **never
  fired in a live run**: the per-swing delete removed the tile before the
  retirement path could ever see it.
- **The `EntityGraph` whitelist "one-line fix"** — would have added three arms to
  a match that never evaluates, because the enum variants did not exist.
- **`plan_created.bots`** — I reported it fixed to mean "the roster". It still
  lied when a script passed its own bots.
- **My "bot at (0,0) means phantom" heuristic** — wrong. A healthy freeplay first
  player is observationally identical.
- **The video encoder's UPS cost** — I retired the screenshots partly on it. Now
  measured: **57.12 vs 57.08 UPS, −0.07%.** Noise. It was never the argument; the
  disk figure (947 MB against 290 MB) was, and that one was real.

Four times tonight I suspected a false success and was **wrong** — including
run 33 itself, where I misread a settle because `action_settled` carries no
action name. Erring that way round is right, but the ratio is worth watching.

## No mod change tonight ever ran in a game, and the safeguard never fired

**Run 4 raised `No such function: botbridge.set_recipe`** — a function committed
in `3de64f62` — and, in the same batch, `the target tree-07 was gone before
mining finished`, which is *precisely* the bug the wood agent had just fixed.
Both fixes were absent from the mod the game had loaded.

**Two mechanisms, and I misdiagnosed it once before getting it right.** My first
call was that `include_dir!("mods")` snapshots at compile time with no
`cargo:rerun-if-changed` for `mods/`. That is wrong for this run: `MODS_CONTENT`
is `#[cfg(not(debug_assertions))]` (`instance_setup.rs:51`), so a **debug build
has no embedded snapshot at all** — my `grep` of the binary returning zero was
expected, not evidence. The real mechanism is the one CLAUDE.md already
documents: in a debug build the checkout is used **only if `workspace/mods` does
not exist**, and a stale copy otherwise wins. **`FACTORIO_BOT_REFRESH_MODS=1` is
a no-op in a debug build** — there is nothing embedded to refresh *from*. I
passed a release-only flag at a debug build and assumed it had worked.

**The safeguard that exists to catch exactly this never fired.** CLAUDE.md says
every run "logs one line naming which directory actually won: `Using mods
directory <path> (<why>)`. That line, not a guess from a traceback, is the
authoritative answer to 'did my edit ship'." It is gated behind `if !silent`
(`instance_setup.rs:363-368`) and printed **zero times across runs 1, 2 and 4**.
The one check the docs tell you to trust is suppressed in exactly the
configuration where staleness bites.

**Consequences.** Every "the mod fix is in" claim tonight is unverified, and
`set_recipe` — the prerequisite the whole stage-2 layout was built on — has
never executed in a game. Without it no machine ever gets a recipe, so no cell
could have produced anything regardless of how good the layout is.

**Remedy for a debug build:** delete `workspace/mods/BotBridge` (scoped to the
bridge mod; the code checks that specifically, so the other mods survive) and
let the checkout be used. `REFRESH_MODS` is for release builds.

**One measurement I cannot account for:** at ~02:45 the workspace copy had 0
`set_recipe` and an 01:06 mtime; at ~03:00, immediately before I deleted it, it
had 6. Something re-seeded it in between and I could not attribute it. What is
not in doubt is that run 4's *game* had the old mod — Factorio loads mods at
server start, and the game itself raised the missing-function error.

## The min_radius fix is GREEN IN TESTS AND STALLS A LIVE RUN

**`c0d4f87a` is currently on master and a run cannot execute with it.** Run 11
(`run-1788418841-48581`), same unchanged `level.zip` as runs 8/9/10:

* rung 1 planned **134 steps**, where runs 8, 9 and 10 all planned **314** on
  this same world — despite the fix being reported as leaving the plan's
  simulation bit-identical, with no pin moved;
* after ~13 minutes the record held exactly `{run_started: 1, milestone_started:
  1, plan_created: 1}` — **zero `action_dispatched`, zero `walk_dispatched`,
  zero open actions**, and the log had stopped growing;
* the only activity was two `too far away, moving first!` warnings **11.6
  minutes apart**, which is `ACTION_RESULT_DEADLINE` (360 wall-clock seconds,
  `rcon.rs:39`) expiring twice with nothing settling in between. Those warnings
  appear in healthy runs too — what is new is the absence of any progress
  around them.

The likely shape: `approach_annulus` returns a goal the bot is already inside,
so nothing is dispatched, while the caller still waits for an arrival that will
never settle. The fix's own test asserts "already inside, nothing to dispatch" —
**returning "no walk needed" and returning "walk complete" must not be the same
value to a caller that then waits.**

**This is the sharpest instance tonight of the rule this project keeps
re-learning: green tests are not the bar, a run executing is.** The change had
485 planner tests, a mutation battery, every pin passing unedited, and a test
built from the real run's coordinates — and it stops the machine dead. The
earlier form of this lesson was "the cell stands is not the cell produces". This
is the same shape one level down: "the suite is green" is not "the thing runs".

Sent back to be fixed or reverted; master should not stay in this state.

## CORRECTION: six of the eight "walk refusals" are a different defect

**I built an elaborate case on a misreading, and it was wrong in a way worth
recording.** I counted eight sub-tile refusals, asserted a shared invariant,
called one pair "byte-identical across runs" and declared the fix "verifiable at
a named coordinate". Six of those eight are not this defect at all.

That message —

```
ERROR: stuck while walking, the destination is unreachable:
the game's pathfinder found no path from (-28.6640625/-26.8046875) to (-28.5/-27.5)
```

— is emitted by `mods/BotBridge/control.lua`'s `walk_repath_finished`: a
**mid-walk re-path, after dispatch**. Their `walk_settled` records carry
non-null `elapsed_ticks` (106–471) and a `to` of `{-31, -31}` — *not* the
coordinate in the message. So `(-28.5, -27.5)` is the original path's **last
waypoint**, and the 0.70 tiles I kept quoting is bot→waypoint, not bot→goal.
`start_walk_repath` re-paths to `w.waypoints[#w.waypoints]` at
`WALK_REPATH_RADIUS = 0.5` — a waypoint that was clear when the path was
computed and has since been **built on**. Those walks all have
`min_radius == 0`, so carrying `min_radius` cannot reach them; fixing them is a
mod-side change.

Also: only **one** event in all of `workspace/runs` carries the executor's own
`failed to path find`. The rest are the mod's stuck-walk message, which I read
as the same thing.

**The lesson is not "count more carefully".** Every number I quoted was real;
what was wrong was assuming two coordinates in one message were the walk's start
and goal. A shared *format* looked like a shared *cause*, and the invariant I
derived from it ("sub-tile distance") was an artefact of measuring the wrong
pair of points. The run record could have settled it at any time —
`walk_settled` carries the actual `to`.

## The real defect, sharper than I had it (fixed, `c0d4f87a`)

The old lowering did not merely *drop* `min_radius` — it **baked it into `to`**
via `arrival_point`, at exactly `min_radius` along a fixed `+x`, and forced
`radius` to `0.0`. So the bug was that **the planner named a stand-point at
all**, in a direction chosen without knowing what is walkable.

And `+x` was worse than arbitrary: every placement in these runs sits at
`x = -53` or `x = -63` because the plan builds in **columns**, so "one clearance
east of the site" points straight down the next column — at a tree in run 10,
and at the plan's own buildings in general. That is why it reproduced to the
decimal place.

Fix: `StepKind::Walk` now carries the `AtPosition` verbatim (`{ to, min_radius,
radius }`) and names no point; `approach_annulus` picks a goal
`min_radius + slack` from the target **towards the bot**, with the whole path
disc inside the annulus. `min_radius == 0` is byte-identical to before, so every
mine, insert, remove and craft is untouched. **No pin moved** — `arrival_point`
still drives the plan's own simulation, so the model is bit-identical.

Two follow-ups the agent named and deliberately did not do, both stop-and-report:
`Insert`/`Remove`/`Mine` set `min_radius: 0.0` although a bot cannot stand on a
furnace either (would move pins), and `approach_radius(10) = 5.0` lets a path
stop five tiles short of an insert's furnace — whose fix is the first item, not
a smaller constant.

## The cell was BUILT and both recipes were SET — all nine actions succeeded

From run 10's record, every cell action settled `success`:

```
success | place assembling-machine-1 at [-12.5, -58.5]
success | place assembling-machine-1 at [-12.5, -62.5]
success | place inserter at [-10.5, -58.5]
success | place inserter at [-10.5, -62.5]
success | place inserter at [-12.5, -60.5]
success | place iron-chest at [-9.5, -58.5]
success | place iron-chest at [-9.5, -62.5]
success | set assembling-machine-1 to iron-gear-wheel
success | set assembling-machine-1 to automation-science-pack
```

**That is the first time `set_recipe` has ever executed in a game, and it
worked.** The layout, the siting, the inserter directions, the pole coverage and
both recipes are now observed in a real world rather than modelled.

**Be exact about what this is not.** Standing and recipe-set are verified;
production is not. The chests still need charging — 30 iron plates and 15 copper
— and the run died in the smelting work that does it, before any witness ran.
"The cell stands" is precisely the claim this project has been wrong about four
times, so it is worth writing the distinction down rather than rounding up:
**nine actions succeeded, zero packs exist.**

What remains between here and stage 2 is the charge and the witness — not the
design.

## Run 10 named the mechanism itself: the walk ends inside a collision box

**The healthiest run of the night, and it ends by proving the `min_radius`
diagnosis.** Run 10 (`run-1788413329-43771`) closed rung 1 with `success=173
pending=0`, **zero failed actions and zero failed walks** — the seventh
consecutive reproduction — planned rung 2 at 107 steps (identical to run 9), and
executed the cell past the point that killed run 9, with **no plant re-siting
refusal**. The adoption fix held.

It then halted on a pre-flight guard that states the cause outright:

```
the walk to [-61.72941750255542, 11] would end at [-61.7265625, 11], inside a
collision box spanning [-61.71, 10.54] to [-60.91, 11.34] — a character cannot
stand there, so the walk could only stall
```

That is `min_radius` exactly. The planner specified an annulus so the bot would
stand *clear* of that box; `StepKind::Walk` dropped `min_radius`; the executor
aimed at the centre. The guard is refusing correctly — the bug is that it is
handed such a destination at all, and it turned seven mysterious "no path"
answers into one precise sentence.

**Read that as a lesson about instrumentation, not just about walks.** Seven
identical failures produced nothing but `failed to path find`. The eighth, met
by a guard that knew what it was looking at, produced coordinates, a box and a
reason. The difference was not more logging — it was a check placed where the
question is actually decidable.

No witness. Stage 2 remains not done.

## Correction: they are not all tile centres — the invariant is sub-tile distance

Run 10's **eighth** refusal targets `(-51.7265625, 11)` from
`(-51.11328125, 11.37890625)`. Neither coordinate is a tile centre, so my
repeated claim that *"every one is aimed at a tile centre from inside that same
tile"* is **wrong** — it was an honest generalisation from the first three
samples and the fourth through seventh happened to agree.

What actually holds across all eight is narrower and still enough:

* the distance is **sub-tile**, 0.60 to 0.76 tiles in every case;
* the game answers `failed to path find` — it searched and found nothing —
  never `try again later`, which would mean a full queue.

The `min_radius` mechanism still explains it: whatever occupies the destination,
`approach_radius` shrinks the acceptance ring to as little as 0.5 tiles around a
point the bot may not stand on, leaving no legal goal. A non-centre target is
consistent with that — a character position or an entity whose box is not
tile-aligned occupies ground just the same.

Keeping the weaker claim on purpose. The stronger one made the diagnosis sound
more precise than the evidence supported, and this is a night that has already
paid for that kind of tidiness several times.

## The walk refusal reproduces to the decimal place — so the fix is checkable

Run 10 produced a **seventh** sub-tile refusal, and it is **byte-identical to
run 9's**: same start `(-28.6640625, -26.8046875)`, same destination
`(-28.5, -27.5)`. Three independent runs (8, 9, 10) refuse the same destination;
two of them from the same start to the last decimal.

Two things follow, and both are useful:

1. **The planner really is deterministic run-to-run in the game, not just in
   tests.** Identical worlds produce identical plans producing identical walks.
2. **The `min_radius` fix is trivially verifiable.** It is not "does the failure
   stop appearing eventually" — it is "does *this* walk, from that start to that
   destination, succeed". A named coordinate to check against is worth more than
   any amount of after-the-fact log reading, and this project has spent a whole
   night on the difference.

## Plant adoption: both call sites asked the wrong question (fixed, `e5402fd8`)

**Both places that need power asked "is there supply within 64 tiles *of the
bot*?" and read "no" as "this world has no power."** `assemble.rs:1350` and
`have.rs:1736` (pre-fix) had the same shape: `nearest_supply_anchor(from,
ANCHOR_SEARCH_RADIUS /* 64 */, want_kw)`, else `plan_plant(...)`.

Run 9's own record shows it firing. At tick **150,645** the plan contains
`place offshore-pump at [9.5, -45.5]`, a boiler and `place steam-engine at
[12.5, -39.5]` — a **second** plant — while `map.jsonl` has the first (pump
`[-5.5,-57.5]`, engine `[-11.5,-54.5]`, pole `[-13.5,-56.5]`) standing from tick
127,098 and never removed. `samples.jsonl` at tick 150,600 puts bot 1 **86.0
tiles** from that pole: just past 64.

Fix: `power.rs:208` `PLANT_ADOPT_RADIUS = 256`, `:466` `enum Supply {
Standing(Position), Build(Plant) }`, `:532` `supply_for(...)` — near anchor,
wide anchor, then build. Adopt-first at any distance rather than comparing
distances, because a walk is `distance / WALK_TILES_PER_TICK` (~1,700 ticks even
at 256 tiles) while a plant is ~45 iron plates that must be mined and smelted
first, **plus one wood, of which a run has four for ever**.

**What adoption verifies:** the pole exists and this crate knows its supply
area; a *generator* sits on a pole in the same wire-connected component; and
generation minus every consumer already on that network is ≥ the wanted kW.
Because both callers pass `kw > 0`, coverage alone can never satisfy it — a lone
pole is passed over and a plant built. **What it assumes, and this is named in
the code:** that the plant is *running*. Nothing in `FactorioWorld` reports
steam, water or a fuel slot, so a boiler that ran dry is adopted at nameplate
900 kW. Adoption does not make that worse — a plant this planner *builds* is
credited 900 kW the moment its `Place` is emitted, long before any coal reaches
it — and the adopting cell tops the boiler up, which a second plant across the
map would not have.

**An honest residual, worth knowing before the next run.** Run 9's *final*
refusal (tick 179,447, `PowerPlantNeedsShore`) is **not** explained by distance.
That run's last keyframe reports pole, engine and boiler in the model with zero
divergence from the game, and the last `bots` sample puts all four bots 59–60
tiles from the pole; replaying `nearest_supply_anchor` on that exact set answers
`Some([-13.5,-56.5])` at radius 64. So the origin that expansion was actually
asked from is not one the run record carries. The 256-tile bound is a margin
chosen to adopt whether the origin is the bot's real position or a stale one —
not a demonstration that the fatal step is understood.

No pin moved; workspace green.

## Run 9: furthest yet — and a standing plant was re-sited to death

**Run 9 (`run-1788408407-02764`) is the high-water mark.** Rung 1 SATISFIED
(the ladder's sixth consecutive reproduction), and **rung 2 passed plan time for
the first time ever** — 107 steps planned, where run 8 refused in three seconds.
The chain-owner fix (`60ade9de`) holds in a real game.

Then a walk was refused, the milestone replanned, and the run died:

```
HALTED: stuck -- refused: the nearest water is 66.7 tiles away, but no shoreline
within 10 tiles of it has room for a pump, a boiler, a steam engine and the pipes
between them
```

**A working plant already stood.** From the action stream at tick ~127,098:
`place offshore-pump at [-5.5, -57.5]`, `place steam-engine at [-11.5, -54.5]`,
a boiler, pipes, and `place lab at [-15.5, -58.5]` — which is *how rung 1 got
satisfied*. The replan nevertheless tried to site a **brand-new** plant,
anchored far from the water the first one used, and refused because no shoreline
near that new anchor had room.

So the earlier report that "`Researched` re-sites the power plant on replan,
spending a pole each time" understates it: re-siting is not merely wasteful, it
is **fatal**, because a replan can refuse work that has already been done and
is standing in the world. An agent is fixing it to adopt a standing plant.

**Six sub-tile walk refusals now**, and this one was the trigger for the fatal
replan — which is the argument for the `min_radius` fix being load-bearing
rather than cosmetic. The chain runs: walk refused -> bot's whole remaining
chain abandoned (`run.rs:258`) -> replan -> plant re-sited -> refusal -> run
dead. Run 9's replan also came back **194 steps against a best of 107**, so a
refusal leaves the plan worse as well as shorter-staffed.

## Two corrections, and a reproducible site for the `min_radius` fix

**The world does not persist between runs — corrected.** I wrote earlier that it
did, on the evidence that run 4 planned 131 steps for rung 1 against 322 and
337. That was true only while an older `level.zip` had accumulated state. The
server reloads `level.zip`, not the autosaves, so once the map was regenerated
each run replays rung 1 from scratch: runs 8 and 9 both planned **314**. Runs
are therefore independent trials now, which is what a repeatability claim
needs, at a cost of ~29 minutes of rung 1 per attempt.

**The sub-tile pathfinder refusal is deterministic, and that is useful.** Run 9
produced a fourth instance — and it refused the *same destination* as run 8,
`(-28.5, -27.5)`, from a different start (`(-28.664, -26.805)`, 0.70 tiles).
Four instances now, all aimed at a tile centre from inside that tile.

Because it reproduces at an identical site across independent runs on the same
map, the `min_radius` fix can be **verified against a known coordinate** rather
than by waiting to see whether the failure stops appearing. That turns a
"seems better" change into a checkable one.

## The rung-2 refusal: the chain owner was lost one level above `bill()`

**Fixed in `60ade9de`.** My hypothesis was right about the shape and the actor
and wrong about the site: `bill()` does emit `Holder::Share(ctx.chain_actor)`
and `HandCraft` does propagate it — the actor is not lost on the way down. What
is lost is the **chain owner**, one level higher.

`AssembleCell::converges` returns `true` (`assemble.rs:1314`), and the
owner-recording block sat inside `if ctx.chain.is_none()` (`method/mod.rs:605`),
so a converging method opened a chain with `owner = None` — `stated_holder` of a
`Producing` goal is `None`. Every bill ingredient then expanded with
`ctx.chain` already `Some(...)`, so the recording branch never ran and **no
owner was recorded for any of them**. Measured, not inferred: **86 of 98 actions
in `ChainId(0)` had `owner=None`**.

`schedule.rs:342-350` then bound the ownerless chain to whichever bot could run
its first ready action cheapest — three bots parked on the ore patch made that
bot 2 — while `expand_goal` had sized the work against bot 1, who held 48 iron
ore. Hence "has 3 iron-ore does not hold for bot 2".

**A diagnostic worth keeping:** `schedule.rs:452` raises
`PreconditionUnsatisfied` rather than `ChainOwnerInfeasible` *only* when the
chain has no owner. The error variant, not just its text, fingerprints an
unowned chain.

Fix: an `else if` arm so a `Holder::Bot`/`Holder::Share` goal met inside an
ownerless chain names that chain's owner. First holder wins; an owned chain is
untouched; `Step::Owned` opens its own chain so a supplier's share never reaches
the arm. No pin moved.

**`even_shares` reachability — the earlier agent's claim was wrong.**
`SplitAcrossBots` is indeed unreachable from a `Producing` goal, but
`worth_converging` (`have.rs:2340`) *is* reachable: `SharedSmelt::claims` is
`!top_level && in_chain && !converging`, and `ctx.converging` stays false because
`AssembleCell` overrides `converges`, not `split_probe`. The red-science plan
contains six `Step::Owned` supplier chains owned by bots 2, 3 and 4. Reachable —
but **not causal here**, and the poorest-first ordering remains unexamined.

## A process pattern matches the process doing the matching (three times tonight)

Every `pgrep -f`/`pkill -f` I wrote tonight matched my own tooling, because the
monitor's or the shell's command line **contains the pattern as a literal**:

1. `pkill -f "stage2-run4.log"` killed the monitor watching that log, not just
   the run.
2. A monitor keyed to the process *name* stayed "alive" when a later run
   started, so it watched a dead log for ever.
3. A monitor keyed to the log *name* never exited when its run finished —
   `pgrep -f "s2r8"` matched the monitor itself — so it reported a stall
   instead of `RUN 8 GONE`.

Keying to a more specific string does not fix this; it makes the self-match more
certain, because the more specific the pattern the more likely it appears
verbatim in the watcher.

**Use a PID, not a pattern.** Capture the run's PID at launch and test
`kill -0 "$PID"`. Where a pattern is unavoidable, break the literal so it cannot
match itself — the classic `pgrep -f "factorio-bot lu[a]"` — but a PID is
better, because it is exact and cannot drift onto a later run that happens to
share a name.

## Run 8: the first clean run, and a recurrence of an already-fixed class

**Run 8 (`run-1788405365-21697`) was the first attempt with a fresh world AND a
verified mod, and it got further than anything before it:** rung 1
`researched("automation")` **SATISFIED** — the ladder's fifth consecutive
reproduction — on **323 action dispatches and 103 walks**, against 267 in run 1.

Rung 2 then refused **three seconds after the goal was accepted**, so this is a
plan-time refusal and not an execution failure:

```
HALTED: stuck -- refused: precondition has 3 iron-ore of action ActionId(41)
                          does not hold for bot 2
```

**This class was diagnosed and fixed once already.** `method/have.rs:6139-6157`
documents `run-1788300756-94802` raising `precondition has 50 iron-ore of action
ActionId(8) does not hold for bot 2` — the identical shape — and records the
cause: *"the production's `insert 50 iron-ore` was welded to nothing while the
mining under it opened a chain of its own, and the two landed on different
bots"*, fixed by having the driver read `whose` off `Produced` as well as
`Have`. It has recurred through the **new `Goal::Producing` / assemble path**,
which did not exist when that fix was made.

The same note carries the warning that matters for the retest: the defect needed
**unequal** per-bot inventories, and *"the same goal planned fine against bots
holding nothing, which is why every existing test passed."*

It also puts a question mark over an earlier agent's claim that `even_shares`'
poorest-first split is "unreachable from a `Producing` goal" — rung 2 *is* a
`Producing` goal. Either the reachability claim is wrong or this is a different
mechanism; an agent is settling which.

**CORRECTION — the run record DOES carry inventories, and I was wrong twice.**
I claimed here and in the wood RCA that the record has none, and ranked
"run-record enrichment" as a top open item on that basis. I had only ever looked
at `events.jsonl`. **`samples.jsonl` carries `kind: "bots"` rows with a full
per-bot `inventory` and `position`, sampled throughout, plus `kind: "force"`
rows with production totals.** The rung-2 fixture was read straight off the
sample at tick 107,820 — exactly as the precedent test read its own run's
samples. The wood RCA did not need reconstructing from craft/place actions
either. The open item stands only in the much weaker form of "events.jsonl
alone is not enough, and nothing says so".

## `min_radius` is dropped at lowering — the same bug this file already fixed once

**Three live refusals, one mechanism, and the precedent is documented in the
file it happens in.**

Runs 1 and 8 refused three walks, every one of them shorter than a tile and
every one aimed at a **tile centre** from inside that same tile:

* `(-19.890, 13.875)` -> `(-20.5, 13.5)` — 0.70 tiles
* `(-28.394, -26.906)` -> `(-28.5, -27.5)` — 0.60 tiles
* `(-47.840, -10.734)` -> `(-48.5, -10.5)` — 0.70 tiles

All three answered `failed to path find` — the game searched and found nothing,
as opposed to `try again later`, which would mean a full queue.

**`Condition::AtPosition` carries `radius` AND `min_radius`**, because the
target is routinely a position the bot must never stand on — the tile a
furnace sits on, the ore a mine consumes — and `min_radius` is what keeps the
acting bot out of its own footprint (`state.rs:822`). `schedule.rs` models that
annulus faithfully: `travel_ticks(from, to, min_radius, radius)` walks a bot
*outwards* when it is too close (`:29-30`), and the simulated arrival point is
picked `min_radius` out along a fixed direction (`:35-54`).

**But `StepKind::Walk` carries only `{ to, radius }`** (`schedule.rs:109-112`),
so `min_radius` is dropped at lowering, and the string does not occur anywhere
in `crates/executor`. The executor then asks for
`Some(approach_radius(radius))` (`rcon_actuator.rs:298`), and `approach_radius`
halves and clamps to 0.5 (`rcon.rs:893`) — a goal disc of radius 0.5 centred on
a tile centre lies **entirely inside the blocked tile**. There is no legal goal,
so the pathfinder is right to refuse.

So the plan's *timing model* honours the annulus while the *dispatched walk*
aims at its forbidden centre.

**The precedent is in the same file.** `schedule.rs:103-108` records that
`radius` "used to be dropped here while `travel_ticks` went on using it, which
made every such walk execute as 'stand exactly on it'", and names the run it
cost. This is that identical defect surviving in its `min_radius` half.

**Fix:** carry `min_radius` in `StepKind::Walk` and pass it through to the
dispatch, so the executor asks for the annulus the planner specified. Until
then, expect a refusal whenever a bot is already standing in the tile it is
being sent to — and each one abandons that bot's entire remaining chain
(`run.rs:258`), which on a roster where bot 1 does ~88% of the work means most
of a batch.

Not implemented; a run held the workspace.

## A save generated without the mod poisons every later run

Runs 6 and 7 both hung at `start waiting` — 134-byte logs, no clients, forever.
The server was *healthy*: BotBridge loaded with a checksum, RCON up on 4321,
`InGame`, ten minutes in. The hang was on the CLI side, which waits by parsing
the server's stdout into `workspace/server-log.txt` — a file still dated 01:09
and not growing.

**The cause chains back to my own mistake.** Run 6 ran with `BotBridge` deleted,
and it *generated a fresh `level.zip` at 04:48 with no bridge mod in it* — a
`script.dat` of 1,242 bytes. Runs 6 and 7 then loaded that save. Restoring the
mod files afterwards does not repair it, because `info.json` stays at version
`0.0.1`: **Factorio only migrates on a version bump, so a same-version content
change leaves the old (here, absent) mod state in place.** The mod loads, the
checksum is logged, and it never initialises.

So a single run with a broken mod directory contaminates the workspace *save*,
and every later run inherits it looking perfectly healthy in the server log.

**Remedy:** move `workspace/server/saves/*.zip` aside and let a fresh map be
generated with the mod present. Kept in `workspace/saves-backup-2026-09-03/`
rather than deleted.

**Worth considering:** bumping `mods/BotBridge/info.json`'s version whenever
`control.lua` changes would make Factorio run `on_configuration_changed` and
turn this class of silent staleness into a migration it can act on.

## Deleting the workspace mod is the WRONG remedy (corrected)

I read CLAUDE.md's debug remedy as "delete `workspace/mods/BotBridge` and the
checkout will be used". **It is not what happens.** Deleting it simply removes
the mod: `workspace/server/mods` and every `workspace/clientN/mods` are
**symlinks to `workspace/mods`**, so the server came up with no bridge mod at
all and the CLI sat at `start waiting` for a handshake that could never arrive —
fourteen minutes of silence with a 134-byte log and not one client spawned.

**The remedy that works is to copy, not delete:**

```
cp -r mods/BotBridge workspace/mods/BotBridge
grep -c set_recipe workspace/mods/BotBridge/control.lua   # verify, every time
```

And there is a second half nobody would guess: **`workspace/mods/mod-list.json`
is regenerated by setup, and a run started while BotBridge was absent rewrote it
without BotBridge.** Restoring the files is not enough — the mod must also be
enabled in that list, or Factorio loads everything except the one mod that
matters.

**The general lesson, and the only check worth trusting:** grep the workspace
mod for the symbol you just added. Not the flag, not the log line, not the
commit. `set_recipe` was committed at 3de64f62 and was still absent from three
consecutive games.

## `Researched` re-sites the power plant on replan, spending a pole each time

Reported by the stage-2 agent, not yet fixed, and it is very likely the
proximate cause of the wood exhaustion: every replan re-sites the plant and
spends another pole (one wood per craft of two). Runs 1 and 2 each replanned
two or three times inside rung 1. With tree mining this is no longer fatal, but
it is still a plan that pays for the same structure repeatedly, and it interacts
badly with bot-1 concentration.

## Run 4 built the cell — the game accepted all eight placements

Worth separating from the failure. In run 4, rung 1 satisfied with zero
failures, rung 2 planned 129 steps, and **the game accepted all eight of the
cell's placements**. The geometry, the siting and the pole coverage work in a
real game. It stopped at `set_recipe`, which is a workspace-staleness trap and
not a planner defect. "The cell produces red science" remains unproven, but
"the cell can be built" is now observed rather than modelled.

## Runs orphan a server that then blocks the next run

Every failed run leaves its Factorio server alive holding 34197/udp and
4321/tcp, because the CLI exiting does not take the game down. The next run then
dies on `Host address is already in use` and leaves *its* server behind — a
self-perpetuating cycle that cost runs 3 and 5. Before launching: kill leftovers
by **explicit PID**, poll until both ports are free, then start.

`pkill -f <pattern>` is not safe here: `pkill -f "stage2-run4.log"` matched the
monitor watching that log and killed it too. Same string-matching trap as keying
a monitor's liveness to a process name.

## Run 2 never tested the fix it appeared to disprove

**Run 2 halted on the identical wood error and it is not evidence against the
tree fix.** The tree fix (`b0e3e12e`) was committed at **01:50:18 UTC**; run 2
started at **01:31:22 UTC** — nineteen minutes earlier, on a binary that did
not contain it. What run 2 actually tested was a *different* agent's
pole-optional fix, which could not clear the halt either, because rung 1 was
rebuilding the plant from scratch and so had no existing network for the cell
to adopt.

**The cause was mine: two agents in one checkout with no run interlock.** I
enforced "no run while an agent builds" on myself and never enforced it
*between* agents. The wood agent explicitly held and asked before running; the
other agent, which I had resumed with a status question, kept working and
started a run against a tree the first agent was still editing. A near-clean
tree is not evidence that a run is safe, and neither is one agent's promise.

**Consecutive runs also collide on ports.** Run 3 died on `Host address is
already in use` — run 2's server had not released 34197 — and its own server
then bound anyway and sat there orphaned with four clients. Wait for
34197/udp and 4321/tcp to be free before launching, and check for orphans
afterwards: the CLI exiting does not take the game down with it.

## Three corrections to what I believed (2026-09-03)

1. **My proposed share-allocation fix would have changed correct code.** I
   filed the wood bug as `SplitAcrossBots` mis-sizing a share. It is provably
   not in the path: its `claims` is `site.top_level && !site.in_chain`
   (`method/have.rs:1884`) and its `applicable` requires `Holder::Anyone`
   (`:1890`), while the failing goal was an in-chain `Share`. The real
   producer is `method/assemble.rs:997`, where `bill()` emits every cell
   ingredient as `Holder::Share(ctx.chain_actor)`. (There *is* a latent bug
   nearby — `even_shares` sorts poorest-first and would hand a one-unit split
   to the bot holding none — but it is unreachable from a `Producing` goal.)
2. **Two more dropped return values, both live.** `resource_mined`'s answer was
   discarded at `factorio/rcon.rs:2163`; it returns `Absent` for exactly a tree
   or a rock, so a chopped stump would have been re-offered for ever. And the
   mod counted delivery as `mined[mining.prototype.name]` (`control.lua:1309`),
   which is nil for a tree — `tree-01` yields `wood` — so a successful chop
   reported "the target was gone before mining finished". That is **six**
   instances of this class now.
3. **`EntityGraph`'s hand-written `Serialize` cannot round-trip JSON at all** —
   `resources` keys a `BTreeMap` by `Pos`, a tuple struct, and `serde_json`
   refuses non-string map keys. Pre-existing, found by a round-trip test, not
   fixed.

## A transient refusal costs a whole batch (found 2026-09-03)

**Two findings from tonight's runs meet here, and together they are a real
defect rather than two annoyances.**

`cannot place item '<x>' because a character is standing in the footprint`
fired **three times across runs 1 and 2**. The mod is right about it: a
character in a footprint is a *transient*, and `PlacementVerdict::
is_durable_refusal` (`crates/core/src/factorio/rcon.rs:506`) deliberately
refuses to remember it, because it "says nothing about the ground". The mod
even acts on it — `step_aside_from_footprint` (`control.lua:3171`) dispatches a
walk asking the blocker to move, so the next attempt would find the ground it
was always going to find.

**But the executor stops the bot anyway.** `crates/executor/src/run.rs:258`
says plainly "Stops that bot at its first failure: later steps in a chain
depend on it", and `abandon_rest(&mine[i..], senders)` throws away *every*
remaining step that bot had. That rule is right for a durable failure and wrong
for a transient the mod is already resolving.

**What makes it expensive is the other finding.** The wood RCA established that
**bot 1 performs essentially all the work** — 235 of 267 action dispatches in
run 1, and every single craft; bots 2/3/4 did 10-11 actions each. So stopping
"that bot" is, in practice, stopping the run: both of run 1's batches ended
early with 127 and 89 steps still pending, each after at least one failure.

So bot-1 concentration is not just a wood problem. It is an amplifier that
turns any single transient into most of a lost batch. Worth fixing at either
end: retry a transient on the same bot instead of abandoning its chain, or
spread work across the roster so one stopped bot is not the whole run. The
first is narrower and matches what the mod already does.

Not implemented — found while a live run held the workspace.

## Open, ranked

1. ~~`supervisor.witness` — the factory's definition of done.~~ **Done and
   exercised** (run 37, stage 1). Its *dead-cell* cost is still unmeasured:
   run 37 short-circuited at 480 of 2400 ticks, so nothing has yet paid the
   full window against four graphical clients.
2. Mod-side actions: the action deadline is a **360-second wall-clock sleep**;
   `control.lua:945` gates the per-player tick behind a `TODO FIXME`.
3. `crates/planner`'s own `BOT_FORCE` copy (`state.rs:66`) — **re-checked
   2026-09-03, and the doc comment above it is now wrong on both counts.**
   It cites `crates/executor`'s `rcon_actuator::BOT_FORCE` as a sibling
   definition; `rcon_actuator.rs:3` *imports* it from
   `factorio_bot_core::constants` and defines nothing. It gives the blocker
   as "the only place both crates can see is `crates/core`, which belongs to
   other work right now" — but `crates/planner` already depends on core
   (`Cargo.toml:13`) and core already defines `BOT_FORCE`
   (`constants.rs:25`). The shared home and the dependency both exist, so
   the stated reason for the copy has evaporated and it really is the
   one-line follow-up the comment promises.
   The third `"player"` spelling is **not a hazard**: all three literals in
   `crates/core/src/factorio/snapshot.rs` (`:219`, `:454`, `:476`) are inside
   *test fixtures*. Production reads `forces.get(BOT_FORCE)`, so changing
   `BOT_FORCE` makes those tests **fail** rather than drift. They are a
   tripwire, and this item was filed backwards.
4. `PLACEMENT_STEP_ASIDE_ACTION_ID = 4712` — a second magic id unknown to Rust.
5. ~~`pole_wire_reach("big-electric-pole")` says 30.0; the prototype says 32.~~
   **Unverifiable here, and probably a false alarm.** We have Factorio 2.1.17's
   *prototype API schema* (`workspace/factorio-api-docs/prototype-api.json`),
   which documents `maximum_wire_distance` as a property but carries no values;
   our own `entity-prototype-fixtures.json` has no wire field at all, which is
   precisely why the hardcoded table exists. 30.0 matches vanilla as far as I can
   establish. **Not changed** — altering a likely-correct constant on an
   unverified claim is the mistake, not the fix. What would settle it: have the
   mod send `LuaEntityPrototype.max_wire_distance`, which would retire the table
   rather than correct one entry of it.

## One process finding worth keeping

**All twelve plan documents are 0% ticked — 575 checkboxes, none checked.** The
convention was never adopted, so checkbox state carries no information and every
progress judgement has to be read off code. Worth knowing before trusting a plan.
