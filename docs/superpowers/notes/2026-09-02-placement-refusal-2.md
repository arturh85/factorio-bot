# Placement refusal, second pass -- run-1788319014-01846 -- 2026-09-02

## Status

Done. All gates green.

## Commit(s)

`fix(planner): treat a character the plan cannot move as an obstacle`
(branch: `feat/axum-server`; touches `crates/planner/src/state.rs`,
`crates/planner/tests/placement_occupancy.rs`)

## Did the record answer it?

**Yes, and it was `samples.jsonl`.** This is the first of the three placement
investigations that did not need a guess.

The three streams in order:

1. **`events.jsonl`** gave the exact tiles. `action_dispatched` now carries
   `target`, so there was no reconstruction step: action 62 at tick 20975 aimed
   at `[-19, 51]`, and action 1 aimed at `[-18, 51]` at ticks 21037, 21056 and
   21074. Five refusals, two tiles, one message.
2. **`map.jsonl`** ruled out the model. The end-of-run keyframe (tick 21074,
   bounds `[-66,-17] x [-1,65]`) covers both tiles. Around them the game has
   one `stone-furnace` at `(-17, 49)` and a field of `copper-ore`; the model has
   the same furnace and the same field. The whole regional divergence is three
   `copper-ore` tiles the model is missing -- `(-14.5, 49.5)`, `(-15.5, 50.5)`,
   `(-18.5, 52.5)` -- and none of them is inside either footprint. Ore does not
   block building anyway. So `game` and `model` **agree**, which by the
   diagnosis rules means the obstruction is of a class the keyframe filters out
   of both sides: `tree`, `rock-small`, `item-entity`, `character`.
3. **`samples.jsonl`** named it. Bot 2 sits at `(-18.2421875, 51.28125)` and has
   not moved since tick 7200 -- 13 800 ticks of a run that ended at 21074. That
   position is inside the stone-furnace footprint at `[-18, 51]` (which spans
   `[-18.8, -17.2] x [50.2, 51.8]`) and inside it at `[-19, 51]` too. Bot 3 is
   parked the same way at `(-11.74, 52.29)`, which is why the *other* furnace in
   the plan kept sliding between `[-12, 49]`, `[-11, 50]` and `[-15, 49]` across
   replans.

The mod corroborates the identity independently, and this is worth recording
because it means the run's log line already carried the answer. `rcon_place_entity`
(`mods/BotBridge/control.lua:2439`) builds with
`build_check_type = defines.build_check_type.manual`, and on a refusal it checks
whether the **acting** player is inside the expanded footprint: if so it answers
`§player_blocks_placement§`, otherwise the generic
`can_place_entity said 'no'`. All five refusals were generic. So the game was
saying, in as many words, *someone who is not the builder is standing there*.
Bot 1 was at `(-16.9, 51.28)` on the last attempt -- outside the box. Bot 2 was
not.

## Root cause

`PlanState::is_area_clear` had four sources of occupancy -- the plan's own
placements, the base world's entity tree, `blocking_boxes_within` (trees,
cliffs, rocks, water; added in `11c2655a`), and ore by tile. **Characters are in
none of them.** `EntityGraph::add` inserts only a whitelist of factory entity
types, so no character ever enters `entity_tree`; `blocked_tree` does not hold
them either. The planner's belief about the ground therefore could not include
a bot standing on it, and open ground was exactly what it saw.

What turned a latent gap into a stuck run is the split sizing landed in
`597bfe24`: rung 4 sized its split to what the world could seat and got a roster
of `[1]`. Bots 2 and 3 were dropped from the roster while still standing where
rung 2 had left them, so nothing would ever move them and nothing modelled them.
`11c2655a` was a real fix for a real class (the forest tile in
`run-1788309767-54739`); it just was not this class. Same call, same message,
different cause.

Note this is *not* the reach-annulus case. `Condition::AtPosition`'s
`min_radius` (`placement_clearance`) already keeps the acting bot out of its own
footprint, and it worked -- that is precisely why the message was the generic
one rather than `§player_blocks_placement§`.

