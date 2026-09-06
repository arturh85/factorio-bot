# Siting a block: what it does, and what is still unproven

2026-09-06. Sub-project 2 of the blueprint work. Spec:
`docs/superpowers/specs/2026-09-05-block-siting-design.md`.

**Status: the live run has NOT happened yet.** Everything below is offline
evidence plus unit tests. The section that would say "entities stood where the
plan said" is empty on purpose, and this note should not be read as proof of a
working feature until it is filled in.

## What changed

`Goal::Built` used to take a bare `anchor: Position` and refuse if that exact
spot was occupied. It now takes a `Site`:

```lua
goal.built(bp, {x = 10, y = -20})        -- Site::At      exact, as before
goal.built(bp, {near = {x = 0, y = -30}}) -- Site::Near    search from a hint
goal.built(bp)                            -- Site::Anywhere search from a stable seed
```

Resolution runs in one fixed order on every expansion:

1. **Recover** the anchor from the block's own entities already standing.
2. Failing that, **search** rings outward from a seed, refusing any site whose
   mining drills would not sit on ore.

## The part that matters: recovery, and why the seed cannot move

`Goal::Built` is re-expanded on **every replan**. If siting recomputed the
anchor each time, and the world had changed because we had been building into
it, a block half-built at site A could restart at site B — two half-factories,
no error, and a production curve that still rises. That is this project's
signature failure and it has shipped twice.

Two mechanisms prevent it, and they are load-bearing together rather than
separately:

- **Recovery** reads the anchor back off standing entities, requiring **two**
  of the block's entities to agree before it will answer. One is not enough: a
  single unrelated entity of the same name silently mis-sited a real blueprint
  in an existing fixture, and a wrongly *asserted* anchor is worse than a
  searched one.
- **A replan-stable seed** covers the window that leaves. With the two-entity
  floor, a block with exactly ONE entity standing recovers nothing and falls
  back to the search — so the search must return the same answer. It does,
  because placements only ever ADD obstacles and the search skips the block's
  own entities, so every earlier ring stays blocked. **That argument only holds
  if the seed is fixed**, which is why `Site::Anywhere` seeds at the nearest
  extractable ore patch (for a block with drills) or the world origin, and
  never at the roster centroid: bots walk, and a moving centroid re-orders the
  rings.

A third hole was found by review: `occupant_of` counted **live characters** as
obstacles, so an unrelated bot standing in a candidate ring blocked it, walked
away, and a later expansion could pick that ring. Siting now asks about durable
ground only (`siting_occupant`), while `expand`'s fixed-anchor pre-check still
names a character — those are different questions. A bot in the way of *this
anchor now* is exactly what a caller needs told; a bot standing where a block
*might* go is noise.

## Ore-awareness, and why it is deliberately conservative

A drill on bare ground places perfectly, passes every geometry check, and
produces nothing. So a site is refused unless **every mining drill has at least
one ore tile under its own 3×3 footprint**.

That is a floor, not a measurement: a real electric mining drill mines a 5×5
area while colliding on 3×3, and no mining radius is available on the
prototypes. So it can refuse a site where the drill would in fact reach ore just
outside its box. **That direction is the safe one** — a false refusal costs a
site, a false acceptance is a drill that produces nothing.

## What is proven, and what is not

**Proven offline:**

- `FurnaceLine` (179 entities) sites and plans: 988 actions, makespan 55,547.
- Determinism: the same world produces a byte-identical anchor across runs.
- Replan stability, recovery-over-`Site::At`, the one-entity floor, and
  bystander-immunity all have tests that have been **seen to fail** — each was
  verified by deliberately breaking the code and watching the assertion mismatch
  before restoring it.

**NOT proven:**

- **Nothing has been built in a live game from a sited anchor.** Section below
  is empty.
- **`MinerLine` does not site at all**, at radius 48, 64, 96 or 128.

## `MinerLine`'s failure is a planner defect, not a limit of siting

This was nearly recorded as structural — its belt-and-pole corridor runs between
the two drill columns, over the same ore the drills need, and the planner
refuses non-drill placement on ore. One read-only query against a live game:

```
ore at (-42.5,-37.5)  belt=true pole=true drill=true
```

