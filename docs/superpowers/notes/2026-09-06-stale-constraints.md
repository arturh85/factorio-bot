# A constraint one file removed, still stated as law in another

2026-09-06. A review of master (`3ec91e41`) for one defect class only:

> **A doc comment or constant states a constraint that was true when written,
> another file has since removed the constraint, and nobody updated the
> first file.**

The code compiles, every test passes, and the next reader reasons from a
premise that no longer exists. This is the documentation-shaped sibling of
`2026-09-06-fixtures-agree-with-their-code.md`: there a *check* agreed with
its subject; here a *claim* outlives its subject.

The case that motivated the review is the template. An agent priced poles
against pipes, verified both recipes correctly against the game's own data,
and then took *availability* from an assumption — "wood is the one item this
crate cannot make, so 8 poles ≈ 56 tiles of wire, ever" — and built a siting
recommendation on it. `method::have::Chop` had already removed that
constraint, and said so in its own doc. Checking the recipe was rigorous;
checking who could supply the ingredient was never done.

Ordered by consequence, worst first. Every claim is quoted; every
falsification names the file and line that makes it false **today**. Where a
finding rests on a fact about Factorio it is marked **verified** against
`workspace/server/data`, or marked unverified.

---

## 1. `method::power`'s siting rule rests on a wood budget that no longer exists

**The claim** — `crates/planner/src/method/power.rs:80-93`:

> **What binds is supply, not price.** Wood is the item this planner cannot
> make: a four-bot run starts with four [...] and no method in this crate
> mines a tree. Four wood is eight poles is about **56 tiles of wire, ever**
> [...] So the crossover is a **supply** crossover at roughly 56 tiles [...]
> teach it to mine a tree and the pole route wins everywhere on materials.
> Until then, wire short runs and pipe long ones.

Restated as fact five more times in the same file:

| where | what it says |
|---|---|
| `power.rs:277-280` | `POLE_CRAFT_WOOD`: *"**The binding constraint on the pole route** [...] no method in this crate makes wood."* |
| `power.rs:341-344` | `pole_run_items`: *"51 poles and 26 wood, which a four-bot run cannot obtain at all."* |
| `power.rs:543-545` | `PLANT_ADOPT_RADIUS`: *"**one wood**, of which a four-bot run has exactly four and can make no more"* |
| `power.rs:910-911` | `supply_for` tier 2: *"one of a run's four irreplaceable wood"* |
| `power.rs:3577-3612` | the test `the_crossover_is_wood_rather_than_price`, with `const STARTING_WOOD: u32 = 4;` and *"Four wood is the roster's whole supply."* |

**What falsifies it.** `crates/planner/src/method/have.rs:2937-3019` defines
`Chop` — *"Chop down what the world is standing on: a tree, a rock"* — and it
is registered in the **production** registry (`registry_for`) at
`have.rs:6520`, ahead of `Mine`. For wood it takes the goal with no cost
comparison at all: `Chop::applicable` (`have.rs:3135-3159`) returns `true` as
soon as `has_minable_source(item)` holds and `has_resource_patches(item)`
misses, and the comment at `have.rs:3146-3150` names wood as exactly the item
that misses on every call.

Two in-tree measurements agree — one in a file that was updated, one in a
file that was not:

- `have.rs:5721-5729`: *"A `wooden-chest` costs two wood, and a `Chop` of one
  dead tree yields exactly two: 372 ticks on the same map, walk included
  [...] `Chop` supplies the shortfall from a map with 6,656 trees standing."*
- `assemble.rs:668-680`: *"`Chop` has since made wood renewable —
  `have:small-electric-pole:4` on the real map chops a dead trunk for two
  more — so a pole here is priced like any other craft."*

The recipe arithmetic in `power.rs` is right, and I checked it against the
game rather than the module's prose: `small-electric-pole` is
`{wood, 1} + {copper-cable, 2}` yielding **2**
(`workspace/server/data/base/prototypes/recipe.lua:889-898`, **verified**).
Only the supply premise is wrong.

**What a reader wrongly does.** This module decides where a power plant
stands, and the 56-tile crossover is its stated governing rule — *"wire short
runs and pipe long ones"*. Two concrete wrong moves:

1. **The designed-but-unbuilt pipe router** (`power.rs:132-145`, and
   `docs/superpowers/notes/2026-09-06-piping-water-is-cheap.md`) is justified
   as the escape from a ceiling that is not there. The same doc says the pole
   route is **seven times cheaper per tile at every distance**
   (`power.rs:74-76`), so with wood renewable the honest conclusion is that
   wire wins everywhere and the pipe router is worth less than this file
   makes it look.
2. `supply_for`'s world-anchored fallback (`power.rs:924-935`) explicitly
   defers to this comparison for the cost of its pole run — *"the wire itself
   costs what [`pole_run_items`] says, and the module doc's 'wire the power,
   pipe the water' section is the comparison that governs it"*. The tier that
   exists to rescue a refused plan is costed against a phantom budget.

