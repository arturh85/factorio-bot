# Material convergence: several bots produce, one bot consumes

**Status: design only. Nothing implemented, no cargo run this pass** (a live
run and a frontend agent hold the CPU; an earlier run connected zero of four
graphical clients because an agent ran `cargo test --workspace` alongside it —
`docs/superpowers/notes/2026-09-02-runs-contend-with-agents.md`). Everything
below is read off source, off the run records in `workspace/runs/`, and off the
game data in `workspace/data/`. Section 12 says exactly which claims a run has
to settle.

## 1. Recommendation in three sentences

Add **one** new expansion primitive — `Step::Owned { whose, steps }`, which
lets a method emit actions into a *nested chain owned by another bot* — and
then express convergence twice on top of it: **stage 1 uses the furnace the
smelt already places** as the handover buffer (no new item, no new entity, no
mod change, and it covers 100% of the bill that the measured failure actually
needs), **stage 2 adds an `iron-chest` buffer** for hand-crafted items a furnace
cannot carry. The chest is the right *general* answer and the wrong *first*
answer: `wooden-chest` is not merely cheap, it is **unplannable in this planner**
(its only ingredient is wood, and wood is not a resource patch, has no recipe,
and therefore has no applicable method), so the chest really costs
`iron-chest` = 8 iron plates, which is a cost the furnace path does not pay at
all. `crates/executor` needs **no change** for either stage; what does need
changing outside the planner is that container contents are today invisible to
every replan, which is a blocking prerequisite for leaving items in a buffer
(§8, stage 3).

## 2. The RCA, checked

### 2.1 Symptom 1 — work concentrates. **Confirmed, mechanism confirmed, and reproduced on fresh data.**

The stated chain is right in every link, and I re-checked each against source:

1. `Goal::Have { pack, 10, Anyone }` is claimed by `SplitAcrossBots`
   (`have.rs:1116`), which emits one `Holder::Share(b)` subgoal per bot, **in
   ascending `BotId`** — the emission order is a `BTreeMap` drain and is
   deliberately ascending so a symmetric roster's plan does not move
   (`have.rs:1276-1290`).
2. The first share to expand meets `RecipeGate::NeedsResearch` in
   `HandCraft::expand` and emits `Goal::Researched(...)` *inside that share's
   chain* (`have.rs:816-828`).
3. `SplitAcrossBots::claims` is `site.top_level && !site.in_chain`
   (`have.rs:1137-1139`), so nothing under that chain can be scattered again; and
   `expand_goal_body` opens a chain only `if ctx.chain.is_none()`
   (`method/mod.rs:491`), so nothing under it can open a chain of its own.
   Every action beneath is stamped with share 1's chain (`run_steps`,
   `method/mod.rs:636-640`) and `net.set_chain_owner` has bound that chain to
   bot 1.

Fresh evidence, from the run that was in flight while this was written
(`workspace/runs/run-1788358260-07659/events.jsonl`, four bots, read with a
`Counter` over each `plan_created`'s `plan[].bot`):

| milestone | steps | makespan | distribution |
| --- | --- | --- | --- |
| 3 smelt iron plates x50 | 24 | 1966 | 6 / 6 / 6 / 6 |
| 4 smelt copper plates x20 | 29 | 3224 | 8 / 7 / 7 / 7 |
| 5 craft iron gear wheels x20 | 12 | 3358 | **9 / 1 / 1 / 1** |
| 6 craft automation science packs x10 | 36 | 26451 | **33 / 1 / 1 / 1** |
| 6 (replan) | 35 | 24944 | **32 / 1 / 1 / 1** |

So the brief's "30 of 33" is if anything understated on the current build, and
the same shape reproduces on a three-bot roster
(`run-1788351494-76427`: milestone 6 is 32/1/1 then 40/1/1).

**Milestone 5 is the finding the brief does not have, and it matters for the
design.** It has nothing to do with research. The plan is:

```
9  bot 2  craft 5 iron-gear-wheel   deps []      start 0
10 bot 3  craft 5 iron-gear-wheel   deps []      start 0
11 bot 4  craft 5 iron-gear-wheel   deps []      start 0
5  bot 1  mine 3 iron-ore           deps []      start 195
6  bot 1  mine 1 coal               ...          start 942
8  bot 1  mine 5 stone              ...          start 1254
7  bot 1  craft 1 stone-furnace     deps [8]     start 1854
1  bot 1  place stone-furnace       deps [7]     start 2390
2  bot 1  insert 3 iron-ore         deps [1,5]   start 2420
3  bot 1  fuel the furnace          deps [1,6]   start 2430
4  bot 1  take 3 iron-plate         deps [1,2,3] start 3198
0  bot 1  craft 5 iron-gear-wheel   deps [4]     start 3208
```

`SplitAcrossBots` did its job perfectly — four equal shares of five gears. Bot
1's share happened to be three plates short, and **the entire smelt that covers
those three plates is welded to bot 1** because it expands inside bot 1's share
chain. Three bots finish at tick ~150 and idle for 3,200 ticks while one bot
mines stone for a furnace. The defect is therefore *not* "the research subtree
is special". It is: **any convergence inside a share is a convergence onto that
share's owner**, and research is merely the largest instance. A design aimed
only at `Researched` would leave milestone 5 exactly as it is.

### 2.2 Symptom 2 — idle bots become obstacles. **Real, but the causal arrow in the brief is half wrong, and the named defect is already fixed elsewhere.**

`docs/superpowers/notes/2026-09-02-parked-bots-block-placements.md` is
unambiguous: run 27's blocker was a **stale position**, not a parked bot as
such. `on_player_changed_position` fires per *tile* crossed, so a bot that
stopped 0.825 tiles into a tile never reported its resting position, and
`FactorioWorld` believed bot 3 was 0.76 tiles from where it stood — the
difference between a site that clears the furnace footprint and one that does
not. That note's own verdict: *"the planner was not wrong, it was misinformed"*
and *"the cheapest true statement about run 27 is that no planner change was
needed."* Both halves were fixed mod-side (report where a character comes to
rest; ask a blocker to step aside).

