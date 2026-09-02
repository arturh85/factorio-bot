# Convergence, stage 1: the furnace is the buffer

Implements stages 0 and 1 of
`docs/superpowers/specs/2026-09-02-material-convergence-design.md`. Stage 2
(the `iron-chest`) and stage 3 (buffer visibility) are **not** here. Stage 3 is
a hard gate before this runs unattended — see "What this does not close".

Everything is inside `crates/planner`. `crates/executor`, `crates/core`, the
mod and the frontend are untouched, exactly as §11 of the spec predicted.

## What was wrong

`SplitAcrossBots` splits a top-level goal perfectly and then **any convergence
inside a share lands on that share's owner**. Milestone 5 (`craft iron gear
wheels x20`) planned 9/1/1/1 with no research anywhere in it: bot 1's share came
up three plates short, the whole smelt that covers those three plates welded
onto bot 1, and three bots idled 3,200 ticks. Milestone 6 is the same shape at
33/1/1/1 across 26,451 ticks, and there the bill is a lab's ~50 iron plates.

The freedom to do better already existed in the *action*: `Smelt`'s `Remove`
carries no `HasItem`, so a furnace really is a buffer whose taker need not be
its loader. It was thrown away one line later by the chain stamp — `run_steps`
stamps every action a method emits with `ctx.chain`, and there was no way for a
method to emit an action addressed to anyone else.

## Stage 0 — the primitive, alone

```rust
Step::Owned { whose: Holder, steps: Vec<Step> }
```

The `run_steps` arm opens a fresh chain, sets `net.set_chain_owner(chain, bot)`,
rebinds `chain_actor`, runs the inner steps, releases their reservations, books
the chain's produce, and restores everything on every exit path including the
error one.

Four supporting changes, each with a reason:

* **`reserve_chain_produce` extracted from `expand_goal_body`.** The produce
  ledger fired only when `chain_on_entry.is_none()`, which a nested chain never
  is. Without the call in the new arm, a supplier chain's output would read as
  spare stock to the *taker's* shortfall arithmetic — the exact defect the
  ledger was added for, coming back through the new door.
* **`GoalSite::converging` + `ExpansionCtx::converging`.** The termination
  argument. A supplier's share is an ordinary `Have` goal; without the flag it
  would converge in its turn, forever. Carried *into* `Step::Owned` rather than
  cleared there, because a supplier's own production must not converge either.
* **`Method::split_probe`.** A converging method needs a concurrency answer at a
  site the scatter-site rule does not cover, and about a goal that is not the
  one it was asked about — `SharedSmelt` is asked for iron *plate* and splits
  iron *ore*. Returning a goal rather than a number keeps the method from
  learning what a seat is. Answering also tells the driver this is a convergence
  point, so the subtree is marked `converging`; the two are one answer because
  they are one decision.
* **`PlannerError::UnownedHandover`.** `Holder::Anyone` in a `Step::Owned` is
  refused rather than treated as "keep the current chain", which would silently
  weld the supplier's work back onto the consumer.

**Inertness is a fact, not an argument.** Every nested stated holder this crate
emitted before stage 1 named `ctx.chain_actor`, so no plan could move. Numbers
are in "Evidence" below.

## Stage 1 — `SharedSmelt`

`Smelt::expand`'s body became `smelt_steps(goal, ctx, shared: Option<SharedOre>)`.
`shared: None` is the old expansion, textually unchanged. `Some` replaces the one
ore insert with one per participating bot:

```
Subgoal(Have coal)            # taker
Subgoal(Have stone-furnace)   # taker
Act(place)                    # taker
Act(fuel)                     # taker
for (b, work_b) in shares:    # ascending BotId
    b == taker  ->  Subgoal(Have{ore, spare+work, Share(b)}); Act(insert)   # inline
    otherwise   ->  Owned{Share(b), [same two steps]}
