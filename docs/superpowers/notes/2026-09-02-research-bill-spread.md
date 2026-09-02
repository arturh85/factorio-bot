# The unlock subtree is one bot's problem, and the named cause is not why

**Verdict: no fix landed. This needs a design decision the specs do not
settle, and I am reporting it rather than guessing.** What did land is two
tests that pin the distribution and its determinism, and the measurements
below, so that whoever picks this up starts from numbers instead of a run log.

## The distribution, as recorded

`workspace/runs/run-1788341905-92036`, the last `plan_created`
(`milestone_index` 6, "craft automation science packs x10"): 33 steps,
makespan 25254, distributed

| bot | steps |
| --- | --- |
| 1 | **30** |
| 2 | 1 |
| 3 | 1 |
| 4 | 1 |

Verified by reading `events.jsonl`, not taken on trust. The three steps away
from bot 1 are the science-pack crafts (`craft 3`, `craft 2`, `craft 2`), each
depending only on node 15, `craft 1 lab`. Everything feeding them — 3 + 47
iron ore, 5 coal, 10 stone, two stone furnaces, three placements, three
fuellings, 13 copper ore, two copper smelts, one 50-ore iron smelt, the cable,
the gears, the belts, the circuits and the lab — is bot 1's.

## Reproduced in the planner, with the counterfactuals

`crates/planner/src/method/have.rs::unlock_state` builds the same shape against
the fixtures: four bots, a `craft-item` trigger on a **lab** (the fixture's lab
really does cost 10 gears, 10 circuits and 4 belts), `automation-science-pack`
locked behind it, `Have { automation-science-pack, 4, Anyone }`. Coal and spare
furnaces are seeded because the fixture's coal patch cannot fuel seven furnaces
and the expansion is otherwise refused outright; fuel is not what this is about.

| scenario | distribution | makespan |
| --- | --- | --- |
| **A** as-is, recipe locked | **49 / 12 / 12 / 12** | **15922** |
| **B** same goal, `asp-tech` already researched in the world | 12 / 12 / 12 / 12 | **2304** |
| **C** `Goal::Researched` hoisted to a top-level sibling of the pack goal | **49 / 12 / 12 / 12** | 16832 |

A against B: **86% of the makespan is the unlock, and every action of it is on
one bot.** 48 of bot 1's 49 actions are ancestors of the lab craft; the 49th is
its own pack craft. Bots 2-4 do 12 actions each — their own shares' gears and
plates — finish around tick 4400 and then idle for eleven thousand ticks.

Of bot 1's 15922 ticks, **8280 are mining** (1 copper, 2 + 2 + 4 iron, 3 × 5
stone, 10 iron, 15 copper, 20 iron). That is the third of the plan a roster
could obviously share, and it is the number worth chasing.

C is the interesting one: **hoisting the research out of the share changes the
distribution not at all**, and makes the makespan *worse*. The subtree simply
opens its own chain, owned by `chain_actor`, and loses the intermediates it
used to share with the share it sat under. Anyone reaching for "just take the
research out of the share chain" can stop here.

## Where the serialisation actually comes from

Not from `Researched` sizing its bill against one bot. That is a true statement
about the code and it is not the binding constraint. The chain of causation is:

1. `Goal::Have { pack, 10, Anyone }` is claimed by `SplitAcrossBots`, which
   emits one `Holder::Share(b)` subgoal per bot.
2. The **first** share to expand meets `RecipeGate::NeedsResearch` in
   `HandCraft::expand` and emits `Goal::Researched("automation-science-pack")`
   as a subgoal — **inside that share's chain**.
3. `SplitAcrossBots::claims` is `top_level && !in_chain`, so nothing under
   there can ever be scattered again, and `expand_goal_body` only opens a chain
   when `ctx.chain.is_none()`, so nothing under there can open a chain of its
   own either. Every action the subtree emits is stamped with share 1's chain
   and `net.set_chain_owner` has bound that chain to bot 1 (commit
   `21a1228a` era; see the owner-binding comment in `method/mod.rs`).