What is still true, and is this spec's business: that note's last line records
*"Milestone 6 also had every step on one bot… it is why the blockers had nothing
to do in the first place."* Convergence removes the **conditions** (three bots
with 13,000 ticks of nothing to do, parked wherever their last action left
them). It does not remove the **mechanism**, and it must not be sold as a fix
for placement geometry: the note explicitly rejected a stand-point model in the
planner and that rejection stands. Expect this design to make placement
refusals rarer, not impossible, and do not add planner geometry here.

### 2.3 Symptom 3 — "shares are sized for a roster that does not execute them". **Refuted as stated.**

The quoted message does not exist in any run record. Searching every
`workspace/runs/*/events.jsonl` for `could not have player client<N> craft <n>
<item> (but only <m>)` returns exactly one distinct string, once:

```
could not have player client2 craft 3 automation-science-pack (but only 0)
```

in `run-1788338409-63794`, whose `run_started.bots` is `[1, 2, 3, 4]` — a
**four**-bot roster, not three. There is no `client3`, no `craft 4`, and no
three-bot occurrence: the one three-bot run in the tree
(`run-1788351494-76427`) records no such error at all and finished milestone 6's
pack crafts as a clean 3-step 1/1/1 plan.

And that message's cause is already diagnosed and fixed.
`docs/superpowers/notes/2026-09-02-craft-ingredients.md` proves from
`samples.jsonl` that **every bot held both ingredients** at the moment the game
said zero; the failure was *ordering*, not sizing — the recipe was locked, three
sibling shares carried no `Condition::Researched`, and `schedule` put them at
`planned_start: 0` before the lab existed. Commit `21a1228a` added
`RecipeGate::PlannedResearch` for exactly this.

There *was* a real share-sizing-versus-roster defect, but it is a different one:
the rung-3/4 finding where the scheduler bound a share's chain to whichever bot
was cheapest rather than the one its bill was sized against. That was fixed on
2026-09-02 by giving a `Holder::Share` chain an owner, at a measured cost of
22,072 ticks of lost concurrency — which is a large part of why symptom 1 is as
bad as it is. **This spec must not undo that**; see §9.

### 2.4 The underlying claim — "no bot can hand an item to another". **Confirmed, and it is a fact about the game, not a gap in this code.**

Factorio has no player-to-player item transfer. The only legitimate handover is
through an entity both characters can reach. The planner's vocabulary for that
already exists: `ActionKind::Insert`/`Remove` carry an entity name, a position
and an `InventorySlot` (`action.rs`), and `InventorySlot::Chest` is already
defined and already maps to the 2.1 defines key `"chest"`.

### 2.5 Correction to the brief's framing of the precedent

The brief says *"`Smelt`'s `Remove` carries no `HasItem`, so a furnace already
acts as a buffer where the taker need not be the inserter"*. The first half is
true (`have.rs`, the `remove_id` action's `pre` is `AtPosition` + `EntityAt` +
the research gate — no `HasItem`). The second half is true of the *action* and
false of the *plan*: the remove is emitted inside the same method, in the same
`ctx.chain`, so `run_steps` stamps it with the same chain and the scheduler
binds it to the same runner. The freedom exists in the action's preconditions
and is thrown away one line later by the chain stamp. **That is precisely the
thing this design has to change**, and it is why the change is a driver change
and not a method change.

## 3. What the planner can and cannot express today

Facts, each checked in source:

* A method emits a flat `Vec<Step>` (`Subgoal | Act | Link`). The only thing
  that rebinds which bot a subtree is sized and run against is
  `Step::Subgoal(Goal { whose: Bot(b) | Share(b) })`, handled in `expand_goal`
  (`method/mod.rs:363-377`) and `expand_goal_body` (`:491-504`).
* Actions a method emits **itself** are stamped with `ctx.chain`
  (`run_steps`, `:636-640`) and cannot be addressed to anyone else.
* A nested chain is impossible: `expand_goal_body` opens one only when
  `ctx.chain.is_none()`.
* Every nested `Holder::Share(b)` in the crate today names `ctx.chain_actor`
  (`Researched` emits `Share(ctx.chain_actor)`; `Smelt`/`HandCraft` propagate
  `whose` verbatim; `SplitAcrossBots` only fires at a top-level, un-chained
  site). **So a rule keyed on "the stated bot differs from the enclosing chain
  actor" is provably inert on every plan this crate produces today.** That is
  the wedge this design uses, and it is testable as an assertion over the
  existing fixtures rather than as an argument.
* Cross-chain ordering: `infer_edges` (`network.rs:181-230`) drops an inferred
  edge across two different chains **only** when the matched condition is
  `HasItem { who: Actor::Role }`. Everything world-scoped — `EntityAt`,
  `AreaFree`, `Researched` — still links across chains. An explicit
  `Step::Link` is never dropped.
* `PlanState` models **no container contents at all**. `BotState.inventory`
  comes from `FactorioPlayer.main_inventory` and nothing else
  (`state.rs:438-455`). See §8.

## 4. The primitive: `Step::Owned`

