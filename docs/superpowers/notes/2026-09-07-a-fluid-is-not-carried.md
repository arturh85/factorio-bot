# A fluid is not carried, so `Have` refuses it by shape

2026-09-07, branch `a-fluid-is-not-carried`, off `c88ba1d8`.

## The defect, and the sentence that gave it away

```
$ factorio-bot plan --world workspace/scripts/map-31337-explored.json \
      --goal have:petroleum-gas:100 --bots 1,2,3,4
Error: the goal did not expand: no method can satisfy goal:
       have 25 petroleum-gas (a share sized for bot 1)
```

Every number in that sentence describes something that cannot exist. The
planner divided 100 petroleum-gas into four shares of 25 and asked bot 1 to
obtain its share — correct for iron plates, nonsense for a fluid. No character
inventory holds a fluid in any amount, so there is no inventory the goal could
be satisfied from, no quarter of it a bot could hold, and the share is an
artefact of the refusal path rather than a fact about the request.

**The missing method was the symptom; the goal kind was the defect.**

## The judgement: `Have` was made honest, and no tenth kind was added

The brief's bias was to prefer making the existing machinery honest over adding
a tenth `Goal` kind, and the evidence supports it — but the argument is sharper
than "a new kind is expensive".

`Goal::Have` means *"this item is in an inventory, and the count is divisible
among bots"*. A fluid is never in one. The right response to a goal whose
**shape** cannot hold its subject is not to stretch the shape and not to invent
a second shape that happens to fit — it is to **refuse, first, before anything
else runs**. That is what landed:

- `Have` is now substance-aware **in the refusal**, not in the satisfaction. It
  does not learn to put a fluid anywhere; it learns to say that it cannot.
- No new `Goal` variant, so none of the three exhaustive matches, the `KINDS`
  list, the Lua binding or the OpenAPI seam moved.
- One new `PlannerError` variant, `FluidNotItem`, which is a *verdict* in
  `crates/scripting_lua/src/globals/goal/mod.rs`: no amount of mining,
  research, charting or building changes the answer, because the shape of the
  request is what is wrong.

**Where a tenth kind would genuinely be needed is item 3, and it was
deliberately not built** — see the design question at the end. Expressing *"a
tank at (x, y) holds 100 petroleum-gas"* cannot be a `Have`: `Holder` has three
variants and every one of them names a character. That is a real gap, and
bending `Have` to cover it would be exactly the lie the brief warned about.

## Where the guard sits, and why not in a `Method::refusal`

`crates/planner/src/method/mod.rs`, in `expand_goal_body`, **before
`registry.find`**:

```rust
if let Some(refusal) = fluid_have_refusal(goal, ctx) {
    return Err(refusal);
}
```

Not as a `Method::refusal`, and the distinction is load-bearing. The driver
asks every method for a refusal only *after* every method has declined — by
which time `SplitAcrossBots` has already claimed the top-level goal and emitted
`Have { 25 petroleum-gas, Share(bot 1) }`. A refusal hook can only describe the
wreckage. A fluid `Have` should not be a competition; it should be the first
thing said.

The test that keeps this honest is `the_splitter_claims_this_goal_which_is_why_
the_guard_is_needed`: it asks the **production registry** through
`MethodRegistry::find`, at a top-level site, which method would take
`Have { petroleum-gas, 100, Anyone }`. The answer is `split-across-bots`. So
the guard, and nothing about the fixture, is what removes the share.

`Goal::Produced` is deliberately **not** covered. "Cause 100 petroleum-gas to
come into existence" is a meaningful request an oil refinery answers, and makes
no claim about an inventory. It is answered by `products::NoProducer` instead.

### The count is the assertion that proves nothing was divided

`FluidRefusal::NotCarryable` carries the count the **caller** asked for — 100.
A guard that fired on the share-divided count would still produce a
fluid-shaped refusal and would still read plausibly. Break 2 below is exactly
that mutation, and it is caught.

