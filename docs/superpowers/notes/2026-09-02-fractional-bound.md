# The fractional bound was a distance, and the planner rejected its own arrival point

Run: `workspace/runs/run-1788325660-10154` — 300 actions dispatched, 297
succeeded, 157,080 ticks, rungs 1–3 satisfied in one iteration each, rung 4
(`research automation`) crashed in planning:

```
bot 1 owns chain ChainId(2) because its bill was sized against it,
but between 1.2705824974445776 and 10 of [-16, 18] does not hold there
```

Status: fixed in `crates/planner/src/schedule.rs`. Gates green.

## 1. What the condition is

Not a resource count. `1.2705824974445776` is the **inner radius of a
placement annulus**, in tiles.

`Condition::AtPosition { pos, radius, min_radius }`
(`crates/planner/src/action.rs`) renders as `between {min_radius} and {radius}
of {pos}` whenever `min_radius > 0`, and only one production site sets a
nonzero one: the `Place` action in `Smelt::expand`
(`crates/planner/src/method/have.rs:402`), which takes it from
`PlanState::placement_clearance` (`crates/planner/src/state.rs:846`) — half the
diagonal of the entity's collision box plus half the diagonal of the
character's. For a stone furnace against real Factorio 2.1 prototypes:

```
hypot(0.69921875, 0.69921875) + hypot(0.19921875, 0.19921875)
  = 1.2705824974445776   (bit-exact, checked against
                          crates/core/tests/entity-prototype-fixtures.json)
```

`10` is the build reach. `[-16, 18]` is the furnace site. So the condition
reads "stand between 1.27 and 10 tiles from the site" — the annulus added by
`docs/superpowers/notes/2026-09-02-placement-annulus.md` so a bot stops planning
furnaces on its own feet.

`DEFAULT_RESOURCE_PER_TILE` is not involved anywhere in this arithmetic.

## 2. Wrong, or merely ugly?

**The bound is correct and legitimately fractional. It must not be rounded.**

It is a distance in tiles, not a quantity of anything; the planner already
carries tile centres at `.5` and collision half-widths at `0.19921875`, so a
17-digit float here is ordinary. Rounding it to an integer would be wrong in
both directions: rounding a *lower* bound **down** (to 1.0) is the direction
that hides a shortfall — it authorises a stand-point inside the furnace's own
footprint, which is precisely the failure the annulus exists to catch, and the
game answers it with `player_blocks_placement`. Rounding **up** (to 2.0) is
safe but arbitrary: it would walk every bot a tile further than geometry needs
for no reason anyone could later justify.

**What was wrong was one ulp, at the other end of the same arithmetic.**

`arrival_point(to, min_radius)` (`crates/planner/src/schedule.rs`) returned
`Position::new(to.x() + min_radius, to.y())`. That sum is *rounded*, and the
offset measured back out of it — which is what `Condition::holds` and
`travel_ticks` both go on, via `calculate_distance` — need not equal
`min_radius`:

```
-16.0 + 1.2705824974445776 = -14.729417502555423
-14.729417502555423 - (-16.0) = 1.2705824974445772     <-- 4e-16 short
```

`min_radius` lives in the `[1, 2)` binade (ulp 2⁻⁵²); the sum for `x = -16`
lands in `[8, 16)` (ulp 2⁻⁴⁹), eight times coarser, and rounded down. The inner
bound is **inclusive** (`distance.total_cmp(min_radius).is_ge()`), so the point
the scheduler constructed specifically to satisfy the condition failed it.