Link(place -> each ore insert, 0)
Act(remove)                   # taker
Link(each insert -> remove, smelt_lag); Link(fuel -> remove, 0)
```

* `place -> insert` would also be inferred (`EntityAt` is world-scoped, so
  `infer_edges` keeps it across chains) but is stated because the method holds
  both ids and a plan should not depend on inference where a statement is free.
* `insert -> remove` **cannot** be inferred: the take has no condition any
  insert's effect satisfies, and a `HasItem { Role }` would be dropped by the
  cross-chain exclusion. That link is the whole handover, and it carries the
  smelt lag — which is why the primitive is `Step::Owned` and not a subgoal.
* The furnace, its stone and its coal stay with the taker. The furnace must
  exist before anyone can insert into it, so a supplier-side furnace buys an
  extra cross-chain edge on the critical path for ~5 stone and 1 coal. A real
  residual, deliberately left: stage 1 changes one thing.
* `converges` stays `false`. That predicate asks whether several *produced*
  items must meet in one inventory so the driver can weld the producers to the
  consumer. This does the opposite of welding. `smelting_never_converges` is
  still the honest answer for a furnace.

### What gets split, and how much

The split is over the ore that still has to be **produced**, not over the
furnace's whole bill: `need = total - available(Share(taker), ore)`. A taker
already carrying the ore does not send the roster out to mine it again, and
`sum(shares) + held` is still the furnace's whole count.

`even_shares` is lifted verbatim out of `SplitAcrossBots::expand` and both now
call it, so there is one rule for who participates and how much each is asked
for. It returns the *work* per bot; a caller wanting a `Have` target adds the
bot's spare back on, which is what `SplitAcrossBots` always did.

### The predicate

`worth_converging` refuses by default. Converging where splitting would have won
is a regression; being slow is not.

* **G1** two distinct known bots, one of them not the taker.
* **G2** `!site.top_level && site.in_chain`, enforced by `claims`. A top-level
  goal stays `SplitAcrossBots`'. Nothing that splits today converges tomorrow.
* **G3** `!site.converging`. Termination.
* **G4** `need >= 2` and `k >= 2`, `k` bounded by seats via `split_probe` →
  `MethodRegistry::concurrency` → `Mine::concurrency`.
* **G5** `solo / k + handover(k) < solo`, integers throughout, where
  `handover(k) = k * (TRANSFER_TICKS + HANDOVER_WALK_TICKS) + TRANSFER_TICKS`.
* **G6** `seats >= k + 2 * roster`. Not in the design; see correction 5.

`solo` is shallow (one level, no recursion into a recipe's own ingredients), so
it under-states the work being spread and the predicate under-fires. `handover`
charges a full transfer per supplier *and* the take, where a solo smelt already
pays one of each — the difference is charged to convergence rather than netted
off. Both approximations err toward not converging.

`HANDOVER_WALK_TICKS = 300` is the design's one tuning constant and is a guess
(~45 tiles). Bot positions do not advance during expansion, so a computed
distance here would be confidently wrong rather than admittedly rough. It is
charged **per supplier**; see correction 4.

## Determinism

The planner's rule is: no I/O, no async, no wall clock, ordered collections
only, floats by `total_cmp`, identical inputs → byte-identical plans. Each new
piece against it:

* **Supplier selection and share sizing** — `even_shares`, one function for both
  callers. Candidates are `(available(Share(b), item), BotId)` sorted
  `sort_unstable` on a key that is a **total order** because `BotId` is unique
  within the deduped roster. Emission is ascending `BotId` out of a `BTreeMap`,
  because emission order fixes `ActionId` allocation and therefore `schedule`'s
  `(end, ActionId, BotId)` tie-break.
* **Which bot is the taker** — read off the goal's own `Holder`, never off
  iteration order. `Holder::Anyone` is refused rather than guessed at.
* **The furnace's position** — `free_area_near`'s fixed integer spiral,
  unchanged, already pinned by `the_same_world_sites_the_same_furnace_twice`.
* **The predicate** — integer `Ticks` arithmetic with no float comparison at
  all, so the answer cannot depend on a rounding mode.
* **`split_probe` / `MethodRegistry::concurrency`** — iterate `self.methods` in
  registration order and take a `min` over `Option<u32>`; order-independent.
* **Chain identity** — `ctx.chains` is a monotone generator driven by expansion
  order, which is fixed by step order.

Asserted, not argued: `a_converged_smelt_plans_identically_twice` compares
labels, per-action chain owners, every `(bot, start, end, step)` and the
makespan across two runs of the same inputs;
`the_unlock_path_plans_identically_twice` does the same for the research path,
which now goes through the converged expansion.

## What this does not close

**Stage 3 is a hard gate before this runs unattended, and it is not here.**
`PlanState` models no container contents; `FactorioWorld::on_some_entity_updated`
is a `// TODO` no-op; the only path that can read a furnace's contents is an
on-demand RCON query no planning path calls. So items left in a buffer are
invisible to a replan, which re-mines them with the ore already gone. That
exposure exists **today** for the furnace and is merely narrow — insert and take
sit in one chain, adjacent in time, on one bot. Cross-bot convergence widens the
window from seconds to "however long the taker takes to walk over".

