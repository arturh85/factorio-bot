# Time-aware mining claims: the ceiling was the seat model, not the ore

**Touches:** `crates/planner/` only — `src/state.rs`, `src/method/mod.rs`,
`src/method/util.rs`, `src/method/have.rs`, `src/test_world.rs`,
`tests/tile_occupancy.rs`. Nothing in `crates/core`, `crates/executor`, the mod
or the frontend. `cargo check -p factorio-bot-executor` and
`-p factorio-bot-scripting-lua` both pass; the public API only gained methods.

Follows `docs/superpowers/notes/2026-09-02-convergence-stage-1.md`, which ended
with:

> `PlanState` commits a mining tile and its neighbours for the whole expansion
> because the model cannot say "at the same time". That single limitation is
> what makes a 121-tile patch seat nine, what forces G6's measured factor of
> two, and what caps how wide any convergence can ever go.

That is now fixed, and the consequence is larger than expected: on the
**shared** fixture — the one stage 1 concluded could host no convergence at all
— the unlock plan goes from **15866 to 12403 ticks**, a 22% cut, and the unlock
subtree from `{bot 1: 48}` to `{bot 1: 48, bot 2: 4, bot 3: 4, bot 4: 4}`.

## (a) How a claim expresses time

A claim is not held for the whole expansion any more. It carries **whose serial
timeline it sits on**:

```rust
struct MiningClaim { centre: Position, runner: Option<ClaimRunner> }

pub enum ClaimRunner {
    Bot(BotId),      // every chain this bot owns, serial on that bot
    Chain(ChainId),  // one unowned chain, serial on whoever gets it
}
```

Two claims conflict for **crowding** purposes unless they name the same runner.
`None` is *unknown*, never a wildcard, and never matches — not even another
`None`.

### Why a runner and not a tick range

Separation exists for one reason: a bot mining one tile stands on the tiles
around it, and standing there stops *somebody else* mining them. It is a
simultaneity rule and nothing else. A tile the first bot has finished with and
walked away from is free.

Expansion has no clock — who runs an action and when is `schedule`'s decision,
taken after the whole network exists. But the scheduler makes two guarantees
that expansion can read off the network it is building:

* **A bot runs one action at a time.** `free_at` in `schedule`, which every
  chosen action advances to its own end.
* **A chain is one bot's.** With an owner, the owner tier in `candidate_tiers`
  is a single-bot tier *with no fallback* — a hard constraint, the same class of
  thing as a pin. Without one, `chain_binding` still welds the whole chain to
  whichever bot takes its first action.

So two claims made inside chains owned by the same bot, or inside the same
unowned chain, are **provably disjoint in time whatever the schedule turns out
to be**. That is the whole of what an expansion can soundly know about time, and
it is exactly the fact the separation needed.

The driver sets the binding on the same lines that open a chain
(`expand_goal_body`, and the `Step::Owned` arm of `run_steps`), and restores it
on every exit path, exactly as it does for `ctx.chain`. It lives in `PlanState`
and nowhere else: a mirror in `ExpansionCtx` was written first and removed,
because it was only ever *written* — every tile selector reads the state — so a
second copy could only drift from the one that is consulted.
`set_claim_runner` returns what it replaced, which makes the state its own save
slot.

### The two objections this had to answer

`PlanState::resource_tile_occupied`'s doc records why "exempt the assignee" was
rejected before. Both halves are answered, and neither is answered by hand-waving:

* **"Three of the four selectors have no bot to exempt."** True, and they still
  do not take one. The binding is a *field on the state*, which all four
  selectors already hold, so `resource_supply_at_least`, `resource_tiles_for`,
  `nearest_resource_tile` and `resource_seats` cannot answer differently.
* **"`ctx.chain_actor` is not the assignee."** Correct, and `chain_actor` is not
  what is used. The binding is the chain's **owner**, which the scheduler treats
  as a hard constraint. Where there is no owner the binding is the chain, which
  the scheduler also honours; where there is neither, it is `None` and nothing
  is relaxed.

### What was deliberately not relaxed

* **Whole-tile exclusivity** (`is_resource_claimed`) stays global. It is not a
  simultaneity rule — it exists because the planner cannot know what a tile
  really holds (`DEFAULT_RESOURCE_PER_TILE`) — and one bot mining one tile twice
  meets that same unknown however far apart the swings are.
  `a_runner_does_not_get_its_own_tile_back` pins it.
* **`resource_seats` asks with `None`, always.** A seat is a spot for a bot that
  is *not yet in the plan*, working at the same time as everyone already in it,
  so every existing claim conflicts with it whoever holds it. Asking with the
  enclosing chain's runner would count that chain's own tiles as free seats and
  promise a split more participants than the ground can hold at once. Spelled as
  an explicit `resource_unclaimed_for(.., None)` rather than inherited, so the
  seat count cannot silently start answering the tile-selection question.
