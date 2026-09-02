# Rung 6: the ingredients were there; the recipe was locked

`workspace/runs/run-1788338409-63794`, milestone 6 (`craft automation science
packs x10`), stuck after 6 iterations with

```
could not have player client2 craft 3 automation-science-pack (but only 0)
```

Commit: `21a1228a` — `fix(planner): a share that did not do the research still
waits for it`.

## The record answered it, in two streams

**`samples.jsonl` refuted the leading hypothesis in one query.** At tick 19380
(the plan for rung 6 was created at 19375) and unchanged all the way to 25620:

| bot | copper-plate | iron-gear-wheel |
| --- | --- | --- |
| 1 | 6 | 5 |
| 2 | **6** | **5** |
| 3 | 6 | 5 |
| 4 | 5 gears, 2 plates | 5 |

`client2` — `player.index == 2`, which is what `sample_bots_body` writes as
`id` — was holding **six copper plates and five gear wheels** at the moment the
game said it could craft zero. Every bot held both. The split-inventory story is
false: no bot needed anything from any other bot, and no item was in a chest.

The inventories are *identical* across all six minutes of the milestone, which
is itself the tell: bot 2 never crafted, never consumed, and every retry got the
same answer. A stable refusal with the ingredients in hand is not an inventory
problem.

**`events.jsonl`'s `plan_created` named the defect.** The rung-6 DAG:

```
{"id":34,"bot":3,"action":"craft 2 automation-science-pack","deps":[],"planned_start":0}
{"id":35,"bot":4,"action":"craft 2 automation-science-pack","deps":[],"planned_start":0}
{"id":33,"bot":2,"action":"craft 3 automation-science-pack","deps":[],"planned_start":0}
...
{"id":18,"bot":1,"action":"craft 1 lab","deps":[19,20,22,23,32]}
{"id":0, "bot":1,"action":"craft 3 automation-science-pack","deps":[4,18,19,23,27,32]}
```

Four crafts of the same item. **One** of them (node 0, bot 1) depends on a lab.
The other three depend on nothing and start at tick 0.

## Root cause

`automation-science-pack` has `enabled = false` in shipped 2.1.17
(`workspace/data/base/prototypes/recipe.lua:1003`). It is unlocked by the
technology of the same name, which is a Factorio 2.0 **`craft-item` trigger**:
craft a lab and the force gets the recipe. The planner models this correctly —
that is what the whole gears/circuits/belt/lab chain on bot 1 is.

`Goal::Have(automation-science-pack, 10, Anyone)` is claimed by
`SplitAcrossBots`, which emits one `Holder::Share(b)` chain per bot. Shares
expand in order:

1. Bot 1's share reaches `HandCraft::expand`. `recipe_gate` says
   `NeedsResearch("automation-science-pack")`, so it emits a
   `Goal::Researched` subgoal **and** a `Condition::Researched` on its craft.
2. That subgoal expands the trigger path down to `Produced(lab, 1, unlocks=…)`,
   and `attach_unlock` hangs `Effect::Researched` on the "craft 1 lab" action.
3. `run_steps` (`method/mod.rs:632`) applies every emitted action's effects to
   `ctx.state` as it goes, so `state.set_researched(...)` fires.
4. Bots 2, 3 and 4's shares then ask `recipe_gate`, which called
   `state.is_researched(&tech)` — and `is_researched` is the **union of the
   world and the plan's overlay**. It answered `Open`.

`Open` means "nothing has to happen first". So those three crafts carried no
`Condition::Researched`, `ActionNetwork::infer_edges` had no condition to match
`Effect::Researched` against, no edge was created, and `schedule` put them at
`planned_start: 0`. They were dispatched before the lab existed.

`sup.first_error` is the *first* failed action of a milestone, so the recorded
`last_error` is iteration 1's — consistent with a craft dispatched at tick 0 of
the milestone, not a late failure.

## Hypotheses ruled out, with the evidence

| Hypothesis | Verdict | Evidence |
| --- | --- | --- |
| Split inventory: plates on one bot, gears on another | **false** | `samples.jsonl` tick 19380–25620: every bot holds both |
| Ingredients consumed by an earlier step of the same plan | **false** | Bot 2's inventory is byte-identical across the whole milestone; nothing it held ever moved |
| Craft assigned to a parked bot that skipped rungs 4–5 | **false** | `run_started.bots` is `[1,2,3,4]`; all four appear in `samples.jsonl` with rung-4/5 output |
| Ingredients in a chest, not an inventory | **false** | Both ingredients are in `character_main` per the sampler |
| `HandCraft` demands ingredients under `Anyone` rather than `Share(actor)` | **not the cause** | `HandCraft` propagates `whose` verbatim and `SplitAcrossBots` hands it `Share(b)`; the ingredient conditions are `HasItem { who: Role }` and were all satisfied |