**A transport belt and an electric pole can both be built on an ore tile.**
Factorio's resources sit on the `resource` collision layer only — which is why a
character walks straight through them, as the mod's own walk-stall comment says.

The planner computes `resource_blocks = !stands_on_resources(name)` at every
placement check, so ore blocks everything that is not a mining drill.
`stands_on_resources` was added because `is_area_free` "refused a drill
everywhere on every map" — the right diagnosis, but it carved an exception for
drills instead of correcting the general rule.

This is not blueprint-specific. `is_area_free` and `placement_occupant` are what
`method::connect` routes belts through and what `method::assemble` sites cells
with, so **a belt route that would legally cross a patch is refused as
`NoRoute`, near any patch, on any map.** It fails in the safe direction —
refusing legal ground, never building on illegal ground — which is exactly why
nothing caught it: it produces "no route" and "no site", never a broken factory.

Being fixed separately (`ore-does-not-block`), from prototype collision masks
rather than from the single query above: *"a belt and a pole are legal on that
ore tile"* is an observation; *"nothing collides with resources"* is a rule, and
only the first was measured here.

### That fix landed, and `MinerLine` still does not site

Re-measured against `21a698ff`, after ore stopped blocking placement. The
refusal changed, which is informative — it is no longer about the corridor:

```
no clear site for a 37-entity block within 48 tiles of [-12.5, -13.5];
nearest obstruction: the electric-mining-drill at (-7, -12)
would stand on no ore it can mine
```

So the belts and poles are now free to cross the patch, and what refuses is the
**ore-coverage rule itself**: siting demands that *every one* of the 13 drills
sits on ore, and no anchor within 48 tiles of the seed satisfies that.

### Resolved: `MinerLine` is unbuildable by policy, at any anchor

The search's message names a drill, which pointed at the ore-coverage rule.
Asking with an **explicit** anchor instead gives the real answer:

```
goal.built(MinerLine, {x = -17, y = -34})
  cannot build transport-belt at tile (-13.5, -33.5): ore, which this
  planner will not bury
```

The same at `(-16.5, -33.5)`, one of the oracle's own 359 anchors. So it is not
the coverage rule, not the oracle's tile model, not the search's reach — it is a
**separate, deliberate policy: this planner will not build over ore.**

That policy survives the `ore-does-not-block` fix and is right to. Ore stopped
*colliding* with placements, because no buildable prototype carries the
`resource` collision layer. But a belt laid over an ore tile still **buries** it
— the ore cannot be mined until the belt is removed — and a planner that buries
its own patch can plan itself into `NoApplicableMethod` for the very ore it
needs. Collision is a fact about the game; burial is a judgement about
consequences, and only the first was wrong.

**So `MinerLine` cannot be built as designed, at any anchor, and that is a
property of the fixture rather than of siting.** Its belt-and-pole corridor runs
*between* its two drill columns, which is to say directly over the patch the
drills are mining. A human building it accepts burying a strip of ore; this
planner does not.

Which of those is right is an open design question, not a defect:

- **Keep the policy** and `MinerLine` needs redesigning — the belt run moved off
  the patch, which costs inserter reach and makes it a different blueprint.
- **Relax it for belts** and the planner may bury ore it later needs, which is
  the failure the policy exists to prevent, and which would surface as an
  unrelatable `NoApplicableMethod` a long way downstream.
- **Make it a cost rather than a refusal** — bury ore only when nothing else
  fits — which is the honest answer and much the largest piece of work.

**Nothing regresses: `MinerLine` has never sited.** But this note should not be
read as "siting handles drill blocks". It handles `FurnaceLine`, live, three
times. The drill case is blocked on a policy question nobody has decided, and
the refusal now names the exact tile and the exact reason, which is what makes
the question answerable at all.

## Cost: the 96 seconds is not siting's

A sited `FurnaceLine` plan takes ~96 s, and it is worth saying plainly that this
was misattributed twice before it was measured.

| | AFTER (siting, `Site::At`) | BEFORE (`293ec331`, bare anchor) |
|---|---|---|
| | 94.6 / 96.5 / 96.9 s | 52.0 / 51.9 / 51.8 s |

