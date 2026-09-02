# Ore underfoot — `run-1788329146-40305` — 2026-09-02

## Status

Done. All gates green.

## Commit

`aad7fbce fix(planner): a character standing on ore takes that tile out of
selection` (branch `feat/axum-server`; touches `crates/planner/src/state.rs`
and adds `crates/planner/tests/ore_underfoot.rs`). Not committed:
`docs/superpowers/notes/2026-09-02-refusal-memory.md` was already staged by
someone else when I started and I left it staged and untouched; this note is
uncommitted by instruction.

## Did the record answer which bot it was, and from which stream?

**Yes, and it took two streams, one each.**

1. **`events.jsonl` gave the tile and the actor.** `action_dispatched` id 40 at
   tick 31 452 is `mine 5 copper-ore`, bot 1, `target {x: 23.5, y: 53.5}`;
   id 58 at tick 46 949 is `mine 10 copper-ore`, bot 1, `target {x: 22.5,
   y: 48.5}`. Both settle 301 ticks later with the mod's message. It also
   answered the "was the blocker mining?" question by elimination: **every**
   `action_settled` row in rung 4 — all 141 of them — names bot 1. Bots 2, 3
   and 4 were dispatched nothing after tick 6052.
2. **`samples.jsonl` named the characters.** Between tick 6000 and the end of
   the run, bots 2, 3 and 4 each report exactly *one* position — the sampler
   fires every 60 ticks and 1 268 samples later none of the three has moved a
   ulp. Bot 2 at `(22.203, 48.785)`, bot 3 at `(24.324, 57.203)`, bot 4 at
   `(23.305, 53.77)`. With the character half-box at ±0.19921875 and a tile
   half-side of 0.5, bot 4's box overlaps tile centre `(23.5, 53.5)` by
   0.504 on x and 0.429 on y; bot 2's overlaps `(22.5, 48.5)` by 0.504 and
   0.414. Both targets, both blockers, exact.
3. **`splits.json`** placed it in the run: rungs 1 and 2 satisfied at ticks
   4967 and 6052, rung 3 already satisfied, rung 4 running 6052 → 79 900 and
   `stuck`. The three parked bots stopped moving *at* the rung 3/4 boundary,
   because rung 2's copper mine is the last work they were given.
4. `map.jsonl` and `frames/` contributed nothing and were not needed.

**Which bot it was, and which kind.** Parked roster bots that were **not
mining at all** — the brief's hypothesis, confirmed. `record.plan_created`
reports `bots: [1]` for all five of rung 4's plans, and per
`2026-09-02-placement-refusal-3.md` that field is derived from the *steps*, so
it says "bot 1 got all the work", not "the roster was one bot". `run_started`
carries `bots: [1, 2, 3, 4]`, which is the roster. So this is the same shape as
the placement runs: a plan sized to fewer bots than the roster, and the
leftovers standing exactly where the previous milestone left them.

## Was this already covered by the tile spacing?