This is **not** the `Researched`-sizes-its-bill-against-one-bot design gap
wearing a different hat. That gap is about *which inventory* a bill is sized
against; this is about *ordering* — a precondition that was never stated. The
two are independent, and this one is a plain bug with a local fix.

## The fix

`RecipeGate` gained a third live variant:

- `Open` — enabled, or the unlocking technology was finished **by the world**
  before planning began. Nothing orders against it.
- `PlannedResearch(tech)` — disabled, and the unlock is already in *this*
  network because a sibling put it there. Emit **no** second research subgoal
  (the work exists), but **do** state `Condition::Researched`, which is what
  `infer_edges` turns into the ordering edge.
- `NeedsResearch(tech)` / `Unobtainable` — unchanged.

`recipe_gate` now consults `PlanState::is_world_researched` (new; the world half
of `is_researched`, which had no separate spelling) before falling through to
the overlay. Both gate sites — `Smelt::expand` and `HandCraft::expand` — handle
the new variant. `trigger_requirement`'s self-unlocking-cycle guard matches on
`NeedsResearch` only, so it behaves exactly as before.

The edge survives `infer_edges`' cross-chain exclusion because
`Condition::Researched` is world-scoped, not `HasItem { who: Role }` — the
exclusion is narrow on purpose and this is the case it was kept narrow for.

### Rejected

- **Move items between bots.** Nothing needed moving; every bot had the
  ingredients. A bot-to-bot transfer primitive is real follow-up work but would
  not have fixed this run.
- **Plan the craft onto the bot that holds the ingredients.** Same: all four did.
- **Demand ingredients in one place to begin with.** Already the case
  (`Share(b)` chains, `HasItem { who: Role }`).
- **Emit a research subgoal per share.** Would order the crafts correctly but
  plan four labs for one force-wide unlock. `PlannedResearch` gets the ordering
  without the duplicate work — the reason the two halves of the old `Open` had
  to be separated rather than collapsed the other way.
- **Attach the condition unconditionally whenever a recipe is disabled in the
  world.** Would also work, but leaves an inert `Condition::Researched` on every
  craft of a recipe the world unlocked long ago — noise in every rendered plan,
  and load-bearing the day a method starts reasoning over preconditions.

## Tests

`cargo fmt`, `cargo clippy --workspace --all-features --all-targets --deny
warnings`, `cargo test --workspace`: 1235 passed, 0 failed.

Two new regression tests, one per unlock path, both verified red against the old
`recipe_gate`:

- `every_share_of_a_locked_recipe_waits_for_the_one_research` — pack-researched
  unlock; asserts each share's craft carries the condition and starts no earlier
  than the research ends.
- `every_share_waits_for_a_trigger_unlock_riding_on_a_craft` — the live shape:
  a `craft-item` trigger, so the unlock rides on an ordinary craft and there is
  no `Research` action at all. Asserts a real edge from the unlocking action to
  every share's craft.

One existing test moved. `an_already_researched_unlocker_leaves_the_recipe_open`
asserted `Open` for an unlocker marked researched via `set_researched` — the
*overlay*. **The old expectation was wrong**, not behaviour changed: it read the
overlay as if it were the world, which is the exact conflation this run cost.
It is now
`an_unlocker_this_plan_researches_leaves_the_recipe_planned_not_open`, plus a
new `an_unlocker_the_world_already_researched_leaves_the_recipe_open` using a
new `test_world::world_with_researched_unlocker` fixture — so the new variant
cannot be satisfied by an implementation that simply never reports `Open`, and
`a_world_researched_unlocker_leaves_the_craft_ungated` pins that the genuinely
open case still emits neither subgoal nor condition.

## What this does not fix

Nothing here addresses whether the rung would then *succeed*. Bot 1's chain must
still mine 50 iron ore, smelt it and craft the lab before any share can craft,
and the previous run's makespan for that chain was ~26k ticks against a 6217-tick
milestone budget. The crafts will now be correctly ordered; whether the
supervisor's iteration limit is generous enough to see them run is a separate
question the next live run will answer.
