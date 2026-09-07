# Decisions waiting for the owner — 2026-09-07 morning

Three things overnight work reached and deliberately did **not** decide. Each is
recorded with the measurement behind it, so none has to be re-derived.

---

## 1. `Goal::Built` should persist its anchor instead of re-deriving it

**This is the one that matters, and it changes what a goal means.**

`resolve_site` opens with `recover_anchor`, first and unconditionally. Recovery
trusts an anchor once **two** of a blueprint's entities stand at the right
relative offsets. Our fixtures are variations of one another by construction, so
that test is ambiguous by default rather than at the edges
(`crates/core/tests/recovery_crosstalk_probe.rs`):

```
ElectricSmelter   21 of its 28 entities  match inside FurnaceLine
SaturatedSmelter  17 of 33               match inside FurnaceLine
TwoRowSmelter     15 of 27               match inside FurnaceLine
OreToPlate        14 of 24               match inside MinerLine
```

The chain scripts run several of these **on one map in sequence**, and
FurnaceLine's 179 entities stood there. So a later block read its anchor off an
earlier block, put a drill where FurnaceLine's geometry wanted it — not on ore —
and the game refused. That is the whole of the stranded-tile bug two sessions
spent a day on.

**Raising the vote threshold cannot fix it.** 21 of 28 is 75% of the blueprint,
and any threshold loose enough to recover a genuinely half-built block accepts a
hijack. Recovery by geometry is ambiguous whenever two blocks share a
sub-layout.

**The options, as we see them:**

| option | cost |
|---|---|
| persist the anchor with the goal | changes `Goal::Built` semantics and the goal's serialised shape |
| identify blocks by a marker entity | needs something on the ground that is not part of the design |
| keep re-deriving, accept the ambiguity | leaves the failure in place; it is silent and strands whole blocks |

### And `Site::At` is not a workaround — so there is currently NO way to put two of these blocks on one map

This was going to be handed over as the mitigation ("use explicit anchors until
the anchor is persisted"). It was checked first, and it is false.

`resolve_site` runs `recover_anchor` **before** the `Site` match and
unconditionally, so **an explicit anchor is outranked** by any two of the
block's entities standing at the right relative offsets — including when those
entities belong to a *different* block sharing a sub-layout. Test in `a22089f7`:
build a four-furnace block A, ask for block B at `(0.5, 0.5)`, get an anchor
inside A.

**Both rulings are individually correct**, which is what makes this worth
stating rather than filing as a bug. Recovery outranking `Site::At` is
deliberate — a stale caller anchor must not start a second half-block, and
`resolve_site_prefers_the_recovered_anchor_over_an_explicit_site_at` is the
guarantee that says so. Recovering on two matching entities is what makes a
genuinely half-built block resumable. **Jointly they leave no way to express
"this is a new block, put it here"**, and closing that gap is exactly what
anchor persistence would do.

**So the practical consequence, which is the version worth deciding on: one
block per fresh map is no longer a measurement-hygiene preference, it is the
only thing that works.** The electric-smelter milestone — several blocks growing
on one map, with solar — is **blocked** on this rather than inconvenienced by
it.

Nobody implemented any of these overnight, deliberately. **Landed instead**
(`06296b0a`): every anchor is now screened for ore rather than only the ones
siting chose, refusing as `BlockDrillUnfed`, which names the drill, the tile, and
whether the remedy is to move the anchor or clear the half-built block. That is
strictly worth having **even after** the anchor is persisted — it turns a silent
stranding into a named refusal — but it treats the symptom.

---

## 2. `maximum_wire_distance` is not on the wire, and one hand-kept value had
already drifted

`pole_supply_half_extent`'s four entries were **all correct** when checked
against the installed game's own prototypes. Its neighbour `pole_wire_reach` was
not: `big-electric-pole` read **30.0** against the game's **32**. Factorio 2.0
moved the value and nothing noticed, *because a hand-kept table of game data is
only ever read by code that agrees with it*. A legal big-pole span read as a
broken network, silently. Corrected in `59ab9ca3` with a test at 31 tiles — the
one-tile window that distinguishes 30 from 32.

**It cannot be derived**: the mod does not send `maximum_wire_distance` and
`FactorioEntityPrototype` has no field for it. So the choice is to ship the
field or to keep a table that has already drifted once.

A related measurement worth seeing, because it explains why the pole table
cannot simply be deleted: with the vanilla fallback removed and nothing else
changed, **all three offline goals refuse to expand at all** — every archived
world predates the prototype field, so every pole supplies nothing. And the
refusal blames *the water*, one layer downstream, with no mention of poles.

**The electrical tables were then checked the same way, and they are CLEAN.** All
11 checkable `consumer_kw` rows match the game's own prototypes exactly —
`electric-furnace` 180, `beacon` 480, `assembling-machine-3` 375, `oil-refinery`
420 and the rest. **So the milestone arithmetic in CLAUDE.md (24 electric
furnaces × 180 kW = 4,320 kW against a 1.8 MW plant) is sound as written, and
the second boiler really is needed.** Shipping the energy fields is therefore a
**mod-compatibility fix, not a bug fix** — worth knowing before deciding how much
it is worth.

One real discrepancy, and it is instructive rather than urgent: `steam-turbine`
is tabulated at **5,800** against a derived **5,820** (0.34%), in an entity
nothing builds yet. It exists because a generator has **no production field at
all** — output is physics, `fluid_usage_per_tick × 60 × heat_capacity ×
(maximum_temperature − default_temperature) × effectivity`, which gives
`steam-engine` exactly 900. **The derivation survives a mod changing a
temperature or a fluid; a tabulated scalar does not, and neither would a
synthesised one.**

---

## 3. Fluids: `Goal::Stored` needs a design, and one of its four unknowns is now
answered

`have:petroleum-gas` now refuses honestly rather than dividing a fluid among four
bots. Storing one still has no goal kind. Of the four things that needed settling
first, the measurable one is settled: **`fluidbox_prototypes` carries
`pipe_connections` and `production_type` and nothing else — there is no `volume`
on the entity prototype**, though `LuaFluidBoxPrototype::get_volume()` exists and
now ships.

So **siting a tank is answerable from data we already receive** (connection
offsets plus input/output direction, per prototype); **capacity is not**, and
closing that gap with a table would repeat the defect §2 just found.

---

## What is NOT waiting on anything

Everything else overnight is merged and green: gathering bills its own unlock,
the refusal names what it found, per-interval delivered tick rate, the furnace
input slot, transport-line contents, beacon prototype fields, the beacon lane
reservation, the world-record flow-graph validation, the supply balance, the
demand side, `entity.status`, refusal expiry, the `blocked_tree` binding, and
entity-graph edges that outlive tick 0.

The flow graph's mean absolute log error against a world-record base went
**0.298 → 0.216 → 0.184** over the night, with every regression published in the
same table as the wins.
