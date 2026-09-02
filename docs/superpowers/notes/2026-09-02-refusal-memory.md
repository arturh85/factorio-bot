# The planner learns from a refused placement — 2026-09-02

## Status

Done. All gates green. One commit; the note itself is not committed.

## Is this the right fix?

Yes, and the alternative the brief offered — ask the game before committing a
placement — is not an alternative. It is the *next* step, and it needs this
one first.

A pre-flight `can_place_entity` query would turn a refusal from a
run-costing failure into a plan-time fact, which is strictly better than
finding out by dispatching. But the planner is pure and headless; the answer
has to be handed to it as data, and there has to be somewhere to put it that
survives to the next `PlanState`. That "somewhere" is the whole of this
change. Without it, a pre-check tells you the site is bad, `free_area_near`
re-derives the same site from the same inputs, and the loop is exactly the one
we have — only now it costs an RCON round trip per iteration as well.

So: build the memory, populate it from refusals we already get for free, and
the pre-check becomes a cheap upgrade later that fills the same ledger
earlier. Reported below, not built.

## 1. Where a refusal is observed

**`FactorioRcon::place_entity_timed` (`crates/core/src/factorio/rcon.rs`), on
the two arms that turn an unrecognised reply line into an error.**

Not `FailureKind`, and deliberately not. By the time a refusal reaches
`classify_failure` it is a string four wrappers deep — `game rejected the
command: Unexpected Response: cannot place item '…' because
surface.can_place_entity said 'no'` — and the *structure* that matters has
already been flattened out of it. At the RCON layer the mod's two answers are
still two separate branches:

- `§player_blocks_placement§` — the acting player is in the footprint. The
  RCON layer already walks the bot to one of eight compass points and retries.
- anything else — including the generic wording above.

Only the generic wording is remembered, and only when it actually contains
`can_place_entity said 'no'`: the same arm also carries `does not have any`
and `place_result is nil`, which are refusals of the *command*, not of the
ground. The match is a substring, at the one place the game's own line is
still a line. A wording change costs a refusal that is not learned from — the
behaviour before this existed — never a site excluded for a reason nobody
gave.

The position comes free there: `entity_position` and `item_name` are the call's
own arguments, and `item_name` is the same name the planner looked the
collision box up under when it chose the site.

**Both arms record, including the post-walk retry.** A refusal that survives
the actor being moved aside is the strongest evidence available that the
blocker is not the actor.

## 2. Where it is remembered

**`FactorioWorld::placement_refusals`
(`crates/core/src/factorio/world.rs`) — an append-only `PlacementRefusals`
ledger on the shared world.**

The brief asked how the supervisor carries state between iterations. The
answer is that it mostly does not: `scripts/supervisor.lua` carries a step-count
tracker and nothing else, and every iteration calls `goal.plan`, which builds a
brand-new `PlanState::from_world`. The `claimed` ledger and the reservation map
were the wrong precedent to follow — both are scoped to *one* expansion by
design, and `tile_reservation.rs`'s own note says so ("claims are
per-`PlanState`, and every production path builds a fresh one … that is
correct"). A refusal has to outlive the plan that earned it, so it cannot live
in the thing that is rebuilt per plan.

What does span iterations, on the Rust side, is the `Arc<FactorioWorld>` every
`goal.*` binding captures. And a refusal genuinely *is* world state: it is an
observation about the ground, arriving from the game, of exactly the kind
`EntityGraph` is supposed to carry and structurally cannot (`add` inserts a
whitelist of factory entity types; trees, cliffs, characters and now
"whatever this was" are all outside it). Putting it on the world also means it
reaches every consumer with no plumbing: `goal.plan`, `goal.holds` and
`obs:recover()`'s tier-2 re-expansion all call `PlanState::from_world`, and all
three now see it without a single call site changing.

Two readers, which is why it sits between them rather than inside either:

- `PlanState::from_world` reads it non-destructively on **every** plan;
- `record.refusals()` writes the ones not yet written, moving a cursor.

That cursor is the one structural difference from the `teleports` queue beside
it, and it is the important one: draining a refusal into the record would make
the planner forget it the moment it was written down.

Purity is untouched. `PlanState::from_world` was already a read of the world
snapshot; this is one more field read at construction. Expansion still has no
I/O, no clock and no interior mutability, and two `PlanState`s built from the
same world and roster produce the same plan. The footprints are **sorted by
geometry** in `from_world` rather than kept in arrival order, so the sequence
the game happened to refuse things in cannot reach a plan at all;
`refusals_reach_the_planner_in_a_deterministic_order` pins that from both
directions. `expansion_is_deterministic` is unaffected.

## 3. How long a refusal is believed

**For the life of the world — the run — and not expired.** Not decayed, not
retried after N iterations.

The brief is right that the message cannot tell a cliff from a bot. The
argument for durability does not depend on telling them apart:

1. **Every transient obstacle the planner can model is already re-read from
   the world on every plan.** Characters (`characters`), placed entities, ore,
   terrain — all of them are rebuilt in `from_world` each time, so a bot that
   walks away stops blocking without anyone forgetting anything. A refusal
   therefore only ever carries information the model *cannot* see. An
   unexplained obstruction is not something a clock can talk us out of.
2. **The cost of over-belief is bounded and small.** One refusal excludes one
   collision box: for a stone furnace, roughly a two-tile neighbourhood of
   candidate centres out of `free_area_near`'s 625-candidate window
   (`FREE_TILE_SEARCH_RADIUS` 12). Only ground the game itself turned down
   ever enters, and a run makes tens of placements, not thousands. Fencing the
   planner off a buildable map is not close.
3. **The cost of forgetting is a milestone, four times observed.** The
   asymmetry is the whole decision, and it is the same asymmetry the
   `characters` field was settled on one run earlier.
4. **Anything time-based would have to be impure or would reopen the loop.** A
   wall clock is banned in the planner; an iteration counter is impure state
   that would have to live somewhere and be threaded through; and either one,
   set short enough to help, re-offers the refused site to the very next plan,
   which is what we have today.

The horizon is the run because the ledger is the world's and a run gets a
fresh world. That is also the escape hatch: nothing is persisted to disk, so a
bad refusal cannot outlive the process that learned it. And the exclusion is
bounded by the refused box, tested by
`the_exclusion_stops_at_the_edge_of_the_refused_box` — durable, but small.

One case is deliberately *not* carved out, and it looks like it should be:
a refusal with a character standing nearby
(`a_refusal_is_kept_even_with_a_character_nearby`). It is tempting to say "that
one was transient, forget it". But the refusal names no cause, which is the
entire difficulty; "a bot was near it" is a coincidence the planner cannot
promote to an explanation, and the `characters` source already handles the case
where it really is one — from the world, on every plan. This source exists for
the cases where it is not.

## 4. Site or search

**The site, and specifically the footprint the game tested — never the
search.**

Nothing touches `FREE_TILE_SEARCH_RADIUS`, the ring order, or `free_area_near`
at all. A refusal makes a set of candidates unavailable through
`is_area_clear`, and the existing outward walk finds the next one. So a
refused site costs the *nearest* alternative and never a wider search than the
one already being run; every caller of `is_area_free` / `is_position_free`
inherits it with no change.

**The box, not the tile.** The game was asked whether a 1.398-tile box centred
at `[-16, -58]` fits and said no. The sound reading is "something in that box
blocks a build"; nothing in the answer says where. Excluding only the exact
tile lets the next plan pick a neighbour whose own box covers the same unknown
blocker — `[-17, -58]`'s box spans `[-17.8, -16.2]`, overlapping the refused
`[-16.8, -15.2]` — and `free_area_near`'s rings then walk into it one tile at a
time, one iteration per step, against a stall limit of 3. Excluding the box
steps clear in one move; the first disjoint candidate, `[-18, -58]`, is
available again.