**The test cannot catch it.** `the_crossover_is_wood_rather_than_price` types
`STARTING_WOOD: u32 = 4` into itself (`power.rs:3598-3607`) and asserts
arithmetic about `pole_run_items` against that literal. Registering ten
`Chop`s would not move it. That is the fixtures note's "assertion weaker than
the claim", applied to a doc rather than to code.

**And it survived because nothing computes from it.** `pipe_run_plates` and
`pole_run_items` have **no callers outside their own file's tests** (grep
across `crates/`). *A module with no caller is a hypothesis* — and so is a
cost model with no caller.

---

## 2. `method::extract`'s "measured ceiling" was removed the same day, by a fix its own tests document

**The claim** — `crates/planner/src/method/extract.rs:91-119`, a whole titled
section:

> # The ceiling, measured: a well more than ~64 tiles from generation refuses
>
> **`crate::state`'s `POWER_SEARCH_RADIUS` is 64 tiles**, and it bounds the
> entities `PlanState::electric_supply_kw` will look at [...] it is binding
> here, and it is the reason this method's reach is ~64 tiles rather than
> [`MAX_POLE_RUN`]'s ~380.

with a measured table (`extract.rs:111-114`) asserting a well at ~121 tiles
**refuses**, and:

> `a_pole_chain_past_the_power_search_radius_is_not_seen` pins the fact
> itself [...] **Raising it is a change to `crate::state` and is not this
> module's to make.**

**What falsifies it.** `PlanState::electric_entities`
(`state.rs:4051-4090`) now finds every entity on the consumer's network *"by
**following the wire** rather than by drawing a bigger circle"* — one
`POWER_SEARCH_RADIUS` disc as a seed, then pole-by-pole expansion with **no
hop limit** (`state.rs:4059-4072`). `electric_supply_kw` reaches it through
`electric_network` (`state.rs:4124-4129`). The constant's own doc records the
fix (`state.rs:105-118`): *"**The bound was a disc around the consumer, while
the thing that carries power is the wire.**"* The commit was `4dfb1438`
("power follows the wire, not a radius around the consumer"), which touched
the tests in this file and left lines 91-119 alone.

**The test named in the doc does not exist.** `grep` finds
`a_pole_chain_past_the_power_search_radius_is_not_seen` in exactly two
places, both prose: `extract.rs:116` and
`docs/superpowers/notes/2026-09-06-a-pumpjack-on-a-well.md:182`. It was
inverted and renamed to
`a_pole_chain_carries_however_long_it_is_but_a_broken_one_does_not`
(`extract.rs:1545`), whose doc says so (`extract.rs:1531-1537`): *"That
happened on 2026-09-06 [...] so the ceiling is gone and this test is inverted
rather than deleted."*

A second stale statement sits five lines above that test — the test module's
own header, `extract.rs:1512-1520`: *"a pole run is useless past it, however
many poles are in it"* — contradicted by the test underneath it.

**What a reader wrongly does.** Oil is the live workstream and seed 31337's
crude is charted at 256–384 tiles (`state.rs:108-109`). A reader of this
section concludes no charted well past ~64 tiles can be planned, and either
abandons it or reaches for the one fix the constant's own doc calls *"the
obvious fix and the wrong one"* (`state.rs:111-114`) — raising
`POWER_SEARCH_RADIUS`. The prose all but instructs them to.

---

## 3. `electric_supply_kw`'s own "what this does not count" list still names the radius

**The claim** — `crates/planner/src/state.rs:3859-3862`, in the function's
list of deliberate omissions:

> * **Anything further than [`POWER_SEARCH_RADIUS`] from `area`.** A bounded
>   search [...] **A power plant beyond that radius reads as absent.**

**What falsifies it.** The body of that very function
(`state.rs:3864-3878`) goes through `electric_network` → `electric_entities`
(`state.rs:4124-4129`, `4051-4090`), which walks the wire without a hop
limit. A plant beyond 64 tiles reads as **present** when a pole chain joins
it — pinned by `a_generator_at_the_far_end_of_a_long_pole_run_is_found`
(`state.rs:5478`, helper `chained` at `state.rs:5450-5476`).

**Why this ranks high despite being one bullet.** It is the doc finding 2
cites as its authority, it is on the function itself, and it sits in a list
of *genuine* residuals (solar, fuel state, accumulators) — which is the
hardest possible place to disbelieve a line.

---

## 4. `FOOTPRINT_PAD`'s enumeration predates the two-engine plant, and the miss would be silent

**The claim** — `crates/planner/src/enclosure.rs:143-154`:

