# A fluid is not an item — and it is not a number either

2026-09-06, branch `a-fluid-is-not-an-item`, worktree, no Factorio run.
Written **before** the code, as the brief asked. It builds on
`2026-09-06-oil-survey.md` §1–§2 and does not re-derive it; §0 below records
the survey claims I re-checked and the two figures I measured myself.

---

## 0. What I verified before designing, and what I measured

Every claim below is against the tree at `7ebe707d` and against real game
data — `workspace/scripts/map.json` (the seed-31337 t=0 dump) and
`crates/core/tests/live-2.1.17-world-snapshot.json` (a byte-for-byte RCON
reply from a real Factorio 2.1.17 game, checked in by someone else, which
matters — see §6).

| survey claim | verified | how |
|---|---|---|
| `pub type ItemId = String` | yes | `crates/planner/src/ids.rs:6` |
| `ingredients_of` drops `ingredient_type` | yes | `method/util.rs:651-661` — it maps `(i.name, i.amount)` and nothing else |
| `FactorioIngredient.ingredient_type` / `FactorioProduct.product_type` carry the game's `"item"`/`"fluid"` | yes | `crates/core/src/types.rs:318-337`, and both are populated in both real captures |
| `ItemId` blast radius | **103 references across 12 files** | `grep -rn ItemId crates/planner/src` |
| `character.crafting_categories = {crafting, hand-crafting}` and `HandCraft` gates on `CRAFTING_CATEGORY` | yes | `entity/entities.lua`, `method/util.rs:668` |

Two things the survey did not measure, which I did, because the whole design
turns on them:

**(a) The two ways to tell a fluid from an item agree exactly, on both
captures.** Classifying every name that appears in any recipe by its declared
`ingredient_type`/`product_type`, and separately asking whether the name is in
`item_prototypes`:

| | `map.json` | `live-2.1.17` |
|---|---|---|
| recipes | 662 | 662 |
| item prototypes | 342 | 342 |
| distinct names declared `fluid` | **21** | **21** |
| names with a **blank** declared type | **0** | **0** |
| names declared *both* item and fluid | **0** | **0** |
| fluid-declared names also in `item_prototypes` | **0** | **0** |
| item-declared names *missing* from `item_prototypes` | **0** | **0** |

The 21: `ammonia, ammoniacal-solution, crude-oil, electrolyte, fluorine,
fluoroketone-cold, fluoroketone-hot, heavy-oil, holmium-solution, lava,
light-oil, lithium-brine, lubricant, molten-copper, molten-iron,
petroleum-gas, steam, sulfuric-acid, thruster-fuel, thruster-oxidizer,
water`.

Two independent oracles, zero disagreements. That is what makes a *classifier*
a safe thing to build here — and it is also why the classifier ships with a
`disagreements()` method, so a later capture that breaks the agreement says so
instead of silently picking a side (§6).

**(b) No vanilla recipe amount is fractional.**
`grep -cE 'amount *= *[0-9]+\.[0-9]' workspace/data/base/prototypes/recipe.lua`
→ **0**. Every fluid amount on the battery chain is an integer: 100 crude-oil,
45 petroleum-gas, 30 water, 30 gas, 100 water, 50 acid, 20 acid, 20 gas.
`RawFactorioIngredient.amount` is already an `f64` that `round_to_u32`s on the
way in (`types.rs:396`), and it has never had anything to round.

This single measurement is why the design below has **no float in it at all**.

---

## 1. The question the brief asks: quantity, or storage?

> Is a fluid a different *kind of quantity*, or a different *kind of storage*?

**Storage.** And the distinction is not academic — it decides which type gets
written.

The quantity story is nearly a non-story. This planner counts in `u32`; every
fluid amount the game will hand it is a small integer (§0b); the game's own
`double` is already narrowed at the crate boundary, deliberately and with a
comment. A `Fluid(f64)` quantity would model a difference that does not exist
in the data and would import a float into a crate whose determinism contract
is the reason anyone trusts a replanned run.

The storage story is total. A fluid has **no addressable place in this
planner's world**:

- no character can hold one — `BotState.inventory: BTreeMap<ItemId, u32>` is a
  character main inventory and the game will not put a fluid in one;
