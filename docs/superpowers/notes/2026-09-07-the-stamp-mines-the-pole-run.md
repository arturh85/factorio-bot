# The stamp mines the pole run

*2026-09-07. Answering the handoff in
`2026-09-07-a-powered-block-that-is-not-powered.md`. Two live runs on seed
31337, headless, 10x, release binary at master `1bb4ec67`.*

## The short version

`ensure_powered`'s `Some` is a claim about a **plan**: *if every step it
returns is executed and every pole in it is still standing, the game's own rule
says the site is powered.* The claim is checked properly — over
`PlanState`'s union-find, not over `POLE_STEP` arithmetic — and in a live
reproduction it was **true**: an electric block ran on planner-sited,
planner-built, planner-wired power with nothing cheated.

What breaks it is the last clause. **`ActionKind::StampGhosts` mines the poles
`ensure_powered` just laid**, and reports nothing, and the run still ends
`done=true failed=0 lost=0 pending=0`.

Of the handoff's three candidates this is **the third** — "something between
`Some` and the standing entities". The first two are falsified below.

## What was run

`scripts/false_power_probe.lua`: the 28-entity `ElectricOreToPlate` blueprint
from `electric_smelter_live.lua`, planned with `goal.built(BP)` and **no siting
hint** — Run A's shape — with no solar cheat and no power of any kind supplied
by the script. It prints every `place` step's name and tile *before* the build,
reads every entity back off the live surface *after* it, matches the two, and
then recomputes wire connectivity and supply coverage from the **game's**
positions with vanilla's numbers written out, sharing no code with the model it
is testing.

```
plan: 54 steps, 41 place steps, 4 bots
build: done=true failed=0 lost=0 pending=0
MOVED small-electric-pole planned (3.50, -2.50) nearest standing (4.50, -1.50) d=1.41
plan kept at the planned tile: 40 of 41 (1 elsewhere or absent)
standing: 9 poles, 1 generators, 6 electric consumers
pole components carrying a generator: 1
VERDICT from the GAME's own positions: 6 consumers lit, 0 dark
```

The planner sited the block at (0, 0), sited a plant on the lake — pump
(46.5, −8.5), boiler (45.0, −5.5), engine (40.5, −5.5), **the same plant Run A
got** — and laid a run of poles 46 tiles back to it.

Asked directly over RCON while the game was held open:

```
POLES: (4.5,-1.5)net=1 (8.5,-3.5)net=1 (14.5,-3.5)net=1 (20.5,-3.5)net=1
       (26.5,-3.5)net=1 (6.5,1.5)net=1 (6.5,4.5)net=1 (32.5,-5.5)net=1 (38.5,-7.5)net=1
steam-engine (40.5,-5.5) status=working   boiler (45,-5.5) status=working
offshore-pump (46.5,-8.5) status=working
inserter x6  energy=268  status=34 (waiting for source items)
```

**One network, nine poles, every inserter energised.** Charging the input
chests by hand then gave `stone-furnace status=working`, plates rising
5 → 6 → 7 and 8 → 9 → 10 across three samples, ore draining 100 → 28.

So two of the handoff's candidates die here, measured rather than argued:

* **"The run cannot route past the water."** It routed 46 tiles from the lake
  to spawn, six poles, no detour trouble.
* **"The run is planned and the game disagrees."** 40 of 41 placements stand at
  the exact planned tile, and the game itself puts all nine poles on
  `electric_network_id = 1` with the engine. Model and game agree.

## The one that does not stand

The forty-first.

```
WARN mining entity in build area: small-electric-pole @ 3.5/-2.5
```

`FactorioRcon::place_blueprint` clears its build area by **mining every
non-character, non-resource entity inside it** before stamping, and
`Actuator::stamp_ghosts` is its first executor caller. Its own doc calls this
"a live hazard ... if this were ever dispatched over a block that already has
real entities standing in it", and argues it cannot happen because
`method::blueprint`'s `is_fresh_site` emits the stamp only on the expansion
where nothing of the block stands yet.

**That argument covers the block's own entities and nothing else.** The pole
run is emitted in the *same* expansion; its poles are ordinary `Place` actions
with no ordering against the stamp; the scheduler ran them first; the sweep ate
one. `failed=0` because the placement itself had already succeeded — mining is
not a failure of anything.