> A generous upper bound on any cell or power plant's own half-diagonal,
> added to [`SEARCH_RADIUS`] when selecting which bots to check [...] The
> largest shape sited today (the power plant: pump, three pipes, boiler,
> **engine**, pole) spans under 10 tiles from its own origin; 12 tiles of pad
> leaves room [...] If a future cell shape is wider than that, this constant
> is the one place to widen.

**What falsifies it.** The enumeration is singular — *one* engine — and that
is no longer what a plant is. `power.rs:156-164`: *"Until 2026-09-06
[`plan_plant`] took no kilowatts at all: one pump, three pipes, one boiler,
one steam engine, one pole [...] [`engines_for`] now sizes the engine row
from the demand."* `layout` places engine *n* along a pitch equal to the
engine's own length (`power.rs:849-864`), and that length is derived from
`ENGINE_STEAM = [(0., 3.), (0., -3.)]` (`power.rs:639`) as
`|3 − (−3)| − 1 = 5`; `MAX_ENGINES_PER_BOILER` is **2** (`power.rs:379`). So
a two-engine plant reaches **five tiles further from its origin** than the
single-engine plant the "under 10 tiles" figure was measured on.

**What a reader wrongly does — and what the code does.** This pad is the
whole justification for `check` not examining every bot:
`state.characters_near(origin, SEARCH_RADIUS + FOOTPRINT_PAD)`
(`enclosure.rs:170`). A bot that only the *second* engine would wall in can
sit outside that window and never be examined, and nothing reports it: the
placement passes prevention and the bot is enclosed. That is precisely the
failure `enclosure::check` exists to prevent, and the constant's doc tells
the next reader the bound was sized with room to spare.

The exact overshoot (the agent's derivation put engine 2 at ~11.4 tiles from
the pump centre, ~14.7 to its far collision corner) is **derived from
`layout()`, not measured against a built plant** — treat the 5-tile
extension as the established part and the absolute figures as arithmetic
worth redoing before anyone changes the constant.

---

## 5. `score-map` still models water the way the planner did before spawn-anchoring — and its test pins it there

**The claim** — `crates/planner/src/score.rs:376-381`:

> // Water short of the planner's own wide scan is reported as missing
> // even though a tile was found, because a plant it cannot reach is a
> // plant that does not get built: `plan_plant` raises
> // `PowerPlantNeedsWater` and the whole goal fails to expand.

rendered to the operator as `"TOO FAR: plan_plant refuses beyond 128"`
(`score.rs:449`), and repeated on the field's own doc (`score.rs:143-148`).

**What falsifies it.** Since `1379b458`, `supply_for` retries every tier from
`plant_world_anchor()` = the origin (`power.rs:1004`, `1046-1049`,
`retry_from_world_anchor`), and `PowerPlantNeedsWater` / `PowerPlantNeedsShore`
are among the refusals retried. Water 57 tiles from spawn is reachable
whatever the caller's position, so "past 128 tiles **from the scoring
origin**" no longer implies the goal fails to expand.

**Consequence, stated precisely.** `score_map`'s default origin is `(0,0)`
(`app/src-tauri/src/cli/score_map.rs:245`), which *is* `plant_world_anchor()`
— so the default path is unaffected. The wrongness appears only when
`--origin` is given a non-spawn point (a roster's parked position, say): the
scorer then reports `Verdict::Incomplete { missing: ["water"] }` and computes
no walk score for a map the planner plans fine. That is the same false
refusal `1379b458` removed from the planner, still live in the scorer.

**And the test guarding it is the exact broken trick.**
`water_the_planner_cannot_reach_counts_as_missing` (`score.rs:651-670`)
manufactures unreachability by scoring the fixture's lake at `(40,40)` from
`Position::new(240., 40.)` — nothing but distance from a chosen origin. Its
name and doc claim "the planner cannot reach this water"; what it actually
asserts is a tautology over `score.rs`'s own arithmetic
(`within_planner_reach = distance <= 128`, `score.rs:336-337`, feeding
`score.rs:378-381`). It remains load-bearing for the flag-to-verdict wiring
and proves nothing about the planner.

---

## 6. `ANCHOR_SEARCH_RADIUS` describes the behaviour its own call site names as a fixed bug

**The claim** — `crates/planner/src/method/assemble.rs:217-222`:

> How far a supplying pole is looked for, in tiles. [...] past this, the
> answer is "build a plant" rather than "walk further".

**What falsifies it.** The constant is only ever passed as `near_radius` to
`power::supply_for` (`assemble.rs:2442`), which tries
`[near_radius, PLANT_ADOPT_RADIUS]` — 64, then **256** — and then
`complete_plant`, before it will build anything (`power.rs:987-991`). The
call site says so in the negative (`assemble.rs:2433-2438`): *"**The adoption
tier is not an optimisation.** This search used to stop at
`ANCHOR_SEARCH_RADIUS` from the bot and build a plant when it found nothing
there, which is how `run-1788408407-02764` came to plan a second offshore
pump..."*

The sibling constant `extract::SUPPLY_SEARCH_RADIUS`
(`extract.rs:130-141`) documents the widening correctly, so the tree carries
two contradictory descriptions of one call.

**What a reader wrongly does.** Reasons about duplicate power plants from
this doc and arrives at the conclusion that caused the named run — the same
run `complete_plant` was written for.

---

## 7. "This crate reads no container contents" — a true conclusion on a false premise

**The claims** — `crates/planner/src/method/assemble.rs`:

- `:90` — *"a cell whose chests have run out (this crate reads no container
  contents)"*
