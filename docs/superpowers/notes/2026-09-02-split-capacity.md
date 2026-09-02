# A split as wide as the world, not as wide as the roster

## The gap, as it stood

`SplitAcrossBots` sized a split from the roster and the shortfall alone. Four
bots on a patch with three seats made four shares; the fourth found no tile;
`Mine::expand` refused it; and the **whole** expansion came back
`NoApplicableMethod` naming the top-level goal. A three-bot plan on a
three-seat patch is a perfectly good plan and was being thrown away.

Two notes flagged it and both declined to fix it for the same reason, which is
the real difficulty and not an excuse: `SplitAcrossBots` splits *items*, and a
seat is a *mining* concept. Passing it a tile count would weld the two
together permanently.

## Where the capacity concept went, and why

**A new question on the `Method` trait**, answered by `Mine` in seats and by
nobody else:

```rust
fn concurrency(&self, goal: &Goal, state: &PlanState, cap: u32) -> Option<u32>
```

"How many holders can pursue this goal at the same time?" `None` — the default
— is "this method names no limit", which is what `Smelt`, `HandCraft`,
`Researched`, `AlreadySatisfied` and `SplitAcrossBots` itself all return. `Mine`
returns `Some(resource_seats(state, item, cap))`.

The driver asks (`expand_goal_body`), because the driver is what holds the
registry, and puts a **number** on `ExpansionCtx::concurrency`.
`SplitAcrossBots::expand` reads that number and caps `chains` with it. It never
learns what produced it. The generic method stays generic: it now has three
bounds — roster, shortfall, capacity — and knows the reason for only the first
two.

Three details that are load-bearing rather than incidental:

* **Asked at scatter sites only** (`site.top_level && !site.in_chain` — exactly
  what `SplitAcrossBots::claims` tests). That is the only place one goal can be
  handed to several bots, so it is the only place an answer can change a plan,
  and asking elsewhere would buy a walk of an ore field per subgoal for a
  number nobody reads. The site is the driver's own concept (`GoalSite`), so
  this is not a special case for one method — any future scattering method is
  claimed at the same sites and served by the same value.
* **Every method is asked, not the one `find` picked**, and the *minimum* wins.
  The goal a splitter sees is the shared form; the goals its shares become
  carry a different count and holder, so which method claims them is not
  settled yet. In practice it is exact anyway: an item is either mined or
  crafted, never both, so only one method has anything to say about any item.
* **Answered whether or not the method is applicable.** A fully committed patch
  makes `Mine::applicable` *false*, and that is precisely the state whose seat
  count matters most. An applicability-gated question would go silent at zero
  and report "no limit", widening the split back to the roster and failing one
  share at a time. So `None` must mean "nothing to say about this goal at all",
  never "cannot help right now" — which is why `Mine::concurrency` returns
  `None` for iron plate (not a resource) and `Some(0)` for a committed iron
  field.

`resource_seats(state, item, cap)` lives in `method/util.rs` beside the other
three tile selectors and reads the same `resource_unclaimed` ledger they do, so
it cannot drift from what `resource_tiles_for` will actually hand out. It is a
**greedy** packing in one global `(x, y)` order — flattening every patch first,
because `PlanState::resource_patches` partitions one field differently from
call to call and a per-patch walk would count differently each time. Greedy
means the count is a set of seats that genuinely exists, never an over-count,
and not necessarily the largest such set. Erring low is the safe direction: a
narrower plan, every chain of which has somewhere to stand.

`cap` exists so a real ore field (thousands of tiles, hundreds of seats) is not
walked further than any caller can use. The driver passes
`state.bot_ids().len()`, which is a *true* ceiling and not merely a plausible
one — see the roster check below.

### What I rejected

* **Passing a tile count, a patch or a separation into `SplitAcrossBots`.** The
  thing all three notes warned against. It reads as the smallest change and is
  the one that cannot be undone: mining is not the last constraint that will
  want to narrow a split (a future "one bot per furnace", "one bot per lab"
  would each need their own field), and every one of them would have to be
  plumbed separately.
