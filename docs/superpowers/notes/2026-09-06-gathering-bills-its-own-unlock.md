# `gathered:` refused a prerequisite every other goal kind bills

2026-09-06 (measurements; the branch landed just past midnight on 09-07). Branch `gathering-bills-its-own-unlock`, off `master` at
`22cbfcd7`. Release binary, `--no-default-features --features cli,lua`, four
bots, offline `plan` against dumps — no Factorio, no RCON.

## The finding, and it was a gap rather than a decision

`Goal::Gathered { entity: "crude-oil" }` refused with
`PlannerError::ExtractorLocked`:

```
extracting from crude-oil takes a pumpjack, whose recipe needs
oil-gathering researched first
```

while `researched:oil-gathering` **plans on that same dump**, and while every
other goal kind bills exactly that prerequisite without saying anything. The
control is `have:`, which reads
`RecipeGate::NeedsResearch` in `have.rs`'s `HandCraft`, emits
`Step::Subgoal(Goal::Researched(tech))` and states `Condition::Researched` on
the craft. So the planner was stating a fact it could satisfy as an
impossibility.

Measured before the change, on `workspace/scripts/map-31337-explored.json`:

| goal | before |
|---|---|
| `gathered:crude-oil` | **refused**, `ExtractorLocked` |
| `researched:oil-gathering` | 1,587 actions, 311,773 ticks (1:26:36) |
| `have:pumpjack:1` | plans (oil survey, 2026-09-06) |

**Was it deliberate?** Looked for a recorded reason and found none:

- no note under `docs/superpowers/notes/` argues for it;
- `PlannerError::ExtractorLocked`'s own doc *describes when it fires* ("nothing
  in this plan unlocks it") rather than arguing that nothing should;
- `method::extract`'s ladder lists it as tier 3, and `method::gather`'s module
  doc inherits the whole ladder by delegation — "everything
  `crate::method::extract` refuses";
- the one test asserting the refusal for a directly-stated goal
  (`an_extraction_goal_stated_directly_is_refused_by_the_same_ladder`, in
  `have.rs`) is about `Goal::Extracted`, and its doc says only that the goal
  "reaches the same ladder", i.e. it asserts *consistency of delegation*, not a
  policy that research must not be billed;
- **`method::gather` had no test for the locked case at all.** Its `OPEN`
  fixture is `PumpjackRecipe::LockedBy { researched: true }`, so every gather
  test ran with the research already done and the rung was never exercised.

The closest thing to a stated intent is in `scripting_lua`'s refusal
classification, which describes `ExtractorLocked` as "the machine's recipe
wants a research **the script can plan first**". That is a caller-does-it
policy for a *script*, not an argument that the planner may not do it — and
`Goal::Gathered` is the goal a script states directly, so it is the one place
that policy costs the most.

Verdict: **a gap, created by delegation.** `Gather::expand` calls
`extract::site_extractor`, which refuses the locked rung on `Extract`'s
behalf; nobody decided that `Gathered` should inherit it.

## The fix: delegate the billing, do not encode it twice

`method::gather` now converts **only** the `ExtractorLocked` rung, by re-asking
`site_extractor` against a **fork** with the technology marked in the overlay
(`site_extractor_billing_the_unlock`).

The research subgoal is **not** emitted here, and that is the point:

- nothing about *extraction* needs the technology. A pumpjack in a bot's
  inventory can be placed whatever the force researched; the gate binds the
  **craft**, and the craft is already asked for as
  `Goal::Have { item: extractor }` — the first step `extractor_steps` emits.
- so `have.rs` bills it, as it does for everything else. Emitting a second
  `Goal::Researched` here would be a second encoding of one rule, and the two
  would agree until one changed. This repo has the scar:
  `inserter_facing`'s doc had to be corrected from "the only place" to one of
  three that agree.
- it would also be **wrong** in the case that matters: a roster already holding
  a pumpjack owes the research nothing, and only `Have` knows that.
- the ordering edge comes free. `HandCraft` states `Condition::Researched` on
  the craft; the craft's `GainItem` satisfies the placement's `HasItem`;
  `infer_edges` draws both. The pumpjack cannot be placed before the technology
  it was crafted under.