Stage 1 can stand without stage 3 only in the sense the spec allows: until
stages 1 and 2 land, the window they widen is the one the furnace path already
has. It cannot stand for a long unattended run.

Two executor-adjacent risks, neither a code change, both worth watching on the
first live run:

* `recover.rs`'s tiers were written for chains with one owner. A plan where a
  chain's inputs are produced by a different chain has never been executed.
* A retried `Insert` into a *shared* buffer is now visible to a different bot's
  `Remove` count. The take is capped at `count`, so the plan is fine, but the
  surplus is stranded — which is what stage 3 recovers.

## Corrections to the spec

The first three were found by reading; the last three by running.

1. **`worth_converging`'s signature cannot serve `applicable`.** `Method::applicable`
   receives only `(&Goal, &PlanState)` — no `ExpansionCtx`, so no
   `ctx.concurrency`. `SharedSmelt::applicable` therefore asks with
   `seats = u32::MAX` and `expand` asks again with the real number, falling back
   to `smelt_steps(.., None)` — byte-identical to `Smelt`'s own expansion — when
   the world's seats narrow the split below two. The spec's "one function, so
   `applicable` and `expand` cannot answer differently" survives at the level
   that matters (the *plan* is the same either way); the claim as literally
   written does not.

2. **The spec's §5 shape splits the furnace's whole ore bill.** Doing that
   sends the roster mining ore the taker is already carrying: a taker holding 50
   iron ore would still have three bots mine 12 each. Implemented over the ore
   *shortfall* instead, with the taker loading its own stock on top of its share.
   `a_taker_holding_the_ore_already_does_not_send_the_roster_mining` pins it.

3. **G2 does not stop sibling shares each converging.** "Nothing that splits
   today converges tomorrow" is true of the *top-level* goal and false of what
   happens inside each resulting share. On a symmetric roster and a symmetric
   goal, `SplitAcrossBots` makes four equal shares and each then decides its own
   smelt is worth converging.

4. **§7's cost model contradicts §7's own worked verdict.** The design charges
   the walk flat — once per handover, however many suppliers — and then works
   milestone 5 at `k = 2`. That run had four bots, and a three-ore shortfall
   seats `k = 3`: the flat formula gives `576 / 3 + (3·10 + 10 + 300) = 532 <
   576` and **converges**, which §7 says is wrong. The walk is now charged per
   supplier, which refuses three ore at every `k` and at both the fixture's
   mining rate and the game's, and still converges the fifty-ore bill by a
   factor of two and a half. Pinned by
   `a_share_sized_shortfall_does_not_converge_because_every_supplier_walks`.

5. **Convergence spends mining seats, which are plan-global and never
   released — and correction 3 is therefore fatal, not merely wasteful.**
   A solo smelt claims one mining seat; a converged one claims `k`, and
   `PlanState::claimed` commits a tile and its neighbours for the whole
   expansion. Measured: `fixture_world`'s iron patch is 121 tiles, which at a
   hand-mining separation of 3.99 seats **nine** miners, and the un-converged
   four-bot unlock plan already uses **eight** of them. Firing on every
   four-ore share exhausted the front, and the symptom was not a slow plan but
   `NoApplicableMethod { goal: "have 2 iron-ore" }` — a goal the planner had
   always been able to satisfy. Nine pre-existing tests across `red_science`,
   `smelt_roots`, `tile_occupancy` and `tile_reservation` went red.

   Two things fix it: the per-supplier walk (correction 4), which puts the
   break-even at about fourteen ore, well above the size of an ordinary share;
   and **G6**, `seats >= k + 2 * roster`, which refuses to spend seats the plan
   cannot spare. G6's factor of two is measured, not tuned: the unlock plan
   wants eight iron seats on a four-bot roster. It is a proxy, and the real fix
   is a seat model that releases a claim when its action ends rather than at the
   end of the expansion — that is a `PlanState` change and not stage 1's.

