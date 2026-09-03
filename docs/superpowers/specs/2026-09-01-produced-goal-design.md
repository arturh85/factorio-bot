# `Goal::Produced` — Design

**Status:** **IMPLEMENTED.** Corrected 2026-09-03 — this line read *"approved in outline, not yet implemented"*. `Goal::Produced { item, count, whose, unlocks }` (`crates/planner/src/goal.rs:96-104`), `demand()` (`crates/planner/src/method/have.rs:107`), `attach_unlock` (`:130`), `holds` → `None` (`:218`). It gained a `whose` field beyond the spec, and is deliberately **not** exposed in Lua.
**Date:** 2026-09-01

## Why

A Factorio 2.0 `craft-item` trigger technology unlocks when the item is
**produced**, and the game then researches it itself. The planner models this
twice wrong:

- it subgoals `Goal::Have { trigger_item }`, which **possession** satisfies, so
  a bot already carrying a lab crafts nothing and the trigger never fires;
- it then emits `ActionKind::Research`, which does nothing at all, because the
  trigger firing *is* the research.

That second action is what looped: a live run dispatched `research electronics`
five times, was told success five times, and the supervisor halted it as
`stuck_silent`. Since `46cc6022` the game's refusal is no longer swallowed, so
the failure is now loud — but `goal.researched(...)` remains unreachable for
any technology behind a trigger, which is most of the early tree.

## The shape of the problem

Two constraints that between them rule out the obvious fixes:

1. **Possession is not production.** `Have` is satisfied by what a bot already
   holds, so it cannot express "cause a craft to happen".
2. **A trigger item is not always hand-craftable.** It may be smelted or mined.
   Inlining a hand-craft in the trigger path (attempted 2026-09-01, reverted)
   refuses technologies the old model planned fine — wrong in the opposite
   direction from the bug it fixes.

And a structural one: **a method cannot attach an effect to its subgoals'
actions.** It never sees their ids, by design (`have.rs`, "No explicit `Link`
steps"). So the `Researched` effect cannot be bolted on after the production is
planned by someone else.

## Decisions

### D1 — A new goal: `Goal::Produced { item, count, unlocks }`

```rust
Produced {
    item: String,
    count: u32,
    /// A technology this production unlocks, if it is a trigger's item.
    /// The producing method attaches `Effect::Researched` to the action it
    /// emits, which is the only place that effect can go.
    unlocks: Option<String>,
}
```

"Cause `count` of `item` to come into existence", as distinct from `Have`'s
"end up holding `count`". **`Produced` never subtracts what a bot already
holds** — that is the entire difference, and the reason `Have` cannot be reused
with a flag.

`unlocks` rides on the goal because the effect has to reach whichever action
ends up producing the item, and only the producing method knows which action
that is. Threading it through the goal is the smallest thing that crosses that
boundary; the alternative is a link mechanism the planner deliberately does not
have.

### D2 — Every producing method claims it

`HandCraft`, `Smelt` and `Mine` gain `Produced` alongside `Have`. Each already
knows how to emit its action; the changes are:

- size the work from `count` directly rather than from `shortfall(...)`;
- append `Effect::Researched(tech)` when `unlocks` is `Some`.

Applicability is unchanged: whichever method can produce the item claims the
goal, exactly as for `Have`. That is what makes a smelted trigger item work
without the trigger path knowing anything about smelting.

### D3 — The trigger path emits one subgoal and no action

`Researched(T)` for a trigger technology expands to its prerequisites plus

```rust
Goal::Produced { item, count, unlocks: Some(T) }
```

and **no `Research` action at all**. There is nothing to issue: the game
researches `T` when the trigger fires, and `add_research` refuses a trigger
technology outright.

A technology reaching the pack path is therefore, by construction, one unlocked
by science packs — which is what the existing bill and `Research` action have
always assumed.

### D4 — `AlreadySatisfied` must not claim `Produced`

`AlreadySatisfied` answers "the world already has this" for every goal kind. It
must **not** do so for `Produced`: a bot holding six labs has not crafted one,
and short-circuiting there reintroduces exactly the bug this goal exists to
fix.

This is the single most likely way to get this wrong, because every other goal
kind wants that method and it is registered ahead of the rest.

## Testing

- **`Produced` ignores inventory.** A bot holding ten of the item still plans a
  production. Against `Have`, which plans nothing. Same fixture, both goals, so
  the difference is the assertion rather than the setup.
- **`unlocks` reaches the action.** The emitted action carries
  `Effect::Researched(T)`, and it is the *producing* action — the craft or the
  smelt — not a separate marker.
- **A smelted trigger item plans**, via `Smelt`, with the effect attached. This
  is the case the reverted attempt broke, and the reason `Produced` is a goal
  rather than an inlined craft.
- **A trigger technology emits no `Research` action.** Asserted by kind over the
  whole network, not by counting steps.
- **`AlreadySatisfied` declines `Produced`** even when the item is held.
- **Live:** `goal.researched("automation")` completes against a real game. It
  currently fails immediately with `unmet_prerequisites=[automation-science-pack]`,
  so the before/after is unambiguous.

## Out of scope

Trigger kinds other than `craft-item`. `trigger_requirement` already refuses
those by name, and that refusal stays — costing an unmodelled trigger at zero
would hand back a plan missing the work.
