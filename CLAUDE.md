# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Factorio Bot is a Tauri+Rust desktop application that orchestrates Factorio game servers and multiple bots via Lua scripting. Use cases include tool-assisted speedruns (TAS), ML training environments, and Factorio experiments.

## Build & Development Commands

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

# Development (starts Tauri + Vite dev servers)
cd app && pnpm start

# REPL mode (faster build, no GUI, for testing scripting)
cargo repl

# Run frontend tests
cd app && pnpm test

# or with nextest
cargo nextest run

# Lint everything
cd app && pnpm lint              # TypeScript + ESLint + Vue type checking
cargo clippy --workspace --all-features --all-targets -- --deny warnings

# Pre-commit check (runs all CI checks locally)
cd app && pnpm run precommit:check

# Production build
cd app && pnpm run tauri:build

# Build with/without default features
cargo build --all-features
cargo build --no-default-features
```

*LSP tools**: Prefer `mcp__rust__lsp_*` tools for refactoring (rename_symbol, find_references, get_definitions)
These leverage rust-analyzer for accuracy with macros and trait implementations

## Architecture

```
Desktop App (Vue 3 + PrimeVue)
    ↓ (IPC + HTTP)
Tauri Commands (app/src-tauri/)
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
  - `lua_docs.rs` - generates `docs/lua/src/{globals,world,rcon,goal}.lua` from
    the `__doc_entry_*` strings in `globals/`. Those four files are build
    artifacts (gitignored) written by `app/src-tauri/build.rs`; edit the Rust
    strings, never the `.lua`. `docs/lua/src/types.lua` is the exception — it
    is hand-written and tracked, mirroring `crates/core/src/types.rs`.

- **crates/server**: axum HTTP server (replaces the former Rocket `crates/restapi`)
  - `webserver.rs` - router assembly, static SPA serving, `start()` entry point
  - `game/` - the `/api/v1/game/*` query and control handlers
  - OpenAPI generated with utoipa; Swagger UI at `/swagger-ui`, spec at `/openapi.json`
  - Serves the built Vue SPA from the configured `web_root` (index.html fallback); with no web root it serves the API only and redirects `/` to the docs

### Frontend (app/src/)

- Vue 3 + PrimeVue 4 components, themed with the `@primeuix/themes` Lara preset
  configured in `main.ts` (PrimeVue 4 dropped the shipped theme stylesheets for a
  runtime theme service, so `app.use(PrimeVue, ...)` is mandatory)
- Monaco editor for Lua scripting (`components/Editor.vue`), whose workers are
  wired through Vite's native `?worker` imports
- Pinia 4 for state management (`defineStore('id', {...})`, not the removed
  object-with-id form) and Vue Router 5 with hash history
- Vite config lives in `vite.config.mts` — `.mts` because the package is not
  `"type": "module"` and some plugins are ESM-only
- ESLint uses flat config in `app/eslint.config.mjs`; `.eslintrc.js` no longer
  works on ESLint 10. `pnpm lint` runs `tsc`, `vue-tsc` and `eslint` over both
  `.ts` and `.vue` files

Known gap: the templates still carry PrimeFlex 1/2 grid class names (`p-grid`,
`p-col-12`, `p-formgrid`, `p-field-radiobutton`). PrimeFlex 3 removed the `p-`
prefix and PrimeFlex itself is archived upstream, so those classes resolve to no
rules and the affected pages render as stacked blocks rather than grids.
Restoring the intended layout is open work.

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
#            Same trap for scripts: workspace/scripts/ is a separate copy, and
#            the CLI resolves a script by bare name against THAT copy, not the
#            repo. Check the path in any traceback to see which one ran.
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
- **Do not debug the mod with `rcon.print`**: its output lands in the RCON
  reply body, and the executor reads that reply as the action's result — so a
  debug line turns a successful action into a reported failure. Use
  `writeout(...)` (stdout, parsed by `output_parser.rs`) instead.
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
