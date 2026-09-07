# A routed surface is a real surface — and one of the three holes had already closed

2026-09-07. Branch `a-routed-surface-is-a-real-surface`, off `c4bfc002`
(master at 108 test blocks).

Three things were named as making a routed surface second-class. **One of them
stopped being true earlier the same day**, two changes landed for other reasons
having quietly closed it; the second was a one-line route with the wire already
in place; the third is the seam working as designed and is now half-ported.

## 1. `connect()` adds nothing a routed surface lacks — the premise expired

The brief's fear: `OutputParser::on_init` calls `entity_graph.connect()` and
`flow_graph.update()` on the **default surface only**, so a routed surface gets
nodes and never edges — silent, and indistinguishable from a surface that
genuinely produces nothing.

**It is not true any more, and the brief said to check before assuming.** Two
changes landed on master earlier today:

- **`EntityGraph::add` wires incrementally** through `connect_nodes_near`
  (`2026-09-07-edges-that-outlive-tick-zero.md`), and **every** route into a
  surface goes through `add`: `FactorioSurface::update_chunk_entities` (the
  bulk `entities` line) and `on_some_entity_created` both call it. Measured
  there on the 39,191-entity world-record base — a full sweep on top of an
  incremental replay found **0** further edges.
- **`FlowGraph` rebuilds on demand**, keyed on the entity graph's `generation`,
  and *every* public reader calls `ensure_current` first. A surface whose
  `update()` was never called carries `built_generation == NEVER_BUILT`, so the
  first read builds it. There is no reachable stale answer.

So `on_init` is an eager first walk for whichever surface exists when Factorio
logs `initial discovery done`, and it is no longer the only maintenance a
surface gets. **Nothing was changed here.** What was added is the pinning, in
`crates/core/tests/a_routed_surface_is_a_real_surface.rs`:

- `a_routed_surface_is_wired_without_on_init` — a chest/inserter/chest chain
  routed to Vulcanus has **2 edges with `on_init` never called**, beside
  Nauvis's own chain of 2. Both surfaces genuinely occupied, at different
  coordinates, so neither count is over an empty graph.
- `on_init_adds_no_edge_a_routed_surface_lacked` — running the sweep by hand on
  **both** surfaces leaves both counts at exactly (2, 2).

**And the second of those is honestly weaker than it looks, measured rather
than reasoned about.** Mutation F below made `connect()` a complete no-op and
the pair stayed **green**, which is correct: the test asserts the sweep finds
*nothing*, and a sweep that does nothing satisfies that. It is a guard against
the two paths *diverging*, not a test of `connect()`. Stated here rather than
left for someone to discover.

**The known incremental-vs-sweep divergence did not move.** The one-extra-edge
case (an underground half dropped into an existing pair's gap; the graph only
appends) is pinned by `a_half_dropped_into_a_tunnel_leaves_the_long_pair_behind`
and `wiring_one_entity_at_a_time_agrees_with_one_full_sweep`, both green on this
branch. No fixture here contains an underground belt.

## 2. `daylight` routes, and the wire needed no change at all

`serialize_surface_daylight` (`mods/BotBridge/types.lua`) has set
`record.surface = surface.name` since the field was added, and
`SurfaceDaylight::surface` has existed to receive it. **It was read by
nothing** — exactly the shape `FactorioEntity::surface` was in before
`b0bb7f53`. So the fix is one line:

```rust
Ok(daylight) => self.route(&daylight.surface).update_daylight(daylight),
```

No mod edit. A record naming no surface goes to the default one, which is
`route`'s rule everywhere: absent is *the sender did not say*, never "this
planet has no daylight". The non-erasure rule in `apply_snapshot` is the same
statement from the other side and is untouched.

**Why this one mattered more than its size.** `PlanState::solar_average_kw` and
`PlanState::accumulators_per_panel` both read `self.base.daylight()` off one
`FactorioSurface`, and the two terms they multiply through —
`solar_power_multiplier` and `ticks_per_day` — are precisely what differs
between planets. A curve filed on the wrong surface sizes a solar array for the
wrong world while reading as a measurement.