It over-excludes when the blocker sits in a corner of the box. That is the
same trade, in the same direction, that the `characters` field was settled on:
a few extra tiles of search against a milestone.

**One extra place it bites, in the executor.** `recover`'s tier 1 reschedules
the *same* network, so a refused `Place` carries its baked position into the
retry. `MAX_TIER_ONE_ATTEMPTS` already bounded that at three, but three
dispatches of a command the planner now believes cannot succeed is three too
many, and the tier-2 re-expansion behind them would have sited it elsewhere on
the first try. So tier 1 is now skipped when a failed `Place` sits on a
refused footprint (`refused_by_the_game`, using `PlanState::is_site_refused`,
which asks *only* the refusal source — not `is_area_free`, because a character
in a planned footprint is an ordinary transient and exactly what tier 1 is
for).

## 5. How it appears in the record

New `EventKind::PlacementRefused { entity, position }`, written by a new
`record.refusals()` binding, called from `scripts/research_run.lua` once per
`ran` transition (same cadence as `record.actions` / `record.teleports`) and
once more after the loop, so a run that died on its last placement still
carries the site.

```jsonl
{"tick":6198,"wall_ms":…,"kind":"placement_refused","entity":"stone-furnace","position":{"x":-16.0,"y":-58.0}}
```

The line is not redundant with the `action_settled` failure beside it. That
one says the action failed; this one says **the planner has written the ground
off** — from this tick to the end of the run every plan sites around that box.
Without it, a planner that suddenly prefers a further-away tile looks like a
planner with a bug, and the next diagnosis starts from the wrong place. The
variant's doc says so, and so does the TypeScript mirror.

An unstamped refusal (a reply with no tick) is clamped by `not_before` to the
log's own clock rather than defaulting to zero, so it cannot sort itself before
the run started.

The console gets one `tracing::warn` per newly-learned site, naming the entity
and the position; repeats are silent because the ledger dedupes.

Snapshot regenerated (`UPDATE_OPENAPI_SNAPSHOT=1 …`, +25 lines), mirrored in
`app/src/api/types.ts` and `app/src/api/openapi.contract.spec.ts`. Nothing in
the frontend switches exhaustively over `EventKind`, so no renderer changed.

## What I rejected

1. **Threading refusals through Lua as `goal.plan(..., { avoid = … })`.** It
   would put the memory where the loop is, which is the brief's own
   suggestion, and it is visible in the script — but every driver has to opt
   in, `obs:recover()` builds its own `PlanState` in Rust and would miss it
   entirely, and the observation surface would have to grow a refusal list for
   the script to shuttle back. Putting the fact where the fact belongs costs
   no call-site changes at all.
2. **A `FailureKind::PlacementRefused` and learning downstream of
   `classify_failure`.** That is where the brief pointed, and it is the wrong
   layer: the mod's two refusals are one string by then, and the position
   would have to be rejoined from `ActionKind::target_position`. Matching
   structure at the source is both simpler and less fragile. (The existing
   `FailureKind::Rejected` classification is unchanged — a reader still sees
   the failure exactly as before, plus the new line.)
3. **Putting the refused box into `EntityGraph`'s `blocked_tree`.** It would
   compose with everything for free, but `blocked_tree` entries mean "there is
   a physical thing here with these bounds", and a refusal means "the game
   said no and would not say why". Keeping them apart is what lets
   `is_site_refused` exist, which is what the executor's tier-1 escalation
   needs.
4. **Expiring a refusal after N iterations.** Covered above: impure, and at
   any N small enough to help it re-offers the site to the next plan.
5. **Excluding the whole search region or bumping the radius.** No evidence
   supports it. The game said something about one box; a wider exclusion would
   be inventing a claim, and a wider *radius* would move the plan further away
   for a reason nobody could read back.

