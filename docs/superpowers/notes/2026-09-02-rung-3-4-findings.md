# Rungs 3 and 4 of `run-1788300756-94802`, diagnosed

Two defects were named from the four-bot run of 2026-09-02. One of them turned
out not to be the defect it looked like; the other reproduced exactly and is
fixed. A third thing — bigger than either — fell out of reading the same log.

Commits: `9a62dd82` (defect 2), `886e9c6b` (defect 1).

---

## Defect 1 — the "false success" at milestone 3 was not false

**The goal genuinely held.** `samples.jsonl` at tick 29330 says so:

| bot | iron-plate | iron-ore | copper-ore | stone-furnace |
|-----|-----------|----------|------------|---------------|
| 2   | 8         | 8        | 2          | 1             |
| 3   | 8         | 8        | 5          | 1             |
| 4   | 8         | 4        | 13         | 1             |

Twenty-four iron plates across the roster. Freeplay starts every player with
eight; nobody smelted anything, and nobody had to. The milestone's goal is
`goal.have("iron-plate", 10)` (`scripts/research_run.lua`), whose holder is
`Holder::Anyone` — *the sum across the roster*, which is what makes multi-bot
gathering splittable. Ten of twenty-four: `shortfall == 0`,
`AlreadySatisfied` claims it, `expand` returns an empty network, zero
iterations, zero ticks.

So the two sub-questions asked resolve like this.

1. **Why did the planner return an empty network?** Because the goal was met,
   not because `Smelt` was wrongly inapplicable. `AlreadySatisfied` is
   registered ahead of every producing method precisely so "we already have
   this" is decided in one place; `Smelt` was never consulted. And the premise
   that the bots had "no furnace" is wrong — each held one. Nothing here should
   have been an error, and nothing should have planned to obtain a furnace.
   *The milestone's **name** ("smelt iron plates x10") promises smelting; its
   **goal** asks for possession. The name is the thing that lied.*

2. **Should an empty plan ever mean satisfaction?** No — and it no longer does.

### What was actually wrong, and what shipped for it

The supervisor could not tell "already satisfied" from "the planner produced
nothing" and said so, at length, in the branch itself: within `crates/planner`'s
registry an empty plan does mean the goal held, but that is an internal
invariant of that crate, not a contract the `goal.*` surface exposed. Reporting
satisfaction from it was a guess that happened to be right.

`886e9c6b` makes it checkable rather than assumed:

- **`planner::holds(goal, state) -> Option<bool>`** (`method/have.rs`), three
  valued. `Some(true)`/`Some(false)` for a goal naming a *state*; `None` for one
  naming an **event** — `Goal::Produced` is not settled by any inventory read,
  and `Goal::Producing` is not modelled at all. Collapsing `None` into `false`
  re-runs work that may be done; collapsing it into `true` is the lie the type
  exists to prevent. `Goal::All` is the conjunction with `false` beating `None`.
- **`AlreadySatisfied` delegates to it**, so the method and the question cannot
  drift apart, and `an_empty_expansion_and_a_held_goal_agree` pins the
  equivalence the Lua side now relies on.
- **`goal.holds(goal, opts)`** exposes it, reading the same world snapshot and
  the same roster `goal.plan` reads, with the same refusal of bots the world has
  never heard of.
- **`scripts/supervisor.lua`** asks on every empty plan: `true` →
  `already_satisfied`, `nil` → `plan_empty` (the one fact observed), `false` →
  **raises**. A contradiction between the planner and the world is a planner
  defect, not a world condition, and there is nothing a retry could change;
  `research_run.lua`'s existing `pcall` records it as `plan_error` with the text.

`SatisfiedReason::AlreadySatisfied` already existed and was already accepted by
`record.milestone_satisfied` and rendered by `RunAnalysisPage.vue`, so no
record-schema or frontend change was needed — the variant simply had no writer.

**Stale comment left alone:** `crates/core/src/record/mod.rs`'s doc on
`SatisfiedReason::Unknown` still says "the planner exposes no way for a script
to check whether a goal already holds independently of planning it, so an empty
plan is always reported as `PlanEmpty`". That is no longer true. The file was
dirty in another agent's working tree at the time, so it was not touched.

**`workspace/scripts/supervisor.lua` was a stale copy** of the pre-change file
(the workspace copy is never re-seeded from the repo — see `CLAUDE.md`). It has
been synced, so the next live run picks the change up.

---

## Defect 2 — reproduced, root-caused, fixed

    precondition has 50 iron-ore of action ActionId(8) does not hold for bot 2

**The precondition is right, the state it is checked against is right, and the
plan is feasible. What was wrong is that nothing held the plan's producers and
its consumer in one pair of hands.**

`automation` sits above `steam-power`, a Factorio 2.0 `craft-item` trigger for
fifty iron plates. `Researched` asks for a trigger's work as

```rust
Goal::Produced { item, count, whose: Holder::Share(ctx.chain_actor), unlocks }
```

and says in its own comment that it uses `Holder::Share` "for the same reason
the pack bill uses it: one action reading one bot's inventory". **That claim
bought nothing.** The driver read `whose` off `Goal::Have` alone — in
`expand_goal` (chain-actor rebinding) and twice in `expand_goal_body` (chain
owner, and whether to open a chain at all). `Goal::Produced` matched none of
them, so:

- the trigger's smelt — `place stone-furnace`, `insert 50 iron-ore`,
  `fuel the furnace`, `take 50 iron-plate` — was welded to **nothing**;
