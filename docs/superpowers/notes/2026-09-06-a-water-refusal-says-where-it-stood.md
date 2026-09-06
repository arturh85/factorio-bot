# A water refusal says where it stood — and the exposure it was raised against is theoretical

2026-09-06, branch `absence-is-not-emptiness`, off `master` at `30bd08cd`.

A peer session landed `109734b9` with the finding that **ungenerated ground
reads as clear, to the world model and to a live RCON query alike**, and
inferred from it that `crates/planner/src/method/power.rs`'s water search and
`crates/planner/src/score.rs`'s water verdict were exposed to the same
blindness. They flagged that inference explicitly as untested. This note tests
it.

**Headline: the exposure is theoretical for `power.rs`, refuted for
`score.rs`, and one real defect was found next door.** The refusal could not
say *where it searched* or *whether it could see*, and a documented worked
example of the very failure it names is no longer reproducible.

Everything below was measured today, offline, against
`workspace/scripts/map.json` (seed 31337 t=0) and
`workspace/scripts/map-31337-explored.json` (same seed after an exploration
ring), with a release binary built from this branch.

---

## 1. The distinction is NOT lost anywhere in the pipeline

Follow the path and it survives every hop:

| hop | what carries it |
|---|---|
| `on_chunk_generated` (`mods/BotBridge/control.lua:1856`) | fires once per generated chunk, guarded per chunk id |
| `writeout_tiles` (`control.lua:3226`) | writes **every** tile of that chunk, `name` plus a collides flag — not just water |
| `output_parser` → `FactorioWorld::update_chunk_tiles` | → `EntityGraph::tile_tree` |
| `PlanState::is_charted` / `charting` (`state.rs:5056`) | a one-tile query of that tree |