- `:199-201` — *"**After it runs out, nothing detects it.** No container
  contents are read anywhere in this crate."*

**What falsifies it.** `PlanState` carries a `buffers` overlay
(`state.rs:1409`), seeded in `from_world` from
`FactorioWorld::observed_inventories` (`state.rs:1720`), and
`method::have::Withdraw` (`have.rs:2545`+, registered `have.rs:6493`) plans
against it. The readings come from `Planner::refresh_buffers`
(`crates/core/src/plan/planner.rs:249-282`), which `goal.plan` calls
immediately before every plan
(`crates/scripting_lua/src/globals/goal/mod.rs:581`). This crate reads
container contents on every planned goal of every real run.

**The nuance, because it changes severity rather than removing the finding.**
The *conclusion* survives, by accident of a prototype name.
`BUFFER_ENTITIES` is `["stone-furnace", "wooden-chest"]` (`planner.rs:111`)
and the assembly cell's chests are `iron-chest` (`assemble.rs:180`), so the
cell's feed chests really are never read and `CELL_CHARGE_TICKS` really is
undetectable when it runs out. But the *reason given* is a claim about the
whole crate and it is false. A reader who generalises it — to the
`wooden-chest` a `Stockpile` deposits into, which **is** read, or to a
furnace, read for both output and fuel — reasons wrongly, and the obvious
follow-on ("teach the planner to top a cell up") looks impossible when it is
one list entry away.

---

## 8. CLAUDE.md: `world.dump` refreshes buffers now, so the offline loop *does* reach `Withdraw`

**The claim** — `CLAUDE.md:207-213`, under "Three blind spots, each of which
has produced a wrong 'the bug is absent'":

> - **`world.dump` never calls `Planner::refresh_buffers`**, so a dump's
>   `inventories` is `[]` [...] **The entire `Withdraw` path -- furnaces
>   handing their contents over -- is unreachable offline.**

**What falsifies it.** `create_lua_world`
(`crates/scripting_lua/src/globals/world.rs:90-101`) builds a
`BufferRefresher` that calls `Planner::refresh_buffers` and hands it to the
`world.dump` binding; the binding's own user documentation says so
(`world.rs:349-355`): *"**It asks the game what is in those containers
first**, one RCON round trip [...] exactly as `goal.plan` does immediately
before it plans."* The readings are serialised into the dump
(`crates/core/src/factorio/world.rs:1711`), read back by the deserialiser
(`world.rs:1773`, `1809`), and loaded into the overlay by
`PlanState::from_world` (`state.rs:1720`).

The *second* half of the CLAUDE.md sentence stays true — the `plan` CLI has
no RCON and refreshes nothing — but it no longer needs to; the dump carries
the readings. The same stale claim lives on the planner's `fuel` field
(`state.rs:1430-1433`): *"Empty in every fixture and in every offline dump,
since `world.dump` never refreshes inventories."*

**What a reader wrongly does.** Two things, the second worse than wasted
effort:

1. Hand-injects inventories to reproduce a buffer bug offline, which the note
   says is the only way and is not — for any dump taken after the containers
   held anything.
2. **Misreads an offline baseline.** A `plan --world <mid-run dump>` now
   expands `Withdraw` steps; someone comparing that makespan to an older
   figure while believing `Withdraw` is unreachable offline attributes the
   difference to their own change. `map.json` is t=0 so today's pinned
   baselines are safe; the first `--resume-from` dump anyone plans against is
   not.

---

## 9. `method::sustain` bought an iron chest to dodge a wood shortage that no longer exists

**The claim** — `crates/planner/src/method/sustain.rs:96-107`:

> **`iron-chest`, and not the cheaper `wooden-chest`, for a planner reason
> rather than a game one.** A wooden chest costs 2 wood, and no method in
> this crate can obtain wood: `expand` answers
> `NoApplicableMethod { goal: "have 2 wood" }` [...] Trees are minable in the
> game and a roster starts with one wood each, but neither fact reaches the
> planner.

