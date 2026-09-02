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

## The new milestone, and where it actually stands

The starter factory. **Stage 1 works and does not.**

Run 36 built a real cell — a burner drill standing on iron ore, a stone furnace
exactly where the drill's drop point lands, both fuelled. Five actions, zero
failures, correct geometry.

**It produced nothing.** `production.made` was `{coal: 37}` — the coal a bot
hand-mined. The drill was fuelled at tick 8822 and the run declared `done` at
**8823**. One tick. A burner drill needs ~240 ticks for a single ore.

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

## Open, ranked

1. `supervisor.witness` — the factory's definition of done. In progress.
2. Mod-side actions: the action deadline is a **360-second wall-clock sleep**;
   `control.lua:945` gates the per-player tick behind a `TODO FIXME`.
3. `crates/planner`'s own `BOT_FORCE` copy, and a third `"player"` spelling.
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