```rust
// crates/planner/src/method/mod.rs
pub enum Step {
    Subgoal(Goal),
    Act(Box<Action>),
    Link { from: ActionId, to: ActionId, lag: Ticks },
    /// A block of steps that belong to **another bot**.
    ///
    /// The driver opens a fresh chain owned by `whose`'s bot, rebinds
    /// `chain_actor` to it, runs `steps` inside it — so every `Step::Act` in
    /// there is stamped with that chain and every `Step::Subgoal` is sized
    /// against that bot's inventory — and then restores everything.
    ///
    /// This is the only way a method can emit an action it does not intend to
    /// run itself, and it exists because material convergence is exactly that:
    /// the insert belongs to the supplier and the take belongs to the
    /// consumer, and both are emitted by one method because only that method
    /// knows the `ActionId`s to link.
    Owned { whose: Holder, steps: Vec<Step> },
}
```

Driver, in `run_steps`:

```rust
Step::Owned { whose, steps: inner } => {
    let (Holder::Bot(bot) | Holder::Share(bot)) = &whose else {
        return Err(PlannerError::UnownedHandover { holder: whose.to_string() });
    };
    if ctx.state.bot(*bot).is_none() {
        return Err(PlannerError::UnknownBot(*bot));
    }
    let previous_actor = ctx.chain_actor;
    let previous_chain = ctx.chain;
    let previous_top_level = ctx.top_level;
    let chain = ctx.chains.next();
    net.set_chain_owner(chain, *bot);
    ctx.chain_actor = *bot;
    ctx.chain = Some(chain);
    ctx.top_level = false;
    let produce_before = (ctx.state.bot_ids().len() > 1).then(|| ctx.state.item_totals());
    let mut inner_promised = Vec::new();
    let result = run_steps(inner, ctx, net, registry, &mut inner_promised);
    for (whose, item, count) in &inner_promised {
        ctx.state.release(whose, item, *count);
    }
    reserve_chain_produce(ctx, produce_before);   // extracted from expand_goal_body
    ctx.chain_actor = previous_actor;
    ctx.chain = previous_chain;
    ctx.top_level = previous_top_level;
    result?;
}
```

Four things to note, each with a reason:

* **Save/run/restore on every exit path, errors included**, exactly as
  `expand_goal` does (`:354-388`) and for the same reason: a caller that
  continues past an error must not inherit a corrupted context.
* **The chain always gets an owner.** A `Step::Owned` names a bot; that is the
  whole point, and it puts it on the same footing as `Holder::Bot` and
  `Holder::Share` in the scheduler (`schedule.rs:314` — an owner is a hard
  constraint with no fallback tier).
* **`reserve_chain_produce` must be factored out of `expand_goal_body`
  (`:538-572`) and called here too.** Today the produce ledger fires only when
  `chain_on_entry.is_none()`, which a nested chain is not. Without the call, a
  supplier chain's output would look like spare stock to the *taker's*
  subsequent shortfall arithmetic — the exact defect the ledger was added for
  ("what a chain made belongs to that chain"), reintroduced through the new
  door.
* **`Holder::Anyone` is refused, not silently treated as "keep the current
  chain".** A handover with no named supplier is a method bug, and it should
  fail where it is written.

`Method` gains one optional hook, so that a method that will scatter something
can get a concurrency answer about the goal it will *actually* split:

```rust
/// The goal whose concurrency limit this method needs, if it is going to hand
/// one goal to several bots.
///
/// `None` — the default — for every method that scatters nothing. A converging
/// method returns the goal it will split, which is not always the goal it was
/// asked about: `SharedSmelt` is asked for iron *plate* and splits iron *ore*,
/// and the seats that bound the split are the ore patch's. Returning the goal
/// keeps the method from learning what a seat is (`Method::concurrency`'s
/// whole point) while still getting the number.
fn split_probe(&self, _goal: &Goal, _state: &PlanState) -> Option<Goal> { None }
```

and `expand_goal_body`'s concurrency computation becomes

```rust
ctx.concurrency = if site.top_level && !site.in_chain {
    registry.concurrency(goal, &ctx.state, cap)
} else if let Some(probe) = method.split_probe(goal, &ctx.state) {
    registry.concurrency(&probe, &ctx.state, cap)
} else {
    None
};
```

The existing comment's objection — "asking anywhere else would buy a walk of an
ore field per subgoal for a number nobody reads" — is respected: the second arm
fires only for a method that has already decided it is going to read the answer.

`GoalSite` gains one field:

```rust
pub struct GoalSite {
    pub top_level: bool,
    pub in_chain: bool,
    /// True anywhere beneath a converging method's own expansion.
    ///
    /// The termination argument. A handover's supplier shares, and the buffer's
    /// own bill, are ordinary `Have` goals; without this a handover would
    /// converge its own inputs, and an `iron-chest` (8 iron plates) would want
    /// a chest to deliver its plates. One level of convergence per convergence
    /// point, deliberately.
    pub converging: bool,
}
```

set by the driver for the whole subtree of a method that returned
`split_probe(..).is_some()`, saved and restored by `expand_goal` like every
other driver-owned field, and **carried into `Step::Owned`** (a supplier's own
production must not itself converge).

## 5. Stage 1 — the furnace is the buffer

**No new item, no new entity, no new action kind, no mod change, and it covers
the whole of the measured case.** The lab that milestone 6 serialises on costs
10 gears + 10 circuits + 4 belts; unrolled that is ~50 iron plates and ~16
copper plates, **and every one of them is smelted**. A chest would carry
nothing here that a furnace does not already carry.