**What falsifies it.** The same `Chop` as finding 1 — and, more pointedly, a
sibling module in the same crate that made the **opposite** choice on the
**opposite** premise: `have.rs:5717-5729` picks `wooden-chest` for a
stockpile *because* `Chop` makes two wood cost 372 ticks against an iron
chest's 2,965. Two modules of one crate stand on contradictory statements
about whether wood exists, both in doc comments a reader would trust. The
quoted `NoApplicableMethod` is a specific, checkable prediction, and it is
wrong.

**A smaller drift in the same file.** The module doc still says the coal is
dropped into a `wooden-chest` (`sustain.rs:52-56`) while `BUFFER` is
`iron-chest` (`sustain.rs:109`) and is what every placement uses
(`:376`, `:388`, `:898`, `:978`). A reader costing the arrangement from the
module doc is out by 8 iron plates a chest. (The `wooden-chest` at
`sustain.rs:883` is only a naming slip: its 1×1 perimeter argument holds for
either chest, as `BUFFER`'s own doc notes.)

---

## 10. `method::assemble` still prices poles against four wood — in the file that corrected itself

**The claims**, three surviving sites in `crates/planner/src/method/assemble.rs`:

- `:154-156` (`MAX_FEED`, justifying two feed chests and not three) — *"a
  pole costs one wood out of the four a whole run has"*
- `:1266-1269` (inside `plan_cell`, justifying a **two-pass ring search**) —
  *"a cell twelve tiles out that needs no pole beats one beside the anchor
  that costs a wood, because wood is the one resource this project has four
  of and cannot make more"*
- `:1646-1647` — *"A pole in a bill nobody places is one of four wood, spent
  for nothing."*

**What falsifies it.** `assemble.rs:668-680`, in the same file: *"**What a
pole costs is no longer what it cost when the cell's own `POLE_OFFSET` was
written.** [...] `Chop` has since made wood renewable [...] so a pole here is
priced like any other craft."* The correction was written; the three other
sites were not touched.

**What a reader wrongly does.** `:1267` is the one that matters: it justifies
a real behaviour — a whole extra ring pass preferring a cell **twelve tiles
further away** over one that costs a pole. That trade was priced against an
irreplaceable resource. It may still be right (a chop is a walk and a swing),
but nobody has re-argued it and the comment says there is nothing to
re-argue. `:155` closes off three-ingredient intermediates on the same
changed cost.

---

## 11. `CELL_CHARGE_TICKS` cites a witness window the script has never had

**The claim** — `crates/planner/src/method/assemble.rs:189-191`:

> It is 3.75x the 2,400-tick window `scripts/factory_stage2.lua`'s witness
> waits, which is what it has to outlive to prove anything

**What falsifies it.** `scripts/factory_stage2.lua:211`:
`local WITNESS_WITHIN_TICKS = 5400`. Against `CELL_CHARGE_TICKS = 9_000`
(`assemble.rs:203`) that is **1.67×**, not 3.75×. Per the auditing agent,
`git log -S"2400" -- scripts/factory_stage2.lua` returns nothing and the
file's history runs 3600 → 5400 (`911ec3c4`), so no version of the script
ever waited 2,400 ticks — this number appears never to have been true.

**What a reader wrongly does.** The doc's next paragraph is *"After it runs
out, nothing detects it"* (finding 7). Someone checking whether the hand
charge outlives the witness believes there is 3.75× of headroom where there
is 1.67×, and the failure mode is a silently stopped cell whose structural
predicate keeps holding.

---

## 12. The retracted "water cannot be moved" premise is still quoted as current — including to the operator

`method::power` retracts this premise **by name** (`power.rs:9-24`): *"This
module used to open with a paragraph that justified siting the plant on the
shoreline and nowhere else [...] **Both halves are wrong**, and they are
stated here rather than quietly deleted because a *design constraint* was
justified by them."* Two places still assert it.

**12a. User-facing, at refusal time** — `crates/planner/src/error.rs:512-514`,
the `miette` `help` on `PowerPlantNeedsWater`:

> the plant is sited at the water because water is the one input that cannot
> be carried

This prints to whoever is running the tool at the moment a plan refuses.
`power.rs:25-27` falsifies it — *"**Water moves. Through pipes. That is what
pipes are for.**"* — with the price table read off
`workspace/server/data/base/prototypes/recipe.lua` (`power.rs:30-46`; the
`pipe` and `small-electric-pole` rows **verified** here independently).

**12b. In a peer module's reasoning** — `crates/planner/src/method/extract.rs:70-72`:

> `method::power` sites its plant at the water because water is the one input
> that cannot be moved.

The *behaviour* half is still true (`power.rs:136-137` confirms the plant is
built at the water today); the *reason* is the retracted premise, and the
reason is what a reader carries away. `power.rs:139-141`: *"What is no longer
claimed is that this is forced."*

**Consequence.** Nothing computes from these, so they rank below the findings
above — but this is the exact sentence that produced the motivating defect,
still in two places, one of them the operator-facing help of a refusal.
Someone debugging that refusal is told the constraint is physical.

---

## 13. Wood scarcity has moved from the code into a fixture, and the fixture's comment says otherwise

**The claim** — `crates/planner/tests/red_science_cell.rs:213-221`
(`powered_state`): *"the planner cannot make wood (`Mine` sources only
resources, and a tree is an obstacle) … It is also the hard cap this cell
lives under: four bots, four wood, eight poles ever."* The same assumption
seeds `standing_site_reuse.rs:81`.