Interleaved, three runs a side, load recorded per run, on a floor deliberately
cleared. Spreads 2.5 % and 0.3 %, so the ~45 s gap is real. **The cause is
`plan_best`**, which runs the whole expand-and-schedule pipeline twice — once
per `DrainPolicy` — keeping the shorter schedule; 96/52 = 1.85. That feature
does not exist at `293ec331`. Siting's own contribution, `recover_anchor`, is
**under 10 ms per call**.

`recover_anchor` *was* genuinely broken — it scanned the whole entity tree once
per blueprint entity, 179 whole-world scans for `FurnaceLine`, worst observed
call 7.86 s — and is fixed by scanning once and bucketing by name. That fix is
worth keeping. It was simply never the 96 seconds.

### Re-measured after `plan_best` was fixed: 96 s → 48 s

`c9547bc9` stops `plan_best` running the pipeline for both drain policies when
nothing could have differed. Re-measured here, same command, same dump, three
runs on a quiet floor: **48 s, 48 s, 48 s** — an exact halving, which is what
removing one of two identical passes should produce.

**And the remaining 48 s is planning, not I/O.** A trivial goal
(`have:iron-plate:5`) against the same 865 MB dump takes **3 s**, twice. So
loading the world costs 3 s and planning this block costs ~45 s. That is worth
stating because the obvious suspicion — "it is just reading an 865 MB file" — is
wrong, and would have sent the next person optimising the wrong thing.

### Every number above is a DEBUG build — release is ~7× faster

An order-of-magnitude disagreement between two careful measurements turned out
to be the build profile, which was in neither party's model. The speedrun
session measured the same CLI path on both binaries:

| | trivial goal | green, 4 bots |
|---|---|---|
| **debug** | 2.70 / 2.72 s | 31.04 / 29.51 s |
| **release** | 0.77 / 0.73 s | 4.55 / 4.24 s |

**~7× on this workload**, and it reconciles the 96 → 48 s here against their
8.57 → 4.62 s exactly: an ~11× gap that is debug-versus-release, not
CLI-versus-in-process as we had both assumed. Both measurements were correct
about their own binary, and **neither of us said which binary it was.**

> **A timing is a claim about a binary, not about a program.** A baseline has to
> name its profile as well as its commit. Two people comparing numbers an order
> of magnitude apart, for a reason absent from both their models, is what the
> omission costs.

So: **quote release for anything anyone will act on.** The figures above are
debug and remain honest as such.

### Measured on release, because the extrapolation was wrong

The ~7× above predicts ~4–5 s for this block. **Measured, it is ~11 s** — three
runs on release, same command, same dump, quiet floor:

| build | `FurnaceLine` sited, 4 bots | trivial goal (load only) |
|---|---|---|
| debug | 48 / 48 / 48 s | 3 s |
| **release** | **13 / 10 / 11 s** | **1 s** |

So the debug-to-release ratio on *this* workload is **~4.4×, not ~7×** — the 7×
was measured on green, and a ratio measured on one goal does not transfer to
another. Extrapolating it would have understated this block by half.

**The honest figure is therefore ~10 s of planning per expansion on release**
(≈11 s wall minus ≈1 s to load the 865 MB world). That is a great deal better
than 45 s, and it is **not** the non-issue that ~4–5 s would have been: a run
that replans four to seven times pays 40–70 s of planning for one block, and
pays it exactly when things are going wrong, which is when replans happen.

The I/O discriminator survives and sharpens: loading the world is 3 s debug and
1 s release, so it is a small fixed cost in either build and the remainder
genuinely is planning.

One more correction from the investigation, against both our guesses: the split
is not even — on this block expansion is 63 % and scheduling 37 % — and
`expand()` rehearses before it plans, so `plan_best` was running **four**
expansions rather than two.

**The method failure is the transferable part.** The first experiment showed a
fixed anchor still cost ~95 s and was read as exonerating the search and
implicating recovery. But a fixed anchor *also* calls recovery, so it separated
the search from everything-else and never separated recovery from the rest.
**Ruling out one of three candidates convicts neither of the others.**

## Siting is bounded by what the model has ingested — and that is now liftable

Ore-aware siting chooses among the resources the model knows, which at t=0 on
seed 31337 is a fraction of what exists: 940 iron tiles against 3,658 within
±672, 462 copper against 2,712, no crude oil against 43, and **no enemy
structures against 154**.

Since `Goal::Charted` landed this is no longer a hard bound:

```lua
goal.charted(0, 0, 384)   -- widen what the model knows
goal.built(FurnaceLine)   -- then site against the wider set
```

Two caveats must travel with that or the sentence misleads. The mechanism is a
**mod-side generate call clamped to 4 chunks, not walking** — a bot cannot cross
the generation frontier on foot at all. And the ground appears before any bot
arrives, so a run that charts must quote `ground_generate_calls` /
`ground_generated_chunks` / `ground_generate_failures`. "We made the ground
exist" is the honest sentence; "the bots explored" is not.

**Siting does not check for enemy structures.** A block can be sited into a
biter base and the planner will not know. One charting ring puts 32 of them into
the threat index, so the data now exists — the check does not.

## The live run

`run-1788663566-25023`, headless, four character bots, game speed 5, seed 31337
on a fresh map. `goal.built(FurnaceLine)` with **no anchor** — siting chose.

**Siting is proven. The build did not finish, for a reason that is not siting.**
Both halves are real and they are separable, so they are reported separately.

### What siting did — proven

- **Anchor chosen: `(0.00, 0.00)`**, with no anchor supplied by the caller.
- **179 of 179 planned placements were consistent with that one anchor**, none
  disagreed. The block was sited coherently, not approximately.
- **The footprint was actually clear**: the script's own pre-scan found **0
  non-resource, non-character entities inside the chosen footprint** before
  `goal.run`. So the site was genuinely empty ground, not merely a spot the
  planner failed to refuse — a distinction this project has been caught by
  before.
- Plan: 183 steps across 4 bots, makespan 1,405 ticks.

### What stood — proven, and the direction migration holds

**129 of 179 entities read back off the live surface at the right tile, with
the right name, facing the right way**, including both underground belts with
the correct half (`input`/`output`).

**Zero wrong directions. Zero wrong halves.** Everything that was built was
built correctly. That is the second independent confirmation of the 8-point to
16-point direction migration, and the first at a **sited** anchor rather than a
hand-chosen one.

### What did not stand — and why it is not siting

**50 entities are missing, and the cause is a single failed placement that
aborted the rest of the batch.**

```
failure[1] id=130 status=failed
  cannot place item 'transport-belt' because a character is standing in the
  footprint; dispatched 4 times over 534 game ticks and refused every time
```

One bot stood where another needed to build, the placement was refused four
times, and the batch stopped — leaving roughly fifty later placements never
dispatched. Two bots also stalled walking, **blocked by the block's own
entities** (`blocked by our own stone-furnace`, `blocked by our own inserter`).

> **RETRACTED, 2026-09-06 05:45 — this run loaded a STALE MOD and its footprint
> evidence is void.** At the run's branch HEAD (`bcadac64`, 02:57) the mod had
> **no `describe_footprint_blockers` at all**; it arrived at 03:30 in
> `5361c7d3`, half an hour later, together with the fix that waits for a busy
> blocker instead of giving up in 1.8 s. So the refusal below is the behaviour
> of an older mod, and the conclusion I drew from it — that the speedrun
> session's footprint fix was incomplete — **was drawn from code I was not
> running and is withdrawn.**
>
> **Provenance did not catch it and could not**: `git.commit` records the
> *checkout's* HEAD, and I launched the worktree's binary from the main
> checkout's directory, so the run recorded `191df2db` — a commit whose mod
> *does* have the diagnostics — while loading the worktree's older copy. A
> symlink target is not the same claim as the bytes that loaded. This is a third
> blind spot beside the two `world.dump` ones already documented.
>
> **What survives is everything that does not depend on the mod version**: the
> anchor, the 179 one-anchor-consistent placements, the clear footprint, and the
> zero wrong directions and halves. **What does not survive is the footprint
> refusal and the ~50-entity shortfall**, both of which are behaviour of a mod
> version nobody is running any more. They must be re-measured before anything
> is concluded from them.
>
> The habit that would have caught this costs nothing and is now the rule for
> this note: **`readlink workspace/mods/BotBridge` before a run, and quote the
> `Using mods directory` line in the write-up.**

Three things follow, none of them about siting:

1. ~~**This is the footprint-refusal defect the speedrun session independently
   hit tonight**, failing headless too, so it is failing in both modes.~~
   **Withdrawn** — see the retraction above. The refusal is real but it is an
   old mod's refusal, and it says nothing about whether the current fix works.
