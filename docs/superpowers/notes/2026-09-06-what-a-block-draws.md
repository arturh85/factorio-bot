# What a block draws, and whether it can distribute it

2026-09-06. `blueprint_demand` / `blueprint_power` in `method::blueprint`.

Two things CLAUDE.md has always said qualitatively about `FurnaceLine` — that
it needs power, and that 138 of its 179 entities have never moved an item — are
now numbers:

```
FurnaceLine:  48 consumers, 624.0 kW, 13 poles, 0 disconnected, 0 uncovered
TJunctionSmelter / TwoRowSmelter / MovingBlock:
               0 consumers,   0.0 kW,  0 poles, every entity unpriced
```

## The measurement settled a design question

I had asked the other session whether `ensure_powered` needed a region-covering
variant. Their API takes one consumer at one position, and `pole_would_supply`
is `boxes_overlap` — right for one machine, and for a 29x11 block bbox
satisfied by **one pole touching a corner**. That is the FurnaceLine failure
shape precisely: a condition that holds for the entity named and is false for
the block.

They confirmed it from the code. But the fixture answers the question outright:

```
(a) poles in one wired component (<= 7.5 tiles):  13 of 13
(b) inserters inside some pole's 5x5 supply area: 48 of 48
```

**The block distributes for itself.** So powering it is *one hop* — from a
supply anchor to any one of its own poles — which is point-to-point, exactly
what the existing API does well, and `boxes_overlap` is correct there because
the target really is one entity. **No new API was needed, and asking for one
would have been the wrong request.** The right place for the coverage question
is not the planner at all: whether a block's own poles reach its own machines
is a property of the *blueprint*, checkable offline with no world.

It also re-reads the finding. CLAUDE.md describes those 138 entities as though
the block were at fault. The block is internally complete. **One missing
generator** is the whole story.

## `None` means two things and must not be folded into zero

`PlanState::consumer_draw_kw` answers `None` for a **deliberately absent
burner** — a stone furnace draws 90 kW *of coal*, and an entry in kW would be a
number in the wrong units every test would agree with — and for a prototype the
table simply does not name. Its own doc says this is the one table in
`state.rs` whose unknown name errs towards *permitting*: an unmodelled machine
is headroom that is not there.

So `BlockDemand` returns the kW, the consumer count, **and the set of names it
could not price**. A caller can then say "624 kW plus six prototypes I cannot
account for", which is a different claim from "624 kW". The burner-block test
asserts every unpriced name is one of four known burner prototypes, so an
electric entity slipped into a burner block fails rather than reading as 0 kW —
the finding this note is about, in miniature.

## Both new assertions were falsified before being believed

`0 uncovered` is exactly the shape that passes when the loop never runs, which
is cause four in this session's catalogue of green falsifications.

| break | result |
|---|---|
| wire reach 7.5 → 0.5 | `disconnected_poles` = 12, assertion fires |
| supply predicate → `false` | all 48 inserters listed, assertion fires |

## The guard, and the fixture that nearly refused

`BuildBlock::expand` now refuses a block that draws power and cannot distribute
it, naming the draw and what is wrong. Giving the analysis a real caller was not
optional: clippy failed the first version with `struct BlockDemand is never
constructed`, which is the same callerless shape that hid `method::connect`'s
geometry defect through four reviews. `allow(dead_code)` would have been the
wrong fix.

**A hand approximation nearly made this refuse `MinerLine`.** A quick Python
check using a 0.8-wide box reported 7 of its 13 drills uncovered. An
`electric-mining-drill` is **3x3**; with the real collision boxes it is
**0 uncovered**. The Rust path asks `collision_area_facing`, so it was right and
the approximation was measuring the wrong rectangle — the third time in one
session that a quick estimate of mine has pointed the wrong way.

`MinerLine` also draws **1,170 kW** (13 drills at 90), which already exceeds a
single 900 kW plant. The ore front end needs two engines before the smelters do.

## Pole recognition, and why not `name == POLE`

`blueprint_power` identifies a pole by asking `pole_would_supply` against the
pole's **own tile**: that answers `false` for any prototype whose supply extent
the crate does not know, so it recognises every pole type the planner models.
`power.rs` compares `name == POLE` because it *places* small poles — a correct
assumption for placing and wrong for recognising, since a medium or big pole
standing in the world reads as not-a-pole. Same defect class as everything else
today: a value correct for one purpose, reused where it is wrong.

## Forward: the plant ceiling binds on a real block

624 kW is the **pre-electric** draw — 48 inserters, with the furnaces burning
coal. An `electric-furnace` draws 180 kW, so 24 of them (one yellow belt's
worth) is **4,320 kW** against a plant that tops out at 900 kW on one engine
and 1,800 on two.

The owner had already said it from play — *"a second boiler is usually needed
after the electricity demands skyrocket once we start using electric
smelters"*. 624 versus 4,320 is that sentence with arithmetic under it, and it
binds on a block that exists rather than on a hypothetical one.
