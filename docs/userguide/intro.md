# Factorio Bot User Guide

[![LUA API Docs](https://img.shields.io/badge/lua-apidocs-blue)](https://arturh85.github.io/factorio-bot/lua/)
[![MIT License](https://img.shields.io/github/license/arturh85/factorio-bot)](https://github.com/arturh85/factorio-bot/blob/master/LICENSE.txt)
[![GitHub issues](https://img.shields.io/github/issues/arturh85/factorio-bot)](https://github.com/arturh85/factorio-bot/issues)
[![Dev Guide](https://img.shields.io/badge/dev-guide-red)](https://arturh85.github.io/factorio-bot/devguide/)
[![Github Repo](https://img.shields.io/badge/repo-github-blueviolet)](https://github.com/arturh85/factorio-bot)

## Intro: What is it?

Factorio Bot is a bot platform for the game
[Factorio](https://www.factorio.com) inspired by [factorio-bot](https://github.com/Windfisch/factorio-bot/)

Goals / Use Cases:
- TAS (Tool Assisted Speedrun) to beat the world record with many bots which share the workload efficiently
- Learning Environment to train Machine Learning algorithms within Factorio
- Playground for Factorio Experiments

It needs **Factorio 2.1**: the bundled BotBridge mod declares
`"factorio_version": "2.1"`, and Factorio refuses a mod whose major.minor does
not match the installed game. Without that mod there is no RCON bridge and
nothing else works. See [Howto: Setup Factorio Bot](./howto_setup.md).

Scripts declare goals rather than spelling out every action —
`goal.have("iron-plate", 5)`, `goal.researched("automation")` — and a planner
decomposes them into actions that an executor runs across the bots. See
[Howto: Lua Scripting](./howto_lua_scripting.md).