2. **A single unresolvable placement costs the whole remainder of a block.**
   Failing fast is defensible, but the blast radius here is 50 entities from
   one bot standing in one tile, and nothing retried or re-planned around it.
3. **The band split did not prevent bots blocking each other.** The spec's
   claim is that bands are the structural reason two bots cannot trap each
   other; `FurnaceLine` is 29 wide by 11 tall, so bands are vertical slabs ~7
   tiles wide, and bots working adjacent slabs still meet at the boundary.
   Bands stop bots *interleaving*; they do not stop them *colliding*.

### Reproduced three times, in separate processes and workspaces

The run above is one of **three independent live runs** — two of them in a
different workspace, on different ports, by a different process:

| run | stood correct / 179 | wrong direction | wrong half | wrong entity |
|---|---|---|---|---|
| A | 129 | 0 | 0 | 0 |
| B | 89 | 0 | 0 | 0 |
| C | 129 | 0 | 0 | 0 |

Two things this buys that one run could not:

- **The anchor was `(0.0, 0.0)` in all three live runs and in the offline plan
  against the world dump.** Deterministic across processes, workspaces and the
  offline path — which is the property the whole design rests on, and it is now
  evidence rather than an argument from the code.
- **Every mismatch in every run was a plain MISSING entity.** Not once a wrong
  direction, a wrong half, or a wrong entity. Whatever got built was built
  correctly, three times over.

### How much to trust this

The failures are **logical, not starvation**: a character occupying a tile is a
refusal the game computes, not a timeout, and it repeated across 534 game ticks
rather than expiring.

But the floor was **not** quiet, and the numbers say so: **delivered tick rate
was 63.8 % and 71.4 % of nominal**, both below the 80 % flag, with sibling
worktrees compiling at load 13-31 even though no other Factorio was running.

So the two halves of this result carry different weights, and the difference is
the point:

- **Siting and placement-correctness are trustworthy.** Three independent
  reproductions, identical anchor, zero incorrect placements. Starvation cannot
  make an entity stand in the wrong place.
- **The completion count is real but load-modulated.** The defect exists at any
  tick rate — 89/179 and 129/179 differ, and the *cause* is identical in all
  three — but **its severity is not established.** Nobody should quote a
  completion rate from these runs. A quiet floor would give a number; these give
  a diagnosis.

**Both of these have since been superseded, and the update is worth more than
the original.** The speedrun session ran eight `furnace_run` block builds on an
identical 183-step plan — same binary, same mod, only the executor arm differing
— and established two things this note could not:

- **The refusal is a RACE, not a property of the plan.** One of the eight runs
  hit no refusal at all, on the same seed, map and plan. So a single clean block
  run proves nothing about this class, and neither did my three unclean ones:
  they bound the *cause*, never the frequency. "Load-modulated" was too generous
  a description of what I had — a race is modulated by scheduling, of which load
  is only one input.
- **A failed action now costs its dependents rather than the batch**
  (`0fedcb95`): before, four failures cost 29 entities; after, five failures
  cost five, with every planned action dispatched. So the blast radius I flagged
  from my void run was real, but it is now *measured* rather than inherited from
  evidence that did not survive.

The consequence for anyone re-running the sited block: **do not read one run.**
Read several, and expect the refusal to be absent from some of them.

### Still not proven, and worth repeating

`FurnaceLine` carries 13 poles and **no generator at all**. The 87 belts, 48
inserters, 2 splitters and 2 underground belts are proven to **stand** and have
never been shown to **move a single item**. Nothing in this run could show it,
and nothing in it tried.

---

## Ghosts mark the site (Task 8) — built and proven live

The owner asked for this directly: a ghost records the siting decision in the
world *before any real entity exists*, and it makes a run legible, because the
viewer shows the plan appear and then fill in.

**Both prerequisites were answered live before a line was written** — the first
time tonight that ordering was right:

- **Ghosts do not expire.** `time_to_live` does not apply to an `entity-ghost`
  at all ("Entity is not combat-robot, highlight-box, smoke, or sticker"), and
  `LuaForce.ghost_time_to_live` does not exist in 2.1.17 either. *Absence of a
  TTL property is not proof of immortality; no long-duration test was run.*
