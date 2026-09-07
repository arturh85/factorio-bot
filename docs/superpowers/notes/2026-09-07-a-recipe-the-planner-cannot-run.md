# The category gate is not a scope decision — it is the edge of what the wire can name

2026-09-07, branch `a-recipe-the-planner-cannot-run`, off `4aa7564c`.
Sequel to `2026-09-07-a-fluid-is-not-carried.md`.

## The question, and the short answer

`produced:petroleum-gas:100` refuses because *"none is in a category this
planner runs (crafting, smelting)"*. The brief asked where that gate lives and
**why it is restricted — a deliberate scope decision, or an accident of what
was needed first**.

**Neither.** It is the exact set of recipe categories whose *machine* the
planner can name at all. Widening it is not a planner change; it is a mod
field.

## Where the gate lives

There is no gate. There are two methods, each of which admits one category
because each stands one machine up:

| | file | admits | machine it names |
|---|---|---|---|
| `Craft` | `method/have.rs:3343` | `crafting` | the character's own hands |
| `Smelt` | `method/have.rs:1191`, `method/produce.rs:179` | `smelting` | the string literal `stone-furnace` |

`products::Categories::planner_runs()` is a **mirror** of those two, built from
`util::CRAFTING_CATEGORY` and `util::SMELTING_CATEGORY` rather than from
literals so that it cannot drift. It is what prints the refusal; it decides
nothing. Grepping for a gate finds the mirror, which is why the brief's
`recipe_gate` guess landed elsewhere — `recipe_gate` is about `enabled` and
research, not about categories at all.

So "widen the gate" means "write a third method". A method has to name a
machine. For `oil-processing` the honest name is `oil-refinery`.

## Why that name cannot be derived, measured

`crates/planner/tests/oil_category_gate.rs::the_world_model_cannot_name_the_machine_for_a_category`,
against `crates/core/tests/live-2.1.17-world-snapshot.json`:

```
oil-refinery             entity_type=assembling-machine  crafting_speed=1
chemical-plant           entity_type=assembling-machine  crafting_speed=1
assembling-machine-1     entity_type=assembling-machine  crafting_speed=0.5
centrifuge               entity_type=assembling-machine  crafting_speed=1
electromagnetic-plant    entity_type=assembling-machine  crafting_speed=2
stone-furnace            entity_type=furnace             crafting_speed=1
```

Every field an entity prototype carries on our wire:

```
collision_box  collision_mask  crafting_speed  entity_type
fluidbox_prototypes  max_underground_distance  mine_result
mining_speed  mining_time  name
```

(plus the fields added since that capture: `mining_drill_radius`,
`supply_area_distance`, `maximum_wire_distance`, `electric_energy_usage`,
`max_energy_production`, `resource_categories`, and `fluidbox_prototypes.volume`.)

**Twelve prototypes declare a crafting speed and share one `entity_type`.**
"It crafts" is knowable; "what it crafts" is not. There is no field, and no
combination of fields, from which `oil-processing → oil-refinery` follows.

**And that is exactly why the two admitted categories are the two that exist.**
`crafting` is the character, which needs no prototype at all. `smelting` is the
one category whose machines are a distinct `entity_type` — and even there the
planner does not derive it, it writes `stone-furnace` by hand and gets away with
it because a stone furnace is what a player starts with. Those two categories
are not a scope decision anybody recorded. They are the two you can reach
without the missing field.

Writing `oil-refinery` into a method would be the same shape as
`pole_supply_half_extent` and the hand-kept smelt rates: a table that is right
for vanilla and a mod-compatibility defect by the project's standing rule.

### The handover

**`LuaEntityPrototype.crafting_categories` exists in 2.1.17 and does not cross
our bridge.** Checked against `workspace/client1/doc-html/runtime-api.json`,
not recalled:

```
ATTR LuaEntityPrototype crafting_categories
     read type: dictionary string -> literal true
```