```rust
/// Smelt the shortfall, with the ore supplied by the rest of the roster.
///
/// Registered ahead of `Smelt` in `registry_for` and **not** in
/// `default_registry` — a single-bot registry has nobody to converge with, and
/// keeping multi-bot behaviour in roster-aware methods is the pattern
/// `SplitAcrossBots` already set.
pub struct SharedSmelt { pub bots: Vec<BotId> }
```

* `applicable` = `Smelt::applicable(goal, state)` **and**
  `worth_converging(..).is_some()` (§7).
* `split_probe` = `Some(Goal::Have { item: <first ingredient>, count: <ore
  total>, whose: Holder::Anyone })` — so the split is bounded by ore seats,
  through `Mine::concurrency`, without this method learning what a seat is.
* `converges` stays `false`. `smelting_never_converges` (`have.rs:3618`) and
  the module doc's "so three bots can each supply one and nothing has to
  converge" are about `Method::converges`, i.e. about *welding producers to a
  consumer in one chain*. This design does the opposite of welding, so that
  statement is not merely respected, it is finally acted on: the module doc has
  said since it was written that a furnace does not need convergence, and the
  share chain was the only thing preventing it.

Expansion, identical to `Smelt::expand` except where marked:

```text
  Step::Subgoal(Have { coal,          coal_bill, whose })          # taker
  Step::Subgoal(Have { stone-furnace, 1,         whose })          # taker
  Step::Act(place_id:  Place  stone-furnace at pos)                # taker
  Step::Act(fuel_id:   Insert coal   -> Fuel)                      # taker

  for (b, work_b) in shares:                                       # NEW
      Step::Owned { whose: Holder::Share(b), steps: vec![
          Step::Subgoal(Have { ore, spare_b + work_b, Share(b) }),
          Step::Act(insert_b: Insert ore -> FurnaceSource, count: work_b),
      ]}
      Step::Link { from: place_id,  to: insert_b, lag: 0 }
      Step::Link { from: insert_b,  to: remove_id, lag: smelt_lag }

  Step::Act(remove_id: Remove item <- FurnaceResult, count: need)  # taker
  Step::Link { from: fuel_id, to: remove_id, lag: 0 }
```

with the taker itself a participant in `shares` when it should be (a one-bot
share needs no `Step::Owned` wrapper: emit it inline, in the enclosing chain,
which is precisely today's behaviour).

Why the edges are what they are:

* `place_id -> insert_b` would also be inferred (`Condition::EntityAt` is
  world-scoped, so `infer_edges` keeps it across chains), but it is stated
  explicitly because the method holds both ids and a plan should not depend on
  inference where a statement is free. `infer_edges` skips duplicates.
* `insert_b -> remove_id` **cannot** be inferred: the take has no condition
  that any insert's effect satisfies, and even if it had a `HasItem { Role }`
  the cross-chain exclusion would drop it. This link is the whole handover, and
  carrying `smelt_lag` on it is why the method must own both ids — which is why
  the primitive is `Step::Owned` and not a subgoal.
* `smelt_lag` keeps its current definition and its one-cycle headroom
  (`have.rs`, the comment recording the 1924-vs-1920 nine-plates-out-of-ten
  failure). Nothing about the lag changes.

**What does not move to the suppliers, and why.** The furnace, its stone and
its coal stay with the taker. The furnace must exist before any supplier can
insert into it, so placing it in a supplier chain buys an extra cross-chain
edge on the critical path for ~5 stone and 1 coal of work. Milestone 5's plan
above says that stone is ~1,250 ticks, so this is a real residual — a follow-up
(`Step::Owned` around the furnace bill too) is cheap once the primitive exists,
and is deliberately not in stage 1 so that stage 1 changes one thing.

## 6. Stage 2 — the chest, for what a furnace cannot carry

A furnace buffers only what is smelted. Science packs, gears, circuits and
belts are hand-crafted, and the pack path
(`Researched` → `Condition::HasItem { Role, pack, N }` on the research action)
needs N packs in one inventory with no machine anywhere on the path. That is
what a chest is for.

### 6.1 Which chest, and what it costs — the brief's assumption is wrong

`wooden-chest` costs 2 wood (`workspace/data/base/prototypes/recipe.lua:719`).
**Wood is unobtainable to this planner.** `Mine::applicable` goes through
`resource_supply_at_least` → `PlanState::resource_patches` →
`EntityGraph::resource_patches`, which indexes entities of type `resource`;
trees are type `tree` and never appear. There is no recipe for wood and no
smelt for it. So `Goal::Have { wood, 2, .. }` has **no applicable method** and
the whole expansion fails with `NoApplicableMethod`. A wooden chest is not a
cheap chest here; it is an impossible one, and any implementation that reaches
for it will fail on the first expansion.

`iron-chest` costs **8 iron plates**, `enabled = true`
(`recipe.lua:1138-1144`), collision box ±0.35 (a fifth of a stone furnace's
footprint area), 16 slots. That is the chest.

**Who pays: the taker.** The chest has to exist before any supplier inserts
into it, so it belongs in the taker's chain for the same reason the furnace
does. 8 plates is ~8 ore plus a share of a smelt — payable once per run if the
chest is reused (§6.2), and it is the reason stage 2 depends on stage 1: with
stage 1 in place those 8 plates are themselves cheap to produce, and without it
they land entirely on the taker at the worst possible moment.

**Reused, not torn down, and left standing.** Reused because a second handover
that finds a chest already there pays nothing; left standing because mining it
back costs an action to destroy 8 plates that the next milestone will want
again, and because a chest still in the world is what makes stranded items
recoverable (§8). The cost of leaving it is one more obstacle on the ground,
which the ordinary `AreaFree` / refusal machinery already handles.

### 6.2 Where the rendezvous goes, deterministically

```rust
/// The buffer this handover will use: an existing one if there is one, else a
/// site for a new one.
///
/// Deterministic in three steps, none of which touches a hash map:
///   1. the plan's own additions — `PlanState::added` is a `BTreeMap<Pos, _>`,
///      scanned in key order for an `iron-chest`;
///   2. failing that, the world's entities within `HANDOVER_SEARCH_RADIUS` of
///      the anchor, ordered by `(distance.total_cmp, Pos)` so ties resolve on
///      position and never on iteration order;
///   3. failing that, `free_area_near(state, &anchor, "iron-chest")`, which is
///      already a fixed spiral over integer tiles and is already pinned by
///      `the_same_world_sites_the_same_furnace_twice`.
///
/// The anchor is the taker's simulated position, `ctx.state.bot(taker).position`
/// — a value in `PlanState`, not a wall-clock or world read.
fn handover_buffer(state: &PlanState, anchor: &Position) -> BufferSite
```

**The interaction with placement geometry, stated honestly.** A chest is a
place two or more bots stand, and a bot standing somewhere is a bot in the way
of a placement — that is the whole parked-bot saga. This design does three
things about it and no more:

* it does **not** add a stand-point model to the planner. That was weighed and
  rejected in the parked-bots note, and the reasons are unchanged: the planner
  is not the component that was wrong.
* it prefers reuse, so a run tends toward *one* chest rather than one per
  transfer, which concentrates the crowding in a place the site search already
  knows about (the chest's own footprint is in `added`, so `AreaFree` excludes
  it from every subsequent site).
* it relies on the mod-side step-aside that landed 2026-09-02 to move a parked
  bot out of a footprint, which is exactly the case a chest manufactures.

The residual risk is real and is not eliminated: **more bots standing in more
places is more placement refusals**, traded against the far larger cost of
three bots parked for 13,000 ticks with nothing to do. If a run shows the trade
going the wrong way, the lever is buffer *siting* (anchor away from the ore
front), not planner geometry.

### 6.3 Shape

```rust
/// Deliver `count` of `item` into the consumer's hands through a chest.
pub struct HandOff { pub bots: Vec<BotId> }
```

* `claims(site)` = `!site.top_level && site.in_chain && !site.converging`.
  Top-level is `SplitAcrossBots`' (splitting with no handover is strictly
  better); un-chained means nothing forces convergence; `converging` is the
  termination guard.
