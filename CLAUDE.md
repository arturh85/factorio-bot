# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Factorio Bot is a Rust application that orchestrates Factorio game servers and multiple bots via Lua scripting, with a browser frontend served by its own HTTP server. It was a Tauri desktop app until plan 5 (`docs/superpowers/plans/2026-08-30-frontend-transport-swap.md`) removed Tauri entirely; the crate directory is still named `app/src-tauri` because renaming it is a deferred mechanical change, not because Tauri is still there. Use cases include tool-assisted speedruns (TAS), ML training environments, and Factorio experiments.

## Build & Development Commands

**Never `git commit --amend` in this checkout, and never `cargo fmt --all`.**
Both assume you are the only writer and neither checks. An agent amended what it
believed was its own HEAD; another had committed in the intervening seconds, so
it amended *their* commit. It was recoverable — `git reset --soft` back to the
original commit object from the reflog, `git diff` verified empty, then commit
separately — but only because it noticed. `cargo fmt --all` has the same shape
and has already rewritten another agent's live files; `cargo fmt -p <crate>`
still rewrites a whole crate, so use `rustfmt --edition 2024 <file>`. **The
edition flag is not optional**: bare `rustfmt` defaults to Rust 2015 and dies on
every `async fn` in the file, which reads as a compile error in code that builds
fine -- and chained with `&&` it silently skips whatever you meant to run next.
It skipped a crate test run here, in the same breath as telling agents to prefer
`rustfmt <file>`.

The same reasoning covers `git add -A`, a bare `git commit`, `git reset --hard`
and `git stash`: commit with explicit paths (`git commit -m "..." -- <paths>`),
and note that **an explicit path is still a path to the whole file** — naming a
file carefully does not separate two writers editing it.

**Every cargo command needs `nix develop -c`.** `pkg-config` and Lua 5.4 come
from the flake, not from the ambient shell, so a bare `cargo build` dies in
`mlua-sys`' build script with

```
cannot find Lua5.4 using `pkg-config` ... The pkg-config command could not be found.
```

which reads like a missing system package and is not one. Same for the
graphical client: the GL/X11 libraries and `SDL_VIDEODRIVER=x11` are set by the
dev shell, so a client launched outside it fails with `No available video
device` — a message that says nothing about the cause (see the Platform Notes).
Prefix the command (`nix develop -c cargo test --workspace`) or wrap a whole
script (`nix develop -c bash -s <<'EOF' ... EOF`). Direnv is not configured
here, so nothing enters the shell for you.

```bash
# Master Check Tool, runs Rust clippy, tests, build, takes a few minutes, only run in the end to finalise a change
# Verify only -- never rewrites a file, so it is safe on a dirty tree and when
# another agent is working in the same checkout.
just test

# Apply what `just test` reports. Rewrites files workspace-wide -- only run it
# when every uncommitted change in the tree is yours.
just fix

# Start Factorio server with BotBridge mod
just factorio

# Frontend dev server on :8080, proxying /api to a `just serve` on :7492
just start          # or: cd app && pnpm start

# The real thing: axum serving the built SPA and the API on :7492 -- this is
# "the viewer". It builds `factorio-bot`'s `viewer` feature alias (cli, lua,
# restapi -- no repl, no tokio-console), equivalent to
# `cargo run --release --no-default-features --features viewer -- serve --web-root app/dist`.
# Do NOT reach for `--all-features` to get `restapi`: it also builds
# `tokio-console`, whose fixed debug port then contends with any other build
# of this binary run alongside it (see the tokio-console note below).
just serve

# REPL mode (faster build, no GUI, for testing scripting)
cargo repl

# Run frontend tests (vitest, watch mode; `pnpm run test:coverage` for one shot
# plus the enforced coverage gate)
cd app && pnpm test

# Rust tests. `cargo nextest` is NOT installed here -- if you install it, note
# that it runs one process per test, which contains a hang to a single named
# failure instead of killing the run, but does NOT run doctests. That is a
# real trade, not a free upgrade.
cargo test --workspace

# Lint everything
cd app && pnpm lint              # TypeScript + ESLint + Vue type checking
cargo clippy --workspace --all-features --all-targets -- --deny warnings

# Pre-commit check (runs all CI checks locally)
cd app && pnpm run precommit:check

# Production build
cargo build --release           # the binary IS the deliverable now; default
                                 # features (restapi, repl, cli, lua) already
                                 # cover it -- no flag needed, and specifically
                                 # not --all-features (see below)
cd app && pnpm run build:web    # and the SPA it serves

# Build with/without default features
cargo build --all-features
cargo build --no-default-features
```

**Do not build with `--all-features` to "get restapi".** Default features
already include it (`default = ["restapi", "repl", "cli", "lua"]`); the only
feature `--all-features` adds beyond that is `tokio-console`, opt-in debug
instrumentation that binds a fixed TCP port (`127.0.0.1:6669` unless
`TOKIO_CONSOLE_BIND` says otherwise). A viewer build and a game-run build both
started with `--all-features` both try to bind it, so the second one either
loses tokio-console (current behavior: `Context::new` probes the port first
and warns instead of installing the console layer when it is taken -- see
`app/src-tauri/src/context.rs`) or, before that fix existed, panicked the
whole process, because `[profile.release]` sets `panic = "abort"` and the
crash came from `console-subscriber`'s own background thread with no mention
of Factorio. For the viewer specifically, use `just serve` or
`--features viewer` (an alias for `cli,lua,restapi`, deliberately excluding
`tokio-console`) rather than `--all-features`.

*LSP tools**: Prefer `mcp__rust__lsp_*` tools for refactoring (rename_symbol, find_references, get_definitions)
These leverage rust-analyzer for accuracy with macros and trait implementations

### The shell here is aliased, and the aliases fail in ways that look like data

Three commands do not mean what they say, and each has silently produced a
wrong answer in this repo rather than an error you would notice.

- **`ls` is `eza`.** `ls -t` does not sort by time -- `-t` is eza's
  `--time <FIELD>` and wants a value, so `ls -1t <dir>` dies with
  `invalid value '<dir>' for '--time <FIELD>'`. Worse, under `zsh` a
  command whose glob fails aborts the whole line, so
  `ls -1t runs/*/events.jsonl 2>/dev/null | xargs ...` prints **nothing at
  all** and looks exactly like "no runs matched". Three separate analyses
  returned empty this way before the alias was noticed. Use `command ls`.
