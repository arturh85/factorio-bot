# Factorio Bot

[![User Guide](https://img.shields.io/badge/user-guide-green)](https://arturh85.github.io/factorio-bot/userguide/)
[![LUA API Docs](https://img.shields.io/badge/lua-apidocs-blue)](https://arturh85.github.io/factorio-bot/lua/)
[![MIT License](https://img.shields.io/github/license/arturh85/factorio-bot)](https://github.com/arturh85/factorio-bot/blob/master/LICENSE.txt)
[![GitHub issues](https://img.shields.io/github/issues/arturh85/factorio-bot)](https://github.com/arturh85/factorio-bot/issues)
[![Dev Guide](https://img.shields.io/badge/dev-guide-red)](https://arturh85.github.io/factorio-bot/devguide/)

## What is it?

Factorio Bot is a platform for [Factorio](https://www.factorio.com) that enables **multi-bot coordination** for automated gameplay. Scripts declare goals; a planner decomposes them into an action network and schedules it across the bots; an executor runs it over RCON.

**Vision:** Say `goal("launch_rocket")` and watch multiple bots coordinate to complete the game automatically. Today `goal.have` and `goal.researched` get you as far as items and technologies.

Requires **Factorio 2.1** — see [Howto: Setup](https://arturh85.github.io/factorio-bot/userguide/howto_setup.html).

## Goals / Use Cases

- **TAS (Tool Assisted Speedrun)** - Beat world records with many bots sharing the workload efficiently
- **Goal-Driven Automation** - Give high-level goals ("research automation") → system decomposes into tasks → bots execute cooperatively
- **ML/AI Research** - Learning environment for training algorithms within Factorio (future)
- **Playground** - Experiment with Factorio automation

## How It Compares

| Project                                                                                       | Approach            | Multi-bot      | Factorio 2.x |
|-----------------------------------------------------------------------------------------------|---------------------|----------------|--------------|
| [Factorio Learning Environment](https://github.com/JackHopkins/factorio-learning-environment) | LLM agents, Python  | No             | Partial      |
| [Factorio-AnyPct-TAS](https://github.com/gotyoke/Factorio-AnyPct-TAS)                         | Pre-scripted TAS    | No             | No (0.18)    |
| **Factorio Bot**                                                                              | Goal planning + Lua | **Yes**        | **2.1**      |

## Youtube Videos

- Reference: [Any% World Record gets automation in 7:33](https://www.youtube.com/watch?v=rHvaZMdjnLE&t=455)
- [Factorio Bot 0.1.2: Research logistics with 4 Bots in 15:51](https://youtu.be/iFhcyjfcjx8)
- [Factorio Bot 0.1.1: Research automation with 1 Bot in 8:57](https://youtu.be/1vbWWiSV6Sw)
- [Factorio Bot 0.1.0: Research automation with 1 Bot in 12:33](https://youtu.be/6KXYuVDRZ-I)

## Features

- [x] Requires Factorio **2.1** — the bundled BotBridge mod declares `"factorio_version": "2.1"`, and a mismatch is reported before the game is launched
- [x] Reads the Factorio 2.0+ inventory format (items carry a `quality` field; quality is not yet modelled when planning)
- [x] Sets up & starts a Factorio server + a configurable number of clients (`--clients`)
- [x] Declarative goals in Lua: `goal.have("iron-plate", 5)`, `goal.researched("automation")`
- [x] Deterministic planner (`crates/planner`): goal → action network → travel-aware multi-bot schedule
- [x] Executor (`crates/executor`): runs a schedule across bots over RCON, with per-action completion signals and lag edges for machine time
- [x] Plan visualization: Graphviz (`plan:graphviz()`) and Mermaid Gantt (`plan:gantt(title)`)
- [x] Integrated [Monaco](https://microsoft.github.io/monaco-editor/) Lua editor for scripting
- [x] Uses BotBridge mod to read game state:
  - [x] Recipes, entity prototypes, item prototypes
  - [x] Entities, resources, player inventories
- [x] Build graphs of:
  - [x] Entity connections with distance-based weights
  - [x] Flow connections with flow rate per belt side/resource
- [x] Planning-only mode for fast iteration: `--clients 0 --bots N` (no graphical client, no connect wait)
- [x] REPL mode for fast iteration (`cargo repl`)
- [x] Seed rolling (`factorio-bot roll-seed --map ...`)
- [x] (Optional) HTTP API with an OpenAPI spec at `/openapi.json` and Swagger UI at `/swagger-ui`
- [x] Builds and is CI-tested on Windows/macOS/Linux
- [x] MIT licensed

## Planned Features

See [PLAN.md](PLAN.md) for the full roadmap.

- [ ] Full game completion: `goal("launch_rocket")` with many cooperating bots
- [ ] Recovery wired into a live run — the three-tier logic exists in `crates/executor/src/recover.rs` but nothing calls it yet
- [ ] `Goal::Producing { item, rate }` — declared, but no method expands it yet
- [ ] Observed (rather than scheduled) execution timings
- [ ] LLM integration (future)

## Quickstart

### CLI/REPL Mode (Recommended)

```bash
# Write a settings file, pointing it at your Factorio 2.1 archive
# (.zip or .tar.xz -- not the headless build)
factorio-bot --factorio-archive /path/to/factorio_2.1.x.tar.xz config init

# Check what the program will actually use
factorio-bot config show

# Build and run REPL
cargo repl

# Or start full Factorio server + client
just factorio

# Run a Lua script from scripts/
just lua <script>.lua
```

### Desktop App

- Download the [latest release](https://github.com/arturh85/factorio-bot/releases) for your OS
- Download [Factorio](https://www.factorio.com) as .zip or .tar.xz (not headless!)
- Start the app and select your Factorio archive under `Settings`
- Use `Start` to launch Factorio instances
- Select and execute Lua scripts

## Development

```bash
# Run tests + clippy + build
just test

# Start Factorio for testing
just factorio

# Frontend development
cd app && npm start
```

See the [Dev Guide](https://arturh85.github.io/factorio-bot/devguide/) for more details.

## Contribute

Send Pull Requests! See [PLAN.md](PLAN.md) for what needs work.

## Contact

Email: [arturh@arturh.de](mailto:arturh@arturh.de)