4. So `Researched`'s `Holder::Share(ctx.chain_actor)` is not what pins the
   bill — the enclosing chain already did. Rewriting those two `Share`s to
   `Anyone` would change nothing: no method would claim them at that site.

**The physical constraint underneath is the real answer.** A lab is *one*
craft. Its ~50 iron plates and ~16 copper plates have to be in one inventory at
one moment, and this planner has no way for a second bot to put an item into a
first bot's hands. Every ore that ends up in those plates therefore has to be
mined by, or delivered to, the bot that crafts the lab. Splitting the *sizing*
across the roster does not create the delivery; it only makes the plan
unschedulable or wrong.

## What I rejected, and why

* **Split the research across the shares.** Four labs for one force-wide
  unlock. Already rejected in
  `docs/superpowers/notes/2026-09-02-craft-ingredients.md`; the
  `RecipeGate::PlannedResearch` variant exists precisely so the sibling shares
  get the *ordering* without the duplicate work.
* **Weaken the `Condition::Researched` edges from `21a1228a`.** Those are what
  stop three shares dispatching at tick 0 before the lab exists. Untouched.
* **Hoist `Goal::Researched` to a top-level sibling.** Measured (row C above):
  identical distribution, worse makespan.
* **Copy `SplitAcrossBots` into `Researched`.** Same as the first item for the
  research action, and for the trigger it would ask four bots to each craft a
  lab.
* **Give the unlock subtree to the least-loaded bot instead of
  `chain_actor`.** Real but worthless here: the four shares are symmetric, so
  every candidate is equally loaded, and the pile only moves.
* **Reduce the pack share of whichever bot carries the unlock.** Also real,
  also worthless: bot 1 finishes at 15922 because of the unlock, not because of
  its two extra pack crafts (300 ticks).

## What would actually work, and what it costs

Both roads are the same idea: **a machine can hand items between bots.** A
furnace is loaded by whoever holds the ore and unloaded by whoever needs the
plates, and `Smelt`'s `Remove` action carries no `Condition::HasItem` at all,
so the taker need not be the inserter. `have.rs`'s own module doc already says
smelting deliberately does not converge, "so three bots can each supply one and
nothing has to converge" — the share chain is the only thing preventing it.

**Option 3′ — furnace-buffered supply.** Split a smelt into a *supplier* side
(mine the ore, mine the stone, craft and place the furnace, fuel it, insert)
and a *taker* side (remove), and let the supplier side belong to a different
bot. On scenario A this spreads all 8280 mining ticks, the three furnace
crafts and the seven placements across four bots; the furnaces already run
concurrently in the model, so the remaining tail is the 20-ore smelt's ~4000
ticks plus ~1200 of crafting. Estimated makespan ~9000 against 15922.

What it costs, honestly:

* **A new way to say it.** A method emits a flat `Vec<Step>`; the only thing
  that rebinds a bot is `Step::Subgoal(Goal::…{ whose })`, and the actions
  `Smelt` emits itself (`place`, `fuel`, `insert`) are not inside any subgoal.
  So this needs either a new goal kind ("supply this machine with N of this")
  or a new step kind (`Step::Owned(BotId, Vec<Step>)`). That is new
  architecture, not a patch.
* **A driver change.** `expand_goal_body` opens a chain only when
  `ctx.chain.is_none()`. A nested `Holder::Share(b)` naming a bot other than
  the enclosing chain's actor would have to open its own chain. This is
  provably inert today — every nested share in the crate names
  `ctx.chain_actor` — but it is a change to the rule the 2026-09-02
  owner-binding fix rests on.
* **It re-opens a decision that was deliberately made the other way.**
  `docs/superpowers/notes/2026-09-02-rung-3-4-findings.md`, "Update: option 1
  implemented", accepted serialising the trigger subtree and the pack subtree
  onto one bot — 22,072 ticks of measured concurrency given up — because a
  share sized against bot *b* was being run by bot *c* and crashing. Any spread
  must re-solve that sizing problem, not just undo the binding. The comment in
  `method/mod.rs` says so in place: "do not 'restore parallelism' here without
  also re-solving the sizing problem it removes."
