# Exclusive resource tiles within a plan

## The defect, reproduced before it was fixed

`crates/planner`, four bots, `Have { iron-ore, 40, Anyone }` on the shared
fixture world. Every `Mine` action in the resulting network:

```
{"iron-ore [-34.5, 35.5]": 40}
```

One tile, four actions, forty ore. That is the run's `ERROR: the target stone
was gone before mining finished -- something else mined it first` seen from the
planner's side: whichever bot got there first mined the entity out from under
the others.

After the fix, the same expansion:

```
{"iron-ore [-34.5, 35.5]": 10, "iron-ore [-34.5, 36.5]": 10,
 "iron-ore [-35.5, 35.5]": 10, "iron-ore [-36.5, 35.5]": 10}
```

Four tiles, one action each, and all four still inside two tiles of the
original — exclusivity costs essentially nothing in locality, because
candidates are already ordered by distance, so the second claimant simply takes
the next one along.

## 1. Where tile choice happens, and where the bookkeeping went

**Choice happens at expansion**, in three functions in
`crates/planner/src/method/util.rs`:

* `resource_tiles_for(state, item, from, need)` — the real chooser. `Mine::expand`
  (`crates/planner/src/method/have.rs`) calls it once per goal and emits one
  `ActionKind::Mine` per returned `(tile, take)`.
* `nearest_resource_tile(state, item, from, need)` — one tile. Used by
  `Mine`'s sibling `Smelt` to anchor a furnace next to the ore it will consume.
* `resource_supply_at_least(state, item, need)` — the applicability test behind
  `Mine::applicable`, which must agree with `resource_tiles_for` (a test says
  so).

Scheduling is the wrong place. By the time `schedule()` runs, the tile is
already baked into the action's `kind`, its `Condition::ResourceAvailable` and
its `Effect::ConsumeResource`; the scheduler picks *who* and *when*, never
*where*. Moving tiles there would mean rewriting emitted actions.

**The planner already reserves items**, and I deliberately did *not* put tiles
on that ledger. `PlanState::reserve`/`release`/`available`
(`crates/planner/src/state.rs`) is keyed by `Holder` — per bot, or once for
`Holder::Anyone` — and answers "how much of this stock is left for some *other*
goal to count towards itself". Its entries are scoped to a method's body and
released when that body ends. A tile commitment is neither: it is not held by
anybody, it is not a quantity of stock, and it must outlive the method that
made it for the rest of the expansion. Sharing the ledger would have meant
inventing a `Holder` for "the map" and suppressing the release.

There *was* already a per-tile ledger, though: `PlanState::consumed`, driven by
`Effect::ConsumeResource` -> `PlanState::consume_resource`, applied in
`run_steps` the moment a mining action is emitted. That is the right place and
the right moment; it was simply the wrong *question*. So the fix adds a second
map beside it:

* `consumed: BTreeMap<Pos, u32>` — how much this plan has taken. Unchanged.
* `claimed: BTreeSet<Pos>` — **new** — which tiles this plan has committed to a
  mining action.

and a second reader beside the physical one:

* `resource_available(pos, item)` — unchanged, physical: what the ground holds.
  `Condition::ResourceAvailable` still reads this, and must, because a claim is
  a fact about the plan and not about the world the executor will meet.
* `resource_unclaimed(pos, item)` — **new** — `resource_available`, or zero once
  the tile is claimed. All three selectors above now read this one.

`consume_resource` claims the tile it takes from, so no method had to change:
emitting a mining action is what commits its tile, exactly as before it was what
decremented its counter.

## 2. How much a tile actually holds

**The planner does not know, and I did not invent a number.**
`state.rs`'s `DEFAULT_RESOURCE_PER_TILE = 500` is a modelling constant, and its
existing doc comment says why: `EntityGraph` keys resources as
`resources: DashMap<String, Vec<Pos>>` — positions and nothing else.
`FactorioEntity::new_resource` leaves `amount` as `None`, and `EntityGraph::add`
routes resource entities into `resources`/`resource_tree` only, never into the
entity tree that carries per-entity data. There is no amount to spend.

That constant is precisely what made the bug invisible: 500 made the nearest
tile look like it comfortably covered four shares of ten, so
`docs/superpowers/notes/2026-09-02-mine-completion-fix.md` could write "this is
by design: the tile's total amount comfortably covers both shares". It was an
assumption, not a fact, and the run falsified it.

**So the fix does not rely on the number at all.** A tile is committed *whole*,
to one mining action. With one action per tile, "can two different takes
over-commit this tile?" stops being a question that can be asked — and the one
remaining bound, a single action's take against a single tile, was already
enforced (`resource_tiles_for` caps each take at `available`).

**What it would take to know the real amount** (not done, out of scope):

1. `mods/BotBridge/control.lua` would have to report `LuaEntity.amount` when it
   dumps resources — it reports position and name today.
2. `FactorioEntity::new_resource` (`crates/core/src/types.rs`) would have to
   carry it instead of leaving `amount: None`.
3. `EntityGraph`'s `resources` map would have to become
   `DashMap<String, Vec<(Pos, u32)>>` (or the resource entities would have to
   enter a tree that keeps their data), and `ResourcePatch::elements` would have
   to carry the amount out to the planner.
4. `PlanState::resource_available` would then read the real capacity rather than
   the constant, and `DEFAULT_RESOURCE_PER_TILE` would become the fallback for
   fixtures only.

Steps 1–3 are all outside `crates/planner`, which is the point: the planner is
pure and headless, and the only alternative — querying the game from inside
expansion — would be far worse than the bug.

## 3. When the patch cannot satisfy the whole plan

It refuses, at plan time.

`resource_supply_at_least` reads the same claim-aware ledger, so a patch whose
uncommitted tiles cannot cover the next share makes `Mine::applicable` false.
No other method can satisfy a raw-ore `Have` goal — ore is not craftable and not
smeltable — so `expand` fails with `PlannerError::NoApplicableMethod` naming
that goal. Nothing is emitted, nothing is dispatched, and no bot is sent
somewhere another bot is already going.

`tests/tile_reservation.rs::a_patch_too_small_for_the_roster_is_refused_not_overcommitted`
pins that: with every iron tile but one committed, two bots asking for one ore
each is refused, while one bot asking for two is still planned. Fewer bots on a
smaller patch is a plan; two bots on one tile is not.

Worth knowing, not fixed here: the refusal is all-or-nothing. `SplitAcrossBots`
decides the number of shares from the roster and the shortfall, with no idea how
many tiles are free, so a roster larger than the free-tile count fails the whole
expansion rather than planning a smaller split. Teaching it to cap the split
would couple a generic item-splitting method to resources; it needs its own
design pass, and the failure mode today is a clean refusal rather than a bad
plan.

## Determinism

Unchanged in shape and re-pinned in a new place. Claims live in a `BTreeSet<Pos>`
(ordered), tiles are still ranked by `distance.total_cmp` then `x` then `y`, and
claims accumulate in expansion order, which was already deterministic.
`red_science.rs::expansion_is_deterministic` still passes; it compares action
*labels*, which do not carry the tile, so
`tile_reservation.rs::tile_assignment_is_deterministic` compares the tiles
themselves across two expansions of the same goal.

Tile centres are handled by construction: nothing here converts a position, it
only passes `ResourcePatch::elements` values around and keys them through the
same `Pos::from` (flooring) that `consumed` has always used. Claims and
consumption therefore share one key space, and a `-40.5` never becomes `-41` on
the way back out.

## Tests

New `crates/planner/tests/tile_reservation.rs` (5):

* `four_bots_gathering_one_item_never_share_a_tile` — the run's shape. No tile
  carries two mining actions; no tile is committed beyond what it holds; the
  whole 40 is still planned.
* `a_multi_item_plan_commits_each_tile_once` — the same at one level up, red
  science on four bots, where the ore is an ingredient rather than the goal.
* `one_bot_on_a_large_patch_still_gets_the_nearest_tile` — the negative control.
  One action, on the tile `nearest_resource_tile` names, no spreading.
* `tile_assignment_is_deterministic` — two expansions, identical tiles.
* `a_patch_too_small_for_the_roster_is_refused_not_overcommitted` — honest
  refusal, plus the control that the last tile is still usable by one bot.

New unit tests in `crates/planner/src/method/util.rs` (3):

* `a_claimed_tile_is_not_offered_to_the_next_caller` — and the tile still reads
  a full 500 physically, which is the whole point of the two ledgers.
* `consuming_from_a_tile_also_commits_it` — the 499 left over must not attract a
  second bot.
* `a_supply_test_follows_what_has_been_claimed` — the applicability test and the
  tile walk agree on claims, not only on consumption.

**Confirmed non-vacuous.** With `resource_unclaimed` temporarily stubbed back to
`resource_available`, three of the five integration tests fail — including the
refusal test, which then *succeeds* in producing a two-bots-one-tile plan. The
two that still pass (negative control, determinism) are controls and are
supposed to.

**No existing test moved.** Nothing was adjusted to make it pass. The pinned
makespans in `tests/scheduling.rs` are built from hand-written networks and
never touch tile selection; `tests/red_science.rs` bounds makespans relatively
rather than pinning absolute numbers, and the tiles it now uses are one to two
tiles from the ones it used before, which does not change any bound it asserts.

## Gates

* `cargo fmt --all -- --check` — clean.
* `cargo clippy --workspace --all-features --all-targets -- --deny warnings` — exit 0.
* `cargo test --workspace` — exit 0 (planner lib alone: 276 passed).

All run through `nix develop --command`; a bare `cargo` cannot build `mlua-sys`
in this checkout (no `pkg-config` outside the dev shell).

## Concerns

* **The 500 is still a fiction, and it still matters in one place.** Exclusivity
  removes the *between-actions* dependence on it, but a single action's take is
  still capped at 500, so a plan that asks one bot for 400 stone will emit one
  action against one tile that may really hold far less — and fail the same way
  the run did, with no second bot involved. That is a different defect with the
  same symptom, and it is only fixable by steps 1–4 above. If the run's
  milestone-1 `iron-ore` failure recurs *with a single bot*, this is why.
* **Claims are per-`PlanState`, and every production path builds a fresh one**
  (`crates/scripting_lua/src/globals/goal/{mod,plan,recovery}.rs` all call
  `PlanState::from_world`), so nothing leaks between iterations. That is
  correct, but it also means two *successive* plans can pick the same tile — the
  second plan legitimately does not know the first one's bots are still standing
  there. Within one plan the fix is complete; across re-plans, exclusivity is
  only as good as the observed world the re-plan reads.
* **A plan now needs as many free tiles as it has mining actions.** Real patches
  have thousands, and the fixture 121, so this is not close to biting; but it is
  a new way for a plan to be refused that did not exist before, and the refusal
  reads as `NoApplicableMethod`, which is honest but does not say "the patch is
  committed". If that message shows up on a map that visibly has ore, this is
  the first thing to check.