**The fork is why this is not the `run-1788338409-63794` bug again.**
`set_researched` writes the plan overlay, so `recipe_gate` would read
`PlannedResearch` — and `HandCraft` emits **no research subgoal** for
`PlannedResearch`. Had the mark landed on the real `ctx.state`, the plan would
have grown a pumpjack and never researched `oil-gathering`.
`siting_past_the_lock_does_not_mark_the_research_as_planned` exists for exactly
that and fails with `left: PlannedResearch("oil-gathering")` when the mark
leaks.

## What still refuses, and it is a different fact

Only `NeedsResearch` is converted. `RecipeGate::Unobtainable` — a recipe
disabled with **no technology that unlocks it** — still comes back as
`NoExtractor { why: "... disabled with no technology to unlock it" }`. "Not yet
researched, and here is the plan" and "nothing in this game unlocks this" stay
different answers. A technology that exists but is itself unreachable refuses
from where that is known: expanding the `Goal::Researched` the craft emits, in
that goal's own words.

`Gather::refusal` is untouched. It delegates to `extract::refusal_for`, which
can still name `ExtractorLocked` — but it is consulted only when **no** method
claims the goal, and `Gather::applicable` never consults the recipe gate, so
for a `Goal::Gathered` this method always claims and that rung is unreachable
from there.

**`Goal::Extracted` is deliberately unchanged.** It is a different goal with a
different owner (`method::extract`), reached from a `mine-entity` research
trigger where the technology is normally already in the plan as a prerequisite.
Changing it was out of this task's boundary and would need its own argument.

## The numbers, same binary before and after

Three baselines on `workspace/scripts/map.json`, four bots — **byte-identical**:

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,463 | 316 / 22,463 |
| `producing:logistic-science-pack:6` | 442 / 47,542 | 442 / 47,542 |

On `map-31337-explored.json`, four bots:

| goal | before | after |
|---|---|---|
| `gathered:crude-oil` | **refused** | **2,115 actions, 317,283 ticks (1:28:08), 76.9% util** |
| `researched:oil-gathering` | 1,587 / 311,773 | 1,587 / 311,773 (unchanged) |

**2,115 > 1,587 is the check that matters.** A plan *smaller* than the research
alone would have meant the research was skipped rather than billed; the 528
extra actions are the pumpjack, its power run, the tank and the pipe run on top
of the research the gather now pays for.

## Falsification

Three tests, each run and found **by name with a count** in the full output
before anything was concluded (`grep -c`, not by eye), then broken one at a
time:

| break | result |
|---|---|
| neuter the retry (`set_researched` on the fork → no-op), 1 match | `a_locked_extractor_recipe_is_billed_rather_than_refused` **and** `siting_past_the_lock_...` FAIL, with the original `ExtractorLocked { technology: "oil-gathering" }` |
| leak the overlay (`ctx.state.set_researched` in `expand`), 1 match | `siting_past_the_lock_...` FAILS (`left: PlannedResearch`), the other two pass — which is precisely why that test exists |
| don't clear `unlocked_recipes` in the third test's fixture, 1 match | `a_recipe_no_technology_unlocks_is_still_refused_by_name` FAILS on its own precondition (`left: NeedsResearch`) |

Every substitution was confirmed to match **exactly once** before the run.

**Stated honestly: the third break is a fixture break, not a code break.** I
could not construct a single-edit break in `method::gather` that changes that
test's outcome, because `Unobtainable` is decided upstream in
`method::util::recipe_gate` and this change only touches the `ExtractorLocked`
arm. So that test is a boundary regression test — it pins that the conversion
did not widen to "ignore the recipe gate" — and its non-vacuity is established
against its own fixture rather than against the conversion.

**And this task wrote both the code and its fixtures**, in the sense the
fixtures note warns about — with one mitigation worth naming: `world_with_oil`
and its `PumpjackRecipe::LockedBy { researched: false }` arm predate this work
by a day and were written for `method::extract`'s tests, so the locked world is
not one this change invented. The `Unobtainable` world is: it is
`world_with_oil` with the technology's `unlocked_recipes` emptied by hand.

## Files

- `crates/planner/src/method/gather.rs` — the conversion, its doc, the ladder
  correction in the module doc, and three tests.
- Nothing else. `state.rs`, `extract.rs`, `have.rs` and `util.rs` are untouched;
  `PlanState::fork` and `set_researched` were already public.