* **`resource_tiles_for`'s in-call spacing** *is* relaxed, on exactly the same
  condition, because it is the same fact: one `Mine::expand` emits one action
  per tile into one chain.

## Seats on the 121-tile fixture, before and after

`fixture_world`'s iron patch is 121 tiles and seats **nine** miners at a
separation of 3.99. That number is unchanged and must be — nine is still the
right answer to "how many bots at once".

What changed is what the plan *spends*:

| | before | after |
| --- | --- | --- |
| seats on the patch | 9 | 9 |
| tiles one runner can work | **9** | **121** |
| seats the un-converged unlock plan spends | 8 (one per mining action) | 4 (one per mining runner) |

`one_runner_may_work_the_whole_patch_the_roster_can_only_seat_nine_of` asserts
both halves of that: the walk takes all 121 tiles, and the patch is genuinely
used up afterwards.

## G6, halved — and that is where the win is

`worth_converging`'s G6 was `seats >= k + 2 * roster`, and the doubled slack term
was calibrated on the old measurement: the un-converged four-bot unlock plan
spent eight of nine seats, so a front had to seat the split *and* twice the
roster afterwards. Eight actions, four runners. The term is now
`seats >= k + roster` — one spare seat per bot, which is the most the rest of the
plan can want at once.

Measured on `unlock_state`, the **shared** fixture, four bots, four packs:

| | steps | unlock subtree | makespan |
| --- | --- | --- | --- |
| stage 1 (as landed) | 49 / 12 / 12 / 12 | `{bot 1: 48}` | 15922 |
| time-aware claims, G6 unchanged | 49 / 12 / 12 / 12 | `{bot 1: 48}` | 15866 |
| time-aware claims, G6 halved | 49 / 16 / 16 / 16 | `{1: 48, 2: 4, 3: 4, 4: 4}` | **12403** |

The 56 ticks in the middle row are time-aware claims on their own: tighter tile
packing, nothing more. **The 3,463 ticks come from lifting G6**, which was only
ever a proxy for the seat model. The old failure mode it guarded against —
`NoApplicableMethod { goal: "have 2 iron-ore" }`, an expansion that came back
*impossible* rather than slow — does not return: the whole suite is green with
the halved term.

`widen_ore_front` is now a control rather than the headline. Sixty-five seats
instead of nine change the plan by **25 ticks** (12428 against 12403). The wider
ore front was never what unlocked the behaviour; it was the seat model all along.

Two tests changed their claim, and both said the opposite of what is now true:

* `the_whole_unlock_subtree_lands_on_one_bot` was explicitly *"a characterisation
  test, not a regression test: it pins a defect"*. The defect is fixed, so it is
  now `the_unlock_subtree_spreads_on_the_shared_fixture`, asserting the spread
  and pinning 12403.
* `the_unlock_subtree_spreads_when_the_ore_front_can_seat_the_roster` asserted a
  necessity that no longer holds; it is now
  `a_wider_ore_front_barely_moves_the_spread_it_used_to_unlock`, pinning 12428.

**No pre-existing makespan pin moved.** `tests/red_science.rs`,
`tests/scheduling.rs`, `tests/tile_reservation.rs`, `tests/split_capacity.rs`
and the rest pass byte-for-byte unchanged. The two numbers above are new pins on
tests whose *claim* was rewritten, not adjustments to old ones.

## (b) G5 and supplier idleness: the premise is falsified, and I did not change it

The task asked for a G5 that can see whether the suppliers are actually idle,
on the reasoning that `solo / k` is right only when they have nothing else to do.
**Measured on the fixture where convergence lost, the suppliers were as idle as
it is possible to be, and convergence still lost.** Idleness is not the missing
term.

The A/B, on the *same* (wide) fixture, with `worth_converging` forced to refuse
as the only difference:

```
convergence off: makespan 15866
convergence on:  makespan 17117   (+1251)
```

Supplier idle time in the losing plan: **14092 / 14318 / 13726 ticks out of a
15866-tick makespan**. Bots 2, 3 and 4 had nothing to do for 87% of the plan.
An idleness term computed from anything the expansion can see would have read
"idle" and converged anyway.

The schedule says what really happened. The one convergence that fired was
`iron-ore need=20 k=4 solo=2400 handover=1250`, shares 5/5/5/5 — predicted gain
550 ticks, actual loss 1251, a model error of 1801. But the suppliers were never
the constraint:

```
bot 2   1652..2252  mine 5 iron-ore
bot 3   1658..2258  mine 5 iron-ore
bot 4   1835..2435  mine 5 iron-ore
bot 1   4880..4910  place stone-furnace at [-36, 34]      <- the suppliers wait for this
bot 2   4910..4920  insert 5 iron-ore
bot 3   4910..4920  insert 5 iron-ore
bot 4   4910..4920  insert 5 iron-ore
```