- `available()` (`state.rs:2153`) sums character inventories, so a fluid is
  permanently `available == 0` — not "scarce", *unreachable*;
- `InventorySlot` has seven variants (`action.rs:549`) and **none addresses a
  fluidbox**, and no Factorio API shape would let one — a fluidbox is
  addressed by index and pipe connection, not by an inventory define;
- the two verbs that move quantity, `Insert` and `Remove`, bottom out in
  `LuaPlayer.insert` / `remove_item`, which cannot take a fluid at all;
- `withdraw_slot` returns `None` for `storage-tank`, `pipe` and `pumpjack`
  (`state.rs:654`), so no fluid container can ever become a `Buffer`.

So: **a bot cannot carry 100 crude-oil, and a bot also cannot carry any crude
oil in any amount.** The failure is not in the number. Change the number type
and every one of the six bullets above is still true.

The corollary, which is the whole design: what the planner needs first is not
the ability to *count* a fluid but the ability to *say* that a name is one —
and then to refuse.

---

## 2. What is chosen

A new, narrow, pure module: `crates/planner/src/substance.rs`.

```rust
pub enum Substance { Item, Fluid }              // Copy, Ord

pub struct SubstanceTable(BTreeMap<String, Substance>);
impl SubstanceTable {
    fn from_parts(recipes, item_prototype_names) -> Self
    fn from_world(&FactorioWorld) -> Self
    fn from_state(&PlanState) -> Self
    fn of(&self, name) -> Option<Substance>      // None = UNKNOWN, never guessed
    fn is_fluid(&self, name) -> bool             // positive evidence only
    fn fluids(&self) -> impl Iterator<Item = &str>
    fn disagreements(...) -> Vec<Disagreement>   // the instrument's self-check
}

pub struct Bill { pub items: Vec<(String, u32)>, pub fluids: Vec<(String, u32)> }
pub fn split_bill(&SubstanceTable, &FactorioRecipe) -> Bill   // the non-lossy `ingredients_of`

pub enum FluidRefusal { NotCarryable { fluid, count, source: FluidSource } }
pub enum FluidSource { None, Recipes { names, categories }, Resource { entity } }
```

`ItemId` stays `String`. Nothing existing changes type. The narrow type is
`Substance`, a fact about a **name**, not a new arithmetic.

### Three properties that are load-bearing

**Determinism.** No float. `BTreeMap` and `Vec` in insertion-independent
order (names are sorted). `Substance` derives `Ord` on variant order. Nothing
here has an iteration order that depends on hashing, and nothing compares a
float, so `total_cmp` never arises — which is a stronger answer to the brief's
float rule than introducing a float and ordering it carefully.

**Unknown is a third answer, and it is not "item".** `of()` returns
`Option<Substance>`. A name with no evidence either way is `None`. This
matters twice: the crate's hand-built fixture worlds are full of names no
prototype table mentions, so *treating unknown as fluid would refuse every
existing test*; and treating unknown as a confident `Item` is exactly the
"confidently about the wrong object" shape the fixtures note names as the
worst thing a record can be. Callers get to decide, and today every caller
would decide "unknown behaves as it does now", which is a *stated* fallback
rather than a silent one.

**Evidence is positive and ordered**, the `extract.rs` shape:

1. the name is in `item_prototypes` → `Item`. A fluid is never in the item
   table; this is the game's own statement.
2. some recipe declares it `"fluid"` → `Fluid`.
3. some recipe declares it `"item"` → `Item`.
4. otherwise → `None`.

Tier 1 before tier 2 is a choice and could be argued the other way. It is
safe only because §0a measures the intersection at **zero on both captures**,
and `disagreements()` exists so that a capture where it stops being zero is
reported rather than resolved by tier order. If that day comes, the right fix
is to look at the data, not to swap the tiers.

### The refusal

One refusal, not four, and that is deliberate. `extract.rs` refuses in four
ordered tiers because a reader can *act* on each: chart the ground, then get a
drill, then research it, then wait for the method. For a fluid asked of a
character there is exactly one wall and it is first, always:

