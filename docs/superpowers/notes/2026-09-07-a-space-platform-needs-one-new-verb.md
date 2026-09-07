# A space platform needs one new verb, not two

*2026-09-07. Established live on 2.1.17 + Space Age, after two wrong answers —
one from a subagent and one from me.*

## The result

```
t+12s: surfaces=2  platform=paused  surf=platform-1
t+24s: surfaces=2  platform=paused  surf=platform-1
```

A space-platform surface exists. The line immediately above it in the same run
is **`LAUNCH -> false`** — the explicit `launch_rocket` call was *refused*, and
the platform came into being anyway.

## The sequence that works

1. `LuaForce.create_space_platform{name, planet, starter_pack}` — creates a
   pending platform in `waiting_for_starter_pack`, `surface = nil`.
2. **Insert** the starter pack into the silo *while it is building a rocket*.
   It loads as cargo into `rocket_silo_rocket`.
3. The silo finishes the rocket and **launches itself**. Nothing asks it to.
4. Surface `platform-1` appears; the platform goes to `paused`.

## What this costs the executor

**One new capability, not two.** Our verbs are Mine, Chop, Craft, Place, Insert,
Remove, Research, SetRecipe, Evacuate.

- `create_space_platform` is **force-level with no `LuaEntity` receiver**, so it
  cannot be expressed as `Place` or `Insert` — that is a structural fact about
  an entity-scoped vocabulary, and it needs a mod function and an action kind.
- **No launch verb is needed.** The silo launches on its own once a rocket
  carrying valid cargo has somewhere to go.
- Everything else is `Insert`, which we already have.

## Why the launch is automatic, and why there is nothing to configure

The owner named the mechanism: a silo auto-launches when it has valid cargo. In
Factorio 1.x that was the writable `LuaEntity.auto_launch`. **In 2.1.17 that
attribute does not exist** — `auto_launch` appears zero times in
`runtime-api.json`. The behaviour is now destination-driven, governed by
read-only prototype properties: `launch_to_space_platforms`, `launch_wait_time`,
`can_launch_without_landing_pads`.

So there is no per-silo boolean for a mod function to set. That is the
practically important half: `Insert` really is sufficient on the silo side.
`launch_wait_time` is the likely reason a first rocket takes a couple of
minutes.

## Three wrong answers on the way, and what each was

**A subagent concluded two new capabilities were needed**, including a *targeted*
launch naming the platform. Its evidence was real — a plain `launch_rocket()`
returns `true`, runs a full flight cycle, consumes the pack, and leaves
`#game.surfaces` at 1 — but the conclusion did not follow. It never tried
inserting the pack during the build.

**I then blamed the shortcut.** The probe had set `rocket_parts = 50` directly,
so I proposed that skipping the build phase left the rocket empty. Plausible,
and wrong: an honestly-built rocket behaved the same way.

**The actual variable was WHEN the pack is inserted.** Into a silo that is
building, it loads onto the rocket. Into a finished, empty rocket, it sits in
`rocket_silo_attached_cargo_unit` and goes nowhere. Neither the agent nor I
tested that ordering until the owner asked *"then we need to insert the starter
pack, did you do that?"* — a question about the obvious step, which is exactly
the step both of us had done once, early, in the wrong state, and never revisited.

## The methodological part

**I read a refused call as the blocker for four consecutive experiments.**
`launch_rocket` returning `false` was true every time and never the reason
anything failed; the game was doing the delivery itself. Every hypothesis I
formed was about *why the call was refused*, when the call was irrelevant.

That is a variant of the class this repo has been cataloguing all week — a
correct signal that sends the reader the wrong way — with an extra turn on it:
here the misleading signal was one **I** had chosen to measure. `occupant_of`
naming terrain misled us because the code said it; this misled me because I
kept asking a question whose answer did not matter.

The tell, in hindsight, was available from the first run: the platform's state
changed from `waiting_for_starter_pack` and the pack was consumed, in a run
where **no launch call had succeeded**. Something was clearly happening without
me. I noticed it, wrote it down as "a promising signal", and went back to
debugging the launch call.
