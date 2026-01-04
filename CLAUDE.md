# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Factorio Bot is a Tauri+Rust desktop application that orchestrates Factorio game servers and multiple bots via Lua scripting. Use cases include tool-assisted speedruns (TAS), ML training environments, and Factorio experiments.

## Build & Development Commands

```bash
# Master Check Tool, runs Rust clippy, tests, build, takes a few minutes, only run in the end to finalise a change
just test

# Start Factorio server with BotBridge mod
just factorio

# Development (starts Tauri + Vite dev servers)
cd app && npm start

# REPL mode (faster build, no GUI, for testing scripting)
cargo repl

# Run frontend tests
cd app && yarn test

# or with nextest
cargo nextest run

# Lint everything
cd app && yarn lint              # TypeScript + ESLint + Vue type checking
cargo clippy --workspace --all-features --all-targets -- --deny warnings

# Pre-commit check (runs all CI checks locally)
cd app && npm run precommit:check

# Production build
cd app && yarn tauri:build

# Build with/without default features
cargo build --all-features
cargo build --no-default-features
```

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
│  • crates/restapi      (optional HTTP)  │
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
  - `graph/task_graph.rs` - Bot task DAG with time estimates
  - `process/` - Factorio process spawning/control
  - `plan/` - Goal decomposition and task execution

- **crates/scripting_lua**: Lua 5.4 bindings exposing host functions for task queuing, graph queries, and RCON commands

- **crates/restapi**: Rocket-based HTTP API with OpenAPI/Swagger docs

### Frontend (app/src/)

- Vue 3 + PrimeVue components
- Monaco editor for Lua scripting (`components/Editor.vue`)
- Mermaid for graph visualization
- Pinia for state management

### Communication Flow

1. User scripts written in Lua via Monaco editor
2. Scripts schedule goals → planners expand to task graph nodes
3. Executor assigns tasks to bots based on availability/travel time
4. RCON commands sent to Factorio via BotBridge mod
5. Entity/event data streams back to update graphs

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
# RECOMMENDED: Pre-compile first to separate compile time from test runtime
cargo build --release --no-default-features --features cli,lua

# Run multi-client test with adequate timeout (180s recommended)
# Use absolute paths when running from different directories
timeout 180 target/release/factorio-bot lua /absolute/path/to/script.lua -c <num_clients>

# Example: Test with 2 clients
timeout 180 target/release/factorio-bot lua \
  /Volumes/2TB/projects/private/factorio-bot/scripts/multi_client_test.lua -c 2
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