* `applicable` additionally requires that the item is **not** smelted
  (`recipe_for(..).category != SMELTING_CATEGORY`), so `SharedSmelt` wins for
  anything a furnace can carry, and `worth_converging(..).is_some()`.
* `split_probe` = `Some(goal.clone())`.
* Registered in `registry_for` after `SharedSmelt` and before `Smelt`.

```text
  Step::Subgoal(Have { iron-chest, 1, Share(taker) })        # guarded by `converging`
  Step::Act(place_id: Place iron-chest at pos)               # taker; skipped if reused
  Step::Subgoal(Have { item, spare_A + work_A, Share(A) })   # the taker's own share

  for (b, work_b) in supplier_shares:
      Step::Owned { whose: Holder::Share(b), steps: vec![
          Step::Subgoal(Have { item, spare_b + work_b, Share(b) }),
          Step::Act(insert_b: Insert item -> InventorySlot::Chest, count: work_b),
      ]}
      Step::Link { from: place_id, to: insert_b, lag: 0 }     # omitted when reused
      Step::Link { from: insert_b, to: remove_id, lag: 0 }

  Step::Act(remove_id: Remove item <- InventorySlot::Chest, count: delivered)  # taker
```

`delivered = sum(work_b)` over suppliers only; the taker's own share never
enters the chest. The `Remove`'s `Effect::GainItem { who: Role, item,
delivered }` is what the enclosing goal's shortfall arithmetic sees, applied
against `ctx.chain_actor` = the taker, exactly as `Smelt`'s take is today.

Share sizing for both stages comes from **one** extracted helper, so there is
one rule and not two:

```rust
/// Split `need` across `bots`, equal work per participant, remainder to the
/// poorest, participation and tie-breaks by `(available(Share(b), item), BotId)`
/// ascending, `sort_unstable` on a total order.
///
/// Lifted verbatim out of `SplitAcrossBots::expand` — including the measured
/// justification for equal work over levelling (2156 ticks against 2427) and
/// the reason emission is by ascending `BotId`.
pub fn even_shares(
    state: &PlanState, item: &str, need: u32, bots: &[BotId], seats: u32,
) -> Result<BTreeMap<BotId, u32>, PlannerError>
```

## 7. The convergence predicate

The rule has to be conservative in one specific direction: **converging where
splitting would have done is a regression**, because splitting costs nothing
and a handover costs an insert, a take and a walk. So the predicate refuses by
default and only converges where the physics forces it *and* the arithmetic
pays.

```rust
/// The supplier shares for a convergence, or `None` when convergence does not
/// pay. One function, so `applicable` and `expand` cannot answer differently.
pub fn worth_converging(
    state: &PlanState,
    item: &str,          // what will actually be split (ore, for a smelt)
    need: u32,           // how much of it
    taker: BotId,
    bots: &[BotId],
    seats: u32,          // ctx.concurrency, or u32::MAX
) -> Option<BTreeMap<BotId, u32>>
```

**G1. More than one bot.** `bots.len() >= 2`, and at least one candidate other
than `taker` that `state.bot(..)` knows.

**G2. The count really must land in one inventory.** Enforced by `claims`:
`!site.top_level && site.in_chain`. A top-level goal is scattered by
`SplitAcrossBots` with no handover at all; a goal outside a chain has no
single-inventory consumer waiting for it. This is the gate that keeps the
measured benefit of splitting intact — **nothing that splits today converges
tomorrow.**

**G3. Not already converging.** `!site.converging`. Termination.

**G4. Splittable at all.** `need >= 2` and `k = min(candidates, need, seats) >= 2`,
where `seats` arrives from `Method::split_probe` → `MethodRegistry::concurrency`
so an ore patch with three seats produces a three-way split rather than a
`NoRoomToWork` for the whole expansion.

**G5. The arithmetic pays.**

```
solo(item, need)   = if it is a resource:  mining_ticks(state, item) * need
                     else if it has a recipe: recipe_ticks * need.div_ceil(output_per_craft)
                     else: 0                      // -> never converge

