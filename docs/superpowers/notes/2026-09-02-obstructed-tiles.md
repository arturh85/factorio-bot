# Obstructed resource tiles: rung 4 stuck mining coal under crash-site debris

Status: fixed. Commit `6a6221b6` (branch `master`, HEAD at the time was
`82d67d65`).

## The failure

`workspace/runs/run-1788317597-64759`, rung 4, stuck after 5 iterations on
100-step plans:

```
ERROR: could not start mining for 301 ticks: expected coal at (-37.5/5.5), found crash-site-spaceship-wreck-medium-3
```

## Where crash-site debris lives in the graph

`EntityGraph::add` (`crates/core/src/graph/entity_graph.rs`) routes every
non-`FlyingText`/non-`Fish` entity with a non-zero collision box into
`blocked_tree`, except resources and rails (those go to `resource_tree` /
are skipped). A second, narrower match then additionally inserts into
`entity_tree` only a whitelist of factory types (`Furnace`, `Inserter`,
`Boiler`, `Lab`, `OffshorePump`, `MiningDrill`, `StorageTank`, `Container`,
`Splitter`, `TransportBelt`, `UndergroundBelt`, `Pipe`, `PipeToGround`,
`LogisticContainer`, `AssemblingMachine`) plus the two named exceptions
`rock-big`/`rock-huge`.

`crash-site-spaceship-wreck-medium-3` is a `simple-entity`, exactly like a
small rock, and its name matches neither whitelist rule. So it lands in
`blocked_tree` only — never `entity_tree` — the identical routing that let a
tonight-earlier bug site a furnace in a forest
(`docs/superpowers/notes/2026-09-02-placement-refusal.md`). `blocking_boxes_within`
already existed as the reader for that fix; it needed no changes here.

## Covered, not gone

Covered. `mods/BotBridge/control.lua`'s mining loop keeps a live
`LuaEntity` reference (`storage.p[idx].mining.entity`) obtained when mining
started; the failure path only runs while `ent.valid` is true, i.e. the coal
entity genuinely still exists. The failure comes from
`player.update_selected_entity(ent.position)` — Factorio's own "what's
selectable here" query, which does not filter by name — returning the wreck
instead of the coal at the same tile. `EntityGraph::add` never removes a
resource entity because something else was placed over it, so the model's
`resources`/`resource_tree` entry for that coal tile was never stale; only
*new mining assignment* to that tile is wrong.

## Does anything remove a mined-out tile from the model?

No — worth flagging even though it's not this bug. `PlanState::resource_available`
subtracts what *this plan* has consumed from a constant per-tile cap
(`DEFAULT_RESOURCE_PER_TILE = 500`); nothing in `EntityGraph` ever deletes a
resource entry when the real ore is exhausted in-game. A tile mined empty by
a previous run keeps reading as available to a fresh plan until the world
snapshot itself no longer reports that ore entity. Out of scope for this fix,
noted for whoever hits it next.

## What was fixed

`PlanState::resource_unclaimed` (`crates/planner/src/state.rs`) is the single
ledger every selector already read (`nearest_resource_tile`,
`resource_tiles_for`, `resource_supply_at_least`, `resource_seats`, via
`Method::concurrency`). Added a third exclusion alongside "claimed" and
"crowded": a new private `PlanState::resource_tile_blocked(&Pos) -> bool`
builds the tile's one-unit `Rect`, queries
`EntityGraph::blocking_boxes_within` against it (mirroring `is_area_clear`'s
fourth source, including the same `removed`-set skip for symmetry), and does
an exact `boxes_overlap` test on each candidate box. A blocked tile now
reports `0` from `resource_unclaimed` and is skipped by every caller of it,
without touching `resource_available`/`Condition::ResourceAvailable`'s
physical reading — so the "covered, not gone" distinction is preserved:
`resource_available` still reports the tile's full count, only *selection*
refuses it.

Because all four selectors already funnelled through `resource_unclaimed`,
one change point kept seats and tile-selection in agreement by construction,
per the task's "read the same ledger" constraint — no separate seat-side
change was needed.

## What was reported, not fixed

- The stale-model question above (nothing purges a genuinely mined-out
  tile).
- `blocked_tree` also carries non-debris obstacles (units, dropped items,
  water) that could equally cover a resource tile; the fix treats all of them
  alike (any overlap excludes the tile), which is the same "erring toward
  fewer false positives, not fewer skipped tiles" choice `is_area_clear`
  already made for placement.

## Verification

- Compiled and tested in an isolated `git worktree` pinned to HEAD
  (`82d67d65`) plus only the two changed files, because
  `crates/core/src/record/{mod,samples}.rs` and
  `crates/scripting_lua/src/globals/record.rs` were mid-edit by a concurrent
  agent in the live working tree and did not compile.
- `cargo fmt --check -p factorio-bot-planner`: clean (after `cargo fmt -p
  factorio-bot-planner` fixed one wrapping issue in the new tests, applied
  directly to the real repo since it only touches my own files).
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings`:
  clean, whole workspace, in the isolated worktree.
- `cargo test --workspace` in the isolated worktree: all green (no
  `FAILED`/panics anywhere in the run), including the pre-existing 291+
  planner tests unchanged.
- New tests added in `crates/planner/src/method/util.rs`, using three
  isolated `uranium-ore` tiles ten tiles apart (fixture ships none, so every
  assertion is exact) plus a `simple-entity` shaped like `FactorioEntity::new_rock`
  covering one of them:
  - `a_resource_tile_under_debris_is_skipped_by_the_nearest_tile_search` —
    `nearest_resource_tile` and `resource_tiles_for` both skip the covered
    tile and pick the next; `resource_available` still reports it non-zero
    while `resource_unclaimed` reports zero.
  - `an_unobstructed_patch_is_unchanged_by_a_block_elsewhere` — negative
    control: the fixture's untouched `copper-ore` patch (nearest tile, seat
    count) is byte-identical whether or not the unrelated uranium tile is
    blocked, and only the one covered uranium tile is excluded, not its
    whole patch.
  - `seats_and_selection_agree_about_an_obstructed_patch` — `resource_seats`
    on the obstructed patch returns `2` (not `3`), and `resource_tiles_for`
    asked for exactly that many tiles' worth of ore hands back exactly `2`
    tiles.
  - Non-vacuity confirmed by removing the new exclusion in the isolated
    worktree: all three new tests failed (with the exact numbers expected —
    the blocked tile was still being offered), while every existing test,
    including the two negative-control assertions embedded in the middle
    test, kept passing.

No existing test moved or changed expectation.

Test summary: 3 new tests added (1 positive, 1 negative control, 1
capacity-agreement), all passing; removing the fix fails exactly those 3 and
nothing else; full workspace suite (`cargo test --workspace`) green in an
isolated worktree; `cargo fmt --check` and `cargo clippy --deny warnings`
clean.
