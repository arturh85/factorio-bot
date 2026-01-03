# Factorio Bot

[![User Guide](https://img.shields.io/badge/user-guide-green)](https://arturh85.github.io/factorio-bot/userguide/)
[![LUA API Docs](https://img.shields.io/badge/lua-apidocs-blue)](https://arturh85.github.io/factorio-bot/lua/)
[![MIT License](https://img.shields.io/github/license/arturh85/factorio-bot)](https://github.com/arturh85/factorio-bot/blob/master/LICENSE.txt)
[![GitHub issues](https://img.shields.io/github/issues/arturh85/factorio-bot)](https://github.com/arturh85/factorio-bot/issues)
[![Dev Guide](https://img.shields.io/badge/dev-guide-red)](https://arturh85.github.io/factorio-bot/devguide/)

## What is it?

Factorio Bot is a platform for [Factorio](https://www.factorio.com) that enables **multi-bot coordination** for automated gameplay. Unlike other TAS tools or AI environments, Factorio Bot can orchestrate 8-16 bots working cooperatively on tasks.

**Vision:** Say `goal("launch_rocket")` and watch multiple bots coordinate to complete the game automatically.

## Goals / Use Cases

- **TAS (Tool Assisted Speedrun)** - Beat world records with many bots sharing the workload efficiently
- **Goal-Driven Automation** - Give high-level goals ("research automation") → system decomposes into tasks → bots execute cooperatively
- **ML/AI Research** - Learning environment for training algorithms within Factorio (future)
- **Playground** - Experiment with Factorio automation

## How It Compares

| Project                                                                                       | Approach            | Multi-bot      | Factorio 2.0 |
|-----------------------------------------------------------------------------------------------|---------------------|----------------|--------------|
| [Factorio Learning Environment](https://github.com/JackHopkins/factorio-learning-environment) | LLM agents, Python  | No             | Partial      |
| [Factorio-AnyPct-TAS](https://github.com/gotyoke/Factorio-AnyPct-TAS)                         | Pre-scripted TAS    | No             | No (0.18)    |
| **Factorio Bot**                                                                              | Scripted + Planning | **Yes (8-16)** | **Yes**      |

## Youtube Videos

- Reference: [Any% World Record gets automation in 7:33](https://www.youtube.com/watch?v=rHvaZMdjnLE&t=455)
- [Factorio Bot 0.1.2: Research logistics with 4 Bots in 15:51](https://youtu.be/iFhcyjfcjx8)
- [Factorio Bot 0.1.1: Research automation with 1 Bot in 8:57](https://youtu.be/1vbWWiSV6Sw)
- [Factorio Bot 0.1.0: Research automation with 1 Bot in 12:33](https://youtu.be/6KXYuVDRZ-I)

## Features

- [x] Factorio 2.0 compatible (including quality system)
- [x] Sets up & starts Factorio server + configurable number of clients
- [x] Multi-bot task coordination with dependency tracking
- [x] Integrated [Monaco](https://microsoft.github.io/monaco-editor/) Lua editor for scripting
- [x] Task graph visualization (Graphviz, Mermaid Gantt charts)
- [x] Uses BotBridge mod to read game state:
  - [x] Recipes, entity prototypes, item prototypes
  - [x] Entities, resources, player inventories
- [x] Build graphs of:
  - [x] Entity connections with distance-based weights
  - [x] Flow connections with flow rate per belt side/resource
  - [x] Bot task dependencies with time estimates
- [x] REPL mode for fast iteration (`cargo repl`)
- [x] (Optional) REST API with OpenAPI specs
- [x] Works on Windows/Mac/Linux
- [x] MIT licensed

## Planned Features

See [PLAN.md](PLAN.md) for the full roadmap.

- [ ] Goal decomposition: `goal("research_automation")` → automatic task generation
- [ ] Full game completion: `goal("launch_rocket")` with 8-16 cooperating bots
- [ ] Seed rolling for optimal map selection
- [ ] LLM integration (future)

## Quickstart

### CLI/REPL Mode (Recommended)

```bash
# Build and run REPL
cargo repl

# Or start full Factorio server + client
just factorio
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