Every supplier finished mining by tick 2435 and then sat for 2,500 ticks waiting
for the **taker** to place the furnace. The handover cost nothing on the iron
path — it *accelerated* it: the 20-plate take moved from tick 14686 to tick
10347, **4,339 ticks earlier**.

The plan got longer somewhere else entirely. In the solo plan bot 1 mines its 15
copper ore at 5810..7610, so the copper smelt's lag runs *underneath* the iron
mining and the copper plates are taken at 10885. In the converged plan bot 1 has
no long iron mine left to hide it under, the greedy scheduler picks differently,
copper mining lands at 11055..12855, and the copper take slips to 15937 —
**5,052 ticks later**. Net: −4339 + 5052 ≈ +713 on that leg, +1251 overall.

**So the 1,251 ticks stage 1 measured are a scheduling-order artefact, not the
handover's cost.** Convergence removed the slack bot 1 was using to overlap the
copper smelt's lag, and `schedule`'s greedy "cheapest ready action" rule did not
find the overlap again. Fixing that is a change to the scheduler's priority rule,
not to `worth_converging`, and it would move every makespan pin in the crate. I
did not attempt it and I do not think it is mine to decide.

What I did instead was leave G5 exactly as it is. Its two approximations still
err toward refusing, and with G6 halved the predicate now fires where it pays
(12403 against 15866 on the shared fixture). Adding an idleness term on a premise
the measurement contradicts would have been a confident guess dressed as a fix.

**What the evidence says the next question is**, if someone wants to keep pulling
this thread:

* Run 29's plans (`workspace/runs/run-1788361433-78052/events.jsonl`,
  `plan_created`) show both shapes cleanly. Milestones 1-5: every bot busy, idle
  0-383 ticks. Milestones 6-7: bot 2 carries 11,580-25,560 ticks of work while
  bots 1, 3 and 4 idle for the **entire makespan** — 23,084 to 43,269 ticks,
  with zero steps in the last three plans. That is the shape the design was
  written for, and G5's arithmetic is right about it.
* The fixture is *also* that shape (suppliers 87% idle) and convergence still
  lost there. So the discriminator is not idleness. On the evidence it is
  whether the taker's saved work was on the critical path, and whether the plan
  had an overlap that the saving destroys — both of which are facts about the
  *schedule*, not about the state an expansion can see.

## Determinism

Nothing new depends on iteration order:

* `claimed` is a `BTreeMap` and always was; the value grew a field.
* `is_resource_crowded_for` returns a boolean out of an `.any()` — order-free by
  construction.
* `ClaimRunner` is compared for equality only, never ordered, never hashed.
  `BotId` is the id the caller stated; `ChainId` comes from `ChainIdGen`, a
  monotone generator driven by expansion order, which is fixed by step order.
* `same_runner` is one pure function used by both the crowding predicate and the
  tile walk, so the two cannot start disagreeing about which tiles are free.
* No float comparison was added. `resource_tiles_for` still sorts by
  `total_cmp`; the relaxation removes a filter, it does not reorder anything.
* Emission order is untouched, so `ActionId` allocation and hence `schedule`'s
  `(end, ActionId, BotId)` tie-break are untouched.

Asserted, not argued: `the_unlock_path_plans_identically_twice` runs the
**narrow** fixture, which now converges, so each run binds the claim runner a few
hundred times, stamps every claim with it, and picks tiles against a crowding
rule that reads it. It compares labels, every `(bot, start, end, step)` and the
makespan across two runs of identical inputs. `tests/red_science.rs::expansion_is_deterministic`
covers the un-researched path as before.

## Red-first

The new API was added inert first — `same_runner` stubbed to `false`, the tile
walk's spacing forced on — which is exactly the pre-change behaviour, so the
tests failed on their assertions rather than on a missing symbol:

```
---- state::tests::a_runner_is_not_crowded_by_its_own_claim stdout ----
panicked at crates/planner/src/state.rs:2307:9:
a bot cannot stand on its own next tile while mining this one

---- state::tests::an_unowned_chain_is_one_timeline_and_only_its_own stdout ----
panicked at crates/planner/src/state.rs:2346:9:
one chain is one runner, whoever it turns out to be

---- method::util::tests::one_calls_own_tiles_are_spaced_only_when_the_runner_is_unknown stdout ----
panicked at crates/planner/src/method/util.rs:1081:9:
one bot's two swings are serial, so it takes the nearer tile:
[(Position { x: -34.5, y: 35.5 }, 500), (Position { x: -38.5, y: 35.5 }, 1)]

---- method::util::tests::one_runner_may_work_the_whole_patch_the_roster_can_only_seat_nine_of stdout ----
panicked at crates/planner/src/method/util.rs:1021:9:
assertion `left == right` failed: one runner works its own patch tile by tile; seats are for other bots
  left: 9
 right: 121

test result: FAILED. 0 passed; 4 failed
```

