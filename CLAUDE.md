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