**Why it still behaves, which is the interesting part.** The shared fixture's
hundred trees are `tree-42`, *"a name the prototype fixture gives no entry"*
— documented at `crates/planner/src/test_world.rs:77-83` and again at
`score.rs:568-574` — so `minables_yielding("wood")` finds no source and
`Chop` cannot fire. `test_world::with_trees` uses `tree-01`, the prototype
that really carries `mine_result {wood: 4}` (`test_world.rs:83-92`), when a
test wants real wood.

So the scarcity is now a **fixture artefact**, not the constraint the comment
names. Nothing is green-while-broken today (the auditing agent found no test
asserting a wood/pole shortfall refusal), but any reasoning in
`red_science_cell.rs` or `assemble.rs` of the form "a pole is irreplaceable,
so this count matters" stops discriminating the moment the shared fixture
gains readable trees — and the comment tells the next reader the cap is
guaranteed by the planner.

---

## 14. Ordering and precedence arguments: 18 hold, three do not

**14a. `BuildCell` is neither last nor the only claimant of `Goal::Producing`**
— `crates/planner/src/method/have.rs:6505-6508`:

> // Last: it claims `Goal::Producing`, which nothing else claims, so
> // where it sits changes no other goal's method.

`BuildCell` is registered at `have.rs:6509` and followed by
`BuildAssemblyCell` (`:6514`), `Sustain` (`:6521`) and `BuildBlock`
(`:6528`) — fourth from last. `BuildAssemblyCell::applicable` matches
`Goal::Producing` (`assemble.rs:2384-2391`), which the **next comment down**
(`have.rs:6510-6513`) says out loud. The comment contradicts itself in four
lines. The order moved after it was written: `d531a0fe` introduced it when
`BuildCell` genuinely was last, `294744ca` inserted `BuildAssemblyCell` after
it.

**Consequence:** the registry is strict first-match (`method/mod.rs:357-362`,
`.find(|m| m.claims(site) && m.applicable(goal, state))`). Someone
registering a new `Producing`-claiming method after `BuildCell` on the
strength of *"nothing else claims it"* gets a method that is silently never
reached. No behaviour is wrong today; this is a trap for the next editor.

**14b.** `supply_for`'s heading *"# The three tiers"* (`power.rs:903`) is
followed by a list of **five** (`power.rs:905-934`), matching the code
(`power.rs:987-1006`). Heading only.

**14c.** `Withdraw` *"is registered ahead of everything"*
(`state.rs:1141-1142`) is third, behind `AlreadySatisfied` and
`SplitAcrossBots` (`have.rs:6493`), as `registry_for`'s own comment at
`:6489` says. The argument it supports (ahead of every *producing* method)
is sound; the absolute wording is not.

**Checked and holding (18).** `PlaceDrill` ahead of `Smelt`
(`have.rs:5029/5031`, `:6499/6503`); `Chop` ahead of `Mine`
(`have.rs:5044`, `:6520`); `Chop` ahead of `Stockpile` with its measured
table (`have.rs:6520/6531`); `Stockpile` ahead of `Mine` falling through
(`have.rs:6531/6533`); `Withdraw` behind `SplitAcrossBots` and ahead of
`SharedSmelt`/`Smelt`/`HandCraft`/`Mine` (`have.rs:6485-6493`,
`tests/buffers.rs:448`); `AlreadySatisfied` ahead of `Scout`
(`have.rs:6482` vs `:6498`, `scout.rs:750`) and ahead of everything
(`have.rs:6482`, `tests/standing_goals.rs:93`); `BuildBlock` last in
`default_registry`, itself dead outside tests (`have.rs:5062`,
`:6522-6528`); `MethodRegistry::find` first-applicable while
`refusal`/`concurrency` ask everyone (`method/mod.rs:340`, `:357-362`,
`:388-393`); `divergence`'s wordings before the clamp line
(`executor/src/divergence.rs:55` vs `:74/:85/:92`); `schedule`'s candidate
tiers and owner-first pin arm (`schedule.rs:569`, `:575-627`); `recover`'s
tier order 0→3 (`recover.rs:312-325`); `route_belt` always called with
`max_underground: None`, making `SpanTooLong` and the `unreachable!()`
genuinely unreachable (claims at `connect.rs:107`, `:142`, `:678`; **I
re-verified the single call site independently** — `connect.rs:619` is the
only `route_belt(` in the tree outside `crates/core`);
overlay-wins-over-base in `entities_named_any`/`ghosts_named_any`
(`state.rs:3633`/`:3693`); `/assets` mounted ahead of the SPA fallback
(`server/src/webserver.rs:40` vs `:43`); researched unlocker preferred
(`method/util.rs:828-843`); `power::finish` running the enclosure check last
(`power.rs:1465`); `goal.start` checking `PendingWork` before spawning
(`globals/goal/run.rs:1006`, `:1035`).