handover(k)        = k * TRANSFER_TICKS            // one insert per supplier
                   + TRANSFER_TICKS                // the single take
                   + if a buffer must be placed { PLACE_TICKS + buffer_bill_ticks } else { 0 }
                   + HANDOVER_WALK_TICKS           // see below

converge  iff  solo / k + handover(k)  <  solo
```

Two deliberate approximations, both erring toward *not* converging:

* `solo` is **shallow** — one level, no recursion into a recipe's own
  ingredients. It therefore under-states the work being spread, so the
  predicate under-fires. Under-firing is a slow plan; over-firing is a
  regression against a measured baseline.
* `HANDOVER_WALK_TICKS` is a constant (proposed: **300**, about 45 tiles at
  `WALK_TILES_PER_TICK`), not a computed distance, because bot positions do
  not advance during expansion — `Smelt::expand` says so in place ("the bot's
  start… never advances during expansion"). A real distance here would be a
  confidently wrong number rather than an admittedly rough one.

**Worked against real data**, which is the reason to believe the shape:

| case | k | solo | solo/k + handover | converge? |
| --- | --- | --- | --- | --- |
| milestone 5, 3 iron plates short (3 ore) | 2 | ~576 | 288 + 330 = 618 | **no** — correct, a 3-plate handover is not worth a walk |
| milestone 6, ~50 iron ore for the lab | 4 | ~9600 | 2400 + 350 = 2750 | **yes** |
| a 1-item shortfall | — | — | — | **no**, by G4 |
| a top-level `Have(iron-ore, 20, Anyone)` | — | — | — | **no**, by G2 — still `SplitAcrossBots` |

The middle row is the whole point: the note measured 8,280 of bot 1's 15,922
ticks as mining, and this is the rule that spreads it.

## 8. What happens if the consumer never arrives

**Today: the items are lost to the planner, and the replan re-mines them.** This
is not a hypothesis, it is a chain of four facts:

1. `PlanState` has no container model. Inventories are per bot, from
   `FactorioPlayer.main_inventory` (`state.rs:438-455`).
2. `FactorioWorld` never learns container contents.
   `FactorioWorld::on_some_entity_updated` is `Ok(())` with a `// TODO`
   (`world.rs:343-346`), and `on_some_entity_created` stores the entity as
   built — for a chest that is an empty one.
3. The only path that can read contents is the on-demand RCON query
   `rcon_inventory_contents_at` (`control.lua:3508`) →
   `FactorioRcon::inventory_contents_at` (`rcon.rs:1827`). Its callers are
   `crates/server`'s HTTP handler and the Lua binding. **No planning path calls
   it.**
4. So a replan sees an empty-handed roster, ore already consumed from the
   ground, and asks for the whole bill again — each iteration slower than the
   last, into `scripts/supervisor.lua`'s `stall_limit = 3`.

This exposure exists **today**, for the furnace, and is merely narrow: insert
and take sit in one chain, adjacent in time, on one bot. Cross-bot convergence
widens the window from seconds to "however long the taker takes to walk over",
so it must be closed before this runs unattended.

**Stage 3 closes it, in two halves, with the I/O outside `crates/planner`.**

*Outside the planner (`crates/core`).* A new
`FactorioWorld::update_entity_inventory(name, position, output, fuel)` that
replaces the stored `FactorioEntity` in the entity graph, and a call site in
the pre-plan path (`crates/core/src/plan/planner.rs`, where the world is
sampled before `PlanState::from_world`) that issues one
`rcon.inventory_contents_at` for the containers and furnaces the entity graph
already knows about. One RCON round trip per replan, for entities the planner
placed itself.

*Inside the planner, pure.*

```rust
// PlanState
buffers: BTreeMap<(Pos, InventorySlot), BTreeMap<ItemId, u32>>,   // plan overlay
pub fn buffered(&self, pos: &Position, slot: InventorySlot, item: &str) -> u32
```

reading the overlay over `entity_at(pos)`'s `output_inventory` /
`fuel_inventory`, plus three new variants:

```rust
Condition::BufferHas  { pos: Position, slot: InventorySlot, item: ItemId, count: u32 }
Effect::BufferGain    { pos: Position, slot: InventorySlot, item: ItemId, count: u32 }
Effect::BufferLose    { pos: Position, slot: InventorySlot, item: ItemId, count: u32 }
```

with `Effect::satisfies` pairing `BufferGain` → `BufferHas` on `(Pos, slot,
item)`, counts ignored for ordering exactly as `GainItem`/`HasItem` are today.
Because a `BufferHas` is **not** a `HasItem { who: Role }`, `infer_edges`'
cross-chain exclusion does not drop it, so the supplier→taker edge becomes
inferable as well as stated. The explicit `Step::Link` stays anyway: it carries
the smelt lag, which no inference can.