The scheduler's feasibility trial (`schedule.rs`, `trial.set_position(bot,
arrival_point(...))`) therefore rejected its own arrival point. Because the
chain had an owner — bot 1, since its bill was sized against it — there was no
second candidate bot and no later tier, so the rejection became
`ChainOwnerInfeasible` and the whole expansion died.

**Why it had never bitten before.** Of the run's 44 dispatched furnace
placements, `x = -16` is the **only** x whose sum rounds down; every other x
(-18, -20, …, -56) rounds up and plans fine. And the two earlier placements
that *did* succeed at `x = -16` (`[-16, 22]` at tick 66797, `[-16, 20]` at tick
94627) succeeded because bot 1 was already inside the annulus both times
(6.01 and 2.05 tiles out, from `samples.jsonl`), so `travel_ticks` returned 0
and `arrival_point` was never consulted. The fixtures in the annulus fix's own
tests use a target at the origin, where `0.0 + 1.5` is exact.

## 3. Is the exhausted-tile gap implicated?

**No, and the record rules it out rather than merely failing to show it.**

`events.jsonl`: 128 `mine` actions dispatched, at **128 distinct tiles** — not
one tile was mined twice in the whole run — and each take was ≤ 5 against a
modelled 500. The failing condition is `AtPosition`, not `ResourceAvailable`,
and its bounds come from collision boxes rather than from
`DEFAULT_RESOURCE_PER_TILE`. The gap reported in
`2026-09-02-tile-reservation.md` and `2026-09-02-obstructed-tiles.md` is real
and still unfixed, but it was never exercised here.

One separate thing the record *does* show, already known and not fixed here:
one of the three failed actions was `ERROR: the target iron-ore was gone before
mining finished — something else mined it first`. That is the cross-*re-plan*
tile collision flagged under "Concerns" in `2026-09-02-tile-reservation.md`
(claims are per-`PlanState`, so two successive plans can pick the same tile).
The other two failures were the run's two `can_place_entity said 'no'`
refusals.

## Did the record answer it, and which stream?

Yes, and it took two streams.

* **`events.jsonl`** gave the answer. The `milestone_stuck` event carries the
  verbatim condition string, which named `min_radius`, `radius` and `pos` —
  enough to identify the condition as `Condition::AtPosition`'s annulus render
  and to match `1.2705824974445776` bit-for-bit against
  `placement_clearance("stone-furnace")`. The 44 `action_dispatched` `place`
  targets then supplied the decisive evidence: replaying `x + min_radius` over
  all 44 of them, `-16` is the single x that rounds short, and `-16` is the x in
  the crash. That is not a coincidence a reading of the source alone would have
  produced.
* **`samples.jsonl`** closed the loop on the two `x = -16` placements that
  *had* succeeded, which otherwise looked like a counter-example: bot 1's
  per-second position shows it already inside the annulus at both dispatch
  ticks, so those two took the zero-travel path where `arrival_point` is never
  called. It also shows all four bots ~76–85 tiles from `[-16, 18]` at the
  crash, confirming the walking path *was* taken there.

`map.jsonl` was not needed.

## The fix

`arrival_point` now rounds the offset **outward** when it has to round at all:

```rust
let mut x = to.x() + min_radius;
while calculate_distance(&Position::new(x, to.y()), to)
    .total_cmp(&min_radius)
    .is_lt()
{
    x = x.next_up();
}
```

* Outward, never inward, for exactly the reason in §2: the bound is a minimum,
  so a point corrected toward the target would stand closer than the footprint
  allows — the shortfall the annulus was added to prevent. A point corrected
  away from it is a fraction of a nanotile further out.
* One `next_up` suffices in every case observed; the loop is there so
  correctness does not rest on "in every case observed". It terminates: `x`
  starts strictly greater than `to.x()` (positive `min_radius` is established
  above it), `next_up` moves it strictly further away, so the measured distance
  strictly increases.
* Purity and determinism are unaffected: no I/O, no allocation beyond the
  `Position`, every comparison through `total_cmp` per the planner's rule.
  `min_radius == 0.` still returns `to` unchanged, byte-identical, so every
  disc-shaped condition is untouched.

The correction flows to all three consumers at once, because all three already
call `arrival_point`: the feasibility trial, the stored `StepKind::Walk`, and
the simulated post-walk position.

## Tests

Two added, both in `crates/planner/src/schedule.rs`:

* `a_walk_into_an_annulus_lands_where_the_condition_holds` — the run
  reproduced: target `[-16, 18]`, `min_radius` the exact clearance constant,
  bot at the origin (75 tiles out, so the walking path is taken). Asserts the
  recorded walk's `to` actually satisfies the `AtPosition` it was emitted for,
  rather than asserting a number.
* `an_arrival_point_never_rounds_inside_the_inner_bound` — the class rather
  than the instance, since whether the sum rounds up or down depends on which
  binade it lands in. 257 integer x coordinates × 5 inner radii (the stone
  furnace's, plus 0.5, 1.5, 2/3 and √2), each checked to satisfy the condition
  *and* to sit no further out than a hair — so this pins a correction, not a
  margin someone could later widen.

**Non-vacuous, checked before the fix existed.** Both fail on the unfixed tree,
and the first fails with the run's own message verbatim:

```
PreconditionUnsatisfied { action: ActionId(0), bot: BotId(1),
  condition: "between 1.2705824974445776 and 10 of [-16, 18]" }
```

(`PreconditionUnsatisfied` rather than `ChainOwnerInfeasible` only because the
minimal network has no owned chain; the rejection is the same one.)

**No existing test moved.** In particular
`a_walk_is_emitted_when_the_bot_stands_inside_the_annulus` still asserts the
walk lands at *exactly* 1.5 from the origin — `0.0 + 1.5` is exact, so no
correction fires — and `a_furnace_placed_where_the_bot_already_stands_gets_
walked_off_first` (`method/have.rs`) still passes unchanged.

## Gates

* `cargo fmt --all -- --check` — clean.
* `cargo clippy --workspace --all-features --all-targets -- --deny warnings` — clean.
* `cargo test --workspace` — exit 0. Planner: 296 lib tests (294 + 2 new) plus
  all 10 integration suites; core 270 + 47; executor 114 + 5; server, scripting
  and app suites all green.

All via `nix develop --command`; a bare `cargo` cannot build `mlua-sys` here.

## Reported, not fixed

* **The executor can still land the bot inside the annulus.** The planner now
  records an arrival point that provably satisfies the condition, but
  `rcon_actuator::walk` passes it to `approach_radius`, which clamps tolerance
  to a minimum of 0.5 tiles — so a real bot may stop up to 0.5 tiles from a
  point that is only 1.27 tiles from the site, i.e. at 0.77. That is the same
  gap `2026-09-02-placement-annulus.md` flagged under Concerns; it is an
  executor-side question and is not what crashed this run.
* **Cross-re-plan tile collisions** (`tile_reservation.md` Concerns) —
  responsible for the run's one mining failure, unchanged here.
* **Nothing retires a mined-out tile** — still true, still unexercised by this
  run.
* **`arrival_point`'s direction is still arbitrarily `+x`** and may point into
  water or a cliff. Unchanged and deliberate: the planner has no terrain
  knowledge. Worth noting that the direction choice is what makes the ulp
  correction cheap — along `+x` the true clearance needed is the sum of
  half-*widths* (0.898), well inside the diagonal bound, so the corrected point
  is nowhere near a real collision.
