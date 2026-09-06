# Nothing rules out poles: the 56-tile supply crossover, retracted and priced

2026-09-06. `crates/planner/src/method/power.rs` carried, in six places, the
claim that the pole route runs out at about 56 tiles:

> *"**What binds is supply, not price.** Wood is the item this planner cannot
> make: a four-bot run starts with four, and no method in this crate mines a
> tree. Four wood is eight poles is about **56 tiles of wire, ever** [...] So
> the crossover is a **supply** crossover at roughly 56 tiles [...] wire short
> runs and pipe long ones."*

`docs/superpowers/notes/2026-09-06-stale-constraints.md` finding 1 falsified
it. This note records what replaces it, because the honest replacement turned
out to be shorter and more useful than the claim: **nothing rules the pole
route out, at any distance this planner can see.**

## Three independent falsifications, any one of which is enough

**1. This crate chops.** `method::have::Chop` swings at any standing minable
entity and is registered in the production registry ahead of `Mine`. Its own
doc records this exact cap being removed after live run
`run-1788396958-07935` halted on it. `method::have::BUFFER_CHEST` then picks a
`wooden-chest` over an iron one *because* wood is renewable. The constraint
was real, was read from a real place, and had been lifted in a file the
pricing never consulted.

**2. Wood is a tier-one artefact.** Read off
`workspace/server/data/base/prototypes/recipe.lua` and
`entity/entities.lua`, folding in `iron-stick` (1 iron plate → 2),
`steel-plate` (5 iron plates → 1) and `copper-cable` (1 copper plate → 2):

| pole | ingredients | iron plates | wire reach | iron plates a tile |
|---|---|---|---|---|
| `small-electric-pole` | 1 wood + 2 copper-cable → **2** | **0** | 7.5 | **0** |
| `medium-electric-pole` | 4 iron-stick + 2 steel-plate + 2 copper-cable → 1 | 12 | 9 | 1.33 |
| `big-electric-pole` | 8 iron-stick + 5 steel-plate + 4 copper-cable → 1 | 29 | 32 | **0.91** |
| `pipe`, for comparison | 1 iron-plate → 1, one tile | 1 | — | 1.00 |

`electric-energy-distribution-1` (120 red + green) unlocks the medium pole,
the big pole and `iron-stick`, and it is already on the oil ladder. Note the
last column: **even paid for entirely in iron, a big pole is cheaper per tile
than pipe**, and a medium pole is within a third of it. There is no tier at
which pipe becomes the material answer.

**Two corrections to the table as
`2026-09-06-piping-water-is-cheap.md` prints it.** That note gives the big
pole a wire reach of **30.0**; `entities.lua:4494` in this repo says **32**.
`crate::state::pole_wire_reach` also says 30.0 — a second site with the same
number, not re-checked here beyond reading the prototype, and out of this
branch's scope to change.

**3. No crossover of any kind can exist, because every route is linear.**
`pipe_run_plates` is `ceil(N)` plates and `pole_run_items` is
`ceil(ceil(N/7)/2)` crafts. Neither has a fixed cost, so their ratio is the
same at 7 tiles and at 700, and no distance can reverse it. **A crossover
requires one route to carry a setup charge the other amortises**, and neither
does. This is the argument that kills the *shape* of the claim rather than one
of its numbers, and it is the one the new test asserts.

## The verdict, stated plainly

**Nothing rules the pole route out.** Three real residuals survive, and none of
them is a distance rule:

* **A chop needs a charted standing minable.** `Chop::applicable` refuses
  without one. On an unexplored or genuinely bare map wood is unobtainable —
  and the failure is a `NoApplicableMethod { goal: "have N wood" }` at
  expansion, not a longer plan. The fix is charting, or research; it is not
  pipe.
* **Time, where the wood is the cheap half.** Derived from the two figures
  `BUFFER_CHEST` measured on the reference map: two wood off one dead tree is
  **372 ticks with the walk in it** (~186 a wood), and eight iron plates plan
  at **2,965 ticks** (~371 a plate, furnace and fuel amortised). One pole
  craft is 1 wood + 1 copper plate and buys 14 tiles — about 557 ticks, so
  **~40 ticks a tile against pipe's ~371**. The pole route wins on time by
  about the order it wins on materials. **The expensive half of a pole craft
  is the copper plate, not the wood**: the item six sites were spent on is the
  cheaper one. (Arithmetic over two measurements taken elsewhere, not a
  measurement of this route.)
* **This crate can only build tier one.** `power::POLE` is hard-coded
  `small-electric-pole`, so rows 2 and 3 of the table above are facts about
  Factorio and not yet capabilities here. A run that can find no tree cannot
  fall back on them today. That is a note for whoever wires the tier up.

## The test that was replaced, and why it could never fail

`the_crossover_is_wood_rather_than_price` typed
`const STARTING_WOOD: u32 = 4` into itself and asserted arithmetic about
`pole_run_items` against that literal. Registering ten `Chop`s would not have
moved it. It is the fourth shape in
`2026-09-06-fixtures-agree-with-their-code.md` — a fixture that satisfies its
own assertion by construction — applied to a doc rather than to code.

Two tests replace it, and both were made to go red on purpose, one break at a
time, with each substitution asserted to have matched exactly once:

| test | break | what went red |
|---|---|---|
| `no_distance_turns_the_pole_route_into_the_dearer_one` | `POLES_PER_CRAFT` 2 → 1 | `56 tiles: 8 poles, 4 crafts` |
| same | `pipe_run_plates` + 10 (a fixed cost) | the 56-tile absolute |
| same | `pipe_run_plates` returns 500 **only at 224 tiles** — invisible to every literal in the test | `112 tiles doubled must double the pipe bill`, alone (24 passed, 1 failed) |
| `the_planner_can_obtain_the_wood_a_long_pole_run_wants` | `Chop` unregistered from `registry_for` | `NoApplicableMethod { goal: "have 26 wood" }` on the wooded world |
| same | the treeless control world given trees | `the shared fixture's tree-42s yield nothing` — a 7-chop network |

The third row is the point: the linearity assertion is the one that forbids a
crossover, and it was shown to bite **in isolation**, by a break no absolute
literal in the test could see. Without that, "the block goes red when pipe
gets dearer" would have proved only that some assertion somewhere fired.

The wood test follows mitigation (b) of the stale-constraints review: **make a
supply claim ask the planner.** It asks `expand` for `have 26 wood` — the wood
half of `pole_run_items(355.)`, typed — and asserts an exact seven chops, so a
bill that silently under-delivers cannot pass. Any future change that really
does make wood unobtainable turns it red and points at the sentence.

## The rule this reinforces

The stale-constraints review's own rule, earned again here: **a doc comment
that states a constraint is a claim about another file, and its author cannot
maintain it.** Every corrected site in `power.rs` is now written as dated
history — *"this doc used to read"*, *"until 2026-09-06"* — which stays true
forever and tells the reader to check. The one claim in the new text that
cannot be checked in-crate (the two lower rows of the pole table, whose
recipes the fixtures do not carry) says so in place, with its source and its
date.
