# Architecture

Factorio Bot orchestrates a Factorio server and one real Factorio client per
bot, all driven by Lua scripts you write. This page covers the pieces from a
user's point of view. For how they are implemented internally, see the [dev
guide's architecture
page](https://arturh85.github.io/factorio-bot/devguide/architecture.html) —
this page does not repeat that.

## The pieces

- **The Factorio server.** factorio-bot starts and owns a headless Factorio
  server process itself; it does not connect to a server you started
  separately.
- **The BotBridge mod.** Bundled with factorio-bot and loaded into every
  instance, server and clients alike. It is what makes the server
  controllable at all: it exposes an RPC-style surface over RCON to read the
  world (recipes, entities, inventories, research) and to act in it (walk,
  mine, craft, place, insert, remove, research).
- **Bots are real Factorio clients**, not a simulation. Each bot is an
  ordinary graphical Factorio client process that joins the server the same
  way a human player would.
- **Lua scripts** are what you write, in a sandboxed Lua 5.4 interpreter. A
  script either declares goals for the planner or issues RCON commands
  directly — see [Howto: Lua Scripting](./howto_lua_scripting.md) for the
  full API and the sandbox rules.

## How world state reaches your script

The BotBridge mod reports world state by writing structured lines to the
Factorio **server's own stdout**. factorio-bot reads that stream as it runs
the server process, which is why factorio-bot has to be the one that launches
the server — it cannot attach to one already running. That stream is what
keeps the world model your script queries up to date.

## `goal.*` vs `rcon.*`

A script has two ways to make bots act:

- **`rcon.*`** issues one direct, low-level command per call — move, mine,
  craft, place an entity, insert or remove inventory items, or cheat an item
  or technology in for testing. You decide the sequence and which bot does
  each step.
- **`goal.*`** is declarative: state an end condition — `goal.have("iron-plate", 5)`, `goal.researched("automation")` — and a planner works
  out the mining, smelting, crafting and walking actions needed, schedules
  them across your bots, and an executor runs them. See [Howto: Lua
  Scripting](./howto_lua_scripting.md) for the full `goal.*` surface.