Blast radius checked: `Condition` and `Effect` appear in no OpenAPI schema
(`app/src/api/openapi.snapshot.json` contains neither `AtPosition` nor
`ResourceAvailable`), no `documented_type_schemas()` root, and no exhaustive
match outside `crates/planner` — the only mentions in `crates/executor` are
literal constructions in tests.

And one new method, which is where the recovery actually happens:

```rust
/// Take what a previous plan left in a buffer, rather than making it again.
/// Registered ahead of `Smelt`, `HandCraft` and `Mine`: a plate in a chest
/// beats a plate in the ground.
pub struct Withdraw;
```

claiming `Have { item, count, Share(A) }` when
`state.buffered(pos, slot, item) >= shortfall` for some buffer within
`WITHDRAW_SEARCH_RADIUS`, expanding to a single `Remove` with
`Condition::BufferHas` + `Effect::BufferLose` + `Effect::GainItem`.

`Withdraw` is independently valuable and independently testable **before** any
of the rest of this design lands, and it is the honest answer to "the consumer
never arrived": the next plan walks over and picks the items up.

## 9. Constraints this design does not touch

* **`Condition::Researched` / `RecipeGate::PlannedResearch` (`21a1228a`) are
  untouched.** Nothing here changes when a research subgoal is emitted, which
  crafts carry the condition, or how `infer_edges` turns it into an edge. What
  moves is *who mines the ore that becomes the plates that become the lab* —
  strictly below the gate. The unlock chain still belongs to one bot and still
  orders every sibling share after it.
* **Owner binding (2026-09-02) is untouched and is not "restored parallelism"
  in disguise.** That fix says: a chain sized against bot `b` must be run by
  `b`. This design creates *more* chains, each with an owner, each sized
  against the bot that owns it. It never hands a chain to a bot its bill was
  not sized against — which is precisely the failure that fix was for. The
  comment in `method/mod.rs` warns against restoring parallelism "without also
  re-solving the sizing problem it removes"; the sizing problem is re-solved by
  `Step::Owned` rebinding `chain_actor` before anything under it is sized, and
  by `reserve_chain_produce` firing for nested chains too.
* **`Method::converges` and `smelting_never_converges` are untouched.** See
  §5.
* **`SplitAcrossBots` keeps every top-level goal.** G2.
* **No `cheat_*` call, no teleport, no item creation.** Every action emitted is
  `Place`, `Insert` or `Remove` — the same three verbs the plan already issues,
  on an entity a character stands next to.

## 10. Determinism

The planner's rule is: no I/O, no async, no wall clock, ordered collections
only, floats by `total_cmp`, identical inputs → byte-identical plans. Each new
piece against that rule:

* **Supplier selection and share sizing**: `even_shares`, lifted verbatim from
  `SplitAcrossBots::expand`. Candidates are `(available(Share(b), item), BotId)`
  sorted `sort_unstable` on a key that is a **total order** because `BotId` is
  unique within the deduped roster — the existing comment says nobody should
  "fix" this to `sort`, and that stays true. Emission is by ascending `BotId`
  from a `BTreeMap`, because emission order fixes `ActionId` allocation and
  therefore `schedule`'s `(end, ActionId, BotId)` tie-break.
* **Buffer siting**: `BTreeMap<Pos, _>` scan, then a distance scan whose ties
  break on `Pos`, then `free_area_near`'s fixed integer spiral. No hash
  iteration anywhere. Distances compared with `total_cmp`, as
  `PlanState::from_world` already does throughout.
* **The nested-chain rule** is keyed on `BotId` equality, not on order.
  `ChainId`s come from `ctx.chains`, a monotone generator driven by expansion
  order, which is itself fixed by step order.
* **The predicate** is integer arithmetic on `Ticks` (`u64`), with no float
  comparison at all.
* **`split_probe` / `MethodRegistry::concurrency`** iterate `self.methods` in
  registration order and take a `min` over `Option<u32>` — order-independent.
* **Inertness of the driver change is testable, not argued.** Every nested
  stated holder in the crate today names `ctx.chain_actor` (§3), so before any
  method emits a `Step::Owned`, the number of chains in every existing fixture
  plan is unchanged. That is a test, not a claim.

**A rendezvous chosen by hash iteration order would be a correctness bug**, and
the three-step siting rule above exists specifically so that it cannot be.

## 11. What changes in `crates/executor`: nothing

Verified end to end:

* `ActionKind::Insert`/`Remove` already carry `entity: String`, `pos` and
  `slot: InventorySlot`. `RconActuator::insert`/`remove`
  (`rcon_actuator.rs:354-401`) pass the entity name straight through and
  resolve the slot against the game's own `defines.inventory`.
* `InventorySlot::Chest.defines_key()` is `"chest"`, which is present in
  `FACTORIO_2_1_INVENTORY_DEFINES`.
* The mod is entity-agnostic: `rcon_insert_to_inventory` /
  `rcon_remove_from_inventory` (`control.lua:3085`, `:3124`) do
  `surface.find_entity(name, pos)` then `entity.get_inventory(inventory_type)`.
  A chest needs no new mod code.
* `Place` of an `iron-chest` is the same action as `Place` of a
  `stone-furnace`.

Two executor-adjacent **risks**, neither a code change and both worth pinning
before a long run:

* `recover.rs`'s tiers were written for chains with one owner. A plan where a
  chain's inputs are produced by a different chain has never been executed.
  Tier 2 in particular re-plans with a chosen `chain_actor`, and that choice is
  now load-bearing across more chains.
