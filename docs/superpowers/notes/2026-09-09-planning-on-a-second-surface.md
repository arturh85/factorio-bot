# Planning on a second surface: what has to change, and who means what

A survey first, written at `55c1b8e8` before any code, so that it survives
the worktree it was made in. Every claim cites a file and, where it matters, a
line read at that commit. The 2026-09-06 survey
(`2026-09-06-surfaces-survey.md`) is the map of the whole space; this note is
the one rung it called "rung 2, cheapest honest version": **one surface per
plan, named.**

## The premise, verified

`SurfaceId` appears in `crates/planner/src/` exactly three times and all three
are inside `#[cfg(test)]`:

```
crates/planner/src/state.rs:7137        surface: Some(SurfaceId::nauvis())   (a test fixture player)
crates/planner/src/method/power.rs:6951 use ...::{SurfaceDaylight, SurfaceId};
crates/planner/src/method/power.rs:6958 surface: Some(SurfaceId::nauvis()),
```

Four more mentions (`have.rs:8911`, `pipe.rs:1809`, `power.rs:3598`,
`power.rs:5804`) are `surface: None` in fixture players. **Nothing in planning
reads, stores or compares a surface.** `PlanState` (`state.rs:1512`) holds one
`base: Arc<FactorioSurface>` and every overlay is keyed by a bare `Pos`.

Two facts that shape everything below:

- **A `FactorioSurface` does not know its own name.** The struct
  (`crates/core/src/factorio/world.rs:1457`) has `globals`, `entity_graph`,
  `flow_graph`, the refusal ledgers and `daylight`; no id. The name lives only
  on the container, as the key of `FactorioWorld::surfaces`
  (`world.rs:1251`). So the moment a caller takes an `Arc<FactorioSurface>`
  out of the world — which is what `only_surface()`, `nauvis()` and
  `surface(&id)` all hand back — **the name is gone**, and nothing downstream
  can recover it. That is why `SurfaceId` never reached the planner: no path
  carried it there.
- **The dump has no surface slot either.** The hand-written `Serialize` for
  `FactorioSurface` (`world.rs:2252-2286`) writes `players`, `forces`,
  `entity_graph`, `inventories`, the ledgers, `benches`, `daylight` — and no
  name. So `plan --world`, `score-map` and `search` load a surface that
  cannot say which it is. Every archived dump *is* Nauvis, because the mod
  guard (`control.lua:2122`) has never let another surface's chunks reach
  Rust, but the dump does not record that; it is true by construction of a
  guard that is meant to be lifted.

## Every `only_surface()` and `nauvis()` caller, classified

(a) genuinely means Nauvis · (b) means "the surface this goal/script/run is
about" · (c) ambiguous, owner decision.

