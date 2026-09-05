# Open items, unowned, as of 2026-09-05 evening

Written because most of these were found inside a session-scoped execution
ledger that gets deleted when its plan finishes. Nothing here is assigned.

## Blocking the blueprint work from being useful

**Siting for fixed-offset blocks.** `Goal::Built` places a blueprint at an
anchor the caller names, and nothing chooses that anchor. A 37-entity
`MinerLine` was attempted at three anchors and never got past planning:
one obstructed tile anywhere along its 21-tile belt run makes the whole block
infeasible, and `PlannerError::ChainOwnerInfeasible` is the correct refusal
rather than a defect. On real terrain that is the normal case.

The sharpest statement of why it matters is not about the planner:
**a researcher's first pasted blueprint will refuse too.** Siting was scoped
out of the blueprint sub-project on the assumption that "given an anchor,
build the block" was complete work. It is not, and this is the evidence.

**Material supply.** Sub-project 3 of the same design. The bill exists and
resolves — `MinerLine` plans in 491 actions at 55% roster utilisation — but
every live build so far cheated the materials in to isolate placement from
gathering. Nobody has measured an honest end-to-end block build.

## Found in passing, in files nobody was working in

**`FactorioEntity::new_stone_furnace` uses a 1.8 collision box** while this
repo's own prototype fixture records `1.3984375`. Found while correcting a
post-mortem that had blamed a test fixture's box for a defect that was
actually its illegal position. The two boxes are indistinguishable in every
current fixture, so nothing fails today.

**The executor's clockless fallback still scales wall-clock by game speed.**
`Actuator::game_tick` returning `None` puts the wait back on
`ticks_to_wall_clock(ticks, speed)`, which assumes the server delivers
`60 × speed` ticks a second. A starved server delivers fewer, which is the
exact failure that once collected from a furnace 433 ticks early. Two things
make this worth revisiting: `game_speed` was stubbed at `1.0` until
2026-09-05, so the >1x path has never been exercised on a loaded box, and
headless runs at 5x are now routine.

**`plan.steps` exposes no `direction` for `place` steps.** A Lua script can
verify a placement's position against the plan but not its facing, so
direction can only be checked against the blueprint's own decode or against
the live game. Both were used to prove the direction migration; neither is
the plan itself.

## Named in the 2026-09-05 morning review and still open

**The horizon is unnamed.** Nothing Space Age specific has been built, and
the gap between here and a rocket is routed belts (built, unwired), oil,
and bulk placement (built for blueprints, unsited). A goal named
"rocket on Nauvis, honest, four bots" would stop a Space Age number being
quoted against work that has never touched Space Age.

**Snapshot and restore recovery.** Deferred in the morning on the grounds
that runs were too expensive to justify it. That reason has expired: a
headless run at 5x costs about three minutes. Worth reconsidering on its
merits rather than on cost.

## What four bots are actually worth (not an open item — a measurement)

Recorded here because it answers a question this project has been asserting
rather than measuring, and because it corrects earlier claims. **The finding
and the numbers are the speedrun session's**, measured after it corrected the
planner's walk model (`3d45f40e`); they live in its own words in the plan
record. Repeated here so they are not lost.

The walk model under-charged every walk by 22-25%: `travel_ticks` credited an
action's `radius` although the bot stops on the inner ring, and the speed
constant was the prototype's `0.15` where measurement over 196,717 ticks gives
`0.1413`. Nothing got slower when it was fixed — a live 5x run came out the
same length to within ten ticks — but **executed over planned moved from
1.059 to 0.998**, which is the model no longer lying about the largest single
cost in a plan.

**The mis-credit flattered multi-bot plans specifically**, so every
four-bots-versus-one speedup quoted in this repo before that commit is
overstated by an unknown amount. On the honest model:

| goal | four bots against one |
|---|---|
| `researched:automation` | **1.25x** |
| red science | **2.11x** |
| green science | **2.48x** |

Eight bots add a further 16% over four for green.

**So bots pay off in proportion to the independent gathering and crafting a
goal contains, and barely at all for automation.** That is a sharper claim
than "multiple bots are faster", and it is the first version of it anyone
here has measured rather than assumed.

## The one with a plan already

**The self-fed cell**, through `method::connect`. Production plateaus at
11:46 for copper and 11:51 for red packs in a run that ends at 15:19 — the
factory stops while the bots keep working, and every run's production
plateaus at exactly the plan's bill because the cell is charged by hand and
nothing feeds it afterwards. This is also the first caller `connect_steps`
would ever have: **it has never placed a belt in a real game**, and its
absence of a caller is what hid a geometry defect through four clean reviews.

Agreed sequencing with the speedrun session: no belt-placing code enters a
measured 1x run until it has crossed a headless 5x run with zero failed
actions on both instances. Six minutes against twenty-five.