> `Goal::Have{petroleum-gas, 100}` — petroleum-gas is a fluid, and no
> character inventory can hold a fluid in any amount, so this goal is not
> unsatisfiable, it is inexpressible. A fluid lives in a fluidbox — a pipe, a
> storage tank, or the machine's own — and this planner has no fluidbox
> concept, no `InventorySlot` that addresses one, and no verb that moves one.
> petroleum-gas would come from `basic-oil-processing` (category
> `oil-processing`), which needs an oil refinery and cannot be hand-crafted
> either.

The last sentence is the diagnostic detail that would otherwise be tiers 2–4:
where the fluid *could* come from, and the crafting-category wall behind it.
It is attached to the one refusal rather than reached by walking past it,
because from `Goal::Have` those tiers are **unreachable** — you can never get
past tier 1 by changing the world. Emitting them as separate refusals would
imply a ladder that does not exist.

---

## 3. What was rejected, and why

**(A) `enum Quantity { Items(u32), Fluid(OrderedF64) }`, replacing every
count.** Rejected. It fixes the number, which was not broken (§0b), and
leaves all six storage facts of §1 exactly as they were — a plan could then
represent "a bot holds 100.0 crude-oil" *precisely*, which is a more
convincing way to be wrong. It touches every count in the crate. And it puts
a float on the critical path of a crate whose contract is determinism, in
exchange for a precision no vanilla recipe asks for. If a mod ever ships
`amount = 2.5`, the honest fix is at `crates/core/src/types.rs:396` where the
rounding already happens and is already documented, not here.

**(B) `enum ItemId { Item(String), Fluid(String) }`.** Rejected, and it is
the most tempting one. 103 references, 12 files, six of them contested; but
the real objection is semantic, not mechanical. `ItemId` is the **key of a
character's inventory map**. Making it able to name a fluid says a fluid is
the kind of thing that could appear in that map. It would compile, every
`BTreeMap<ItemId, u32>` would accept `Fluid("crude-oil")`, and the planner
would go on doing exactly what it does today with a type that now claims the
opposite. The brief's own suggestion — a new type alongside — is right, and
the reason is that the alias is a *storage key*, not a name.

**(C) An eighth `InventorySlot::FluidBox` variant.** Rejected as premature and
as a lie about the game. `InventorySlot` maps onto Factorio inventory defines;
fluidboxes are not one, and the mod's `rcon_insert_to_inventory` has nothing
to extend. This variant only becomes honest alongside a fluid-link geometry
predicate and a new `ActionKind`, which is rung O4 of the survey and is not
this task. Adding it now would make `Insert{FluidBox}` expressible in a plan
the executor would dispatch and the mod would fail.

**(D) A `Goal::Stored { fluid, where }` now.** Rejected as inventing. The
survey names it as the honest analogue of `Have` for a fluid and it probably
is — but a goal with no method, no site, no `available()` and no verb is a
hypothesis, and this repo has just paid for a module with no caller
(`connect_steps`, four clean reviews, unbuildable bill). `Goal::Extracted`
already exists in exactly that state and is doing the useful half of the job
by refusing well. One such placeholder is enough.

**(E) Doing nothing until a fluid method exists.** Rejected because the
current behaviour is the bad kind of silence. `recipe_for(state,
"petroleum-gas")` returns `None` — recipes are stored by *recipe name* and
looked up by *product name*, and there is no recipe called `petroleum-gas` —
so every method *declines* and the driver reports the generic "no method can
satisfy goal", which reads as "this is hard" rather than "this is not a thing
a bot can do". Four separate mechanisms in this repo have been found
reporting nothing while broken; this is a fifth in waiting.

---

## 4. What this does NOT do

Stated plainly so no reader has to infer it:

- **It does not move any fluid, plan any pipe, or count anything in a
  fluidbox.** There is no fluid simulation here and none is wanted.
- **It does not make `researched:oil-processing` or `have:plastic-bar:2`
  plannable.** They still refuse. The change is that they would refuse by
  naming the fluid and the wall, instead of "no method can satisfy".
- **It does not touch the crafting-category wall**, which is a separate and
  *earlier* wall: `chemistry`, `oil-processing` and `advanced-crafting`
  recipes cannot be hand-crafted whether or not their ingredients are fluids,
  and `engine-unit` (`advanced-crafting`, no fluid in sight) hits it first.
  The refusal text names the category precisely so that this wall is visible
  rather than confused with the fluid one.
