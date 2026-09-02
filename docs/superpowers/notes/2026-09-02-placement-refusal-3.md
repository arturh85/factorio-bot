# Placement refusal, third pass -- run-1788322836-81715 -- 2026-09-02

## Status

Done. All gates green.

## Commit

`f9ff6aee fix(planner): a roster bot is not a bot the plan will move`
(branch `master`; touches `crates/planner/src/state.rs`,
`crates/planner/tests/placement_occupancy.rs`)

## Did the record answer it?

**Yes -- `events.jsonl` and `samples.jsonl` between them. `map.jsonl`
contributed nothing this time, and that is worth a line of its own.**

1. **`events.jsonl`** gave the tile and the ownership in one read.
   `action_dispatched` carries `target`: action 1 aimed at `(-16.0, -58.0)` at
   ticks 6198, 6204 and 6209, three refusals of one site. `plan_created` for
   milestone 4 reports `steps: 114`... and `bots: [2]`. That second field is
   the trap: `record.plan_created` derives `bots` from the *steps* (see
   `crates/scripting_lua/src/globals/record.rs`), so it names the bots that got
   work, **not the roster**. The previous investigation read the same field as
   "the rung sized its split down to one bot", and that reading is what made
   the roster carve-out look safe.
2. **`samples.jsonl`** named the character. At tick 6180 -- the last sample
   before the plan at 6190 -- bot 3 is at `(-15.328125, -58.2890625)` and has
   been since tick 5940. The stone-furnace box at `[-16, -58]` spans
   `[-16.8, -15.2] x [-58.8, -57.2]`; the character box is +/-0.19921875, so
   bot 3 spans `[-15.527, -15.129] x [-58.488, -58.090]`. The two overlap by
   0.327 tiles on x and lie wholly inside on y. Nobody else is near: the actor
   (bot 2) is at `(-16.215, -54.301)`, bot 4 at `(-14.230, -61.480)`, bot 1 at
   the origin.
3. **`map.jsonl` was useless here, and not by accident.** The file holds one
   keyframe, at tick 3814, bounds `[-16.6, 16.6]^2` -- nowhere near
   `(-16, -58)`. The supervisor asks for a keyframe at every milestone
   boundary, but `record.keyframe()` answers `false` when nothing has been
   placed, and this run never placed anything, so the three boundary keyframes
   produced no rows. **A run that fails on its first placement gets map
   coverage of the origin and nothing else** -- exactly the run where the map
   would be most useful. Reported below, not fixed.

The mod corroborates independently, as it did last time: `rcon_place_entity`
answers `§player_blocks_placement§` when the *acting* player is in the
footprint and the generic text otherwise. All three refusals were generic, so
the blocker was someone other than the builder -- which is what bot 3 is.

Confirming the roster took one more hop, because no stream states it. The
driver is `scripts/research_run.lua`, whose roster is `rcon.players()` -- every
player the *game* has with a character. `samples.jsonl` shows four players, but
bot 1 sits at `(0, 0)` for the entire run with inventory
`{burner-mining-drill: 1, stone-furnace: 1, wood: 1}`, which is precisely what
`Planner::initiate_missing_players_with_default_inventory` invents (and notably
lacks the 8 iron plates the three real clients start with). So three clients
connected, the roster was `[2, 3, 4]`, and bot 1 is a phantom. Milestone 1's
three-way split `6 + 7 + 7` over bots 2, 3, 4 agrees.

## Was it a roster bot?

**Yes.** Bot 3, on the roster, given zero of milestone 4's 114 steps, standing
in the footprint. The hypothesis in the brief was right, and the previous
note's exclusion is what let it through.

## Root cause

`foreign_characters` -- the fifth occupancy source added yesterday -- filtered
`base.players` down to the players *not* on the plan's roster. In this run the
roster was the whole connected game, so the filter removed all three real bots
and left the field holding only the phantom's box at the origin. The planner's
belief about the ground was, for every bot that could possibly be in the way,
exactly as wrong as it was before that fix landed.

The reasoning the filter rested on has two steps, and the second is false:

- *The plan moves a roster bot, so its base position is stale.* It does not
  necessarily move it. Which bots a plan moves is not settled until `schedule`
  assigns the work, and here the scheduler put all 114 steps on one bot.
- *Its simulated `BotState::position` is plan narrative rather than ground
  truth.* There is no simulated movement to distrust: `BotState::position` is
  seeded from `base.players` in `from_world` and **never advances during
  expansion**. `method::have`'s furnace siting says so in place -- it anchors
  on the ore rather than the bot precisely "because [the bot's start] never
  advances during expansion, otherwise every chain walks ore-patch, origin,
  ore-patch". So the observed position is not one of two candidate beliefs; it
  is the only position the planner has for any bot, roster or not, and it is a
  fact about the ground.

## What I chose

`foreign_characters` becomes `characters`: every player in `base.players`,
roster included, with the filter deleted. `is_area_clear`'s fifth source is
otherwise unchanged.

