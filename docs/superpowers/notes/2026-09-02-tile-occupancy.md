# Bots standing on each other's ore

Run `run-1788313837-06402`, rung 1 (`gather iron ore x20`), 13 mine actions,
7 succeeded, 6 died on

```
ERROR: could not start mining for 301 ticks: another character is standing on the iron-ore
```

Tile reservation (`ee2d2108`) is what made this visible rather than what caused
it. Before reservation four bots raced for one tile and the losers failed
cleanly; after it they were spread neatly across four *adjacent* tiles and
blocked each other physically. The note that landed with reservation
(`2026-09-02-tile-reservation.md`) named the gap in as many words: the tile
selector has no reachability or occupancy notion.

## 1. Where a bot stands to mine, established rather than assumed

Three layers say something, and only one of them is a *guarantee*.

* **The plan** emits `Condition::AtPosition { pos: tile, radius: reach,
  min_radius: 0 }` (`crates/planner/src/method/have.rs`, `Mine::expand`), with
  `reach` read off the bot's own `resource_reach_distance`. That is a
  tolerance, not a position: it admits the whole disc.
* **The executor** walks to `approach_radius(radius)`
  (`crates/executor/src/rcon_actuator.rs::walk`, and again in
  `FactorioRcon::player_mine_timed`'s corrective walk), which is `radius / 2`
  clamped at half a tile — 1.35 tiles for a live character's 2.7. That is where
  the walk *aims*; where it lands is up to about `aim + 1.1`, because the path
  ends on a tile centre and the mod's follower stops within a 0.3-by-0.3 box of
  it. `approach_radius`'s own doc records that measurement.
* **The game** enforces `resource_reach_distance` silently: a `mining_state`
  aimed past it produces no event, no error and no progress. Both
  `within_resource_reach` (`crates/core/src/factorio/rcon.rs`) and BotBridge's
  mining branch (`mods/BotBridge/control.lua`) refuse the swing rather than let
  it hang.

So the only *guaranteed* statement is the third one: **every bot that actually
mines tile `T` is somewhere inside `disc(T, resource_reach_distance)`**. The
executor's aim is half that, but a separation has to be built on the bound the
game enforces, not on the aim, or it is a prediction dressed as an invariant.

`resource_reach_distance` is 2.7 for a live 2.1.17 character
(`crates/core/tests/live_2_1_payloads.rs` pins it, fractional and all), 3.0 for
a planner fixture bot with no player, and `f64::MAX` for a *player with no
character*. The last one is guarded (`MAX_PLAUSIBLE_RESOURCE_REACH`, 1000,
mirroring `mine_step_aside_waypoint`'s own guard) — propagating it would crowd
every tile on the map out of every plan.

## 2. The separation, and where each term comes from

`PlanState::mining_tile_separation()`:

```
separation = max plausible roster reach
           + (TILE_HALF_SIDE + character_half_x).hypot(TILE_HALF_SIDE + character_half_y)
```

* **First term, the reach**, for the reason above. The roster's *maximum*, not
  each bot's own: one tile has to keep every other bot off it, so the bound
  that matters is the largest reach anybody swings from.
* **Second term** is the furthest a character's *centre* can be from a tile's
  centre while still standing on that tile. Two axis-aligned boxes share ground
  exactly while the gap on both axes is under the sum of their half-sides, so
  the extreme is corner to corner — that hypotenuse. `TILE_HALF_SIDE` is 0.5
  because a tile spans one unit and the ore is reported at its centre. The
  character's half-box is read from the world's own `character` prototype
  (`character_half_box`, the same table `character_mining_speed` reads), with
  `VANILLA_CHARACTER_COLLISION_HALF_SIDE` (±0.19921875, captured off live 2.1)
  as the fixture fallback — that constant was already in this file for
  `placement_clearance`, which now shares the helper.

Their sum is the triangle inequality applied once: if `d(A, B) >= separation`,
then a bot anywhere within `reach` of `B` is further than the occupancy radius
from `A`, and so cannot be standing on `A`.

Numbers: **3.689** tiles live (2.7 + 0.989), **3.989** in the planner fixtures
(3.0 + 0.989). Nothing is tuned and nothing is a round number; change either
input and the separation moves with it, which is what
`the_mining_separation_is_reach_plus_the_occupancy_radius` pins.

The second term is not argued, it is walked:
`the_occupancy_radius_is_exactly_where_a_character_stops_standing_on_a_tile`
steps a 720-point ring at `radius + 1e-6` and asserts nothing overlaps, then
checks the corner direction at `radius - 1e-6` still does — so the radius is
the supremum of `character_stands_on_tile`, neither unsafe nor slack.

## 3. Where it is enforced

Exclusivity already had the right hook, so this rides it rather than adding a
second one.

* `claimed` changed from `BTreeSet<Pos>` to `BTreeMap<Pos, Position>`. **The
  value is the point.** `Pos` floors, so `From<&Pos> for Position` hands back
  `(-41, 40)` for ore that really sits at `(-40.5, 40.5)`. Harmless while a
  claim is only tested for equality; off by up to 0.71 tiles the moment a
  distance is measured to it. The claim now carries the centre it was made
  from. `crowding_is_measured_from_tile_centres_not_from_floored_keys` fails if
  anyone reintroduces the round trip.
* `PlanState::is_resource_crowded(pos)` — is `pos` within `separation` of a
  claim *other than its own*? Claimed and crowded stay separate questions so
  either can be read on its own.
* `PlanState::resource_unclaimed` asks both. That is the single reader all
  three selectors in `method/util.rs` already went through
  (`nearest_resource_tile`, `resource_tiles_for`, `resource_supply_at_least`),
  so none of their signatures changed and they cannot drift apart.
* `resource_tiles_for` additionally spaces its own picks from each other, since
  the claim only lands when `run_steps` applies `Effect::ConsumeResource` —
  after `expand` has returned every tile.

`PlanState::character_stands_on_tile(stand, tile)` is public because it *is*
the mod's `another character is standing on the <ore>`, stated as geometry, and
the tests assert against it rather than against a proxy for it.

**Determinism is untouched.** Claims live in a `BTreeMap<Pos, _>` (ordered),
tiles are still ranked `distance.total_cmp` then `x` then `y`, the crowding
scan is a boolean over an ordered map, and the roster's reach is folded with
`total_cmp`. No new tie-break was introduced, so there was none to make
deterministic. `expansion_is_deterministic` and
`tile_reservation.rs::tile_assignment_is_deterministic` both still pass.

## 4. Planner spacing, not the mod's step-aside

**Relying on the planner.** The step-aside that landed tonight
(`mine_step_aside_waypoint`, half the reach on the diagonal) is the recovery
for a bot that is *already* blocked, and it is genuinely needed — claims are
per-`PlanState` and every production path builds a fresh one, so a re-plan
cannot know where the previous plan's bots are still standing. But a mechanism
that fires once per blocked episode and hopes is a backstop; the fix for "the
plan created the block" belongs where the plan is made. Nothing in the mod
changed here.

## 5. A patch too small

It refuses, and it refuses *harder* than before — which is the honest reading of
a rule that says a tile inside another bot's standing room is not a seat.

The refusal path is unchanged in shape: `resource_supply_at_least` reads the
same claim-and-crowding ledger, so `Mine::applicable` goes false, no other
method can satisfy a raw-ore `Have` (ore is neither craftable nor smeltable),
and expansion fails with `NoApplicableMethod` naming the goal. Nothing is
emitted and no bot is sent where another bot is going.

**I ran into the `SplitAcrossBots` gap the reservation note flagged, and did not
fix it.** The split still decides its share count from the roster and the
shortfall, with no idea how many *seats* are free, so a roster larger than the
free-seat count fails the whole expansion instead of planning a smaller split.
Four bots on a patch with three seats is a legitimate three-bot plan and comes
back as a refusal. Spacing makes that reachable on smaller patches than
exclusivity alone did (a seat is now ~3.7 tiles across rather than 1), so the
design pass that note asked for is more wanted now than it was, but it is a
different change: capping the split needs `SplitAcrossBots` — a generic
item-splitting method — to learn something about resources, and doing it badly
would couple them permanently.

**One existing test moved, and the old expectation was wrong under the new
rule.** `tile_reservation.rs::a_patch_too_small_for_the_roster_is_refused_not_overcommitted`
claimed every iron tile but one and asserted the last tile was still plannable
for a lone bot. It is not, and should not be: that tile sits inside the standing
room of the tiles all around it, which is precisely the condition that killed
the run. The test now commits the patch down to a single *seat* — it claims the
tiles a separation away from one anchor — and **asserts** rather than assumes
that what remains is one seat, by checking every surviving tile is within a
separation of every other. The refusal and the one-bot control then read the
same as before.

No other test moved. `tests/scheduling.rs` builds networks by hand and never
touches tile selection; `tests/red_science.rs` bounds makespans relatively, and
the tiles it now uses are a few tiles further out than before, which does not
cross any bound it asserts.

## 6. The one place the two selectors stop being identical

`resource_supply_at_least` and `resource_tiles_for` read the same per-tile
ledger, so they agree exactly on which tiles are available. They can differ only
when a *single* call needs more than one tile, because `resource_tiles_for` also
spaces its own picks and the supply test cannot: whether `k` spaced tiles fit
depends on which tile the walk starts from, and applicability has no origin.

So the supply test is an upper bound in that one case. When it over-reports,
`Mine::expand` finds no tile set and returns the *same* `NoApplicableMethod` a
false answer would have produced one frame earlier — nothing plans a mine it
cannot execute either way. With `DEFAULT_RESOURCE_PER_TILE` at 500 the case
needs a single share above 500 ore to arise at all. Giving the supply test an
origin would fix it exactly and would also turn every applicability check into a
full sort of the patch, which is the cost its author deliberately avoided.

## Tests

New `crates/planner/tests/tile_occupancy.rs` (4):

* `no_bots_standing_position_lands_on_another_bots_tile` — the run's shape. Four
  bots, `gather iron-ore 20`, four tiles; for every ordered pair, the *worst
  legal* standing position for one miner (the point of `disc(tile, reach)`
  nearest the other tile) is asserted not to be standing on the other tile.
* `a_multi_item_plan_never_seats_a_bot_on_another_bots_tile` — the same one
  level up, red science on four bots, where the mining actions come from four
  unrelated chains rather than one split.
* `one_bot_on_a_large_patch_is_not_spaced_away_from_the_nearest_tile` — the
  negative control. One action, on the tile `nearest_resource_tile` names, no
  spreading.
* `the_four_nearest_tiles_would_have_collided` — the control on the control. The
  four nearest tiles, chosen the way the pre-fix planner chose them, *do*
  collide under the same predicate, so the positive test is not passing on a
  patch where nothing could ever have collided.

New unit tests in `crates/planner/src/state.rs` (5):
`the_mining_separation_is_reach_plus_the_occupancy_radius`,
`the_occupancy_radius_is_exactly_where_a_character_stops_standing_on_a_tile`,
`an_unbounded_reach_does_not_become_an_unbounded_separation`,
`crowding_is_measured_from_tile_centres_not_from_floored_keys`,
`a_tile_next_to_a_claim_is_crowded_without_being_claimed`.

**Confirmed non-vacuous.** With the separation temporarily forced to zero, the
two positive integration tests fail and all four state unit tests fail; the two
controls keep passing, which is what a control is for.

## Gates

* `cargo fmt --all -- --check` — clean (only the new file needed formatting;
  formatted file-by-file with `rustfmt`, not workspace-wide, because another
  agent is editing `crates/scripting_lua/src/globals/record.rs`).
* `cargo clippy --workspace --all-features --all-targets -- --deny warnings` — exit 0.
* `cargo test --workspace` — exit 0 (planner lib alone: 285 passed).

All through `nix develop --command`.

## Concerns

* **Separation is per-plan, like the claims it rides on.** Two *successive*
  plans can still pick tiles a tile apart, because the second one legitimately
  does not know the first one's bots are still standing there. That is where
  the mod's step-aside earns its keep, and it is the residual failure mode to
  watch for if `another character is standing on the` reappears — it will be
  across a re-plan boundary, not within one plan.
* **The separation is symmetric and unconditional, including between two tiles
  of the same bot.** `resource_tiles_for` spaces its own picks even though one
  bot cannot block itself, which costs that bot some walking. It only bites
  above 500 ore in one share, and making it actor-aware would mean attributing
  claims to bots — `Effect::apply` does carry a binding, so it is doable — for
  a saving that no reachable plan currently sees.
* **A seat is now ~3.7 tiles across.** A plan needs that much room per mining
  action, which is nothing on a real patch (thousands of tiles) and comfortable
  on the 121-tile fixture, but it is a new way for a plan to be refused, and the
  refusal still reads `NoApplicableMethod` rather than "the patch is committed".
  If that message shows up on a map that visibly has ore, this and the claim
  ledger are the first two things to check.