- **It changes no plan.** All three known-good baselines are byte-identical
  (§7). It cannot change one: nothing calls it yet.

---

## 5. The wiring this needs, which I did not do

Every file below is contested or outside the files this task owns, so the
module ships **with no caller in production code**. That is stated here rather
than discovered later, because this repo's own record says a module with no
caller is a hypothesis, and my tests were written by the same hand as the
code (§6 says what mitigates that and what does not).

Three edits, smallest first:

1. **`crates/planner/src/error.rs`** — one variant:
   ```rust
   #[error("{0}")]
   FluidNotItem(#[from] crate::substance::FluidRefusal),
   ```
   *Blast radius:* `PlannerError` is matched exhaustively in exactly one place
   in the workspace — `crates/scripting_lua/src/globals/goal/mod.rs:355`
   (per the fixtures note, which enumerates them). A new variant needs an arm
   there and a `cargo test --workspace`, not `-p factorio-bot-planner`.

2. **`crates/planner/src/method/have.rs`** — in `Have`'s expansion, before any
   method is asked: if `SubstanceTable::from_state(state).is_fluid(item)`,
   return `FluidRefusal::NotCarryable`. One `if`, at the top.
   *Why it must be there and not in a `Method::refusal`:* the driver asks
   every method for a refusal and picks one; a fluid `Have` should not be a
   competition, it should be the first thing said.

3. **`crates/planner/src/method/util.rs`** — `ingredients_of` stops discarding
   `ingredient_type`, or its callers move to `substance::split_bill`. This is
   the survey's "cheapest single change in the whole survey" and it is the one
   that makes `HandCraft` stop emitting `Condition::HasItem{20 sulfuric-acid}`
   at a bot. It is *only* safe once (1) and (2) exist, because today that
   condition is the only thing standing between a fluid bill and a plan that
   quietly assumes a bot can carry it.

A fourth, larger and separate: a **product → recipes** index, so that
`recipe_for` stops silently returning `None` for every name that is not also a
recipe name. That is not a fluid problem — it is why a fluid goal fails
*quietly* today — and it wants its own task, because multi-product recipes
(`advanced-oil-processing`, three fluid outputs) have no home in the current
one-recipe-per-name index at all.

---

## 6. How this is tested, and what the tests cannot prove

The classifier is an instrument, and the fixtures note is explicit that a new
instrument's first reading is evidence about the instrument. So:

- **The primary test runs against `crates/core/tests/live-2.1.17-world-snapshot.json`,
  a real RCON capture I did not write and cannot have written to agree with
  me.** It asserts both halves, the way `recipe_probability.rs` does: the
  *premise* (the capture really does declare 21 fluids, so a passing
  classification is not vacuous) and the *claim* (the classifier finds exactly
  those 21, calls every one of the 342 item prototypes an item, and reports
  zero disagreements between its two evidence sources).
- **Hand-built fixtures exist only for the shapes the capture cannot show**:
  an unknown name, a blank `ingredient_type` (an older capture — the live one
  has none), and the refusal text.
- **I wrote both this code and those hand-built fixtures.** Naming it, as the
  rule asks. What they assume: that a blank type string means "no evidence"
  rather than "item", and that the refusal's wording is what a reader wants.
  Neither is checked against anything outside this branch.
- **Every test was falsified** — broken on purpose, with the substitution
  count asserted — and **counted by name in the full test output** before
  being believed. The exact commands and counts are in the branch's commit
  message and the task report.

What none of it proves: that the refusal fires. Nothing calls the module
(§5). The first real caller will ask a question I did not think of — that is
what happened to `connect_steps` — and until one does, the honest status of
this module is *"a hypothesis with good evidence about its inputs"*.

---

## 7. Baselines

`target/release/factorio-bot plan --world workspace/scripts/map.json --bots
1,2,3,4`, release build, commit `7ebe707d`:

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 actions, 21,784 ticks | 176 actions, 21,784 ticks |
| `producing:automation-science-pack:6` | 316 actions, 22,463 ticks | 316 actions, 22,463 ticks |
| `producing:logistic-science-pack:6` | 442 actions, 47,542 ticks | 442 actions, 47,542 ticks |

Identical, as it must be — the module has no caller. A *changed* baseline here
would have been the bug, not the feature.
