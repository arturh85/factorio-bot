# The placement annulus fix

## Status

Fixed, tested, gates green. `cargo fmt --all -- --check`, `cargo clippy
--workspace --all-features --all-targets -- --deny warnings`, and
`cargo test --workspace` (all crates, all suites) are clean.

## Commit

Not yet committed at the time this note was written (I write the note before
committing per the task's instructions). See the actual commit log for the
sha; the change touches:

- `crates/planner/src/action.rs`
- `crates/planner/src/schedule.rs`
- `crates/planner/src/state.rs`
- `crates/planner/src/method/have.rs`
- `crates/planner/src/network.rs` (test literal only)
- `crates/planner/tests/scheduling.rs` (test literals only)
- `crates/executor/src/recover.rs`, `crates/executor/src/run.rs` (test
  literals only — they construct `Condition::AtPosition` directly in fixtures)

## The fix

`Condition::AtPosition` gained a `min_radius: f64` field alongside the
existing `radius`, turning the reach check into an annulus:
`min_radius <= distance <= radius` (both bounds inclusive, compared with
`total_cmp`). Every non-placement condition sets `min_radius: 0.0`, which
collapses the annulus back to the original disc — mining, inserting, crafting
are unaffected both in code and in every existing test.

`Smelt::expand`'s `Place` action (`crates/planner/src/method/have.rs`, the
only production site that builds an `ActionKind::Place`) is the one caller
that sets a nonzero `min_radius`, from a new `PlanState::placement_clearance`
method in `crates/planner/src/state.rs`.

`crates/planner/src/schedule.rs`:
- `travel_ticks` takes `min_radius` and now charges a walk in either
  direction: `distance - radius` when too far (unchanged), or
  `min_radius - distance` when too close (new). All three bound comparisons
  go through `f64::total_cmp`.
- A new `arrival_point(to, min_radius)` helper returns `to` unchanged when
  `min_radius <= 0.` (byte-identical old behaviour), and otherwise a point at
  exactly `min_radius` from `to` (arbitrarily placed along `+x`, since
  `Condition::holds` only measures distance, never bearing).
- The scheduler's own feasibility trial, the *stored* `StepKind::Walk` step,
  and the simulated post-walk position all now agree on `arrival_point`
  rather than assuming arrival lands exactly on the target. This mattered in
  practice, not just in theory: the first version of this fix left the stored
  `Walk{ to: target, radius }` untouched (still "get within `radius` of the
  furnace's centre") while only fixing the internal ticks/feasibility
  bookkeeping. `tests/red_science.rs::every_expansion_replays_in_time_order`
  — which replays a schedule in tick order and re-checks every precondition
  against wherever the plan's own `Walk` steps say the bot arrived — caught
  this immediately: it set the bot's replayed position to the furnace's exact
  centre (since that is what the stored step said) and the annulus precondition
  failed there, same as the live bug. The fix was to make the *recorded* walk
  target `arrival_point` too (with `radius: 0.0` for that leg), so the plan's
  own data is a claim that is actually true, not just a ticks charge.

## Where the inner radius comes from

`PlanState::placement_clearance(name)` (`crates/planner/src/state.rs`):
half the diagonal of the entity's `collision_box` (already present on
`FactorioEntityPrototype`, read via `entity_prototypes.get(name)` — the same
table `PlanState::collision_area`/`is_area_free` already use) plus half the
diagonal of the `character` prototype's own `collision_box` (same table,
key `"character"`). Both numbers are queried, never hardcoded, following the
exact pattern `character_mining_speed` already uses for the same table.

The sum-of-half-diagonals bound is the standard triangle-inequality argument:
no point of a box is farther from that box's own centre than the box's own
half-diagonal (the distance from centre to corner), so if two boxes share a
point, their centres are at most `half_diag_a + half_diag_b` apart. Centres
farther apart than that cannot share a point — conservative (a same-orientation
axis-aligned pair can share a corner touch at exactly that distance, in the
worst-case diagonal orientation, but never overlap beyond it), which is the
right side to err on for a placement check. `PlanState::is_area_clear` already
uses this exact reasoning to widen its own neighbour-search radius; this is
the same argument applied to a minimum instead of a maximum.

Returns `None` (not a guessed size) when the world carries no prototype for
`name`, mirroring `collision_area`. `Smelt::expand` falls back to
`min_radius: 0.0` in that case — which is moot in practice, because
`Condition::AreaFree` on the same action needs the same prototype and already
refuses the placement outright if it's missing.

The character's own box falls back to a vanilla constant
(`VANILLA_CHARACTER_COLLISION_HALF_SIDE = 0.19921875`, read from the
`character` entry of `crates/core/tests/entity-prototype-fixtures.json`,
itself captured off a live Factorio 2.1 game) only when the world's own
`entity_prototypes` carries no `character` entry at all — real games always
supply their own player-character prototype; only a hand-built fixture (and
one new geometry test deliberately checking this exact fallback) exercises
it.

No prototype data was missing for this fix — the planner's existing
`FactorioWorld` snapshot already carries collision boxes for both the entity
being placed and the `character`, via the same `entity_prototypes` table
`is_area_free` reads. Nothing new had to be plumbed in from the game.

## Existing tests that moved, and why

**Genuinely different behaviour** (the fix working as intended), not a wrong
old expectation:

- `crates/planner/tests/red_science.rs::every_expansion_replays_in_time_order`
  failed transiently while I had the `min_radius`/`travel_ticks` logic right
  but the *stored* `StepKind::Walk.to` still wrong (see above). Once the walk
  target and radius were fixed to agree with `arrival_point`, it passes again
  unmodified — I did not touch its assertions, only the production code the
  fix required.