| Site | What it does with the surface | Class | Notes |
|---|---|---|---|
| `crates/core/src/process/process_control.rs:53` `FactorioInstance::surface()` | `world.only_surface()` — the seam itself | (b), a seam | Its doc says so. Callers below inherit its class. |
| `app/src-tauri/src/cli/lua.rs:307-335` `resolve_surface` / `pick_surface` | **Already ported.** `--surface <name>` (default `nauvis`, stated in `--help`) → `world.surface(&SurfaceId::from(name))`, refusing by name with the held surfaces listed. | (b), **done** | The *name* is then dropped at `lua.rs:441`: `Planner::new(world.clone(), rcon)` takes the `Arc<FactorioSurface>` only. **This is the exact point where the surface identity is lost on the production path.** |
| `app/src-tauri/src/repl/run_script.rs:26` | `instance_state.surface().unwrap()` → `Planner::new` | (b) | Unported; will panic on a two-surface world. Same shape as the lua CLI before `--surface`. |
| `app/src-tauri/src/repl/dump.rs:32-53` | `instance_state.surface()` → `world.dump(...)` etc. | (b) for `World`; (a)-ish for the three prototype dumps | Prototypes are game-global (`GameGlobals`); any surface answers the same. The world dump means "the surface I am looking at". |
| `crates/server/src/game/mod.rs:29` `require_surface` | every `/api/v1/game/*` query and control handler | (c) | Doc says: "the handlers, and the routes' shapes, are what would then need the surface named". A route parameter is an API-shape decision — owner. |
| `crates/server/src/manage/execute.rs:182` | `POST /api/v1/scripts/execute` picks the surface a script runs on | (b) | Same as the lua CLI; needs a request field or a default *stated in the API*, which is the (c) half of it. |
| `crates/core/src/process/output_parser.rs:799` `surface_or_create(&SurfaceId::nauvis())` | `on_init` connects the default surface's graph | (a) | The initial-discovery replay is Nauvis-only at both ends (`control.lua` ~1332/1364); the guard comment names this as item 4 of what blocks lifting it. |
| `crates/core/src/factorio/snapshot.rs:625` | a fixture player on Nauvis | (a), test | — |
| `crates/scripting_lua` — every `PlanState::from_world` (`goal/plan.rs:1124`, `goal/mod.rs:1532,1588`, `goal/recovery.rs:94,149`) | plans on `planner.world()`, whichever surface `Planner` was built on | (b) | Receives an `Arc<FactorioSurface>` and no name; cannot state what it plans on even when the CLI knew. |
| `crates/executor/src/recover.rs` (5 × `from_world`) | replans on the surface the run is on | (b) | **Not touched here**: agent `aa86dc2cef23ac66b` owns the executor's placement/evacuation path. Stays "unstated". |
| `app/src-tauri/src/cli/{plan,score_map,search}.rs` (`load_world` → `from_world`) | offline, on a dump | (b), blocked on the dump format | The dump cannot say. A `--surface` flag with a stated default would work, but the right fix is a surface slot in the dump; see "left open". |
| `crates/planner/src/method/orbit.rs` `FIRST_PLANET = "nauvis"` | the orbit the first platform is created in | (a) | Genuinely Nauvis: the silo is there. Documented as a constant on purpose. |
| `mods/BotBridge/control.lua:2122` (the guard) and the `game.surfaces[1]` observation sites listed in the 2026-09-06 survey §2(a) | drop / pin to Nauvis | (a) today, by design | Not touched. The guard's own comment lists what still blocks lifting it (items 3 and 4). |

**Class (c) is small and specific: the two HTTP entry points.** Everything
else is either Nauvis on purpose or "the surface I was handed" — and the
latter is unambiguous *except that the name was dropped in transit*.

## What must change for `PlanState` to plan on a non-Nauvis surface

In dependency order. Nothing here touches geometry — the 2026-09-06 survey's
finding that every position-keyed structure is correct *within* a surface
still stands.

1. **Carry the name with the surface, from where it is known to where it is
   used.** The CLI knows it (`--surface`); `Planner` and `PlanState` do not.
   `Planner` gains a `surface: Option<SurfaceId>` and a builder to state it;
   `run_lua` → `create_lua_goal` → `create_lua_goal_with` →
   `install_goal_plan` / `install_goal_holds` / `PlanOrigin` thread it; and
   `PlanState` gets a constructor that takes it. `None` throughout means **the
   caller never said** — the same seam `only_surface()` is, moved one layer
   down — and is never Nauvis. That keeps the 333 existing `from_world` call
   sites (320 of them fixtures) compiling and honest: a fixture that never
   named a surface is planning on an unnamed one, which is what it was doing
   yesterday.

2. **Use the name where the plan's model of a bot is built**, which is the
   one place the 2026-09-06 survey named as genuinely ambiguous:
   `GameGlobals::players` is game-global and `PlanState::from_world` reads it
   as "the bots I may give steps to" — a per-surface question. Concretely,
   `FactorioPlayer::surface` (`types.rs:673`) has been carried since 2026-09-06
   and read by nobody in the planner:
   - a **roster bot** whose player stands on another surface must not be
     given steps here. `from_world` records it (`bots_elsewhere`), the way it
     records `unknown_bots`, and `goal.plan` refuses by name, the way
     `refuse_unknown_bots` does. Silently planning for it is the survey's
     "five-tick walk to the wrong planet".
   - a **bystander** on another surface must not block ground here:
     `characters` (`state.rs:1912`) is built from every player in the game,
     so a character at (10, 10) on a platform would shadow tile (10, 10) on
     Nauvis. Filtered by surface when both sides state one.
   A player whose `surface` is `None` (the mod did not say) is neither
   elsewhere nor here — it is kept, as today.

3. **Disclose it.** `plan_created` and `provenance.json` carry no surface
   (survey rung 0). Not done here; listed under "left open".