- the `Have { iron-ore, 50, Share }` beneath it *did* open a chain, so the
  mining was welded to one bot;
- the scheduler mined 42 ore onto one bot and offered `insert 50 iron-ore` to
  a different one, which held 8.

Reproduced before touching anything, in `method::have::tests::
the_live_four_bot_research_run_plans_and_schedules`: the run's roster, the run's
inventories at the tick it died, and a force with `automation` above a
`steam-power` trigger. Without the fix it raises the message above verbatim;
with it, it schedules.

The fix (`9a62dd82`) is one `stated_holder(goal)` helper covering both goal
kinds, used by all three reads so they cannot drift again. Three tests fail
without it (two driver-level, one end-to-end).

### The rest of the story — a finding, not a fix

**A `Holder::Share(b)` chain is sized against `b` and may be run by anyone.**
That is stated as a feature (`goal.rs`: a share "carries no commitment about
*who* runs it"), and it is sound only while bots are interchangeable. They are
not, after two gathering milestones: 8 / 8 / 4 iron-ore.

Welding the subtree fixes the crash but leaves this latent. Demonstrated by
adding one line to the new regression test:

```rust
s.set_position(BotId(4), Position::new(-38., 36.));  // the 4-ore bot, on the patch
```

Bot 4 is then cheapest for the chain that opens at the iron patch, takes it,
mines the 42 that were sized against bot 2's eight, ends with 46, and the same
error comes back naming bot 4. **The fix as shipped removes the crash for the
recorded run's geometry; it does not make the class impossible.**

Relevant history: `f3a22e29` deleted `check_bots_interchangeable` and
`PlannerError::BotsNotInterchangeable` on the ground that "after those three
fixes nothing in the crate still makes that assumption". That is not so.
`Researched` sizes its entire bill — packs and trigger production alike —
against `Holder::Share(ctx.chain_actor)`, a single bot, which the share-sizing
spec explicitly left out of scope. `SplitAcrossBots` has the same shape more
mildly: it emits `Have { spare(bot) + work, Share(bot) }`, so a chain bound
anywhere but `bot` asks a bot for stock sized against someone else's.

Three coherent ways out, none of them obviously right, all of them
design decisions rather than bug fixes:

1. **A `Share(b)` chain runs on `b`** (owner, or a first candidate tier). Makes
   the sizing true by construction. Costs parallelism exactly where it matters:
   every share `Researched` emits names `chain_actor`, so the trigger subtree
   and the pack subtree — bots 2 and 3 concurrently in the repro, 22072 ticks —
   would serialise onto one bot. `ChainOwnerInfeasible` also reads "a caller
   named this bot", which would then be false.
2. **Size a share against the roster minimum** rather than against `b`. Keeps
   shares freely assignable and over-produces a little; directly undoes the
   per-bot sizing that landed tonight (`6fcbba5c`, `7d614681`).
3. **Give `Researched` a real multi-bot decomposition**, so its bill is not one
   bot's problem in the first place. The largest, and the only one that removes
   the tension rather than choosing a side.

Not attempted. Whoever picks it up starts from the two lines above, which turn
the question into a failing test in about a minute.

---

## The aside: were the retries doing useful work?

**Yes — and the `too far too mine` rejections are not what cost the
iterations.** Across milestones 1 and 2 there were 22 dispatches, 3 rejections
and 1 dispatched-and-never-settled. Most mine actions succeed first time.

What actually costs the iterations is that **a mine action reports success
while delivering roughly half of what it was asked for.** Plan totals per
iteration, from `events.jsonl`:

    milestone 1 (iron):   20 -> 13 -> 9 -> 3 -> satisfied
    milestone 2 (copper): 20 -> 13 -> 4 -> 2 -> satisfied

Each plan is sized from the real inventory, so the drop between plans *is* what
arrived. Milestone 2's first iteration dispatched three mines totalling twenty
copper ore, **all three reported success**, and the next plan still asked for
thirteen — seven arrived. Second iteration: thirteen dispatched, all successful,
nine arrived. The end state is exactly right (20 iron, 20 copper in the
samples), so nothing is being lost after the fact; the actions are simply
returning before they have mined what they were asked for.

Corroborating detail: `mine 4 copper-ore` on bot 2 settled in **266** ticks
against a planned 480, and `mine 1 copper-ore` settled in **0** ticks (and was
rejected). A reply that arrives well before the modelled duration is a reply
that arrives before the mining is done.

So the supervisor's loop is healthy — every plan is strictly smaller than the
last and it converges — but it is paying four iterations for work one should
have finished. Left alone as instructed; it belongs with whoever is changing the
walking and dispatch code, and it is the same family as the already-recorded
"tried to remove 10 copper-plate but removed 9".

---

## Verification

`cargo fmt` (scoped to the two packages touched), `cargo clippy --workspace
--all-features --all-targets -- --deny warnings` clean, `cargo test --workspace`
green (exit 0). Thirteen tests added, two rewritten: three pinning the
`Produced` welding plus its `Holder::Anyone` control, one for the live run
reduced to a fixture, four for `holds` in the planner, three for `goal.holds`
through the bindings, and two for the supervisor's new branch — one that fails
if the check ever stops being asked, one that fails if an unheld goal is ever
reported satisfied. The three that pin defect 2 were confirmed to fail with the
fix reverted, on the exact live error message.