**Prose drift, not an error:** `have.rs:6496` and `:5044` say `Scout`
*"claims `Goal::Charted`, which nothing else claims"*; `AlreadySatisfied`
also claims it (`scout.rs:748-751`). The stated consequence holds.

---

## 15. Smaller drifts, one line each

| claim | where | what falsifies it |
|---|---|---|
| `MAX_LABS = 8` is *"more than any roster this planner has been run with"* | `have.rs:5010-5012` | eight bots have been run (`CLAUDE.md:756`, *"Eight bots have been run (2026-09-05), nothing above"*), so it is *equal to*, not more than — and lab count is driven by roster size (`have.rs:4894-4903`), so the cap binds sooner as the roster grows |
| `LAB_SEARCH_RADIUS` is *"the same bound `PlanState::electric_supply_kw` searches under"* | `have.rs:3503-3505` | `electric_supply_kw` no longer searches under a 64-tile bound at all (finding 3). 64 is still a real bound *here*; the shared justification is gone, and this file's own call site states the widening correctly at `have.rs:4177-4183` |
| `score.rs`'s caveat that *"the planner measures from the plant site [...] rather than at spawn"* | `score.rs:150-152` | tier 5 now measures from `plant_world_anchor()` = the origin (`power.rs:1046-1049`), which is where `score` measures from too — the caveat describes a planner that no longer exists in full (harmless direction) |

---

## 16. Unresolved, and reported as such

- **Four adoption tests are weaker than they read.**
  `power.rs:2513-2528` (`a_plant_that_already_stands_is_adopted_from_past_the_near_search`,
  bot at +86 tiles), `power.rs:2538-2552`, `have.rs:13087-13160`
  (`a_research_adopts_a_plant_that_already_stands`) and
  `tests/red_science_cell.rs:526+` all stand a bot far from a standing plant
  and assert adoption over duplication. The fixture plant is ~57 tiles from
  `plant_world_anchor()` = `(0,0)` and `PLANT_ADOPT_RADIUS` is 256
  (`power.rs:570`), so `retry_from_world_anchor`'s own
  `nearest_supply_anchor(&anchor, PLANT_ADOPT_RADIUS, kw)`
  (`power.rs:1099-1101`) reaches that pole from the origin regardless of
  where the bot stands. They would therefore stay green under a large
  narrowing of the caller-anchored tiers. **This is reasoned from the code
  path and was not falsified by experiment** — proving it needs a code edit,
  which this review did not make. Per the fixtures note, do not believe it
  until a break has been seen to change something.
- **`extract.rs:113`'s third measured row** (`[300.5, 100.5]`, ~281 tiles,
  *"refuses, `PowerPlantNeedsWater` (no lake)"*) was measured before the
  world-anchor retry landed. On the dump it was taken against, the origin
  does see a lake, so the refusal should now change identity. **Not
  re-measured, and no test pins the row.**
- **`MAX_BANK`'s geometry claim** (`have.rs:422-424`, *"eight sites are used
  up by ring 3 — three tiles from the anchor"*): `free_area_near_where` steps
  rings by one tile, not by a furnace's 2×2 cell
  (`method/util.rs:525-535`), so which ring the eighth site lands on depends
  on `is_area_free` rejections nobody has simulated. Conservative in the safe
  direction if anything, but not derivable from the code as written.
- **`PowerPlantNeedsWater`'s doc says "of the acting bot"** (`error.rs:488`).
  Not false — the returned error *is* the roster-anchored one
  (`power.rs:1052-1057`) — but silent about the world-anchored retry that
  also failed. **Incomplete rather than wrong; not counted as a finding.**
- **Whether `Chop` can always supply wood: it cannot.** It needs a standing
  minable in the entity graph (`state.rs:5203-5215`). On the reference map
  that is 6,656 trees (`have.rs:5729`); in a fixture of `tree-42` it is
  nothing (finding 13). The honest replacement for *"this planner cannot make
  wood"* is **"wood costs a walk and a swing, and needs a charted tree"** —
  not "wood is free". Findings 1, 9, 10 and 13 should be read with that
  caveat, and none is rescued by it: all four assert a hard cap of four.