It is an **attribute**, not a method — so unlike `get_crafting_speed()`,
`get_supply_area_distance()` and `get_max_wire_distance()`, the plain read is
the correct one and the `pcall` has nothing to swallow. `mods/BotBridge/types.lua`
already sends `resource_categories` for the mining half of exactly this
question ("a resource carries a category, a miner carries the categories it
supports, and mining is allowed iff the former is in the latter"). This is the
crafting half of the same rule, and it is missing.

Two files, both outside this branch's boundary:

* `mods/BotBridge/types.lua` — beside `resource_categories`, near line 730.
* `crates/core/src/types.rs` — `FactorioEntityPrototype::crafting_categories:
  Option<Vec<String>>`, with `None` meaning *the sender did not say* and an
  empty vec meaning *this entity crafts nothing*, the same
  `None`-versus-`Some(vec![])` split `resource_categories` already documents.

The test `the_world_model_cannot_name_the_machine_for_a_category` scans the
**raw JSON** for a key containing `crafting_categor` and asserts none exists —
deliberately not through the struct, because serde drops unknown keys silently
and a capture that carried the field would deserialise identically. The day the
field ships and the capture is regenerated, that test goes red with a message
saying the wall is down. It carries a paired non-vacuous control: the same scan
for `crafting_speed` finds 18.

## What `basic-oil-processing` actually looks like

The brief's premise was right, and it is now an oracle rather than a belief.
Read off the capture:

```
basic-oil-processing   category oil-processing   energy 5   enabled false
  in :  crude-oil       fluid  100
  out:  petroleum-gas   fluid   45
```

**One fluid in, one fluid out.** The multi-output problem is real but belongs
to `advanced-oil-processing` (3 products, asserted in the same test as the
control) — it is not what blocks the first rung.

## The two walls behind the gate, located by experiment

Both are strictly *behind* the category gate, so no measurement on an
unmodified world can reach them. `crates/planner/tests/oil_category_gate.rs`
therefore edits the live capture's recipe table so an oil recipe lands in a
category the planner already runs, and asks. **Nothing in `src/` changes; the
world moves.** These are characterisation tests: they pin what the planner does
today so the next agent sees the wall move rather than discovering it live.

### Wall two — a fluid ingredient is asked of a bot as if it were an item

`plastic-bar` (coal 1 item + petroleum-gas 20 fluid → 2 plastic-bar) moved into
`crafting` and enabled. `have:plastic-bar:2`:

```
petroleum-gas is a fluid, and no character inventory can hold a fluid in any
amount, so `have 20 petroleum-gas` is not unsatisfiable -- it is inexpressible.
```

**`20` is the finding.** That is `plastic-bar`'s own ingredient amount, so the
expansion really did walk the bill and ask a character for the fluid:
`method::util::ingredients_of` maps `(name, amount)` and drops the declared
type, so a fluid becomes an ordinary `HasItem` at a bot, and
`PlanState::available` sums character inventories and would answer 0 forever.
The previous branch's `Have` guard catches it one level down — which is the
right behaviour and also the reason the wall was invisible.

It is **20, not 5** — the four-bot splitter did not divide it. Asserted, because
a refusal merely *naming* petroleum-gas would be equally explained by the
category gate still firing.

`substance::split_bill` is the prepared seam for fixing this. It is correct, it
has its own unit tests (break 3 below kills exactly those two), and it still has
**no production caller** — because no recipe in a runnable category has a fluid
ingredient, so there is nothing to wire it to until the gate opens. Do not wire
it speculatively; wire it in the commit that adds the third method.

Paired control on the same world construction:
`the_same_recipe_without_its_fluid_plans_fine` drops the petroleum-gas from the
bill (asserting the bill went 2 ingredients → 1) and the recipe plans, with a
non-empty network from the same call. Without it, the refusal above is equally
explained by "recategorising breaks a recipe" or "this world plans nothing".

### Wall three — opening the category makes the *message* worse

This one was not predicted and is the most useful thing here.

`basic-oil-processing` moved into `smelting` and enabled.
`produced:petroleum-gas:45`:

```
no method can satisfy goal: produce 45 petroleum-gas
```

The five-recipe diagnosis is **gone**. `products::NoProducer` speaks only when
every producer is in a category the planner cannot run — that is its tier 2. The
moment one producer becomes runnable it declines, no other method claims the
goal (a furnace has no fluidbox and this planner models none), and the driver
falls through to its own unactionable sentence. That is precisely the shape
`products.rs`'s own module doc calls *"the defect this exists for"*.

**Whoever opens the category gate for real will hit this message first**, and it
names neither the fluid, nor the furnace, nor the fluidbox that is missing.

Deliberately **not fixed**. A fourth `ProductRefusal` tier — "exactly one
runnable recipe produces it and no method took it" — is unreachable on any
vanilla world today: `solid-fuel`, `plastic-bar`, `concrete` and `uranium-235`
were each checked against `workspace/scripts/map.json` and all four get the good
tier-2 message. Code written for a world that does not exist yet is how the
`method::connect` geometry defect survived four reviews. The test carries the
before/after on one world so the tier can be added with evidence when it becomes
reachable.

## Fluid segments: the coordinator's correction, and where it lands

Mid-task the coordinator corrected the mental model: Factorio 2.0 makes a
connected pipe run **one segment** with one fluid level, one capacity and a
queryable id — no per-pipe throughput, no length penalty, no pressure gradient.
Verified on disk in `workspace/client1/doc-html/runtime-api.json`: ten
`*_fluid_segment_*` methods on `LuaEntity`, including
`get_fluid_segment_id`, `get_fluid_segment_capacity` and
`get_fluid_segment_extent_bounding_box`.

Three things follow, and one of them supersedes a claim in the previous note.

1. **`2026-09-07-a-fluid-is-not-carried.md`'s closing section is stale on
   capacity.** It says a tank's capacity "is not derivable today, do not close
   that gap with a table". That was true of the *prototype* — and two things
   have changed since: `fluidbox_prototypes.volume` now ships
   (`get_volume()`, in `mods/BotBridge/types.lua`), and
   `get_fluid_segment_capacity` answers for a whole connected run.
2. **Connectivity is an id comparison, not a graph walk** — the same shape as
   the electric ledger's union-find, so there is a precedent to follow rather
   than a subsystem to invent.
3. **None of it crosses our bridge.** The mod sends `fluidbox_prototypes`
   (connection offsets, `production_type`, and now `volume`) and nothing about
   segments. And the segment API is a **runtime** API: it answers about a live
   game, not about a dump — so it is unavailable to the offline `plan` path
   that every measurement in this project is taken on. That is a second, separate
   reason it cannot be reached for now.

**This does not change the rung.** Segments answer the *storage* question
(`Goal::Stored`, still an open owner decision). The blocker measured here is
upstream of all of it: the planner cannot name the machine.

## The ladder, and the order the walls must fall in

```
researched:oil-processing   PLANS    2,012 actions
have:pumpjack:1             PLANS    1,595
have:oil-refinery:1         PLANS    2,095
gathered:crude-oil          PLANS    2,115 / 317,283   (explored map)
produced:petroleum-gas:100  REFUSES  <- category gate

  wall 1  name the machine        needs LuaEntityPrototype.crafting_categories   MOD
  wall 2  fluid in the bill       needs substance::split_bill wired + a fluid
                                  precondition that is not HasItem-at-a-bot      PLANNER
  wall 3  fluid product landing   needs a fluidbox concept, or Goal::Stored      OWNER
```

Wall 1 is first, it is small, and **nothing in `crates/planner` can start
without it.** Every machine on the oil path already plans; the planner can build
an oil refinery and cannot know that an oil refinery is what runs oil.

## Measurements

Release binary from `4aa7564c` + this branch, `--bots 1,2,3,4`. Only a test file
was added — no `src/` change — so the four baselines are unchanged by
construction, and were re-measured on the same binary rather than quoted.

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 / 21,784 | **176 / 21,784** |
| `producing:automation-science-pack:6` | 316 / 22,457 | **316 / 22,457** |
| `producing:logistic-science-pack:6` | 441 / 47,478 | **441 / 47,478** |
| `gathered:crude-oil` (explored map) | 2,115 / 317,283 | **2,115 / 317,283** |

`produced:petroleum-gas:100` refuses identically before and after, with the same
five-recipe diagnosis.

`nix develop -c cargo test --workspace` — **exit 0**, 896 planner tests plus the
rest, on a tree whose only modification was this file's test.

### Falsification

Four breaks, one at a time, each script-asserting its substitution matched
**exactly once** and restoring before the next.

| # | break | subs | red in this file |
|---|---|---|---|
| 1 | fluid `Have` guard removed from `expand_goal_body` | 1 | 1 — `a_fluid_ingredient_recurses_…` |
| 2 | `Categories::planner_runs()` → `Categories::any()` | 1 | 1 — `opening_the_category_silences_…` |
| 3 | `split_bill` files a declared fluid as an item | 1 | **0** |
| 4 | `SubstanceTable::is_fluid` always false | 1 | 1 — `a_fluid_ingredient_recurses_…` |

**Break 3's green is the expected answer and was checked rather than assumed.**
Re-run across the whole crate it turns **2 red** —
`substance_tests::a_bill_separates_what_a_bot_could_carry` and
`…falls_back_to_the_table_for_a_blank_type`. So `split_bill` is tested and
uncalled, not untested; this file cannot see it because nothing in a plan
reaches it. That is the same fact wall two records, arrived at from the other
side.

Breaks 1 and 4 kill the same single test, which is honest rather than redundant:
that test depends on both the classifier saying `petroleum-gas` is a fluid and
the guard acting on it, and it is a **new, independent witness** for the guard —
`tests/fluid_have.rs` exercises the top-level `Have`, this one exercises the
ingredient path two levels down.

The two premise tests are oracles over the checked-in capture and have no
production code to mutate; `the_world_model_cannot_name_the_machine_for_a_category`
is falsified by the capture gaining the field, which is exactly the event it is
there to announce.

## What this does not prove

* **That the third method is easy once the field ships.** It establishes only
  that the machine becomes nameable. Siting a refinery, pipe-joining it to a
  pumpjack and landing a fluid product are three further pieces, and the last
  one is an owner decision.
* **That `oil-refinery` is the only machine that would come back.** With
  `crafting_categories` on the wire, `oil-processing` may resolve to several
  prototypes on a modded world, and `ProductRefusal::Ambiguous` is the existing
  precedent for how to refuse rather than pick.
* **That the tier-4 refusal is wanted.** Wall three is pinned, not fixed, on the
  argument that it is unreachable today. If someone opens the gate behind a flag
  before the mod field lands, that argument expires.