- **`cat` is `bat`**, so `cat -v` and `cat -A` fail rather than showing
  non-printing characters.
- **`git diff` is difftastic**, so `git diff | git apply` cannot work. Use
  `git diff --no-ext-diff` when something machine-readable is wanted, or
  `git revert`.

**A pipeline reports the LAST command's exit code, not the interesting one.**
`cargo test --workspace | grep -E "^test result"` exits 0 whenever *grep*
matched something, even with a failing test in the output. That has already
shown a green `[exited with code 0]` over a genuinely red run. Redirect to a
file and test the exit code, or set `pipefail`; never read a pipeline's status
as the tool's status.

The general shape: **an aliased command that fails still exits into a pipe**,
and a pipe that receives nothing reads as an empty result rather than as a
broken command. When an analysis over run records comes back empty, check the
command before concluding anything about the data.

### Evaluate a planner change OFFLINE first -- seconds, not a 20-minute run

Since `db612be9` a plan can be made against a **dumped world**, with no
Factorio, RCON, workspace or settings file:

```bash
# in a live run, from Lua -- writes <scripts>/map.json
world.dump("scripts/map.json")

# then, offline, in ~4 seconds against an 864 MB dump
factorio-bot plan --world workspace/scripts/map.json \
    --goal researched:automation --bots 1,2,3,4 --steps
factorio-bot score-map --world workspace/scripts/map.json --bots 1,2,3,4
```

`score-map` reports resource distances, a walk score, a verdict, and the full
`PlanReport` -- actions, makespan, steps/acts/walks per bot, planned and idle
ticks, roster utilisation. **This is the loop to iterate in.** It has caught a
13:05 planning ceiling, a green-science capability gap and a whole workstream's
result without spending a run.

**Since 2026-09-05 `workspace/scripts/map.json` is the seed-31337 t=0 dump**
(fingerprint `c161fa3f437221d0`, four headless character bots with the
honest freeplay inventory, `researched:automation` = 177 actions / 22,044
ticks, `producing:logistic-science-pack:6` = 623 / 71,167), kept also as
`map-31337-t0.json`. Before that it was the OLD map's baseline, which lives
on as `map-t0-baseline.json` (fingerprint `dfac0f4caa0a7500`) -- every
offline number in the 2026-09-03/04 record was made on that map while the
live runs were on 31337. Regenerate with
`factorio-bot lua dump_31337.lua --headless --bots 4 --seed 31337 --new`
(the script is `world.dump("map-31337-t0.json")`); a one-bot dump plans the
other three with fabricated empty inventories and reads 8:48 for automation.
Anything overwriting `map.json` -- a seed search, a `--resume-from` dump --
destroys it unless you copy it aside first. That has already happened once.

**Three blind spots, each of which has produced a wrong "the bug is absent":**

- **`world.dump` never calls `Planner::refresh_buffers`**, so a dump's
  `inventories` is `[]`, and the `plan` CLI does not refresh either (only the
  Lua `goal.plan` path does). **The entire `Withdraw` path -- furnaces handing
  their contents over -- is unreachable offline.** A real double-spend bug lived
  exactly there and needed hand-injected inventories to reproduce.
- **A dump is t=0-shaped unless you make it otherwise.** A `--resume-from`
  savepoint restores *saved* inventories and positions, not the live ones at the
  moment of failure. Two separate bugs needed the dump perturbed -- one with bot
  positions, one with inventories -- before they appeared at all.
- **A fresh map has charted almost nothing.** Distances read off an early dump
  measure what has been *seen*, not what exists.

### Ask the RUNNING game directly -- `rcon -s localhost`

`factorio-bot rcon -s localhost -- '<command>'` attaches to an **already
running** instance and prints the reply. No MCP server, no schema, no second
process: it is the fastest loop in this project for any question about live
game state, which is exactly the class `world.dump` cannot answer (its
`inventories` is always `[]`).

```bash
factorio-bot rcon -s localhost -- '/c rcon.print(game.tick)'
factorio-bot rcon -s localhost -- '/c rcon.print(serpent.line(remote.interfaces))'
```

Worked example, from a run whose `BatchProgress` counters had been frozen for
690 s with one action in flight:

```
1:char   2:char MINING   3:char MINING   4:char MINING
```

Bot 1 idle while the others mined -- so the game was healthy and **bot 1's
completion signal was lost**, not a lag wait. Thirty seconds, against the
25-minute run that question used to cost.

The mod's interface (`remote.interfaces.botbridge`) includes `world_snapshot`,
`inventory_contents_at`, `find_entities_filtered`, `find_tiles_filtered`,
`player_info`, `player_force`, `players`, `set_recipe`, `savepoint`,
`session_reset` and the `action_start_*` family. **`freeplay` also exposes
`set_chart_distance`** -- relevant to exploration, which this mod otherwise
never does.

**Two rules.** Keep it **read-only during a measured run**: a query is cheap,
but mutating a live game contaminates the measurement, and provenance has no
field that would record it. Anything goes against a savepoint-resumed world.
And do not add `rcon.print` *inside* a mod function the executor calls -- that
output lands in the RCON reply body and the executor reads it as the action's
result. An ad-hoc command on your own connection is a different thing and is
safe.

### Action ids are PER-PLAN, not global

A run replans, and ids are reused across plans: in `run-1788465258-49050`, id 38
is `craft 2 iron-gear-wheel` in the first plan and `craft 1 offshore-pump` in
the third. **44 of 147 ids collide in that one run.**

So **never key an aggregate on `id`, and never join anything on `id` across a
plan boundary.** Doing so once understated a bot's executing time by 8,067 ticks
(it silently dropped 44 settles); doing it again invented three "blocker cleared
in 10,000-40,000 ticks" measurements that were replan boundaries. Aggregate over
**settle events**; identify actions by `(id, dispatch order)` or by position.

### Milestone savepoints make a failure reproducible

Every milestone writes `runs/<run>/savepoints/milestone-N.zip` plus a `.json`.
`--resume-from <run>[:<milestone>]` starts a run on that world (`lua` and
`start` both take it). A resumed run is marked in provenance and `--compare`
refuses to compare it against a fresh one -- correct, and expected.