* **Propose at full width and prune downstream.** The pruning would have to
  happen after `Mine::expand` has already failed, which means unwinding emitted
  actions, allocated `ActionId`s, chain ids and the state overlay that
  `run_steps` has been mutating. `ActionNetwork` has no rollback and the ids
  are monotone; adding both to serve one method's optimism is a large change
  with a wide blast radius, and the plan it produces would be identical to the
  one capacity gives directly.
* **Retry the expansion at decreasing widths.** Same rollback problem, plus it
  turns one expansion into up to `n`, and it discovers the answer by failing
  rather than by asking. It also cannot produce a good error: the last failure
  it saw is `NoApplicableMethod` on a share, which is the message this change
  exists to improve on.
* **Making the seat count a `PlanState` question** (`state.concurrency_for(goal)`).
  `PlanState` knows nothing about methods, so it would have to grow a
  per-goal-kind switch — the coupling moved rather than removed, into a type
  that is deliberately just a world overlay.
* **Gating `SplitAcrossBots::applicable` on capacity.** It has no `ctx` and so
  no answer; and refusing there would fall through to `Mine`, which is
  inapplicable for the same reason, and the caller would get the same
  unreadable `NoApplicableMethod`. `Researched::applicable` already sets the
  precedent: claim the goal so you can refuse it *by name*.

## What a zero-seat refusal now says

New variant, `PlannerError::NoRoomToWork { goal, holders }`:

```
nothing in this world can seat a bot to work on have 12 iron-ore (anyone); 4 were available to share it
help: a mining goal is seated by its patch, and a tile inside another miner's
      standing room is not a seat — so a patch this plan has already committed
      to offers none
```

against the old `no method can satisfy goal: have 12 iron-ore (anyone)`, which
reads the same whether the world has no iron ore at all, the recipe is locked,
or the plan itself has taken every seat. `holders` is there so a reader can tell
"nobody fits" from "not everybody fits" — the latter is no longer an error at
all.

Refusing rather than planning zero work is deliberate and is the second
requirement of the design: an empty plan is indistinguishable from a finished
goal, so a silent zero-width split would be worse than the failure it replaces.

## An adjacent hole this closed on the way

`SplitAcrossBots::expand` now checks its **whole** candidate roster against the
state before sizing anything, returning `PlannerError::UnknownBot`.

That check already existed in `expand_goal`, but it fired only for a bot that
actually received a share — so it was already silent whenever the split was
narrower than the roster (a shortfall of two across four bots has never checked
bots 3 and 4). Capacity makes narrow splits ordinary rather than exceptional,
which would have turned an incidental gap into a systematic one, and it is also
what makes `cap = state.bot_ids().len()` honest: without it, a registry roster
naming a bot the state does not know would be silently narrowed away instead of
reported. `mod.rs::expanding_against_a_bot_the_state_does_not_know_is_an_error`
still passes, now for a stronger reason.

## Determinism

Nothing new was introduced that needed a tie-break, and the ordering that
decides *which* bots keep their seats was already total: candidates are sorted
by `(spare, BotId)` with `BotId` unique within the deduped roster, so the
prefix taken by a narrower `chains` is as deterministic as the full list was.
`resource_seats` sorts tiles by `x.total_cmp` then `y.total_cmp` over a
flattened, deduped tile set, and walks a `Vec` in that order; the crowding test
it calls reads a `BTreeMap`. No floats are compared except through `total_cmp`.

`red_science.rs::expansion_is_deterministic` still passes;
`tile_reservation.rs::tile_assignment_is_deterministic` still passes; and
`split_capacity.rs::a_narrowed_split_is_deterministic` pins the new case
directly, including that the answer does not depend on the order the caller
listed the roster in.

## Tests

New `crates/planner/tests/split_capacity.rs` (5):

* `four_bots_on_a_three_seat_patch_plan_three_shares` — the whole point. Six
  tiles in three touching pairs, ten tiles apart: three seats, three shares,
  three distinct tiles, and all 40 ore still planned.
* `four_bots_on_a_one_seat_patch_plan_one_share` — the narrow end.
* `a_patch_that_seats_nobody_is_refused_by_name` — `NoRoomToWork`, `holders ==
  4`, message naming both the shortage and the goal.
* `a_crafting_goal_still_splits_across_the_whole_roster` — the negative
  control, run on the very world where iron ore seats three, with plates seeded
  so no mining can reach the plan by the back door. Four shares, four crafts,
  zero mines.
