# How oil is actually played

2026-09-06, from the owner, who has played the game extensively. Recorded
because **the planner had been solving a harder problem than the game poses**,
and three separate design decisions were resting on that.

## The topology

> *"usually the tanks have two connectors, and one is connected with pipes to
> all the pumpjacks, while the other tank is connected a long way to another
> tank at the refinery area ... there are also directional pumps to control the
> direction the fluids move, but i think with the 2.1 update the bandwidth is
> pretty huge so fluids move fast."*

    pumpjacks ──pipes──> TANK ═══════ long trunk ═══════> TANK ──> refinery area
    (one per well)     (at the patch)                  (at the base)

So a wellhead needs a **pumpjack and a tank**, and the patch needs **one
trunk**. Not a refinery, therefore not water, therefore **no power plant at the
well at all**.

## What this corrects

**1. The refusal this lane spent a day on was self-inflicted.** *"A power plant
needs water, and the plan can see none within 128 tiles"* was the planner
siting a plant at the *consumer* — the pumpjack — and then requiring water
there. Real play never puts a plant at the well. The plant stays where the
water is; power crosses the distance on poles.

**2. The long distance is paid ONCE PER PATCH, not once per well.** The
gather-then-trunk shape means the expensive part is amortised over every
pumpjack on the patch. On six wells that is a sixfold difference on the only
expensive component. It also changes the siting *question*: not "where does
this pumpjack connect" but "where does this patch's tank go", which is a far
smaller search and can be anchored on the patch centroid.

**3. Throughput is not a constraint worth modelling.** If 2.1's bandwidth
carries a patch's output down one trunk, the planner needs **connectivity and
direction**, not flow rates. That is the difference between a graph problem and
a simulation, and it is worth stating out loud because the instinct is to model
flow — effort spent on a constraint that no longer binds.

## What the model already has

Verified against `crates/core/tests/live-2.1.17-world-snapshot.json`, which the
mod already serialises into `fluidbox_prototypes` with connection positions:

| prototype | fluid boxes | pipe connections |
|---|---:|---:|
| `storage-tank` | 1 | **4** |
| `pipe` | 1 | 4 |
| `pump` | 1 | **2** |

The tank's four connections are what give the two usable sides. **The pump's
two are its directionality** — in one side, out the other.

## The trap in the pump, named before anyone hits it

A pump is not a pipe segment: it has an orientation that decides which way
fluid moves. Placing one backwards yields a layout that **places 100%
correctly, passes every geometry check, and moves nothing** — placement and
function being separate concerns.

This repo has already paid for exactly this shape once, with inserters:
*"an inserter's `direction` points at the side it PICKS UP from, not the side
it drops into ... the failure is silent."* Establish the pump's convention
empirically, the way the inserter's was, and write it down beside that one.

## Consequences for the fluid work

The tank is **not an optimisation, it is the precondition**: no character
inventory can hold a fluid (see `2026-09-06-a-fluid-is-not-an-item.md`), so
until a tank exists there is nowhere for crude to be. Tank siting should be
per-patch from the start rather than retrofitted.
