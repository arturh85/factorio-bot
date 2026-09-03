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
#   debug    mods resolve to the repo checkout ONLY IF workspace/mods does
#            not already exist. Once a workspace exists, that copy wins and
#            there is NO refresh path -- editing mods/BotBridge has no effect
#            and the run silently uses the stale copy. Edit workspace/mods/
#            directly when iterating, or delete it to re-seed.
#            Every run -- either build, no flag needed -- logs one line
#            naming which directory actually won: "Using mods directory
#            <absolute path> (<why>)". That line, not a guess from a
#            traceback, is the authoritative answer to "did my edit ship".
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
#            skipped when the directory already exists.
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
4. **Wait loop** polls `rcon_players()` every 1 second for up to 90 seconds
5. **Timeout warning** may appear if clients take >90s to fully connect (expected, not a failure)
6. **Script runs** - clients should be connected by this point
7. **Multi-bot coordination** verified via task graph execution

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