- **A real placement consumes the ghost beneath it**: `ghost_before=1
  real_placed=true ghost_after=0 ghost_still_valid=false`.
- **A ghost reports `name = "entity-ghost"` with the real name in
  `ghost_name`.** Matching on `ghost_name` rather than `.name` is what stops a
  ghost being read as an already-built entity.

### The live proof — `run-1788681431-44834`

Headless, four bots, 5×, fresh seed-31337, **mod verified before the run**
(the run's own line names `.worktrees/ghosts/mods/BotBridge`, matching the hash
taken beforehand).

- **The stamp is genuinely dispatched**: `kind=stamp_ghosts`, **exactly once**
  on a fresh site. A ghost path nobody dispatches would be inert, and 2,452
  green tests would not have noticed.
- Siting again: anchor `(0.00, 0.00)` self-chosen, 179/179 placements
  one-anchor-consistent, footprint verified clear.
- **176/179 stood correct, zero wrong directions, zero wrong halves** — up from
  129/179, the executor's blast-radius fix working: three failures cost three
  entities where one failure previously cost fifty.

### My assertion was wrong, and the truth is better

The check asserted **"ghosts surviving the build: want 0"**. Three survived —
and exactly three entities were missing, all `transport-belt`, at (16.5,10.5),
(6.5,5.5) and (20.5,5.5).

**The correspondence is the feature.** A real placement consumes the ghost
beneath it, so a ghost survives precisely where its entity was never placed.
**The leftovers are the remaining work, made visible** — which is the viewer
benefit arriving in a form nobody designed for. The correct assertion is
`ghosts_surviving == entities_missing`; zero is right only for a block that
finished. I asserted the happy path and the run corrected me.

### Two defects the work found, both of the "marker breaks the thing it marks" kind

- **`occupant_of` treated a standing ghost as a colliding entity**, which would
  have refused the real placement over its own marker. Fixed by excluding the
  ghost name from both entity-scanning loops. Ghosts do not collide in
  Factorio; the planner's model said otherwise.
- **`place_blueprint` mines** any non-character, non-resource entity in its
  build area **regardless of `only_ghosts`.** Safe today only because
  `is_fresh_site` emits the stamp solely when `recover_anchor` finds nothing —
  a planner-side promise the actuator cannot see or enforce. Documented in
  `rcon_actuator.rs`, and it needs a hard look if `StampGhosts` ever gets a
  second call site.

### Not proven

**That recovery reads a ghost in a live replan.** This run built in one pass, so
the ghost path in `recover_anchor` was exercised only by unit tests — verified
load-bearing by disabling it and watching exactly the ghost test fail, but not
by a game. Proving it needs a run interrupted mid-block and re-planned.

---

## Ghost recovery does NOT work live — measured, and it is inert end to end

The ghosts note above says plainly that recovery reading a ghost was **not
proven**, only unit-tested. It has now been measured, and the answer is
negative.

A script stamped the 9-entity `MovingBlock` as ghosts through
`rcon.place_blueprint(only_ghosts = true)`, at an anchor 24 tiles from spawn —
deliberately far enough that "recovered from the ghost" and "searched from the
origin" could not be confused. Then it asked the planner where it thought the
block was:

```
ghosts standing after the stamp:  9   (want 9)
ghosts the PLANNER can see:       0   (the game sees 9)
REAL block entities standing:     0
plan: 38 steps, 10 placements, min placement (-10.0, -17.0)
distance from the ghost anchor: dx=34.5 dy=23.5
FAIL: the planner sited elsewhere -- it searched afresh and ignored the ghosts
```

**The discriminator is the middle line.** `rcon.find_entities_in_radius` asks
the *game* and sees nine. `world.find_entities_in_radius` asks the *planner's
own model* and sees none. So `recover_anchor`'s ghost pass is not wrong — **it
is correct code that can never fire, because the model never carries a ghost to
read.**

Where they are lost: `on_some_entity_created` has no ghost filter and would
stream one happily, but a script blueprint build does not raise that event, so
nothing is ever written out. Neither the executor's `ActionKind::StampGhosts`
nor a hand `rcon.place_blueprint` reaches the world model, because both go
through the same call.

