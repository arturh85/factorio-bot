# Morning summary — 2026-09-03

The detail is in `2026-09-02-morning-report.md`, which is 1,200 lines of
append-only log. This is the two-minute version.

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