**No, and it could not have been.** `mining_tile_separation` (3.689 live,
3.989 in fixtures) spaces the tiles of *one plan* so that no bot standing to
mine tile A can be standing on tile B. It rides `claimed`, which is per
`PlanState`, and every plan starts with an empty ledger. Both blockers here
were standing on ore **before the plan existed** and had no claim, no action
and no place in the plan at all. The `2026-09-02-tile-occupancy.md` concerns
section predicted exactly this residual ("it will be across a re-plan boundary,
not within one plan") — 25 000 ticks across, in this case.

## Is the assignee known at selection time?

**No — not usably, and in three of the four selectors not at all.**

* `resource_supply_at_least` (from `Method::applicable`) and `resource_seats`
  (from `Method::concurrency`) are handed `&PlanState` and nothing else. There
  is no bot in scope to exempt. `resource_seats` is what `SplitAcrossBots`
  sizes a split from, so a rule that only the tile-handing selector could apply
  would make seats promise bots that selection then refuses to place — the one
  failure mode the four-selector agreement exists to prevent.
* `Mine::expand` does have `ctx.chain_actor`, and uses it (the bot's position
  is what tiles are ranked from). It is **not** the assignee. Methods emit
  `Actor::Role` with `pinned: None`; `schedule` decides the runner. A chain
  gets an *owner* — and so a guaranteed runner — only for `Holder::Bot` and
  `Holder::Share`; a chain opened because its method `converges` deliberately
  gets none ("who runs it stays the scheduler's decision",
  `expand_goal_body`). So a tile chosen while exempting `chain_actor` is a tile
  the scheduler may hand to someone else, which reproduces this failure in a
  narrower form. That is the same argument
  `2026-09-02-placement-refusal-3.md` settled for placement, and I did not
  re-litigate it in the opposite direction.

**What would make it expressible.** Two things, together: an owner on every
chain that mines (so expansion's bot is the runner), and a bot parameter on
`resource_unclaimed` threaded to all four selectors — which means
`Method::applicable` and `Method::concurrency` learning a bot, which they are
deliberately free of. Neither is worth it for what the exemption buys (below).

## Own target: exception, or not a problem?

**Not a problem, and therefore not an exception.** The check I make is:

> a resource tile is out of *selection* while **any** character's collision box
> overlaps it — `character_stands_on_tile(character_centre, tile_centre)`, the
> mod's `another character is standing on the <ore>` stated as geometry, and
> the same predicate `mining_tile_separation` is derived from.

No `who`. The distinction the brief asks about — whose feet — is real in the
game (the mod refuses only when `player.selected` resolves to a character that
is not the miner) but is not available to state here, per the section above.
What matters is that **not** stating it is cheap in exactly one direction:

* Refusing the occupant its own tile costs it *one step of an already-sorted
  walk*. `resource_tiles_for` ranks the whole patch by distance and skips
  excluded tiles; the tile next door is 1.0 away on a patch of thousands. The
  fourth test asserts this bound (`<= 1.5` tiles) rather than describing it.
* Accepting a tile with someone else's feet on it costs the milestone: 301
  ticks per attempt, replanned identically, eight iterations, `stuck`.

A character shadows at most a 2x2 neighbourhood of tiles (its box is 0.4
across), and in both the run's cases and the tests' it shadows exactly one. So
this is not a rule that can eat a patch.

**Empirically the exemption would also have bought nothing in this run.** At
rung 4's planning tick bot 1 was at `(20.254, 51.777)`, standing on copper tile
`(20.5, 51.5)`. Under the new rule that tile is refused to it and it takes a
neighbour — one tile of walking, on a plan that ran 74 000 ticks.

## The change

`crates/planner/src/state.rs`, one new private method and one line in
`resource_unclaimed`:

```rust
fn resource_tile_occupied(&self, position: &Position) -> bool {
    self.characters
        .values()
        .any(|character| self.character_stands_on_tile(&character.center(), position))
}
```

placed third of four, after claimed/crowded (a map lookup and a scan of an
almost always tiny claim set) and before `resource_tile_blocked` (a quad-tree
query).

* **`characters` already existed** and already holds *every* character keyed by
  player id, roster included, because `2026-09-02-placement-refusal-3.md` put
  it there. Nothing new is read from the world, and the field's own doc already
  argues the roster question this change depends on.
* **Tile centres, not floored keys.** `position` is what
  `EntityGraph::resource_patches` hands out, which restores the half-tile
  offset (`k + 0.5`), and `character_stands_on_tile` takes centres on both
  sides, so there is no round trip through `Pos` to get wrong. The character's
  centre comes back from `Rect::center()` on a box `from_world` built
  symmetrically around it.
* **The character's real collision box**, from the world's own `character`
  prototype via `character_half_box`, falling back to the live-captured
  ±0.19921875 only for fixtures with no prototype table.
* **Purity holds.** No I/O, no clock; `characters` is a `BTreeMap`, and the
  test is a boolean `any` over it, so iteration order cannot matter anyway.
  Nothing new is compared with `<` on floats beyond the two existing
  half-side comparisons. `expansion_is_deterministic` passes untouched.

## Did seats stay in agreement?

**Yes, by construction, and a test pins it.** All four selectors —
`resource_tiles_for`, `nearest_resource_tile`, `resource_supply_at_least` and
`resource_seats` — read `resource_unclaimed`, so one change point moved all
four. `seats_and_selection_agree_about_a_tile_with_someone_on_it` asserts the
consequence directly: with three isolated uranium tiles and a character on one,
`resource_seats` returns 2 (not 3) and `resource_tiles_for` asked for exactly
two seats' worth of ore hands back exactly two tiles, while three seats' worth
fails closed. This is the same seam the obstructed-tile fix used and for the
same reason.

## Existing tests

**None moved.** The full workspace suite passes unchanged, including all 296
planner unit tests, `tile_occupancy.rs`, `tile_reservation.rs` and
`split_capacity.rs`. That is not luck: `fixture_world()` ships no players at
all, so `characters` is empty in every pre-existing planner fixture and the new
exclusion cannot fire in them. Every test that exercises it had to create its
own characters, which is what the new file does.

## Tests added

`crates/planner/tests/ore_underfoot.rs`, four tests:

* `a_tile_under_a_parked_bot_is_not_handed_to_another_bot` — the run's shape.
  Roster of four; bots 2, 3 and 4 parked on the three copper tiles nearest the
  origin, each at the run's own offset from the tile centre
  (`(-0.195, +0.27)`, bot 4's real displacement from `(23.5, 53.5)`), so the
  test exercises the box test rather than an equality and shadows exactly one
  tile each. Asserts: each parked tile still reports its ore physically
  (`resource_available > 0`) but zero to selection; `nearest_resource_tile`
  walks past all three to the fourth; and every `Mine` action in
  `expand(gather copper-ore 20 for bot 1)` lands on a tile no character is
  standing on, tested with `character_stands_on_tile` — the mod's own
  condition, not a proxy.
* `the_tiles_the_bots_are_parked_on_are_the_ones_a_clear_plan_takes` — the
  control on the control. With the ground clear the plan takes exactly the tile
  the test then parks a bot on, so the test above is not passing on tiles the
  planner never wanted.
* `seats_and_selection_agree_about_a_tile_with_someone_on_it` — capacity
  agreement, on three isolated uranium tiles ten apart (the fixture ships no
  uranium, so every number is exact), with a control run at three seats /
  three tiles before the character is placed.
* `a_bot_parked_on_ore_still_mines_its_own_patch_one_tile_over` — the
  occupant control. One bot, standing on ore, is the only character in the
  world: expansion still succeeds, still mines copper, and the tile it gets is
  within 1.5 of the one under its feet. It then asserts the deliberate half —
  that the tile underfoot *is* excluded and its ore *is* still physically
  there — so the no-exemption choice is recorded as an assertion rather than a
  comment.

**Non-vacuity confirmed.** With the exclusion short-circuited off, three of the
four fail with the exact numbers expected (500 offered where 0 is required;
3 seats where 2 are required) and the control keeps passing, which is what a
control is for.

## Gates

* `cargo fmt --all -- --check` — clean (exit 0; the two changed files were
  formatted individually with `rustfmt --edition 2024`, not workspace-wide,
  since another agent has an unrelated staged file in this tree).
* `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  exit 0.
* `cargo test --workspace` — 47 suites, every one `0 failed`; planner lib 296
  passed. All through `nix develop --command`.

## Reported, not fixed

1. **This is still per-plan belief about a moving world.** The exclusion is
   read once, at `from_world`, from the snapshot the plan was built on. A bot
   that walks onto ore *after* the plan is made is invisible to it, exactly as
   the placement version is. The mod's `mine_step_aside_waypoint` remains the
   backstop for that, and it is still the only thing covering it.
2. **The phantom player shadows the origin, now for mining too.**
   `Planner::initiate_missing_players_with_default_inventory` invents a
   `FactorioPlayer` at `(0, 0)` for every requested bot the game has no player
   for, and nothing distinguishes it from a real bot parked there. Since
   refusal-3 that box already blocked *placement* at the origin; it now also
   removes any resource tile at the origin from selection. Bounded (one tile)
   and in the safe direction, but it is a second consumer of the same upstream
   defect, which strengthens the case for fixing it upstream — either do not
   invent the player, or mark an invented one.
3. **The replanner still does not learn from a refusal.** Rung 4 chose
   `(23.5, 53.5)`, was refused, replanned, and the next iteration chose
   `(22.5, 48.5)` — a *different* occupied tile, so this run is not the
   identical-resite case, but nothing carried "a character was standing here"
   from the run into the next plan. There is now a `placement_refusals` ledger
   doing precisely that for builds (`PlanState::refused`); a mining equivalent
   would be the obvious symmetry, and would also cover residual (1). Carried
   over, still unfixed.
4. **Nothing purges a mined-out tile from the model.** Unchanged from
   `2026-09-02-obstructed-tiles.md`; not touched here.
5. **`record.plan_created`'s `bots` field still reads like a roster and is
   not one.** It reports `[1]` for every rung-4 plan in this run while the
   roster was `[1, 2, 3, 4]`. Refusal-3 asked for this to be renamed
   (`bots_used`) or for the roster to be recorded alongside; it has not been,
   and it misled the first pass over this run's evidence too. `run_started`'s
   `bots` is the roster and is the field to read.

## Concerns

* **A seat is a tile, and a character now takes one.** On a real patch this is
  noise. On the 121-tile fixture patches, four parked characters can in
  principle shave a seat off a `resource_seats` count and narrow a split by
  one bot — which is a *correct* narrowing (that bot genuinely has nowhere to
  stand) but is a new way for a plan to be smaller than the roster, and the
  refusal at zero seats still surfaces as `NoRoomToWork`.
* **The occupant costs itself a tile, forever.** The exemption is not
  impossible, only unavailable today; if chain ownership is ever extended to
  every mining chain, revisiting this becomes a real (small) optimisation, and
  `resource_tile_occupied`'s doc says so in place so the next reader does not
  have to rediscover why it is written the way it is.