This is what turned "reproduce the failure by paying for the whole 25-minute
prelude again" into a short server start plus an offline plan.

## Architecture

```
Browser (Vue 3 + Tailwind v4 + reka-ui)
    ↓ (HTTP + SSE, no IPC)
crates/server (axum: /api/v1/*, serves the built SPA)
    ↓
┌─────────────────────────────────────────┐
│ Rust Workspace                          │
│  • crates/core         (orchestration)  │
│  • crates/scripting*   (Lua via mlua)   │
│  • crates/server       (axum HTTP+SPA)  │
└─────────────────────────────────────────┘
    ↓ (RCON + process management)
Factorio Runtime
    ├─ Server Instance (headless)
    └─ Client Bots (graphical)
    ↓ (RCON/Events)
BotBridge Mod (Factorio mod for RPC)
```

### Key Rust Crates

- **crates/core**: Main orchestration engine
  - `types.rs` - Shared data models
  - `factorio/rcon.rs` - RCON protocol implementation
  - `graph/entity_graph.rs` - Spatial entity relationships
  - `graph/flow_graph.rs` - Material flow throughput
  - `process/` - Factorio process spawning/control
  - `plan/planner.rs` - `Planner`, the Lua runtime's context holder (rcon,
    real_world, plan_world). NOT a planner any more: the task-graph planner it
    was named for was deleted, and goal decomposition now lives in
    `crates/planner`. The name is kept because `run_lua` takes it.

- **crates/planner**: the goal planner. Pure and deterministic — no I/O, no
  async, no wall-clock, ordered collections only, floats via `total_cmp`.
  `Goal` -> `expand()` -> `ActionNetwork` -> `schedule()` -> `Schedule`.
  Methods live in `method/`; `state.rs` overlays a `FactorioWorld` snapshot.

- **crates/executor**: runs a `Schedule` across bots over RCON. Per-action
  completion signals (`tokio::sync::watch`, not polling), lag edges modelling
  machine time, pre-flight wait-graph cycle rejection, and recovery tiers in
  `recover.rs`. Issues only legitimate player actions — no `cheat_*` calls.