## Reported, not fixed

1. **A pre-flight `can_place_entity` query is now worth building.** The mod
   already computes it; one RCON call before dispatch, its answer fed into the
   same ledger, would move refusals from "cost a dispatched action" to "cost a
   plan-time exclusion", and this change is the thing that makes the answer
   worth keeping. It needs its own design pass (which placements, at what
   cadence, and what it costs on a plan with dozens of them).
2. **The fourth cause is still unidentified.** This change is deliberately not
   a diagnosis: it makes the *next* unknown cause cost one placement instead
   of a milestone. If run 17 still fails, `events.jsonl` now names the refused
   sites directly, which is a better starting point than four notes of
   forensics.
3. **The three items carried forward from
   `2026-09-02-placement-refusal-3.md` are still open**:
   `record.plan_created`'s `bots` field reads like a roster and is not one; a
   run that never places anything gets no map coverage where it failed; and
   the phantom player invented by
   `initiate_missing_players_with_default_inventory` shadows the origin
   unconditionally.
4. **Tier 1's baked positions are only patched for `Place`.** A `Mine` or
   `Insert` whose target the world has since made impossible still burns its
   three attempts. Nothing observed needs it; noted because the shape is now
   available.

## Test summary

**23 new tests, all four layers, plus one behaviour change in the executor.**

`crates/planner/tests/refusal_memory.rs` (10) — the planner half, on the real
runs' coordinates. A control that all three sites are open ground in the
fixture (so nothing below passes for the wrong reason); the defect itself (a
refused site is not offered again); that `free_area_near` walks past it rather
than failing, with the anchor asserted as the pre-refusal answer so the test is
about the refusal and not the fixture; that the exclusion is the footprint the
game tested (the overlapping neighbour is refused too) and stops at its edge
(the disjoint one is not); that the rest of the map is untouched; that
refusals accumulate; that the same site refused twice is one exclusion; that
arrival order does not reach the plan; and that a refusal is kept even with a
character nearby.

`crates/core/src/factorio/rcon.rs` (4) — the discriminator. The generic
refusal is remembered with its site and tick; `§player_blocks_placement§` is
not; `does not have any` / `place_result is nil` / an unrelated error are not;
and a site refused three times is one ledger entry, reported once, still
visible to the planner afterwards.

`crates/scripting_lua/src/globals/record.rs` (2) — `record.refusals()` writes
each site exactly once **and leaves it on the world** (the assertion that
distinguishes this from the teleport queue), and an unstamped refusal is
clamped rather than written at tick 0.

`crates/executor/src/recover.rs` (1) — tier 1 reschedules an ordinary
placement failure (the control, same log, clean world) and does **not**
reschedule the same failure once the world carries the game's refusal.

**Confirmed non-vacuous.** With the sixth occupancy source stubbed out
(`if false && boxes_overlap(…)`), 5 of the 10 planner tests fail — the defect
test, the search test, the footprint test, and the character-nearby test —
while the 5 controls (open ground, edge-of-box, dedupe, map-untouched,
determinism) keep passing, which is what controls are for. With the
`can_place_entity said 'no'` guard stubbed out, 2 of the 4 rcon tests fail —
the two that assert a refusal is *not* remembered.

**No existing test moved and no existing expectation changed.** The one
behaviour change to existing code — tier 1 skipping a refused placement —
cannot fire on any pre-existing test: it requires a `PlanState` built from a
world with a recorded refusal, and no fixture had one until this commit.

## Gates

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` — exit 0.
- `cargo test --workspace` — exit 0, 1187 passed, 0 failed.
- `pnpm lint` (tsc + vue-tsc + eslint) — clean.
- `pnpm run test:coverage` — 48 files, 781 tests, 0 failed; coverage gate met.

All Rust gates through `nix develop --command`; a bare `cargo` cannot build
`mlua-sys` in this checkout.