**So the feature is inert end to end**, and the unit tests could not have shown
it: they construct ghosts directly in `PlanState`, which is exactly the step
the live path never performs. A reader with nothing to read — the same shape as
a module with no caller, one level down.

**What the earlier live run did and did not prove.** `run-1788681431-44834`
showed a stamp is *dispatched* and that ghosts *appear in the game*, and its
three surviving ghosts corresponded exactly to three unbuilt entities. All of
that stands. **It never showed a ghost being read**, and this note said so at
the time rather than being corrected into it.

**The fix is mod-side and is not mine**: created ghosts need to reach the entity
stream, either by writing them out explicitly after a blueprint build or by
including them in the world snapshot. Until then, `recover_anchor` falls through
to the vote path on every real expansion, which is the behaviour that existed
before Task 8 — so nothing regressed, and nothing was gained either.

---

## A block that MAKES — proven live

`MovingBlock` showed a block moves items. `SmeltingBlock` shows a machine
inside one consuming an item and producing a different one, with the arms
either side moving them and nothing carried by hand.

Headless, two bots, 5x, fresh seed-31337, on a **second instance**
(`scratch/blocks.toml`, rcon 4360, `workspace/blocks`) running in parallel with
the speedrun session's work rather than queueing behind it.

- Plan: **6 steps** — five placements plus the ghost stamp. Sited itself; the
  furnace stood at the blueprint's own `(5.0, 3.0)`.
- Build: done, 0 failed, 0 lost, **0 pending**.
- Everything within 10 tiles afterwards: `burner-inserter x2, iron-chest x2,
  stone-furnace x1` — the whole block.

| tick | source ore | sink plates |
|---|---|---|
| 444 | 50 | 0 |
| 2,042 | 42 | 5 |
| 4,453 | 29 | 18 |
| 6,054 | 21 | 26 |
| end | — | **28** |

**About one plate per 200 ticks, against a stone furnace's own 192-tick smelt
time.** The rate corroborates the mechanism rather than merely counting output:
a hand-fed chest would not track the furnace's cadence.

After the charge the script issued only read-only queries — no dispatch, no
`insert`, nothing carried. (Stated as what the script did; this run wrote no
`record`, so there is no event log to cite, unlike `run-1788685081-91006`.)

### Why this block exists rather than FurnaceLine

`FurnaceLine`'s 48 inserters are the **electric** `inserter`, which is not
enabled at t=0 on seed 31337 — the defect the self-fed cell's first caller
exposed in `connect_steps`. So the project's flagship fixture cannot be built
early at all. `SmeltingBlock` is burner-only and needs no electricity and no
generator.

**Its geometry is inherited, not authored.** Every offset is lifted from
`FurnaceLine`, which stood 176 of 179 entities correctly live: furnace on
integer coordinates because it is 2x2, inserters on half-integers, both
direction 0. Getting that parity wrong yields a layout that places perfectly
and does nothing.

### The limitation, which the run was built to measure

**A burner-only block has a fuelled input side and a starving output side.**
The input arm carries coal and self-fuels from it. The output arm carries iron
plates, never coal, so it has no fuel source and runs exactly as long as the
charge it is given — five coal here, disclosed in the script header. Scaling
this block means either electric inserters (post-research) or a coal path to
the output side. That is the same shape as the speedrun session's "one fuel
charge" finding, arriving from the block end.

### Four instrument errors this run made before it made a measurement

Recorded because each produced a *plausible wrong number*, not an error:

1. **Cheated the materials after planning.** The planner saw empty inventories
   and solved gathering too: three furnaces rather than one, because it decided
   to smelt the plates for the chests. The block's own placements then waited on
   production a single batch never finished — and reported `done`.
2. **Read `inventory` where a chest keeps `output_inventory`.** Reported an
   empty chest holding 50 ore.
3. **Indexed the inventory by item name.** Factorio 2.0 returns an *array* of
   `{name, count, quality}`, not a map, so the lookup was nil — an empty chest
   again, by a second route.
4. **Measured a plateau in samples rather than ticks.** Eight identical readings
   two ticks apart declared the output stopped, 16 ticks into a 192-tick smelt.
   The same error as sampling 40 times in 36 ticks, one loop further along.

Each read as "the block made nothing". None of them were about the block.