* `recover.rs:55` notes that `Insert` is safe to re-run because "the items go
  into the chest or furnace a second time". With a *shared* buffer, a
  double-inserted supply is now visible to a different bot's `Remove` count, so
  a retry that succeeds twice over-delivers. Harmless for the plan (the take is
  capped at `count`) but it strands the surplus, which is exactly what stage 3
  recovers.

## 12. Failure modes

| failure | what happens | mitigation |
| --- | --- | --- |
| a supplier's insert never runs | the take returns short; the enclosing craft fails its `HasItem` | replan; with stage 3 the replan sees the partial buffer and asks only for the difference |
| the taker never arrives | items stranded in the buffer | stage 3's `Withdraw`; without it, re-mined |
| buffer full | `iron-chest` is 16 slots; iron plate stacks 100, so 1,600 plates — not reachable by these bills, but `FactorioItemPrototype.stack_size` is available and the cap should be asserted, not assumed | cap a single handover at `slots * stack_size`, refuse above it |
| two takers share one buffer | contents are not per-taker; taker 2 can take taker 1's items | the plan reserves per handover in the `buffers` overlay; the *physical* race is real and is why the reuse rule prefers one chest and the take names an exact count |
| buffer destroyed or mined between plan and execution | `Condition::EntityAt` fails at schedule time | ordinary re-site; the refusal ledger already handles a site the game rejects |
| a parked bot stands in the new chest's footprint | placement refused | mod-side step-aside (2026-09-02); §6.2 |
| cycle in the network | impossible by construction — edges run supplier → taker only — and `net.validate()` still runs | — |
| a supplier is asked for ore there is no seat for | `NoRoomToWork` for the *split*, not the expansion | `split_probe` bounds `k` by seats before any share is sized |
| a handover recurses into itself | impossible | `GoalSite::converging` |

## 13. Staged plan

**Stage 0 — the primitive, alone. Independently testable, zero behaviour change.**
`Step::Owned`, the `run_steps` arm, `reserve_chain_produce` extracted,
`GoalSite::converging`, `Method::split_probe`, `PlannerError::UnownedHandover`.
No method emits any of it. Tests: a hand-built method that emits
`Step::Owned` puts its actions in a second chain with the named owner; the
context is restored on the error path; **every existing fixture produces the
same number of chains, the same assignments and the same makespan as before**
(the inertness proof). Green `cargo test --workspace` is the whole acceptance
criterion — no live run needed.

**Stage 1 — `SharedSmelt`.** The furnace as buffer, `even_shares` extracted,
`worth_converging`. Tests: the `unlock_state` fixture from the research-bill
note goes from 49/12/12/12 to a distribution whose busiest bot carries less
than twice the idlest (stated as a ratio, so a recipe change moves the number
and not the claim); `the_whole_unlock_subtree_lands_on_one_bot` **flips** and
becomes the after-column of its own doc comment; a three-plate shortfall does
**not** converge (milestone 5's arithmetic, pinned as a predicate test);
`the_unlock_path_plans_identically_twice` still passes; a single-bot registry is
byte-identical to today. Expect churn in `tests/red_science.rs`,
`tests/scheduling.rs`, `tests/smelt_roots.rs`, `tests/seeded_roster.rs`,
`tests/split_capacity.rs`, which pin makespans this moves — that churn is the
deliverable, not a side effect, and each changed number needs a sentence saying
why.

**Stage 2 — `HandOff` and the `iron-chest`.** Everything in §6. Tests: the
pack path splits; the chest is placed once and reused on the second handover;
`wooden-chest` is never named anywhere; the chest's own 8-plate bill does not
converge (`converging`); a smelted item is claimed by `SharedSmelt` and not by
`HandOff`.

**Stage 3 — buffer visibility.** §8. Two halves, each testable alone: a
`crates/core` test that a world told about a chest's contents reports them
through the entity graph, and a `crates/planner` test that a `PlanState` built
from such a world lets `Withdraw` claim a goal the roster could otherwise only
re-mine. **Stage 3 must land before this design runs unattended for hours**,
for the reason in §8; stages 1 and 2 may land first because until they do, the
window they widen is the one the furnace path already has.

## 14. What I could not determine without running

1. **Whether the makespan actually improves.** The note's ~9,000-tick estimate
   for the furnace split is arithmetic on `fixture_world`'s model figures, and
   `fixture_world` has no players, a synthetic ore layout and no forces. Every
   number in §7's table is model arithmetic, not observed play.
2. **Whether a furnace placed by one bot and unloaded by another survives the
   executor.** `Condition::AtPosition` should make the taker walk there;
   `recover.rs`'s tiers have never seen a chain pair that hands off through a
   machine. This is the single biggest unknown in the design.
3. **Whether two bots inserting into one furnace hit a real
   `crafter_input` limit.** The mod clamps an insert to what the player holds
   and complains when `inventory.insert` returns less than asked, but a furnace
   input slot's capacity under a 50-ore load has not been observed.
4. **`HANDOVER_WALK_TICKS = 300` is a guess.** It is the one tuning constant in
   the predicate, and the first live run should be read for whether handovers
   are firing where they should not.
5. **Whether the supervisor's iteration budget would see the improvement.**
   Milestone 6's budget was 6,217 ticks against a 25,254-tick plan; halving the
   plan still does not fit, so the budget is a separate blocker and this design
   does not address it.
6. **Whether `find_non_colliding_position` and the step-aside cope with a
   chest that two bots service repeatedly.** The parked-bot note already lists
   the underlying assumption as unverified.
7. **Whether the chest is ever needed in practice.** If stage 1 spreads enough
   of the bill, stage 2 may buy little for 8 plates and an obstacle. Stage 1's
   measured distribution is the input to that decision, and stage 2 should not
   be written before it exists.