- **crates/scripting_lua**: Lua 5.4 bindings exposing host functions for task queuing, graph queries, and RCON commands
  - `sandbox.rs` - the restricted interpreter every user script runs in
  - `lua_docs.rs` - generates all five files in `docs/lua/src/` — build
    artifacts, gitignored, written by `app/src-tauri/build.rs` (which is the
    CLI crate's build script; the directory name is a leftover). Never edit a
    `.lua` there.
    - `{globals,world,rcon,goal}.lua` come from the `__doc_entry_*` strings in
      `globals/`; edit the Rust strings.
    - `types.lua` comes from `schema_for!` on the roots listed in
      `documented_type_schemas()`, i.e. from the `JsonSchema` derive on
      `crates/core/src/types.rs`, which reads the same serde attributes that
      decide the wire shape. It was hand-written and tracked until it had
      accumulated two outright lies and was missing the type they lied about.
    - The two halves are held together: the roots must be **exactly** the set
      of `` `types.X` `` names the four binding files mention. A `@return`
      naming a type with no root fails the build; a root no binding hands out
      fails it too. So documenting a new return type means adding
      `schema_for!(That)` — and `JsonSchema` is required transitively, so
      everything it contains is described as well.

- **crates/server**: axum HTTP server (replaces the former Rocket `crates/restapi`)
  - `webserver.rs` - router assembly, static SPA serving, `start()` entry point
  - `game/` - the `/api/v1/game/*` query and control handlers
  - OpenAPI generated with utoipa; Swagger UI at `/swagger-ui`, spec at `/openapi.json`
  - Serves the built Vue SPA from the configured `web_root` (index.html fallback); with no web root it serves the API only and redirects `/` to the docs

### Frontend (app/src/)

- Vue 3 with Tailwind v4 and headless `reka-ui` primitives. **PrimeVue and
  PrimeFlex are gone** (plan 6) — there is no component library to register, so
  `main.ts` installs only Pinia and the router. Shared primitives live in
  `components/ui/` in the shadcn style: single-word names, owned in-tree rather
  than imported, and listed in the `vue/multi-word-component-names` ignore list
  in `eslint.config.mjs` — extend that list when adding one.
- Icons come from `@lucide/vue`; `@vueuse/core` supplies the composables.
- Monaco editor for Lua scripting (`components/Editor.vue`), whose workers are
  wired through Vite's native `?worker` imports
- Pinia 4 for state management (`defineStore('id', {...})`, not the removed
  object-with-id form) and Vue Router 5 with hash history
- Vite config lives in `vite.config.mts` — `.mts` because the package is not
  `"type": "module"` and some plugins are ESM-only
- ESLint uses flat config in `app/eslint.config.mjs`; `.eslintrc.js` no longer
  works on ESLint 10. `pnpm lint` runs `tsc`, `vue-tsc` and `eslint` over both
  `.ts` and `.vue` files

The frontend/backend contract is pinned by a **snapshot seam**, and it fails
from both ends on purpose. `app/src/api/openapi.snapshot.json` is generated
from the utoipa spec (`UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p
factorio-bot-server --features lua --test openapi`), and
`app/src/api/openapi.contract.spec.ts` ties every published schema to a
declaration in `app/src/api/types.ts` via `objectContract<T>`. Adding a field
in Rust fails the Rust snapshot test until regenerated, then fails the
TypeScript contract test until mirrored. Neither half can drift quietly.

### Logging: two systems, on purpose

**Narration is `paris`, on stdout. Diagnostics are `tracing`, on stderr.**

A line the user reads *while the tool runs* is narration: progress, the
spinner, the address it bound, "press Ctrl-C to stop". A line that explains
something to whoever debugs it *later* is a diagnostic. `paris` was kept for
narration deliberately -- it has zero dependencies and does colour, glyphs and
timestamps unaided, so replacing it would add `console` plus a time crate
rather than remove anything.

Two traps here, both of which have already been paid for:

- **A `tracing` event goes nowhere unless a subscriber is installed**, and
  silently -- the macro still compiles and runs. Every `tracing::` call in this
  workspace emitted nothing until `74269106`, including two error paths, which
  went unnoticed because a `paris` line one statement away said something
  similar. The subscriber lives in `app/src-tauri/src/context.rs`.
- **`crates/core/src/lib.rs` has `#[macro_use] pub extern crate paris`**, so
  `info!`/`warn!`/`error!` resolve with no import anywhere in that crate --
  ~148 call sites name no logging system at all. Import `tracing` explicitly at
  each converted site; never grant `tracing` the same global, or a
  half-converted file compiles cleanly with no way to tell which macro a line
  called. `lib.rs` says so in place.

Colour markup (`<bright-blue>{}</>`) is `paris` syntax and renders only through
`paris`. A string moved to `tracing` must have its tags stripped or they print
literally.

### Communication Flow

1. User scripts written in Lua via Monaco editor
2. Scripts schedule goals → planners expand to task graph nodes
3. Executor assigns tasks to bots based on availability/travel time
4. RCON commands sent to Factorio via BotBridge mod
5. Entity/event data streams back to update graphs

### Script Execution (HTTP)

`POST /api/v1/scripts/execute` takes either a `path` under the scripts
directory or inline `code`, answers `202` with a `job_id`, and runs the script
detached (`crates/server/src/manage/execute.rs`, `crates/server/src/jobs.rs`).

- **One script at a time.** The job registry holds a single execution slot.
  While one script runs, a second request is refused with `409`, whose body
  carries `running_job_id` — the id of the job actually holding the slot, so a
  caller can attach to it rather than guess. The slot is taken *last*, after
  the body, the running Factorio instance and the script path have all been
  checked, so a request that was going to fail anyway never holds it.
- **Output comes over SSE, not in the response**, at
  `GET /api/v1/jobs/{id}/events`. A subscriber that attaches mid-run receives
  the backlog of lines already printed and then the live ones, so nothing is
  lost by attaching after the `202`. The stream carries *script* output only,
  not the Factorio process's stdout. `GET /api/v1/jobs/{id}` is the polling
  alternative and returns the job's status.
- **Scripts are sandboxed to the scripts directory.** The endpoint is
  unauthenticated, so this boundary is what stands between an HTTP caller and
  the host. `crates/scripting_lua/src/sandbox.rs` builds the interpreter with
  `table`, `string`, `math` and `coroutine` only — no `io`, `os` or `package`,
  with `require`, `dofile` and `loadfile` removed and `load` restricted to
  text chunks (Lua 5.4's bytecode loader does not validate untrusted input).
  That leaves `include`, `file_read`, `file_write` and `world.draw` as the
  only filesystem access, and each resolves through `resolve_script_path` /
  `resolve_write_path` (`crates/core/src/scripts.rs`), which refuse any path
  that leaves the scripts root.

### Replay and Video

**Per-camera screenshots ("frames") were retired on 2026-09-02 and removed on
2026-09-03.** The mod no longer calls `game.take_screenshot` on a beat, there is
no `frames/` directory, no `/api/v1/frames*` route, and no `ArchivedFrame`.
Video is the visual record. Do not reintroduce a screenshot cadence without
reading `docs/superpowers/notes/2026-09-02-screenshots-retired.md`: one run
wrote 2,164 JPEGs / 947 MB against 290 MB for the same 45 minutes of video, and
`take_screenshot` renders *synchronously inside the game loop* where the video
grabber reads a frame the GPU already drew.

Three unrelated things are still called "frame" and must survive a grep:
entity-map **keyframes** (`map.jsonl`, `lib/runMap.ts`), **video frames**
(`crates/core/src/record/video/`, `api/videoClock.ts`), and
`requestAnimationFrame`.

A run produces artefacts joined in the UI by `game.tick`, the only clock they
share.

- **Replay** — the executor serialises what it actually did, reaching the
  browser over the job's SSE stream (`WireEvent::Replay`), not a REST route: it
  belongs to the run that produced it, so it travels with that run's output.
  `app/src/api/replay.ts` narrows it at runtime (`parseReplay`) rather than
  casting.
- **Video** — opt-in per run (`record.start({video = true})`), filmed from a
  graphical client's window by ffmpeg and joined to the plan through
  `ticks.jsonl`, the `(tick, wall_ms)` table the viewer interpolates. It is
  *not* tick-exact: a stalled game keeps writing the last drawn image, which
  looks exactly like a game that was running and idle. The event log and
  `map.jsonl` are the tick-exact record.
- **Samples** — `samples.jsonl`: research, production, power and bot
  inventories, on 300- and 60-tick beats. These ride on the mod's *sampling
  session* (`rcon.sampling_start(run_id)` / `sampling_stop()`), which is what
  the screenshot capture used to ride on. Removing the session would take the
  whole world-state stream with it, silently — that is why it outlived the
  cameras.

**Planned ticks start at zero; observed ticks are absolute `game.tick`.** A run
dispatching its first step at tick 60,551 would otherwise draw the whole plan in
the first 1.4% of the axis. `ReplayScrubber.vue` works in shifted ticks and
converts in exactly one place — `observedOrigin()`.

**The join is checked, never assumed.** `app/src/api/runMatch.ts` compares tick
ranges and run ids and reports `contradicted | confirmed | consistent |
inconclusive`; `api/videoJoin.ts` applies it to a recording. A missing run id is
**unknown, never "no match"** — treating absence as mismatch refuses a good
join; treating it as a match asserts something nobody established.

## Lua Scripts

User scripts are in `/scripts/`. Key files:
- `lib.lua` - Helper library
- `example.lua` - Example bot automation

Lua API docs: https://arturh85.github.io/factorio-bot/lua/

## Release Process

```bash
cargo install cargo-release git-cliff  # if not installed
cargo release <patch|minor|major>      # dry run first
cargo release <patch|minor|major> --execute  # actual release
```

Uses git-cliff for changelog generation with conventional commits.

## Documentation

- User Guide: https://arturh85.github.io/factorio-bot/userguide/
- Dev Guide: https://arturh85.github.io/factorio-bot/devguide/
- Lua API: https://arturh85.github.io/factorio-bot/lua/
- Rust docs: https://arturh85.github.io/factorio-bot/doc/factorio_bot/

Local docs: `cd docs/userguide && mdbook serve` or `cd docs/devguide && mdbook serve`

## Platform Notes

- **Windows**: Window resizing utilities in `crates/core/src/windows.rs`
- **macOS**: Lua 5.4 via Homebrew with pkgconfig
- **Linux**: Requires libwebkit2gtk-4.1-dev, libsoup-3.0-dev

## Cargo Configuration

`.cargo/config.toml` enables `tokio_unstable` for tokio-console support and defines the `repl` alias.

## Multi-Client Testing

The system supports running multiple graphical Factorio clients controlled by Lua scripts for multi-bot coordination.

### Headless character bots: `--headless`, and the two modes

**A bot is either a connected graphical client or a server-side `character`
entity, and a run is all of one or all of the other.** The mix is refused by
name, in the mod and in core, before a process is spawned.

```bash
# Iterate: four character bots, no client, world at 5x
just headless factory_stage2.lua
factorio-bot lua <script> --headless --bots 4 --game-speed 5

# Measure or film: clients, 1x, the number you quote
just bench <script>
```

Why it exists: a client costs ~26 s of sprite load each plus a connect wait
bounded at 300 s, and **`--headless` had the script running 12-13 seconds after
launch** with four bots. `--game-speed` scales `game.speed` and the executor's
wall-clock deadlines with it (`Actuator::game_speed` now reads the real value
over RCON instead of assuming 1.0).

Three things that are **not** interchangeable between the modes:

- **A character bot is honestly equipped; a `--clients 0` bot is fabricated.**
  Spawning asks freeplay for its starting items
  (`remote.call("freeplay", "get_created_items")`), so each character holds the
  8 iron plates, furnace, drill and wood a joining player gets. `--clients 0`
  has no players at all, so
  `Planner::initiate_missing_players_with_default_inventory` **synthesises** a
  roster holding wood/furnace/drill and no plates. `goal_smoke.lua` asserts a
  positive makespan for `have("iron-plate", 5)`, which is true only of the
  fabricated roster: on `--headless` the goal is already satisfied and an empty
  plan is the correct answer, not a regression.
- **No video.** It is filmed from a client window and there is none, so
  `record.start{video = true}` warns and records everything else rather than
  failing the run.
- **Provenance says which mode ran**: `bot_mode` (`clients` / `characters`) and
  `game_speed`, written at run start from `<instance>/run-mode.json`.
  `just analyse` treats a difference as a note, not a refusal, but **do not
  compare a 5x headless run's timings against a 1x client run**.
- **Trigger technologies are emulated, and only the `craft-item` ones.**
  Factorio 2.0 unlocks 32 technologies by *doing* — `automation-science-pack`
  by crafting one lab, `electronics` by 10 copper plates, `steam-power` by 50
  iron plates — and the game fires those from **player** actions, which a
  server-side character never performs. Left alone, a headless run crafts a
  lab, places it, and still cannot craft red science: it halts `stuck` at
  milestone 1 with a precondition that can never become true.
  `emulate_research_triggers` completes such a technology when the force has
  **already** produced what the trigger names, sweeping every 60 ticks, only
  while character bots exist, and writing a `research_trigger_emulated` event
  naming the counts that earned it. Two counters are consulted and **the larger
  is taken, never the sum**: machine production (the force's statistics) and
  hand crafts (`storage.crafted_tally`), because **a hand craft does not appear
  in production statistics at all** — measured, and the reason the first
  attempt unlocked both plate triggers and never the lab one.
  The `mine-entity`, `build-entity`, `capture-spawner` and
  `create-space-platform` triggers are **not** emulated. Since `ffc56270` the
  mod does send their payload (`oil-processing` arrives as
  `{"type":"mine-entity","entities":["crude-oil"],"count":1}`, verified on a
  server-only dump), so the planner can refuse `oil-processing` by the next
  rung rather than by the trigger, but emulating one honestly needs the
  qualifying act to have actually happened, and a `mine-entity` trigger means
  a real extractor mining a real patch. A headless run therefore still cannot
  cross them.
- **Nothing above 4 bots has been run.** `PlayerId` is `u8`, so 255 is the
  arithmetic ceiling; before that, the mod polls **every bot every tick**
  (whole main inventory, sorted signature, crafting-queue scan) and every
  action is its own RCON round trip, so bot count is what costs tick rate.
  Parallel *runs* are the cheap axis instead: a headless run needs only a
  server, its own workspace and its own ports, so the limit is cores.

### Important Timing Considerations

- **Archive extraction**: 8-10 minutes per client instance on first setup (macOS DMG extraction)
- **Server startup**: ~12-17 seconds to initialize and be ready for connections
- **Client loading**: ~26 seconds per client to load sprites before connecting
- **Connection wait**: System polls for up to 90 seconds waiting for clients to connect
- **Total time**:
  - First run with new clients: 15-20 minutes (due to archive extraction)
  - Subsequent runs: 120-180 seconds for multi-client tests (2-4 clients)

### Running Multi-Client Tests

```bash
# Iterating? Use a DEBUG build. Timing a run? Use --release.
#
# This is not a preference, the two builds behave differently:
#
#   debug    workspace/mods/BotBridge is a SYMLINK to the repo's
#            mods/BotBridge, created or repaired on every setup, so an edit
#            to mods/BotBridge/control.lua is what the next run loads. There
#            is nothing to refresh and no copy to go stale. workspace/mods
#            itself stays a real directory: the other mods live there, and
#            Factorio rewrites mod-list.json and mod-settings.dat in it.
#            Do NOT "delete workspace/mods to re-seed" -- every instance's
#            mods dir is a symlink to it, so deleting it leaves the server
#            with no bridge mod: it hangs at `start waiting` forever after
#            writing a level.zip with no bridge state, which poisons every
#            later run (Factorio only migrates on a version bump and
#            info.json is pinned at 0.0.1).
#            Setup also re-enables BotBridge in workspace/mods/mod-list.json
#            if a previous run dropped or disabled it -- a mod present on
#            disk but absent from an existing mod-list.json is a DISABLED
#            mod, and that failure is completely silent.
#            Every run -- either build, no flag needed, and NOT gated on
#            --verbose any more -- logs one line naming which directory won
#            and what BotBridge is: "Using mods directory <absolute path>
#            (<why>)". That line, not a guess from a traceback, is the
#            authoritative answer to "did my edit ship". It used to be
#            suppressed by `silent` (which every CLI path sets unless you
#            pass --verbose) and so printed on no run at all.
#            Same trap for scripts: workspace/scripts/ is a separate copy, and
#            the CLI resolves a script by bare name against THAT copy, not the
#            repo -- but scripts has no repo-checkout fallback at all (even in
#            a debug build, a missing workspace/scripts/ is created empty, not
#            seeded from the repo) and no equivalent log line yet.
#            Data dir is ~/.local/share/factorio-bot-dev/
#   release  mods and scripts are include_dir!-embedded into the binary at
#            COMPILE TIME and extracted once into the workspace. Editing
#            mods/ or scripts/ has NO effect until you rebuild -- and no
#            effect at all on an existing workspace, because extraction is
#            skipped when the directory already exists. There is no symlink
#            here and there must not be: a release binary has no checkout to
#            point at. FACTORIO_BOT_REFRESH_MODS=1 refreshes the workspace
#            copy from the embedded snapshot; it is a no-op in a debug build,
#            which has no snapshot.
#            Data dir is ~/.local/share/factorio-bot/
#
# Editing the mod against a release build costs two confusing runs. Ask.
cargo build --no-default-features --features cli,lua            # debug: iterating
cargo build --release --no-default-features --features cli,lua  # release: timing

# FAST PLANNING LOOP: no graphical client, no 90s connect wait.
# --clients is how many Factorio processes to spawn; --bots is how many bots
# to plan for. They used to be one flag, which made `-c 0` plan for zero bots
# and `-c 1` demand a display. Planning only needs bots.
factorio-bot lua goal_smoke.lua --clients 0 --bots 4

# Run multi-client test with adequate timeout (180s recommended)
#
# The script name is resolved against the workspace scripts directory
# (`<workspace_path>/scripts`), NOT against the current working directory, so
# pass a bare name and run from anywhere. An absolute path does NOT work: the
# leading `/` is stripped and the rest is joined onto the scripts root, so
# `/home/me/scripts/foo.lua` is looked up as `<scripts_root>/home/me/scripts/foo.lua`
# and reported as not found.
timeout 180 target/release/factorio-bot lua <script_name>.lua -c <num_clients>

# Example: Test with 2 clients
timeout 180 target/release/factorio-bot lua multi_client_test.lua -c 2
```

### Expected Behavior

1. **Server starts** and outputs "waiting finished" when ready (~12-17s)
2. **Clients spawn** as graphical Factorio windows (not headless)
3. **Clients load** sprites and resources (~26s per client)
4. **Wait loop** polls `rcon_players()` every 1 second. The bound is
   `CONNECT_STALL_TIMEOUT` (`crates/core/src/process/connect_wait.rs`), **300
   seconds of NO PROGRESS** -- it restarts every time another client appears,
   so a trickle of arrivals cannot trip it. Raised from 90s on 2026-09-03
   after two runs on a loaded machine stalled at 0/4 while every client did
   eventually connect (one reached `Factorio initialised` at 123.5s).
5. **A stall does NOT abort the run -- it proceeds with whoever showed up**,
   logging `Gave up waiting for clients ... The run continues without them`.
   That degrades silently to a one-bot roster and a *different plan*. **Check
   `plan_created.bots == [1,2,3,4]` before comparing a run to anything.** A
   one-bot run misread as a four-bot regression cost a good commit a revert.
6. **Script runs** - clients should be connected by this point
7. **Multi-bot coordination** verified via task graph execution

### Measuring a run, and the traps that have produced wrong answers

**Every wrong conclusion drawn on 2026-09-03 came from a measurement, not from
carelessness.** Read this before quoting a number.

**Use `just analyse` (`tools/run_analysis.py`).** It reports milestone spans in
game time, per-verb dispatch->settle ticks, `steps/bot`, `planned ticks/bot`,
per-bot failed walks, frozen-position detection, repeated refused destinations,
sample coverage, per-network power and per-machine status. It exists because
the ad-hoc one-liners that produced those wrong answers were unrepeatable.

- **Game time, not wall clock.** `roster ready -> SATISFIED` on the clock
  includes client load and startup. One run read as 24.4 min on the clock and
  21.4 min of game time.
- **`walk_dispatched` has NO `id` field.** Joining it to `walk_settled` on `id`
  makes Python key everything under `None` and attribute every failure to the
  last dispatch. **`walk_settled` carries `bot` directly -- read it.** This
  produced "all twelve failed walks were bot 1's" when bot 1 had *zero*.
- **A verb histogram cannot see waiting.** Verbs that settle in their dispatch
  tick (`place`, `insert`, `fuel`, `take`) contribute 0, and *idle* time appears
  nowhere. "88% of the run is hand-mining" was 88% of the *timed* verbs, while
  walking was 31.8% of the milestone and 39.1% was one bot waiting on a furnace.
  Ask what a number cannot see before acting on it.
- **Runs are only comparable if the map is.** Two runs 13 hours and ~20 commits
  apart, on different maps (one had a resource 100 tiles east the other lacked)
  produced "four bots do double the work of one". Retracted. `factorio-bot lua`
  takes **`--seed`** -- but read the next section before trusting it, because
  **`--seed` alone does nothing at all on a workspace that already has a map.**
- **Step counts, furnace counts and plan sizes move for reasons that are not
  progress.** `steps/bot` has been the number that mattered all along; it sat at
  `{1: 103, 2: 4, 3: 4, 4: 4}` while two whole classes of defect were fixed and
  the headline time did not move.

### Reproducible runs: the seed, and the trap in it

**`--seed` is ignored, silently, whenever `workspace/server/saves/level.zip`
already exists.** This was measured, not assumed. The seed only ever reaches
Factorio as `--map-gen-seed` on a `--create` invocation
(`crates/core/src/process/instance_setup.rs`), and `--create` runs only inside
`if !saves_level_path.exists()`. The single thing that deletes that file is
`recreate_save`, i.e. the `--new` / `-n` flag, which defaults to false. So:

```bash
# WRONG. Prints nothing, generates nothing, runs on whatever map was there.
factorio-bot lua factory_stage2.lua -c 4 --seed 31337

# RIGHT. --new deletes level.zip, so the seeded map is actually generated.
# DESTRUCTIVE: the previous map, and everything built on it, is gone.
factorio-bot lua factory_stage2.lua -c 4 --seed 31337 --new
```

This is worse than not passing a seed, because the run *looks* controlled. It
now prints a loud `--seed was IGNORED` warning that is deliberately **not**
gated on `silent` -- every CLI path sets `silent`, which is exactly how the
`Using mods directory` line came to print on no run at all.

Two further paths accept `--seed` and can never use it: `lua --connect` (never
starts a server) and `--server <host>` (never sets one up).

**The benchmark seed is `31337`**, chosen by an owner decision on 2026-09-04:
*"lets do the seed search, like i said we don't need a perfect/optimal one,
just a reasonable one where everything is close to the start."*

It was picked by scanning **16 arbitrary seeds** (`20260903 20260904 1 2 3 42
100 777 1234 4242 9001 12345 31337 65535 99999 123456`) with
`factorio-bot score-map`. **All 16 were `Viable`**; `31337` won on both metrics
at once.

| | previously used (unidentified) | `20260903` | **`31337`** |
|---|---|---|---|
| iron ore | 40.4 | 26.3 | **18.4** |
| copper ore | 58.3 | 44.1 | 54.9 |
| coal | 54.0 | 44.5 | **32.1** |
| stone | 32.7 | 35.4 | 33.3 |
| water | 46.7 | 37.0 | 48.1 |
| walk score | 1550 | 1250 | **1246** |
| planned makespan | 30,077 | 31,764 | **29,000 (8:03)** |

Fingerprint `c161fa3f437221d0`. `20260903` — the seed this file previously
named, chosen for being a date rather than a map — came **9th of 16**.

**The argument this file used to make against choosing a seed still stands, and
is now a caveat rather than a policy.** A map with ore near spawn flatters every
timing and makes it less comparable to the ~9-minute manual solo baseline, which
was not run on an optimised map. So: quote the seed with every number, and do
not read a `31337` time as beating a baseline set elsewhere. What changed is
that a *reproducible* map is worth more than an unbiased one — every result
before 2026-09-04 was measured on a map nobody can regenerate, because `--seed`
was silently ignored until `61ec7364`.

This was a scan, not a search: 16 seeds, ranked, first acceptable winner taken.
It is not an optimum and no one should describe it as one.

`just bench <script>` is that run. **It has not yet been executed once**, so the
seed is unvalidated: a map whose nearest shoreline does not fit a pump/boiler/
engine has genuinely refused a run here (`the nearest water is 66.7 tiles
away`). Confirm it before quoting any number against it.

**What a run now records about its map** (`crates/core/src/record/provenance.rs`):
`provenance.json`, written at run *start* rather than at finish. That timing is
the point -- `manifest.json` is written only in `finish()`, and 9 of the 24
archived runs have none, which are precisely the killed runs whose identity
somebody later needs. It carries the seed, the map-exchange string, the game
version, the git commit **and whether the tree was dirty**, the build profile,
the requested roster, and a `map` fingerprint.

That fingerprint (`EntityGraph::resource_fingerprint`) is the only identity
available for a map whose seed nobody wrote down, which is every map used so
far. **Its two answers are not symmetrical: equal digests mean the same map, a
different digest means unknown**, because the resource table holds *charted*
tiles and charting grows as bots explore.

**None of this can be backfilled.** No archived run recorded a seed, and the mod
never read `map_gen_settings`, so the maps behind every timing quoted in the
2026-09-03 notes are permanently unidentifiable.

### Silence is not success

Four separate mechanisms have been found reporting nothing while broken. When
adding any check, ask what a reader sees when it *fails*, and prefer a record
entry over a log line:

- the `Using mods directory` line was gated behind `if !silent`, which every CLI
  path sets, so the authoritative "did my edit ship" answer printed on no run;
- 19 of 20 walk failures were archived as `kind: "other"` because
  `classify_walk_failure` did not know the wording this build emits;
- archived samples stopped at the last closed milestone while the mod sampled
  the whole run -- one run lost 199,449 ticks, the entire window its cell
  existed in, and it was only recovered from the mod's live file;
- a batch's events are written only **after** `goal.run` returns, so while one
  is in flight the record cannot tell "working" from "stalled". This one bit
  hard: a run that was executing normally -- 44,846 ticks of progress past its
  last recorded event, bot inventories climbing across consecutive samples --
  was read as a two-hour hang from the record's silence and **killed**. It had
  been running 13m45s, and an earlier batch of the same run had already gone
  ~18 minutes silent without incident. `EventKind::BatchProgress` (`83c42346`)
  now beats every 30 s with counters and **no verdict**; `just analyse`
  distinguishes executed / never-dispatched / killed-mid-batch / **UNKNOWN**,
  and says explicitly when absence "is NOT evidence of a stall".
  **9 of 24 archived runs end on a `plan_created` with nothing after it**; only
  the newest can ever be cross-checked, because the mod's live sample file is
  overwritten per run.

### Working alongside other agents

- **`git commit -- <path>` takes the WHOLE file.** If another agent has
  uncommitted work in a file you need, committing yours ships theirs under your
  message. Land one, then the other -- or hand over a patch. Two agents have
  had to stop for this.
- **Develop in a throwaway worktree** (`git worktree add /tmp/x HEAD`) when a
  live run holds `target/debug/factorio-bot`.

  **Be precise about why, because the obvious statement of it is wrong.**
  Linux does not lock a running binary the way Windows does -- you can
  `unlink` it or `rename` over it while it executes, and that succeeds. What
  fails is an **in-place write** to a running ELF image: `ETXTBSY`, reported
  as `Text file busy`. Verified here rather than assumed:

  ```
  cp $(command -v python3) ./t && chmod 755 ./t
  ./t -c 'import time; time.sleep(20)' &
  cp $(command -v python3) ./t   # cp: cannot create regular file './t': Text file busy
  mv other ./t                   # succeeds
  ```

  So a build that links straight to the output path trips over a live run,
  and one that writes a temp and renames does not. Copying the binary aside
  and running the copy sidesteps it either way -- `tools/seed_search.sh`
  honours a `BIN` override for exactly that.
- **A worktree run redirects `workspace/mods/BotBridge` at that worktree's
  copy.** A run launched from `/tmp/x` loaded `/tmp/x/mods/BotBridge` and
  produced no machine samples even though the feature had landed in the main
  checkout. Restore the symlink afterwards and `readlink` it before trusting a
  run's mod-dependent data.

### Known Issues & Workarounds

- **Timeout warnings**: Clients may take 90+ seconds to connect on first run. The warning is informational - the script will still run successfully once clients connect.
- **macOS GUI processes**: Clients must use `Stdio::null()` for stdin/stdout/stderr, otherwise GUI windows fail to render.
- **Lock file conflicts**: Server and clients each need separate `--config` paths pointing to instance-specific `config.ini` files.
- **JSON parsing**: BotBridge's `helpers.table_to_json({})` returns `"{}"` for empty tables, not `"[]"`. The Rust RCON client handles both cases.
- **A graphical client works from the dev shell; you only need `DISPLAY`.**
  `flake.nix` now supplies `SDL_VIDEODRIVER=x11` and the X11/GL libraries, so
  `DISPLAY=:0 workspace/client1/bin/x64/factorio` reaches
  `Initialised OpenGL … Factorio initialised`.

  **If you are debugging this anyway, the error message lies about the cause.**
  `No available video device` says nothing about what is missing, and an
  earlier version of this note diagnosed it as `/run/opengl-driver/lib` being
  masked. That was incomplete. The real chain, each step visible only after
  fixing the one before it:

  1. SDL picks the **wayland** backend on a wayland session (Hyprland here) and
     fails. `SDL_VIDEODRIVER=x11` uses Xwayland and surfaces the next error
     instead of hiding it.
  2. Then `x11 not available` — no X11 **client** libraries in the shell
     (`libXrandr.so.2` first).
  3. Then `Failed loading libGL.so.1`, which is **not** in
     `/run/opengl-driver/lib`: that path holds the Mesa *driver*; `libglvnd`
     holds the GL dispatch library. Both are needed, driver path first, since
     Mesa must match the running kernel.

  Audio still fails (`libasound.so.2`) unless `alsa-lib` is present; that is a
  warning, not a failure, and `flake.nix` includes it anyway to keep the log
  readable.
- **Do not debug the mod with `rcon.print`**: its output lands in the RCON
  reply body, and the executor reads that reply as the action's result — so a
  debug line turns a successful action into a reported failure. Use
  `writeout(...)` (stdout, parsed by `output_parser.rs`) instead.
- **An inserter's `direction` points at the side it PICKS UP from**, not the
  side it drops into. Established empirically (chest / burner-inserter / chest,
  then machine / inserter / chest): `direction = 12` ("west") is what moves
  items *west to east*. Getting this backwards produces a layout that places
  100% correctly, passes every geometry check, and does absolutely nothing --
  the failure is silent because placement and function are separate concerns.
  For a row fed from a belt to its north: input and output inserters are both
  `direction = 0`, feeder and takeoff inserters are `direction = 12`.
- **`only_ghosts = true` validates nothing.** Ghosts do not collide, so a
  blueprint whose entities overlap places exactly as many ghosts as a correct
  one. A ghost-placement count is not evidence that geometry is legal; only a
  real build (`only_ghosts = false`, with the materials present) is.
- **Power *coverage* is not power *capacity*.** A layout can have every
  consumer inside a pole's supply area, be fully connected, and still do
  nothing at all, because generation is short. This does not degrade
  gracefully into "slow" -- an under-supplied network can read as completely
  dead. Solar is the worst offender for tests: output depends on the in-game
  time of day, so the same blueprint runs or does not run depending on when
  the run starts. Check generation against demand (assembling-machine-2 is
  150kW, inserters ~13kW each), and prefer a deterministic source when what
  you are testing is geometry rather than power.
- **`Option::None` reaches Lua as mlua's null sentinel, which is light
  userdata and therefore TRUTHY.** So `local inv = r.output_inventory or {}`
  does *not* substitute the default, and the next `pairs(inv)` raises
  "table expected, got light userdata". Guard with `type(x) == "table"`.
- **Resource positions are tile centres.** Every real resource entity sits at
  `(-40.5, -48.5)`, never `(-41, -49)`, and the mod's
  `surface.find_entity(name, position)` matches exactly. `EntityGraph` keys
  resources by `Pos(i32, i32)`, which floors, so anything reading a position
  back out of that map must restore the half-tile offset. Getting this wrong
  made mining fail with "no entity to mine" for every ore on every map, while
  every test passed — `test_utils::spawn_ore` builds ore at integer positions,
  the one input for which the lossy round-trip is lossless.

### Critical Bug Fix (Jan 2026): config.ini Creation

**Problem**: Client instances would fail to launch with "Error: Specified config file doesn't exist" even though setup completed successfully.

**Root Cause**:
- Factorio archive extraction creates an empty `config/` directory
- Original code in `instance_setup.rs:298` checked `if !config_path.exists()`
- When the directory existed (but was empty), it skipped creating `config.ini`
- Clients would then crash immediately after spawning

**Fix**: Changed condition to `if !config_ini_path.exists()` (instance_setup.rs:299)
- Now checks for the actual file, not just the directory
- Creates `config.ini` even if `config/` directory already exists
- Properly handles the case where archive extraction creates empty config directory

**File**: `crates/core/src/process/instance_setup.rs:297-309`

**Verification**: On macOS, successful multi-client launch shows N Factorio icons in the Dock (one per client + server if graphical).

**Debugging Multi-Client Issues**:
1. Add diagnostic logging to process spawn loop (see process_control.rs:161-166 for example)
2. Check process IDs - both clients should spawn successfully
3. Manually test client launch: `workspace/client2/MacOS/factorio --mp-connect localhost ...`
4. Check client logs for error messages (stored in workspace/clientN-log.txt if write_logs enabled)
5. Verify instance directory structure:
   - `workspace/clientN/config/config.ini` must exist (264+ bytes)
   - `workspace/clientN/MacOS/factorio` must exist (executable)
   - `workspace/clientN/mods` should be symlink to workspace/mods (shared)

### Multi-Client Test Script

See `scripts/multi_client_test.lua` for reference implementation showing:
- Individual bot position queries
- RCON commands to specific bots
- Task graph generation for multi-bot coordination
- Parallel task execution across multiple bots