The block still worked **by luck**: the surviving run pole at (8.5, −3.5) is
4.47 tiles from the block's own pole at (4.5, −1.5), inside the 7.5-tile wire
reach, so the chain closed without the mined pole. A geometry in which the
mined pole is the only link leaves the block dark, with the plant standing and
every counter green — **the exact signature Run A reports.**

## And the swept rectangle is not the block

`blueprint_build_area` returns the extent in the blueprint's **own offset
space**; `place_blueprint` then throws that position away and re-centres a
same-sized rect on the anchor:

```rust
let width_2  = build_area.width()  / 2.0;
let height_2 = build_area.height() / 2.0;
let build_area = Rect { left_top:     position - (width_2, height_2),
                        right_bottom: position + (width_2, height_2) };
```

For this blueprint at anchor (0, 0) the entities span **x[3, 11] y[−4, 6]** and
the swept rect is **x[−4, 4] y[−5, 5]** — they meet only in the strip x[3, 4].

Measured with two markers, each the other's control
(`scripts/false_power_sweep.lua`), because a single marker cannot tell a
mis-centred sweep from a correct one:

| marker | where | prediction if mis-centred | result |
|---|---|---|---|
| A (−3.5, 0.5) | inside the sweep, **6.5 tiles clear of the block** | mined | **mined** |
| B (10.5, −2.5) | outside the sweep, **inside the block's footprint** | survives | **survived** |

```
markers before the build: A=true  B=true
WARN mining entity in build area: small-electric-pole @ 3.5/-2.5
WARN mining entity in build area: iron-chest @ -3.5/0.5
build: done=true failed=0 lost=0 pending=0
markers after the build:  A=false  B=true
```

So the sweep clears the wrong ground **in both directions**: it destroys things
the block will never occupy, and it does not clear the footprint it exists to
clear. The pole at (3.5, −2.5) was mined in both runs.

This also kills the obvious planner-side workaround. Keeping the pole run out
of the block's own bounding box would not help — the ground at risk is *west of
the block*, where a run coming from a plant to the west necessarily arrives.

## What to do, and where

Nothing in `method::power` can fix this; the defect is in `crates/core`'s
`place_blueprint` and in what `crates/executor` dispatches. Three separable
pieces, in the order they are worth doing:

1. **Offset the build area instead of re-centring it.** `blueprint_build_area`
   already returns the right rectangle; `place_blueprint` should translate it
   by `position`, not rebuild it around `position`. One-line class of change,
   two-marker test above is its witness.
2. **Do not mine at all under `only_ghosts = true`.** A ghost collides with
   nothing (`only_ghosts = true` validates nothing — see Known Issues), so a
   stamp has no footprint to clear. The mining sweep exists for real bulk
   builds and this project has none.
3. **Put `Powering::powered` on the block's placements.**
   `ensure_powered` hands the condition back precisely so a caller can state
   it; `method::extract` does, and `method::blueprint` drops it
   (`(powering.steps, powering.ids)`). With it, a block placement whose power
   has evaporated fails by name at dispatch instead of standing there dark.
   **A loud refusal beats a silent dead factory** — this is the cheapest place
   to buy one.

## What is still not established

**Run A's own zero.** Its records went with its worktree, and its blueprint was
46 entities rather than 28. What is established is that a defect of exactly its
signature — everything placed, nothing failed, a plant standing, a block dark —
is live and reproducible, and that the two mechanisms the handoff proposed are
not it. Whether Run A died of this particular mined pole cannot be checked and
is not claimed.

**And `Some` is not generally false.** That deserves saying plainly, because
the handoff's framing invites the opposite conclusion: on the same map, the
same planner, the same plant and 46 tiles of pole run, an electric block asked
for power, was given it, and smelted. `ensure_powered` did its job.

## Verification

Four offline baselines, re-measured on this binary before and after the change
(doc-only, so the numbers are a control on the measurement rather than on the
edit):

| goal | world | actions / ticks |
|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,115 / 317,283 |

## A second question about the same promise, landed alongside

An owner ruling arrived while this was being diagnosed — size a plant to 1.5x
its demand rather than to its demand — and it lands in the same file. It is
recorded separately in
`2026-09-07-a-plant-is-built-bigger-than-it-is-asked-for.md`, because *how much*
`ensure_powered` promises and *whether it is delivered* are different questions,
and a bigger plant does not help a block whose pole run was mined out from
under it.