* **Wide test churn.** `tests/red_science.rs`, `tests/scheduling.rs`,
  `tests/smelt_roots.rs`, `tests/seeded_roster.rs` and
  `tests/split_capacity.rs` all pin makespans or placement counts that this
  moves.

**Option B — a bot-to-bot transfer primitive.** More general and it would also
fix the pack path (see below), but it needs an executor action and a mod-side
action, both outside this crate and one of them in another agent's files.

**The pack path has the same shape.** For a technology unlocked by science
packs rather than by a trigger, `Researched` emits
`Condition::HasItem { Role, pack, N }` on the research action, so all N packs
must be in one inventory — and in the game a *lab* eats them, which
`have.rs::Researched`'s own doc already admits is "approximate about who spends
them". Splitting the pack bill therefore needs the same machine-as-buffer
concept, with a lab instead of a furnace. One design fixes both; neither is
fixable alone.

## What landed

Two tests in `crates/planner/src/method/have.rs`:

* `the_whole_unlock_subtree_lands_on_one_bot` — **a characterisation test, not
  a regression test.** It asserts that every ancestor of the unlocking action
  is scheduled onto a single bot, and that the busiest bot carries at least
  three times the idlest. It passes today because the defect is present. When
  a spread lands, this assertion flips and the numbers in its doc comment are
  the before column.
* `the_unlock_path_plans_identically_twice` — same inputs, same labels, same
  assignments, same start/end ticks, same makespan. The crate already pins
  determinism for the un-researched path in `tests/red_science.rs`; the unlock
  path additionally walks a technology table that arrives through a `DashMap`
  and writes `PlanState`'s research overlay mid-expansion, so it is worth its
  own pin.

Both are stated as ratios and structural facts rather than as 49/12/12/12, so
a recipe or geometry change moves the numbers without moving the claim.

## Corrections to the brief I was given

* **"Option 3 in the share-sizing spec".** It is not there.
  `docs/superpowers/specs/2026-09-01-per-bot-share-sizing-design.md` §7 only
  flags the research path as unmeasured and explicitly out of scope ("this spec
  does not fix it and does not claim to"). The three numbered options are in
  `docs/superpowers/notes/2026-09-02-rung-3-4-findings.md`, "The rest of the
  story", and that note already calls option 3 "the largest, and the only one
  that removes the tension rather than choosing a side" and "a larger change
  than a one-night fix". This note is the measurement that note asked for.
* **"The `Researched` goal sizes its whole bill against a single bot rather
  than decomposing across the roster."** True as a description of the code,
  false as the cause. The enclosing share's chain has already bound the subtree
  before `Researched` states anything, and row C measures that: hoisting the
  goal clear of the share changes the distribution by zero actions.

## What could not be verified without a live run

1. **That the estimated ~9000-tick makespan for option 3′ is real.** It is
   arithmetic on this fixture's model figures (mining rate, `smelting_ticks`,
   walk speed), not observed play. `fixture_world` has no players, a synthetic
   ore layout and no forces.
2. **That a furnace placed by one bot and unloaded by another survives the
   executor.** `Condition::AtPosition` should make the taker walk there, and
   `crates/executor/src/recover.rs`'s tiers were written for chains with one
   owner; a chain pair that hands off through a machine has never been run.
   I did not touch `crates/executor` (another agent holds it).
3. **Whether a `craft-item` trigger with `count > 1` is satisfied by several
   players on one force crafting a share each.** If it is, trigger bills with a
   count above one could split with no new architecture at all — but the live
   case is `craft 1 lab`, so it would not have helped this run.
4. **Whether the supervisor's iteration budget would even see a 9000-tick
   milestone finish.** Milestone 6's budget was 6217 ticks against a 25254-tick
   plan; halving the plan still does not fit.