So **"the model has a tile here" is exactly "this chunk was generated and
written out"**, and `PlanState::charting` already reports it. It is not an
inference; `state.rs:5075` states the contract in place ("a miss means the
chunk was never written out rather than that the ground is empty"), and
`PlannerError::NotCharted` already carries a `ChartingSummary` built from it.

Confirmed against the dump rather than read off the code: `map.json`'s
`tile_tree` holds **exactly 409,600 tiles spanning x,y ∈ [-320, 320)** — a
hard-edged 640×640 square with no holes, which is the generated spawn region
and not a map. `map-31337-explored.json` holds 640,000 over [-384, 416).

## 2. `nearest_water_tile` is not actually blinded on any map we have

`plan_plant` searches from the caller, and `supply_for` retries from
`plant_world_anchor()` = (0, 0) with `PLANT_WATER_WIDE_SCAN_RADIUS` = 128. So
the question is whether that 128-tile disc ever falls on ungenerated ground.

`factorio-bot score-map --world <dump> --from <x,y> --no-plan`:

| dump | from | nearest water | charting |
|---|---|---|---|
| `map.json` (t=0) | 0,0 | **48.1** | 17/17 probes |
| `map.json` (t=0) | 255,249 | **47.4** | 7/17 probes |
| `map-31337-explored.json` | 0,0 | **48.1** | 17/17 probes |
| `map-31337-explored.json` | 255,249 | **47.4** | 12/17 probes |

Three things follow.

* **The world-anchor retry is never blind.** 17/17 on both dumps, and every
  point of a 128-disc around the origin satisfies `|x| ≤ 128 < 320`, so it is
  inside the generated square by construction for any `--create` save this mod
  has replayed. Water sits at 48.1 tiles, comfortably inside even the cheap
  64-tile tier. **Theoretical, not real.**
* **But blindness and a water answer already coexist.** At (255, 249) on the
  t=0 dump the model is blind in **10 of 17 directions** and still returns
  water at 47.4 tiles. That answer was luck, not coverage. Had the seed put no
  water in the 41% it could see, the refusal would have been the blind kind
  with nothing in it to say so. **Constructible, and one seed away.**
* **A documented worked example is now wrong.** `plant_world_anchor`'s doc
  tabulated `map-31337-explored.json` with bots at (255, 249) as *"355 tiles
  from water, refuses"*. It reads 47.4 today, on both dumps, and neither
  refuses from there. Both dumps were rewritten by the exploration work on
  2026-09-06 (`map-31337-explored.json` at 13:01) and an exploration ring is
  precisely what turns ungenerated ground into charted water — so the 355 was
  probably true of a file that no longer exists. It cannot be checked. The
  citation is dropped rather than repeated, and the property it illustrated is
  proven by fixture instead, which is what a test can hold.

## 3. `score.rs`'s water verdict: refuted, it already discloses

`Verdict::Incomplete`'s own doc says *"This is a statement about the dump, not
about the map"*, `MapScore` carries `charting`, and `score-map` prints it
**above** the verdict with a `READ THIS FIRST` prefix plus a repeat as a
trailing note:

```
charting  7/17 probes on charted ground -- READ THIS FIRST: 10 direction(s)
          of the search disc are unexplored, so a resource missing below may
          simply not have been looked at
verdict   INCOMPLETE -- not charted within the radius: iron-ore, copper-ore, coal
```

No change made. The peer's flag here does not survive contact with the output.

## 4. What was actually wrong, and is now fixed

`PlannerError::PowerPlantNeedsWater` carried a `radius` and nothing else, so
*"none within 128 tiles"* was a true sentence about an **unstated place**, from
a search with **two possible anchors**. That cost the requesting session an
hour this morning. It also could not distinguish the two states that produce
the identical `None` out of `nearest_water_tile`.

It now carries `anchor_x` / `anchor_y` and the charting score of that same
disc:

```
a power plant needs water, and the plan can see none within 128 tiles of
(0.0, 400.0), where charted ground covers 0 of 17 probes
```

The asymmetry is the point, and it is the one this repo already insists on in
`EntityGraph::resource_fingerprint` and `runMatch.ts`:

* `covered == probes` — the ground was written out and it is dry. About the
  **map**. Walking will not help.
* `covered < probes` — part of that disc was never generated. About the
  **dump**. Walking a bot in and replanning can change the answer.

Cost: seventeen tile-tree probes, on the refusal path only. The three offline
baselines are byte-identical before and after on the same binary —
`researched:automation` 176 / 21,784, `producing:automation-science-pack:6`
316 / 22,463, `producing:logistic-science-pack:6` 442 / 47,542.

Deliberately **not** done: searching further. A wider scan grows as `R²` for a
benefit that is speculative, and it would answer a different question from the
one that was unanswerable.

## 5. RETRACTED BY ITS AUTHOR: there was no tree

**The peer withdrew the whole finding (`a8460ee5`), and the retraction is
sharper than my caveat below.** There is no tree at (44.5, 0.5) at all, so
neither lazy generation nor the writeout gap I proposed is needed to explain
anything.

The load-bearing step was an assumption nobody stated: *"zero entities in a
10-tile disc of a fresh map was the tell"* takes for granted that a fresh
Factorio map is wall-to-wall trees. It is not — open grass is ordinary. So an
empty disc is **evidence of nothing whatsoever**, and a mechanism was built on
top of it and written into CLAUDE.md. The owner's summary: *"what, you thought
everything will be 100% filled with trees? lol"*

The discriminator was already in the bindings — `rcon.*` asks the **game**,
`world.*` asks the **model** — so one run settled it:

    before any walk    GAME=0  MODEL=0   planner ACCEPTS the anchor
    after the walk     GAME=1  MODEL=0   planner ACCEPTS the anchor
                       (the 1 is the scout itself)

Nothing appeared after walking. **The tile is simply empty**, which also rules
out my writeout-gap alternative — a better-reasoned hypothesis than the
original, and still wrong.

What survives: the refusals only ever occurred on a **replan after a partial
build**, so the occupant was something those runs created themselves. Left
**unexplained** rather than replaced with a second unmeasured story. And the
scout walk does help — by putting bots near the site so walk routing succeeds,
not by making ground real.

**Nothing in sections 1-4 depends on any of this.** The exposure was measured
against the dumps directly (409,600 tiles, hard-edged square, no holes) and the
refusal now saying where it stood is right on its own terms. This section is
kept because the reasoning that produced the retraction is worth more than the
claim was — an unstated assumption about the *world*, underneath a careful
chain about the *code*.

### The caveat as originally written, before the retraction

Their build hit a tree at **(44.5, 0.5)** — 44 tiles from spawn, which is well
inside the 640×640 square a fresh `--create` generates. On seed 31337 at t=0
this repo's dump has that tile **charted** and holds **zero** blocked boxes
within 6 tiles of it, on the explored dump too. So either their run was not on
that map, or the chunk was generated and its *entities* did not reach the
model — which is a defect this repo has already recorded
(`score.rs` cites 54 of 419 chunks whose `writeout_entities` came back `{}`,
`docs/superpowers/notes/2026-09-02-resource-double-count.md`).

Their live-RCON evidence is not affected by that: `rcon.find_entities_in_radius`
really does reach the game (`crates/scripting_lua/src/globals/rcon.rs` →
`rcon_find_entities_filtered`, `control.lua:5413`) with no type filter, so trees
would have been returned. The `0 entities` is a real answer from the game. But
"the model showed nothing" and "the game showed nothing" have different
explanations available, and only the second one requires ungenerated ground.

Worth one `rcon -s localhost -- '/c rcon.print(game.surfaces[1].is_chunk_generated{x=1,y=0})'`
next time it happens. Not chased further here; the mod is not this branch's.

## Open

* Whether a `PowerPlantNeedsShore` refusal wants the same treatment. It
  already reports a measured distance, so it is never the blind kind — but it
  does not name its anchor either.
* Nothing here makes the planner *act* on the blind case. It says which kind
  of refusal it is; deciding to explore is still the caller's.
