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
