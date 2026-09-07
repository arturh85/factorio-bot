# The globals move off the surface, and a world can hold two

2026-09-07. Branch `the-globals-move-off-the-surface`, merged on top of
`95c183be`.

`FactorioWorld` refused every second surface by name
(`SurfaceNotYetSeparable`) and said in its own doc why: recipes, prototypes,
`forces` and their research, and the action id counter were still fields on
`FactorioSurface`, so a second surface would have given the run **two copies
of the research state**. Lifting the refusal meant moving those fields first.
This is that change.

## What moved and what did not

The split is the one `FactorioWorld`'s doc already argued, field by field.
Nothing here was redesigned.

| moved to `GameGlobals` | stayed on `FactorioSurface` |
|---|---|
| `forces` | `entity_graph` |
| `recipes`, `entity_prototypes`, `item_prototypes`, `graphics`, `image_cache` | `flow_graph` |
| `actions`, `next_action_id`, `path_requests` | `daylight` |
| `research_triggers` | `inventories` |
| `teleports`, `deaths` | `placement_refusals`, `walk_refusals` |
| `surface_chunk_drops` | `enclosures`, `step_asides` |
| `players`, `benches` — **still flagged ambiguous** | |

`players` and `benches` are global today and are **still called ambiguous in
both types' docs**, not quietly resolved. The argument is unchanged: a bot has
one identity and stands on one surface at a time, and `FactorioPlayer` already
carries a `surface`, but `PlanState` reads `players` as "the bots I may give
steps to", which is a per-surface question — and splitting them would mean
*moving a row when a bot crosses*, which nothing in this project can do.

## The invariant, and why it is `ptr_eq` and not equality

`FactorioWorld` owns one `Arc<GameGlobals>`. Every surface it holds holds the
**same** `Arc`, and `insert_surface` checks that with `Arc::ptr_eq` before
accepting one.

Not by value. Two `GameGlobals` that happen to be equal today are exactly what
drifts apart tomorrow, and the failure would be invisible: a plan that thinks a
technology is open on one planet and closed on the other. Identity is the
property that cannot silently stop holding.

## The refusal is not gone, it is narrower

`SurfaceNotYetSeparable` said "a second surface is impossible".
`SurfaceGlobalsNotShared` says "**this** surface would bring a second research
state with it". That is a sharper claim about a smaller set, and it still
refuses rather than accepting something silently wrong.

**A cloned surface is one of those, deliberately.** `Clone` deep-copies the
globals because the plan world is a speculative fork whose writes must not
reach the live model — an `Arc` share there would let a plan's *imagined*
research escape into the world the executor reads. So a clone cannot be
inserted back, and the test that pins it says so in those words. Build a second
surface with `FactorioSurface::with_globals(world.globals().clone())`.

## What earned the lift

`crates/core/tests/world_holds_surfaces.rs::two_surfaces_share_one_force_one_recipe_table_and_one_action_id_counter`
holds two surfaces and reads the globals **through both** — one row per line of
the type's own table:

1. a force's `automation` written through Nauvis and read back researched
   through Vulcanus;
2. `recipes` and `entity_prototypes` the **same object** by `Arc::ptr_eq`, not
   an equal copy;
3. an `action_id` minted through one surface advancing the counter the other
   reads.

Three mutations, applied one at a time, each verified to match exactly once in
the breaking edit:

| mutation | tests killed |
|---|---|
| `FactorioSurface::clone` shares the globals `Arc` instead of forking | 1 — `a_cloned_surface_is_refused_because_a_clone_forks_the_globals` |
| `with_globals` ignores what it is handed and makes fresh globals | 2 — `two_surfaces_share_...` and `re_inserting_the_held_surface_replaces_it_and_stays_one_surface` |
| `insert_surface` never refuses (`if false`) | 2 — `a_surface_with_its_own_globals_is_refused_by_name` and `a_cloned_surface_is_refused_...` |

The two that kill two tests are **not** redundancy and were checked rather than
assumed: both pairs assert different things (sharing versus replacement; the
named refusal versus the fork), and both members of each pair depend on the one
mechanism the mutation broke.

## Verification

- `cargo test --workspace` — **105 test blocks green, exit 0**, the same count
  master carries. One earlier run had `crates/server/tests/bind.rs` fail twice
  on `Connection refused`; it passes standalone on repeat and binds a port, so
  it is a parallel-load race in a test unrelated to the world model. Said, not
  swept.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  clean.
- **Four offline baselines, byte-identical.** `plan --bots 1,2,3,4`:
  `researched:automation` 176 / 21,784 · `producing:automation-science-pack:6`
  316 / 22,457 · `producing:logistic-science-pack:6` 441 / 47,478 on
  `workspace/scripts/map.json`, and `gathered:crude-oil` 2,115 / 317,283 on
  `map-31337-explored.json`. `gathered:crude-oil` still refuses on `map.json`
  (`no crude-oil is charted anywhere this plan can`), which is correct.

  Measured before and after by this session. **Not on literally one binary**:
  the before was a release build of `9da3447a`, the after a release build of
  that plus `95c183be` plus this change, because master moved mid-task. Both
  read the same four pairs, and the merged commit is mod- and types-side only.
- **Both archived dumps loaded** — `map.json` (865 MB) and
  `map-31337-explored.json` (1.45 GB). The wire shape is unchanged by the move:
  a dump is still one flat object with `recipes` and `forces` beside
  `entity_graph`, and only the destination of those fields after loading
  changed. `crates/planner/tests/world_round_trip.rs` passes.

## How the port was done, and why it is worth repeating

Roughly **340 call sites** read a moved field. They were not found by grep.
`cargo check --workspace --all-targets --all-features --message-format=json`
was run in a loop, and a 50-line script (`scratch/fix_globals.py`, not
committed) inserted `globals.` at the **span rustc itself reported** for every
`E0609`/`E0615` naming a moved field on a `FactorioSurface`, refusing to touch
anything else. Fourteen rounds, 308 edits, zero hand-written call-site changes,
and the terminating condition is "the compiler has nothing left to say".

The point is not the script. It is that **the compiler knows the exact byte**,
and a textual search does not: `.actions` alone appears 395 times in this tree
and almost none of them are this field.

## Handed on, not taken

The coordinator offered three census pieces (`WorldSnapshot.surfaces`, a home
for the census, the `writeout_surfaces` stdout half). **None taken** — the
third needs `mods/BotBridge/control.lua`, which was off limits to this task,
and the first two are a feature rather than a move. One finding for whoever
does take them:

**The census cannot live on `FactorioWorld` if the parser must write it.**
`FactorioWorld` is constructed in exactly one place
(`process_control.rs:246`) and reached only through `FactorioInstance`; the
`output_parser` and `snapshot` paths hold a `FactorioSurface` and nothing
above it. `GameGlobals` is the aggregate they *can* reach, through
`surface.globals`, and it is game-global for the same reason the census is —
a surface cannot hold the list of surfaces. So put it there, and expose it on
`FactorioWorld` through `globals()` if a reader wants it from the top.