## Fixed

`PlanState` gains `foreign_characters: BTreeMap<PlayerId, Rect>`, computed once
in `from_world`: the character collision box of every player in `base.players`
whose id is **not** in this plan's roster. `is_area_clear` gained a fifth source
that tests it.

Roster bots are deliberately excluded, and the field doc says why at length.
`base.players` holds only where a roster bot *started*; the plan moves it, and
its simulated `BotState::position` is a fact about the plan's narrative rather
than about the ground at dispatch time. Blocking on either would refuse
placements a bot is about to walk away from -- the opposite mistake, equally
silent. The acting bot is already covered exactly by `min_radius`. What is left
is the case with no other guard: a character nothing in this plan will ever tell
to walk.

Purity holds: no I/O, no clock, ordered map keyed by player id (the `players`
`DashMap`'s iteration order moves with the hash seed, so the collection is
sorted rather than taken as it comes), boxes compared with the existing
`boxes_overlap`. `expansion_is_deterministic` is unaffected, and no existing
test moved.

## Reported, not fixed

1. **Phantom players.** `Planner::initiate_missing_players_with_default_inventory`
   invents a `FactorioPlayer` for every roster id the game has no player for,
   and `FactorioPlayer::default()` sits at `(0, 0)`. This run asked for four
   bots (`run_started.bots: [1,2,3,4]`) and got three clients, so player 4 is
   such a phantom. Once a split drops it from the roster it now shadows a
   ~0.4-tile box at the origin. That is a bounded false refusal -- the ring
   search in `free_area_near` steps to the next candidate -- and I took it in
   preference to silently believing a real parked bot is not there. The proper
   fix is upstream: do not invent the player, or mark an invented one so
   consumers can tell. Documented in place on the `foreign_characters` field.
2. **Mining under a parked character.** The mod's
   `another character is standing on the <ore>` is the same defect on the
   mining path: `resource_unclaimed` spaces tiles against *other claims in this
   plan* (`mining_tile_separation`) but knows nothing about a character outside
   the roster. `character_stands_on_tile` is already the exact predicate, so
   the fix is a one-line extension of `resource_unclaimed`. I did not make it
   because in this run it is not live -- bots 2 and 3 are parked at
   `(-18.24, 51.28)` and `(-11.74, 52.29)`, and neither position touches an ore
   tile (the nearest `copper-ore` on row 51.5 starts at `-16.5`). It is the
   obvious next failure if a bot ever parks on ore.
3. **Model duplicates.** Unrelated to the refusal, but the keyframe shows it
   plainly and it is worth someone's time: the model reports every resource
   twice. 900 `iron-ore` against the game's 417, 488 `copper-ore` against 248,
   316 `stone` against 152, 236 `coal` against 108. The divergence list does not
   flag it because both copies match a game entity. Roughly a 2x inflation of
   the resource half of `EntityGraph`.
4. **The replanner does not learn from a refusal.** Rung 4 chose `[-18, 51]`,
   was refused, replanned, and chose `[-18, 51]` again -- twice. The two plans
   differ only in the *other* furnace's site. Making the planner's belief
   correct (this change) is the right fix and removes the symptom here, but a
   refused site is information the next plan currently throws away.

## Test summary

`crates/planner/tests/placement_occupancy.rs`, five tests, using the run's own
coordinates unchanged: a control that both refused sites are clear with nobody
standing there (so the regression cannot pass on a stray fixture tree); the
regression itself, that a parked non-roster bot refuses both; that
`free_area_near` walks past it rather than returning it; that a bot *in* the
roster does not block its own site; and that the exclusion is the character's
0.4-tile box and no larger. Verified red before the change (2 of 5 failing) and
green after.

Gates: `cargo fmt --all -- --check` clean;
`cargo clippy --workspace --all-features --all-targets -- --deny warnings`
clean; `cargo test --workspace` exit 0, no existing test moved.