**Categories where I found nothing further.** Beyond finding 5, the sweep for
tests that manufacture unavailability by distance-from-a-bot turned up only
cases already fixed and documented as fixed:
`extract.rs:1697-1712` (*"This test used to put the generator 150 tiles away
[...] that construction stopped testing anything"*) and
`crates/scripting_lua/src/globals/goal/mod.rs:2769`, already migrated to
`fixture_world_without_water()`. Tests where distance is the *subject* rather
than a way to hide something (`the_same_footprint_far_from_any_bot_is_clear`,
`blueprint.rs:1977`'s `set_position(500,500)` anchor-stability check) are
correctly constructed. Constant justifications that were checked and **still
hold**: `WALK_TILES_PER_TICK` 0.14 and its "only travel speed" claim
(`schedule.rs:45`), `ARRIVAL_MARGIN` 0.6 and the `R + 1.1` retraction
(`crates/core/src/factorio/rcon.rs:1638-1671`), `LAB_POWER_KW` /
`consumer_kw` / `COAL_BURN_TICKS`'s shared "the mod does not send
`energy_usage`" premise, `THREAT_STANDOFF`, `PARTIAL_CELL_SCAN_RADIUS`,
`MAX_FEED`'s `(-2,3)` mouth claim, and
`FOOTPRINT_BUSY_BLOCKER_BUDGET`'s eighth-of-`ACTION_RESULT_DEADLINE`.

---

## Why this class keeps happening here

Three structural causes, all visible above.

**1. The correction is written where the fix lands, not where the claim
lives.** Every one of findings 1-12 has a *correct*, updated, often eloquent
statement of the new truth somewhere in the tree — `have.rs:3019` for wood,
`state.rs:105-118` for the power radius, `power.rs:9-24` for water,
`extract.rs:1531` for the ceiling, `assemble.rs:2433` for the adoption tier.
The author of a fix updates the doc they are editing. Nobody greps for the
claim they just falsified.

**2. Prose is the only place many of these facts live, so nothing can go
red.** The 56-tile crossover, the ~64-tile ceiling and the four-wood budget
are computed nowhere: `pole_run_items` and `pipe_run_plates` have no
production callers at all. A number nothing reads cannot be invalidated by
anything.

**3. A test written beside a claim encodes the claim, not the world.**
`the_crossover_is_wood_rather_than_price` types `STARTING_WOOD = 4` into
itself; `water_the_planner_cannot_reach_counts_as_missing` asserts
`score.rs`'s own arithmetic back at it. Both are green today and would be
green under the change they exist to notice. This is the fixtures note's
failure one level up: the *doc* and the *test* were written together, so the
test defends the doc rather than the code.

### Mitigations, cheapest first

**(a) One grep at the commit that removes a constraint.** Grep the tree for
the constraint's own vocabulary before committing — "cannot make wood",
"four wood", "POWER_SEARCH_RADIUS", "cannot be moved", "reads no container
contents". Every finding here would have surfaced. `1379b458` did this for
`power.rs` and not for `extract.rs` or `error.rs`, which is exactly the shape
of finding 12.

**(b) Make a supply claim ask the planner.** Where a doc says "this crate
cannot obtain X", the checkable form is one assertion that the registry
cannot expand `Goal::have("wood", 2)`. Any method that later supplies wood
turns it red **and points at the sentence**. For findings 1, 9, 10 and 13
that is a handful of assertions replacing several paragraphs of prose. The
value is not the assertion; it is that the claim acquires an owner that can
fail. The pattern already exists in-tree: `extract.rs:155-171` refuses to
compare `WIRE_REACH` against a copy of itself and asks `PlanState` which
poles share a network instead — *"a test that merely compared this constant
to a copy of itself would agree with the code that wrote it."*

**(c) A doc that names a test must name a test that exists.** `extract.rs:116`
cites `a_pole_chain_past_the_power_search_radius_is_not_seen`, renamed on the
day the constraint was lifted. A CI grep — every `` `snake_case_name` `` in a
doc comment that looks like a test must resolve to a `fn` in the tree — would
have caught finding 2 at the commit that caused it, mechanically, with no
judgement involved. This is the one item here that is genuinely automatable,
and it is the discipline `lua_docs.rs` already enforces between the binding
docs and `documented_type_schemas()`: the two halves are held together, and a
name with no referent fails the build.

### The rule this review would add

> A doc comment that states a constraint is a claim about **another file**.
> Its author cannot maintain it, because the code that will falsify it has
> not been written yet. So either give it an assertion that lives with the
> code that would break it, or write it as history — *"as of `<commit>`,
> nothing here made wood"* — which stays true forever and tells the reader to
> check.

Every stale claim above reads as timeless. Every corrected one — `have.rs`'s
*"before this method"*, `power.rs`'s *"this module used to open with"*,
`extract.rs`'s *"this test used to assert the opposite"*, `assemble.rs`'s
*"this search used to stop at"* — is written as dated history and is still
accurate. **The tense is the mitigation.**