## `recipe_for` does not answer for fluids, and it should not

The brief asked whether `recipe_for` now answers for fluids. The honest answer
is that **the question is about the wrong function**, and the measurement
behind that is already in the tree.

`method::util::recipe_for` is `state.base().recipes.get(item)` — a lookup keyed
by **recipe name**. There is no recipe called `petroleum-gas`, so it answers
`None`, and it would answer `None` for `iron-ore`, `coal`, `wood`,
`solid-fuel` and 58 other products too. Measured against
`crates/core/tests/live-2.1.17-world-snapshot.json`: 394 of 662 recipes are not
named after any of their own products, and 62 products have no same-named
recipe at all.

It is nonetheless **exact over the set this planner can reach**: within
`crafting` and `smelting`, the same capture says the product→recipe map is
one-to-one, total and name-preserving — 194 products, none made by two recipes,
none lacking a same-named recipe, no recipe with more than one product. So
rewriting it would be churn across 30+ call sites with no behavioural gain, and
would build a 662-recipe index per call.

**The honest product lookup already existed and had no caller.** It does now.
`crates/planner/src/products.rs` — `ProductIndex::sole_recipe_producing`,
keyed by product, one-to-many in both directions, refusing in three named tiers
— reaches production through `NoProducer`, registered **last** in
`registry_for` and `default_registry`. It claims nothing and satisfies nothing
(`applicable` and `claims` are hard `false`), so it cannot take a goal from a
method above it; it can only replace the driver's unactionable *"no method can
satisfy goal"* with the recipes that produce the item and the category no
machine here runs.

So: the planner now answers, for a fluid, which recipes produce it and in what
category. It answers through the index built for the job, not through a
name-keyed `get` that was never asking the right question.

### Registering it changed three refusal messages, and that is stated rather than absorbed

`NoProducer` is a last-resort speaker, so it speaks wherever a better-informed
method declined to. Three existing tests moved, none of them silently:

| test | was | now |
|---|---|---|
| `an_item_nothing_yields_is_still_no_applicable_method` | `NoApplicableMethod` | `ProductNotMakeable`, adding *"no prototype table mentions the name at all — check the spelling"* |
| `power::…the_wood_a_long_pole_run_wants` (bare-map control) | `NoApplicableMethod` | `ProductNotMakeable`, adding *"it is an item, so it comes out of the ground"* |
| `products::no_producer_driver_tests` (×2) | `Have { petroleum-gas }` | `Produced { petroleum-gas }` — the fluid guard now pre-empts the `Have` form, correctly |

The first two were asserting a **variant** where their stated intent was a
**property**: that expansion *fails* rather than quietly planning something
short. Both now assert the property, the item's name, and the new diagnosis.
No information was lost in either — both messages name the item, and the new
one says why.

## The two changes are independent, and only one of them meets the requirement

This is the finding worth carrying, and it came out of falsification rather
than reasoning.

With the guard disabled but `NoProducer` registered, `have:petroleum-gas:100`
still does **not** print "a share sized for bot 1" — it prints
`ProductNotMakeable { product: "petroleum-gas", … }`, because `NoProducer`
answers about the *product* and never mentions the share.

So both changes remove the misleading sentence. But only the guard meets the
brief's actual requirement — **the bot-share division must not happen for a
fluid**. With `NoProducer` alone the splitter still divides 100 into four 25s,
still opens four chains, and then refuses for a reason about recipe categories
rather than about the shape of the request. The share is invisible in the
message and real in the expansion.

**A refusal that stops naming the wrong thing is not the same as a planner that
stops doing the wrong thing.** Had the falsification only checked the message
text, the guard would have looked redundant.

## Before and after

Same binary before and after each column, release, `--bots 1,2,3,4`. Baselines
re-measured rather than quoted.

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 / 21,784 | **176 / 21,784** |
| `producing:automation-science-pack:6` | 316 / 22,457 | **316 / 22,457** |
| `producing:logistic-science-pack:6` | 441 / 47,478 | **441 / 47,478** |
| `gathered:crude-oil` (explored map) | 2,115 / 317,283 | **2,115 / 317,283** |