Purity holds unchanged: no I/O, no clock, `BTreeMap` keyed by player id (the
`players` `DashMap`'s iteration order moves with the hash seed), boxes compared
with the existing `boxes_overlap`. `expansion_is_deterministic` is unaffected.

## What I rejected

1. **Exempting the acting bot.** Tempting, and available: `Condition::holds`
   already takes a `BotId`, and `have.rs` has `ctx.chain_actor` at siting time.
   Rejected because it is unsound in the same way the roster filter was. The
   bot expansion has in hand is not necessarily the bot the *scheduler* hands
   the step to, and a site chosen while exempting bot A but executed by bot B
   reproduces this exact failure in a narrower form. The guarantee that the
   actor is clear of its own footprint comes from `Condition::AtPosition`'s
   `min_radius` at run time, and that guarantee attaches to whoever executes --
   not to whoever expansion guessed.

   The price of not exempting: the planner refuses the tile a bot currently
   stands on even when that bot is about to be walked off it. That costs one
   ring of `free_area_near`'s outward search (radius 12, hundreds of
   candidates; a character shadows a ~2x2-tile neighbourhood of candidate
   positions). The price of the other direction is a milestone: the site is
   refused, replanned identically, refused again, `stuck`. The asymmetry
   decides it -- and note that only the *conservative* direction is
   self-correcting.

2. **Dropping a roster bot once the plan has moved it.** The elegant middle:
   block on the base box only while the plan has not yet walked the bot. It is
   a no-op, because nothing moves a bot during expansion (above). It would be
   the right shape if simulated movement ever lands, and the field doc says so.

3. **Making the placement depend on the occupying bot's next move** (the
   brief's scheduling-rather-than-geometry suggestion). Rejected for this run:
   the occupier here has no next move -- bot 3 has zero steps in the plan, so
   there is no node to depend on and no edge that would ever release. A "walk
   aside" action the planner could synthesise for an idle bot is a real design,
   but it is a new action kind, a new effect on `BotState::position`, and a new
   way for the wait-graph to deadlock (two bots each waiting for the other to
   step aside). Geometry that is merely conservative buys the same run for two
   lines.

4. **Blocking ore tiles under a parked character** (`resource_unclaimed`, the
   previous note's item 2). Still unfixed, still the obvious next failure, and
   now more likely than before: bots park where they last mined, which is on
   ore. But it is *not* the same fix. A mining bot legitimately stands on or
   beside the tile it mines, so "no character on this tile" would refuse the
   actor's own target; the actor problem that makes exemption unsound for
   placement makes the naive predicate wrong for mining. It needs its own
   evidence and its own design. Not this change.

## Reported, not fixed

1. **`record.plan_created`'s `bots` field reads like a roster and is not one.**
   It is derived from the steps, so a plan that schedules everything onto one
   bot reports `bots: [2]` regardless of how many bots were available. The
   previous investigation read it as the roster and drew the wrong boundary
   from it; I nearly did too. Either rename it (`bots_used`) or record the
   roster alongside it -- `goal.plan` knows it, and `PlanOrigin` already keeps
   it.
2. **A run that never places anything gets no map coverage where it failed.**
   `record.keyframe()` returns `false` with nothing placed, so the supervisor's
   milestone-boundary keyframes silently produced nothing and `map.jsonl` holds
   only the origin-centred start keyframe. The bounds should be able to come
   from somewhere other than placed entities -- the bots' own positions, or the
   tiles the failed actions targeted -- precisely so that the first-placement
   failure is the case that *is* covered.
3. **The phantom player now shadows the origin unconditionally.**
   `initiate_missing_players_with_default_inventory` invents a `FactorioPlayer`
   for every requested bot id with no player, at `(0, 0)` with plausible
   reach distances -- there is no field that distinguishes it from a real bot
   parked at the origin. Previously it only blocked when a roster filter
   admitted it; now it always does. Still a bounded false refusal (one ring of
   search), still the right trade against believing a real parked bot is not
   there, and still fixable only upstream: do not invent the player, or mark an
   invented one.
4. **The replanner still does not learn from a refusal.** Rung 4 chose
   `[-16, -58]`, was refused, replanned, chose it again, twice. Carried over
   from the previous note unchanged. Making the planner's belief correct
   removes the symptom again; it does not make a refused site into information.
5. **Model duplicates**, from the previous note, were not re-checked -- this
   run's single keyframe is too small to say anything about them.

## Test summary

`crates/planner/tests/placement_occupancy.rs`, six tests, both runs' own
coordinates unchanged: the control that all three sites are open ground with
nobody standing there; run 1's regression (a bot outside the roster blocks);
**run 2's regression (a bot inside the roster blocks too)**; that
`free_area_near` walks past each of them rather than returning the site; and
that the exclusion is the character's 0.4-tile box and no larger.

One existing test moved: `a_bot_in_the_roster_does_not_block_its_own_placement`
is replaced by `a_parked_bot_inside_the_roster_blocks_a_placement_too`, which
asserts the opposite. **The old expectation was wrong**, not merely
superseded: it encoded the deliberate policy this run falsified, and it passed
only because it asserted the policy rather than any observed behaviour of the
game. Verified red before the change (2 of 6 failing, both the new roster
cases) and green after, by re-applying the filter and re-running.

Gates: `cargo fmt --all -- --check` clean;
`cargo clippy --workspace --all-features --all-targets -- --deny warnings`
clean; `cargo test --workspace` all suites `0 failed`.