No other existing test's assertions needed changing. I checked every test
file that constructs `Condition::AtPosition` or asserts on `StepKind::Walk`
schedule structure or exact makespans in the smelt/scheduling paths
(`crates/planner/tests/scheduling.rs`, `smelt_roots.rs`, `red_science.rs`,
`seeded_roster.rs`, `crates/planner/src/method/have.rs`'s ~90 tests,
`crates/planner/src/schedule.rs`'s own suite, `crates/executor`'s `run.rs`/
`recover.rs`) — none pin an exact makespan or step count that a newly-emitted
furnace-placement walk would perturb; they check `> 0`, relative comparisons
between two schedules, or properties (one bot per furnace, parallelism,
determinism) that are insensitive to an extra walk. The only edits to those
files were adding `min_radius: 0.0` to hand-built `Condition::AtPosition`
literals so they keep compiling as plain discs — no assertion in any of them
changed.

## Tests added

- `crates/planner/src/action.rs`: `at_position_respects_the_annulus` — a bot
  standing exactly on the target fails a positive-`min_radius` condition;
  one tile out holds; both the inner and outer edges hold (inclusive
  boundaries); past the outer edge still fails, same as a disc. Also
  strengthened `an_action_survives_a_json_round_trip` to carry a nonzero
  `min_radius`.
- `crates/planner/src/schedule.rs`:
  - `a_walk_is_emitted_when_the_bot_stands_inside_the_annulus` — the required
    "bot on the target now gets a Walk" case, plus asserts the stored walk's
    `to`/`radius` are themselves a checkable claim (lands exactly on the inner
    edge, `radius: 0.0`).
  - `no_walk_is_emitted_at_a_legitimate_annulus_distance` — the required
    "still no spurious walk at a real distance" case.
  - `travel_ticks_treats_both_annulus_edges_as_already_arrived` — pins the
    exact tick arithmetic at both boundaries and just inside the inner one.
- `crates/planner/src/state.rs`:
  - `placement_clearance_keeps_the_characters_own_box_off_the_footprint` — a
    *real geometric* check, not a ghost-placement stand-in: it drives the
    same private `boxes_overlap` the production box-against-box test
    (`is_area_free`) uses, with an actual character-sized box at the computed
    clearance, along the diagonal (the axis where two axis-aligned squares
    actually touch at the sum of their half-diagonals — an on-axis probe
    would have passed for a clearance far too small, since the true on-axis
    threshold is the smaller sum of half-*widths*). Confirms both "just
    clears at the computed distance" and "0.05 tiles inside it still
    collides."
  - `placement_clearance_grows_with_the_entity_and_is_none_for_an_unknown_one`.
- `crates/planner/src/method/have.rs`:
  `a_furnace_placed_where_the_bot_already_stands_gets_walked_off_first` — the
  full production path (`Smelt::expand` → `schedule`), reproducing milestone
  4 exactly: takes the real `Place` action's real `min_radius` (asserted equal
  to `placement_clearance("stone-furnace")`, not just nonzero), forces the
  bot onto the furnace's own future site, schedules, and checks the walk
  immediately before the placement lands at exactly the annulus's inner edge
  — then independently replays every one of the placement's own preconditions
  at that landing point (mirroring what
  `every_expansion_replays_in_time_order` checks generically) to catch the
  exact "ticks charged but the recorded claim is still false" class of bug
  this fix nearly shipped with.

## Test summary

`cargo test --workspace`: every suite passes, 0 failures, across
`factorio-bot-core`, `factorio-bot-planner` (273 unit tests + all 6
integration suites), `factorio-bot-executor` (111 unit tests + snapshot
suite), `factorio-bot-scripting-lua`, `factorio-bot-scripting`, `factorio-bot`
(app/src-tauri), and `factorio-bot-server`.

## Concerns

- **The real executor's walk primitive is not touched, and I did not verify
  it end-to-end (per instructions, I did not run Factorio).** The scheduler
  now emits a genuine `StepKind::Walk{ to: <point min_radius from the
  furnace>, radius: 0.0 }` for the escape leg. `crates/executor`'s
  `rcon_actuator::walk` passes `to`/`radius` into
  `FactorioRcon::move_player_timed`, which asks the game's pathfinder for a
  path that ends within `approach_radius(radius)` of `to` —
  `approach_radius` clamps to a minimum of `0.5`, so a `radius: 0.0` input
  does not zero out the tolerance or misbehave, but I have not run the real
  game to confirm the bot actually paths to a point that far from its start
  (1-2 tiles) rather than the pathfinder treating a very close goal as
  already-arrived. This is exactly the class of gap the task warned about
  ("tests that pass while the real thing fails"): the planner-level fix is
  now self-consistent and replay-checked, but the live RCON round trip is
  unverified. If it turns out the game's `request_path` does something
  surprising for a near-in goal, the fallback is the diagnosis's option 3
  (a cheaper escape dance in `mods/BotBridge/control.lua`) layered on top —
  not a reason to revert this fix, since the plan is now honest regardless of
  how well the executor currently acts on it.
- `arrival_point` picks an arbitrary direction (`+x` from the target) rather
  than a real walkable one, deliberately (documented in code): the planner
  has no terrain/collision knowledge to pick a better direction, the same
  limitation the pre-existing disc-case "walk to the centre" approximation
  already lived with. A future executor-side improvement (letting the game's
  own escape-dance logic choose the real point) would not need to touch the
  planner.