Byte-identical, as they must be: no vanilla `crafting` or `smelting` recipe has
a fluid ingredient, so no plan the three baselines make ever states a fluid
`Have`, and `NoProducer` cannot claim a goal.

`have:petroleum-gas:100`, after:

```
Error:   × the goal did not expand: petroleum-gas is a fluid, and no character
  │ inventory can hold a fluid in any amount, so `have 100 petroleum-gas` is
  │ not unsatisfiable -- it is inexpressible. A fluid lives in a fluidbox (a
  │ pipe, a storage tank, or a machine's own), and this planner has no
  │ fluidbox concept, no inventory slot that addresses one, and no action that
  │ moves one. it would come from advanced-oil-processing, basic-oil-
  │ processing, coal-liquefaction, empty-petroleum-gas-barrel, light-oil-
  │ cracking (category chemistry, crafting-with-fluid, oil-processing), which
  │ no character can craft
```

**100, not 25. No bot named. And the producers come off the world's own recipe
table**, so a modded world names its own.

## How this is tested, and what the tests cannot prove

`crates/planner/tests/fluid_have.rs`, 8 tests, all confirmed present by name in
the full `cargo test --workspace` output (`grep -c` = 1 each).

The world is the **fixture terrain with the live capture's recipe and
item-prototype tables installed over the top**. The fixture's own recipe table
is hand-written and contains no fluid at all, so a classifier run over it would
be reading a table written by the same hands as the code. `petroleum-gas` is a
fluid in these tests because a real Factorio 2.1.17 RCON reply said so.

Three premise tests run before any claim, so a green result cannot be vacuous:
the capture itself declares the fluid (read off the JSON fields, not through
the classifier); the splitter really would claim the goal; and ordinary item
goals still plan on this world.

### Falsification

Six breaks, **one at a time**, each substitution asserted to match exactly once
and each restored before the next. The counts are from the mutation scripts
themselves, which `assert` on the count and refuse to write otherwise.

| # | break | subs | result |
|---|---|---|---|
| 1 | guard removed from the driver | 1 | 2 red |
| 2 | guard reports the share-divided count (`count/4`) | 1 | 1 red |
| 3 | guard fires for every `Have`, not only a fluid | 1 | 2 red (the discrimination tests) |
| 4 | guard covers `Produced` too | 1 | 1 red |
| 5 | `NoProducer` unregistered | 2 (both registries) | 1 red |
| 6 | `ProductIndex::recipes_producing` regressed to a recipe-name lookup | 1 | 2 red |

**Break 4 came back green the first time and it was not a passed check** — the
`assert` on the substitution count failed (the pattern occurs twice in the
file, once in a test module), so the run tested unmodified code. This is
`docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`'s first
cause, "the break was never compiled in", caught by the guard the note asks
for. Re-applied by line number, it went red.

### What none of this proves

- **That any fluid goal ever succeeds.** Nothing here plans a fluid; it only
  refuses one honestly. The rung below (`gathered:crude-oil`, 2,115 actions)
  still stands a pumpjack and a tank, and it is unchanged.
- **That `NoProducer`'s tier 1 is the message a reader wants** for a raw
  resource on a bare map. It replaced `NoApplicableMethod` in two places and
  the new text is strictly longer and strictly more specific; whether "mine or
  extract it" is *better* than silence there is a judgement, not a
  measurement, and it is called out here so it can be reversed cheaply.
- **The wording of `FluidRefusal::NotCarryable`.** It was written by an earlier
  task and adopted unchanged. Nothing outside this branch has checked that it
  is what a reader wants.

## Left open, deliberately: the design question

**Item 3 — "a fluid in a tank the plan built is expressible" — was not built,
and it needs a goal kind.**