6. **The design's headline case cannot be demonstrated on the shared fixture,
   and stage 1 does not yet pay for itself on the one where it can.** With a
   nine-seat front and eight seats already wanted, `unlock_state` has room for
   no convergence at all; G6 correctly declines and the plan comes out exactly
   as before (49/12/12/12, makespan 15922). `widen_ore_front` adds a block of
   iron clear of the existing patches — what a real ore field looks like — and
   on it the handover really happens: the unlock subtree goes from
   `{bot 1: 48}` to `{bot 1: 48, bot 2: 2, bot 3: 2, bot 4: 2}`. But the plan
   goes **49/12/12/12 at 15922 to 49/14/14/14 at 17122**. Convergence costs
   1,200 ticks there and saves none.

   Two measured reasons, both of which matter for stage 2:

   * **The bill is not one smelt, it is many.** `HandCraft` decomposes the lab
     into gears, circuits and belts, and each asks for its own plates, so the
     largest single iron-plate goal is about twenty — not the fifty the design
     costed. A handover's overhead is paid per smelt.
   * **G5 assumes the suppliers are idle.** It compares `solo` against
     `solo / k`, which is right only when the other bots have nothing else to
     do. In the fixture they have their own pack shares, so a supplier's detour
     delays its own chain *and* the taker's take waits for whichever supplier
     arrives last. In the run this design was written for, three bots idled
     13,000 ticks — that is the case where the assumption holds, and no fixture
     in this crate is it.

7. **The fixture's mining rate is not the game's.** §7's worked table uses ~192
   ticks per iron ore; `fixture_world` gives 120 (`mining_time` 1.0 s ÷
   `character` `mining_speed` 0.5 = 2 s). The decisions in that table survive
   the difference under the corrected cost model.

## Evidence

**Stage-0 inertness.** With `Step::Owned`, `reserve_chain_produce`,
`GoalSite::converging` and `Method::split_probe` all in place and `SharedSmelt`
unregistered: **323 lib tests and all 69 integration tests pass**, every
makespan pin included. Only the three new stage-1 tests are red, which is what
red-first looks like. The unlock fixture measures 49/12/12/12 at makespan 15922,
with all 48 unlock actions on bot 1 — the design's documented before column,
reproduced.

**Mutation results.** Each mutation was applied alone and the whole suite run.

| mutation | tests that failed |
| --- | --- |
| the `reserve_chain_produce` call in the `Owned` arm | `a_supplier_chains_output_is_not_spare_stock_for_the_taker` |
| the `set_chain_owner` line in the `Owned` arm | `an_owned_block_puts_its_actions_in_a_chain_owned_by_the_named_bot` + both convergence tests |
| the `Holder::Anyone` refusal | `a_handover_that_names_nobody_is_refused_where_it_was_written` |
| the `converging` guard | `a_converging_site_refuses_to_converge_again` |
| shortfall-vs-total split | `a_taker_holding_the_ore_already_does_not_send_the_roster_mining` |
| G6, the seat-cost gate | `the_whole_unlock_subtree_lands_on_one_bot`, `the_unlock_path_plans_identically_twice` |
| the per-supplier walk | `a_share_sized_shortfall_does_not_converge_because_every_supplier_walks` |
| the `place -> insert` links | **none** |

The last row is a gap and is left as one. `Condition::EntityAt` is world-scoped,
so `infer_edges` reproduces every edge that loop states; there is no observable
difference for a test to assert on. The loop stays for whoever reads the plan
and for the day a condition stops being world-scoped, and the code says so in
place rather than implying a coverage that does not exist.

The per-supplier-walk row was **also** empty on the first pass: both cost models
agreed on every case the suite happened to contain, so a revert would have gone
unnoticed. `a_share_sized_shortfall_does_not_converge_because_every_supplier_walks`
was written for exactly that hole, at the one count where the two models
disagree at this fixture's mining rate, and with `seats` passed high enough that
G6 cannot be what refuses.