4. **The dump format**: a surface slot, `#[serde(default)]`, so `plan --world`
   can state what it plans on. Needs `FactorioSurface` to know its name or
   the world to be what is dumped — a shape decision, listed under "left
   open".

5. **Then, and only then, the mod guard.** Items 3 and 4 of its own comment
   (the Nauvis-only initial-discovery replay; `on_init` connecting the default
   surface's graph) are the observation-side port and are not touched here.

## What was implemented (this note's commit is followed by the code)

Steps 1 and 2, the unambiguous part of class (b):

- `crates/planner/src/state.rs`: `PlanState::on_surface(surface, base, bots)`
  beside `from_world`; `PlanState::surface() -> Option<&SurfaceId>`;
  `PlanState::bots_elsewhere() -> &BTreeMap<BotId, SurfaceId>`; `characters`
  filtered by surface. Three tests pin the three answers (stated and
  elsewhere → recorded and unshadowed; stated and same → kept; unstated →
  nothing changes).
- `crates/core/src/plan/planner.rs`: `Planner::surface: Option<SurfaceId>`
  and `Planner::on_surface(self, SurfaceId)`.
- `app/src-tauri/src/cli/lua.rs`: the resolved `--surface` name is no longer
  dropped at `Planner::new`.
- `crates/scripting_lua`: the surface rides `create_lua_goal` →
  `create_lua_goal_with` → `install_goal_plan` / `install_goal_holds` /
  `PlanOrigin`; `refuse_bots_elsewhere` beside `refuse_unknown_bots`.

Behaviour on every existing run is unchanged by construction: every player
the mod has ever reported is on `nauvis` or reports no surface, so the filter
admits everyone and `bots_elsewhere` is empty. Measured, not assumed: on one
release binary built in this worktree (`cli,lua`), before and after, against
`workspace/scripts/map.json` with bots 1,2,3,4 --
`researched:automation` 176 / 21,784 · `producing:automation-science-pack:6`
316 / 22,457 · `producing:logistic-science-pack:6` 571 / 54,371. Identical.

Two things that bit on the way, for the next person:

- `crates/scripting_lua/src/doc_guard.rs` reads a bindings file **up to its
  first `#[cfg(test)]`** as the production half. Gating a now-test-only
  helper with `#[cfg(test)]` above `install_goal_holds` made the guard
  report `goal.holds` as never installed. `#[cfg_attr(not(test),
  allow(dead_code))]` is the form that keeps the file whole.
- `crates/planner/tests/planning_work_ceilings.rs` returns early without
  `workspace/scripts/map.json`, which no worktree has. A symlink
  (`.worktrees/<x>/workspace/scripts/map.json` -> the main checkout's) makes
  it run; the log line `the_three_map_json_baselines_stay_within_their_work_ceilings ... ok`
  is the proof it did, and `SKIPPED` is the proof it did not.

## Left as owner decisions or follow-ups

- **(c) `require_surface` and `/api/v1/scripts/execute`**: a route parameter
  or request field, and what its default is *in the API*. Shape of the
  public contract; the snapshot seam will fail on both ends when it changes.
- **The dump format carries no surface.** Either `FactorioSurface` learns its
  name (then `FactorioWorld::new(id, surface)` and `insert_surface(id, ..)`
  hold a second copy that can disagree with the key) or the dump becomes a
  `FactorioWorld` dump (a format change for an 864 MB artefact). Until then
  `plan`/`score-map`/`search` plan on an unstated surface.
- **`provenance.surface` and `plan_created.surface`**: rung 0 of the
  2026-09-06 survey; cheap, not done here to keep the change to one seam.
- **`Goal` carries no surface.** `Goal::Orbiting` deliberately does not
  (`goal.rs:430`). Whether a goal names its surface or the plan does is the
  design question rung 3 opens; one surface per plan is enough for a platform
  that does not move.
- **`benches` follows `players`** (`globals.rs:52`): a benched bot's position
  is on some surface; filtering it the same way is a one-liner once someone
  decides a bot can be benched on a surface the plan is not on.
- **The executor's five `from_world` calls** in `recover.rs` stay unstated
  until the executor agent's work lands.
- **The mod guard** stays. Lifting it is items 3 and 4 of its own comment, on
  the observation side, and is a separate piece of work.