The reason is precise. Every way of saying "this is held" in the planner today
routes through `Holder`, which has three variants — `Anyone`, `Bot(id)`,
`Share(id)` — and **all three name a character**. A fluidbox is a *place*, not
a bearer: it is addressed by entity position and fluidbox index, and it has a
capacity fixed by the prototype rather than by a stack size. There is no
honest way to write `Have { petroleum-gas, 100, Holder::??? }`, and inventing a
fourth `Holder` variant meaning "a tank over there" would put a place into a
type whose whole meaning is *who*.

So the shape it wants is something like:

```rust
Goal::Stored { fluid: ItemId, amount: u32, where_: FluidSite }
```

with `FluidSite` mirroring `Site` (`At(Position)` / `Near` / `Anywhere`), and
it is a genuine tenth kind: three exhaustive matches, the `KINDS` list, the Lua
binding, the OpenAPI seam. **That cost is correct here and was not correct for
the refusal**, which is the whole distinction this note is making.

Four things that must be settled before writing it, none of which is settled:

1. **What satisfies it.** `PlanState::available` sums character inventories and
   would answer 0 forever. A fluid's shortfall has to be read off a fluidbox,
   and nothing in `state.rs` addresses one.
2. **What fills it.** `method::gather` already sites a pumpjack and a storage
   tank on a field centroid with pipes — so the *physical* story has a start,
   and `Gathered` may be closer to the right goal than `Stored` is.
3. **What `FactorioEntityPrototype::fluidbox_prototypes` actually says.** The
   field is on the wire and nothing in the planner reads it. Its shape decides
   whether a capacity and a connection point can be derived from prototypes —
   which this project requires — or whether they would have to be tabulated,
   which would be a mod-compatibility defect.
4. **Whether any of it is reachable without a fluid-network model.** A tank the
   plan built, standing next to the pumpjack that fills it, plausibly is. A
   fluid moved anywhere else is not, and pipes, flow direction, pressure and
   throughput between fluidboxes are a large subsystem that is **not** what is
   blocking the oil ladder.

The ladder as it stands: `researched:oil-gathering` 1,587 actions,
`researched:oil-processing` 2,012, `gathered:crude-oil` 2,115 (billing its own
unlock). Everything up to the fluid plans. The fluid itself now refuses for the
right reason, in words that name the fluid, the count the caller asked for, and
the five recipes that would make it.

---

## Answering one of the four unsettled questions: capacity is NOT derivable today

Measured 2026-09-07 against `crates/core/tests/entity-prototype-fixtures.json`
(the live 2.1.17 capture), immediately after this branch merged. The note above
lists *"what `fluidbox_prototypes` actually says — its shape decides whether
capacity can be derived rather than tabulated"* as one of four things to settle
before `Goal::Stored`. It says less than hoped:

```
storage-tank.fluidbox_prototypes[0] keys:  pipe_connections, production_type
storage-tank prototype keys:               collision_box, collision_mask,
                                           crafting_speed, entity_type,
                                           fluidbox_prototypes,
                                           max_underground_distance,
                                           mine_result, mining_speed,
                                           mining_time, name
```

**There is no `volume` anywhere on the wire.** So a capacity number could only
be *tabulated*, which under this project's standing rule is a
mod-compatibility defect — the same shape as `pole_supply_half_extent` and the
smelt rates that were confidently wrong until the world-record base falsified
them. `LuaFluidBoxPrototype.volume` is the field the mod would have to send.

**What we DO have is the connection geometry and the direction**:
`production_type` is `input` / `output` / `input-output`, and
`pipe_connections.positions` gives the tile offsets, per prototype —
`pumpjack` one output box, `oil-refinery` separate input and output boxes,
`storage-tank` and `pipe` input-output. That is enough to decide **where** a
tank or refinery can be joined, which is the siting half of `Goal::Stored`. It
is not enough to say **how much** it holds.

So the four questions reduce cleanly: siting is answerable from data we already
receive; capacity needs one more mod field before it can be honest. Do not
close that gap with a table.