**Proved with two surfaces and two derived numbers**, not merely with the field
landing somewhere (`two_surfaces_carry_different_daylight_and_different_solar_numbers`):

| | nauvis | the other planet |
|---|---:|---:|
| `ticks_per_day` | 25,200 | 12,600 |
| `solar_power_multiplier` | 1.0 | 2.5 |
| `average_solar_fraction(1, 0)` | **0.7** | **1.75** |
| accumulators per panel | **0.8467** | 0.423 |

Nauvis's two are asserted at their own known values — the 0.7 average and the
0.8467 that reproduces the 25:21 rule of thumb — so this is not merely "the two
differ", which a bug that put both curves on one surface and then corrupted one
would also satisfy. The accumulator arithmetic is restated in the test from the
same two inputs (`night_deficit_fraction`, `ticks_per_day`, a panel's 1,000
J/tick, an accumulator's 5 MJ) so it measures the daylight and not the planner —
`crates/planner` is another session's file this week.

`a_curve_that_names_no_surface_goes_to_the_default_one` is the other half: an
unnamed curve lands on the default surface **and leaves the routed surface's own
curve alone**, and the record still reports `surface: None` rather than the
parser filling it in on the sender's behalf.

## 3. `only_surface()` — the `lua` path is ported; the server and REPL are not

Since routing landed, `factorio-bot lua` could not run **any** script against
the world-record save: the run died at `cli/lua.rs` with `Failed to start
Factorio (no world available)` while holding four perfectly usable surfaces.
The seam was working — `only_surface()` answers only while there is exactly one
— and this path had never been told which surface it meant.

**Ported, by making the caller say.** `factorio-bot lua` takes `--surface`,
defaulting to `nauvis`; `FactorioInstance` gains `surface_named(&SurfaceId)` and
`surface_ids()`; `pick_surface` resolves, and **refuses by name, listing the
surfaces the world does hold**, when the name is not one of them.

**This is not `only_surface()` returning the default, and the difference is not
cosmetic.** A default flag value is a decision made *by the caller*, visible in
`--help` and in the invocation, and a name the world lacks is refused rather
than silently resolved. A lookup that cannot answer returning the same value as
one that answers is the defect the whole surface split exists to prevent, and
mutation **B** below is the test that forbids it — it adds exactly that fallback
and `a_named_surface_answers_where_only_surface_refuses` goes red.

`only_surface()` itself is untouched and still refuses on a two-surface world;
the test asserts that on the *same* world the named lookup answers.

### What remains, and it is small and mechanical

Two other callers still reach through the seam and will refuse on a
multi-surface world:

- **`crates/server/src/game/mod.rs::require_surface`**, used by ten handlers in
  `game/control.rs` and `game/query.rs`, plus
  `crates/server/src/manage/execute.rs`. The shape is the same: a `surface`
  query parameter (or a field on the request body) defaulting to `nauvis`,
  resolved through `FactorioWorld::surface`, refused by name with the census
  listed. It touches the OpenAPI snapshot and therefore the TypeScript contract
  seam, which is why it was **not** done here — `app/src/api/` is another
  session's file this week.
- **`app/src-tauri/src/repl/{dump,run_script}.rs`**, five `instance_state
  .surface()` calls. Cheapest of the three: the REPL already has a session
  object to hang a `surface` on, and nothing downstream of it is contract-bound.

Neither blocks a script from running, which is what item 3 was asked to unblock.

## Falsification

Every mutation asserted to substitute **exactly once** before the build; the
file restored afterwards.

| | mutation | tests killed |
|---|---|---|
| A | the `daylight` arm files every curve on the default surface | 2 |
| B | a named lookup that cannot answer hands back Nauvis (**the forbidden fix**) | **1** |
| C | `--surface` stops defaulting to `nauvis` | **1** |
| D | the no-world case stops saying "no world available" | **1** |
| E | `EntityGraph::add` stops calling `connect_nodes_near` | 2 (of this file) + 6 elsewhere |
| F | `connect()` becomes a no-op | **green — investigated, see §1** |

**A killing two is the feature, not redundancy**: with no routing the second
curve *replaces* the first on one surface, so one test fails because the derived
numbers collapse and the other because the surface it asks for was never
created. **E killing both graph tests is the same shape** — they are two
consequences of one property (`add` wires what it added), which is what the pair
is for.

**F is the one worth reading.** Green was predicted and then measured rather
than assumed, and it is a true statement about what the test can and cannot
prove, not a redundancy finding. Recorded in §1 in place.

### A trap in the falsification harness itself, which cost a false red

`shutil.copy2` preserves mtime, so restoring a mutated file gives it an mtime
**older** than the artifact cargo built from the mutation. Cargo then considers
the crate up to date and **re-runs the mutated binary**. A verification run
after the mutations reported
`a_half_dropped_into_a_tunnel_leaves_the_long_pair_behind` and
`wiring_one_entity_at_a_time_agrees_with_one_full_sweep` as failing on a tree
`git status` reported clean. `touch` on the restored files and a full re-run
came back green.

Same family as the stale-binary entries in `CLAUDE.md`: **a clean `git status`
is not evidence that what ran was the code on disk.**

## Verification

- `nix develop -c cargo test --workspace` — **109 test blocks, exit 0**, exit
  code taken from the command and not from a pipe (master carries 108; this
  branch adds one file). Re-run after the mutations, with the restored files
  touched, for the reason above.
- `nix develop -c cargo clippy --workspace --all-features --all-targets --
  --deny warnings` — exit 0. (`build_command` crossed clippy's 100-line ceiling
  with the new flag; the four session-scoped args moved to `session_args`.)
- `git status --porcelain crates/` in the main checkout at session start showed
  `crates/planner/{error.rs,method/blueprint.rs}` and
  `crates/scripting_lua/src/globals/goal/mod.rs` dirty — another session's work.
  None of them was touched; the work was done in an isolated worktree and the
  workspace test run was **not** scoped, because the worktree's own tree carried
  only this branch's changes.
- **Four offline baselines, byte-identical, on the release binary built from
  this branch** (`--no-default-features --features cli,lua`, `--bots 1,2,3,4`):

  | goal | world | expected | measured |
  |---|---|---|---|
  | `researched:automation` | `map.json` | 176 / 21,784 | **176 / 21,784** |
  | `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | **316 / 22,457** |
  | `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 | **441 / 47,478** |
  | `gathered:crude-oil` | `map-31337-explored.json` | 2,115 / 317,283 | **2,115 / 317,283** |

  `gathered:crude-oil` still refuses on `map.json`, naming the charting radius —
  correct, not a regression. **Both dumps loaded**: `workspace/scripts/map.json`
  and `workspace/scripts/map-31337-explored.json`.

  **One caveat, stated rather than papered over**: these were measured on the
  post-change binary only. The pre-change build was overwritten before it was
  measured, so this is "matches the four values three independent sessions
  confirmed today", not a before/after on one binary. It is the weaker of the
  two and it is what was done. Nothing here can move them in principle — the
  offline `plan` path deserialises a `FactorioSurface` and never runs the
  parser, the CLI seam or `on_init`.

- **No live run**, and no Factorio was started. `workspace/mods/BotBridge` was
  `readlink`-verified as pointing at the main checkout before and after
  (`/home/arturh/projects/private/factorio-bot/mods/BotBridge`); nothing here
  could have moved it.
- `rustfmt --edition 2024` on each edited file individually — note that
  `app/src-tauri` has its own `rustfmt.toml` (2-space) and the crates have none,
  so the two halves must be formatted from their own directories.

## Still open

- The Nauvis guard and its four upstream reasons, unchanged and untouched.
- `require_surface` and the REPL, above.
- **Nothing on the writeout wire is unrouted any more.** Six writeouts route
  (four `FactorioEntity`-shaped, `tiles`, and now `daylight`); the rest are
  about a game or a force rather than a place.