`left: 9 / right: 121` is the ceiling, in one line.

One pre-existing integration test went red on the real change and had to be
rewritten rather than fixed: `tile_occupancy.rs::a_multi_item_plan_never_seats_a_bot_on_another_bots_tile`
compared **every** pair of mined tiles, which is the runner-blind invariant. It
now reads bots off the *schedule* and asserts the cross-bot half, plus a
`packed_same_bot_pairs > 0` guard so the cross-bot half cannot pass because
nothing got close. Both halves matter: the failure was a same-bot pair, and the
guard is what proves it.

## Mutation results

Each mutation applied alone, whole planner suite run.

| mutation | tests that failed |
| --- | --- |
| `same_runner` always `false` (all of (a), reverted) | `a_runner_is_not_crowded_by_its_own_claim`, `an_unowned_chain_is_one_timeline_and_only_its_own`, `one_runner_may_work_the_whole_patch…`, `the_unlock_subtree_spreads_on_the_shared_fixture`, `a_wider_ore_front_barely_moves…`, `the_unlock_path_plans_identically_twice`, `a_multi_item_plan_never_seats_a_bot_on_another_bots_tile` |
| `same_runner` treats two unknowns as one timeline | `an_unknown_runner_matches_nothing_including_another_unknown`, `a_tile_next_to_a_claim_is_crowded_without_being_claimed`, `crowding_is_measured_from_tile_centres_not_from_floored_keys`, `seats_are_counted_blind_to_whose_claims_they_are`, `a_patch_too_small_for_the_roster_is_narrowed_not_overcommitted` |
| `resource_seats` asks through the current runner | `seats_are_counted_blind_to_whose_claims_they_are` |
| the tile walk always spaces its own picks | `one_calls_own_tiles_are_spaced_only_when_the_runner_is_unknown` |
| an unowned chain names no timeline (`Chain` variant dropped) | `an_unowned_chain_binds_a_timeline_the_tile_walk_can_see` |
| G6 keeps the doubled slack term | `the_unlock_subtree_spreads_on_the_shared_fixture`, `a_wider_ore_front_barely_moves_the_spread_it_used_to_unlock` |
| `expand_goal` does not restore the claim runner | **none** |
| `Step::Owned` does not restore the claim runner | **none** |

The `same_runner`-always-`false` row killing `the_unlock_path_plans_identically_twice`
is not a determinism failure: with G6 halved, that fixture converges, and without
time-aware claims the 121-tile patch runs out and the expansion comes back
`NoApplicableMethod`. It is the stage-1 catastrophe reproduced on demand.

**The last two rows are gaps, and they are gaps by construction rather than by
oversight.** Both restores are inert today, for a reason that can be stated
exactly:

* Chain opening is guarded by `ctx.chain.is_none()`, so **nothing nested inside a
  chain rebinds the runner**. The only step that does is `Step::Owned`.
* In `SharedSmelt`, the only method that emits `Step::Owned`, the supplier blocks
  are followed by exactly one step — `Act(remove)` — which claims no tile. So a
  leaked binding has nothing left to stamp.
* Outside any chain, the only goals that can follow a chain-opening sibling
  either open a chain of their own (overwriting the binding) or emit only
  subgoals (`SplitAcrossBots`), never a mining action of their own.

So no test can go red on them. What they *do* pin is the same save-and-restore
discipline `ctx.chain`, `ctx.top_level`, `ctx.converging` and `ctx.chain_actor`
already have, and they exist so that the first method that mines *after* a
handover cannot silently book its tiles to the supplier. That failure would be
invisible: the plan still validates, still schedules, and puts two bots on
neighbouring ore.

What is *not* a gap is the property they would break.
`a_converged_plan_never_seats_two_bots_on_adjacent_tiles` runs the converging
unlock plan, reads bots off the schedule, and asserts the game's own condition
(`character_stands_on_tile`, which is `another character is standing on the
<ore>` stated as geometry) for every cross-bot pair — with a same-bot packing
guard so it cannot pass vacuously.

## Test counts

**339 lib tests and all 69 integration tests pass**, against 328 + 69 at
`86c2d28e`. Eleven added — six in `state.rs`, three in `method/util.rs`, two in
`method/have.rs` — and two renamed in place, none removed. `cargo clippy -p factorio-bot-planner --all-targets -- -D warnings` is
clean; `cargo fmt -p factorio-bot-planner` applied.

**Not run, deliberately:** the workspace build, `just test`, and anything
touching Factorio. A live four-bot run was executing throughout.