* `a_narrowed_split_is_deterministic` — two expansions plus a reversed roster.

These build their own worlds rather than claiming tiles out of
`fixture_world()`, because a claim also *crowds* its neighbours: the seat count
of a partly-claimed fixture patch is an emergent number nobody can read off the
test. Three pairs of touching tiles is three seats by construction — and, being
six tiles, also says the count is seats rather than tiles.

New unit tests in `method/util.rs` (4):
`a_patch_seats_far_fewer_bots_than_it_has_tiles` (121 tiles, 9 seats, derived
from 11 tiles per axis at a 3.989 separation rather than read back off the
function), `a_fully_committed_patch_seats_nobody` (with the untouched copper
field as its control), `counting_seats_stops_at_the_cap`,
`an_item_that_is_not_a_resource_has_no_seats`.

New unit tests in `method/have.rs` (2):
`only_mining_names_a_limit_and_it_names_it_in_seats`,
`a_committed_patch_still_reports_its_zero` (which asserts `!Mine.applicable`
first, so it pins the applicability-independence rather than assuming it).

**Confirmed non-vacuous.** With `Mine::concurrency` forced to return `Some(cap)`
— the roster size — all four capacity-dependent tests in `split_capacity.rs`
fail, as does the rewritten `tile_reservation.rs` one. The controls keep
passing: `a_crafting_goal_still_splits_across_the_whole_roster`, all four
`tile_occupancy.rs` tests, all seven `red_science.rs` tests including
`expansion_is_deterministic`, and the other four `tile_reservation.rs` tests.

### The one existing test that moved

`tile_reservation.rs::a_patch_too_small_for_the_roster_is_refused_not_overcommitted`
→ `..._is_narrowed_not_overcommitted`. **The old expectation was wrong**, not
merely outdated: it committed a patch down to one seat, gave two bots a
two-ore goal and asserted a refusal. One seat and two bots is a one-bot plan.
The test now asserts the plan — one mining action, carrying the whole goal, on
the tile that is actually free — which is still the property the file exists
for (both bots must not be sent to the one seat). Its refusal half moved to
`split_capacity.rs`, where the patch seats nobody and a refusal is right.

No other test moved. `scheduling.rs` builds networks by hand and never reaches
expansion; `red_science.rs` and `seeded_roster.rs` split crafted items, which
name no limit; `smelt_roots.rs` likewise.

## Gates

* `cargo fmt --all -- --check` — clean.
* `cargo clippy --workspace --all-features --all-targets -- --deny warnings` — exit 0.
* `cargo test --workspace` — exit 0 (planner lib alone: 291 passed).

All through `nix develop --command`.

## Concerns

* **A seat per participant, not per mining action.** A share large enough to
  need two tiles needs two seats and this does not count that. It takes a
  single share above `DEFAULT_RESOURCE_PER_TILE` (500) ore to arise, and the
  over-count is then caught by `Mine::expand`'s own tile walk failing — the
  same refusal, one frame later, with the old unreadable message. If
  `NoApplicableMethod` on a raw-ore goal reappears, this is the first thing to
  check.
* **The greedy packing is order-dependent, and per-share selection uses a
  different order.** `resource_seats` walks tiles in `(x, y)`; each share's
  `resource_tiles_for` walks them nearest-first from its bot's position. Both
  respect the same separation, but a nearest-first walk can in principle end up
  with fewer seats than a corner-first one on an awkwardly-shaped patch. That
  is the same asymmetry `resource_supply_at_least` already documents in §6 of
  the tile-occupancy note, and it fails the same safe way. Giving the seat
  count an origin would fix it exactly and would make it a different number for
  every bot, which is not a quantity a single split can be sized from.
* **Capacity is per-`PlanState`, like the claims it reads.** Two successive
  plans can both size themselves against a patch whose seats the previous
  plan's bots are still occupying. Within one plan this is complete; across
  re-plans it is only as good as the observed world the re-plan reads, and the
  mod's step-aside remains the backstop.
* **`Method::concurrency` is asked of every method, applicable or not.** That
  is deliberate (above) but it means a method answering expensively pays on
  every scatter site regardless of whether it could have helped. Only `Mine`
  answers at all today, and only at scatter sites, so the cost is one bounded
  patch walk per top-level goal.
