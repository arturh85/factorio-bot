# Factorio Bot

[![User Guide](https://img.shields.io/badge/user-guide-green)](https://arturh85.github.io/factorio-bot-tauri/userguide/)
[![LUA API Docs](https://img.shields.io/badge/lua-apidocs-blue)](https://arturh85.github.io/factorio-bot-tauri/lua/)
[![MIT License](https://img.shields.io/github/license/arturh85/factorio-bot-tauri)](https://github.com/arturh85/factorio-bot-tauri/blob/master/LICENSE.txt)
[![GitHub issues](https://img.shields.io/github/issues/arturh85/factorio-bot-tauri)](https://github.com/arturh85/factorio-bot-tauri/issues)
[![Dev Guide](https://img.shields.io/badge/dev-guide-red)](https://arturh85.github.io/factorio-bot-tauri/devguide/)

# What is it?

Factorio Bot is a platform for the game
[Factorio](https://www.factorio.com) inspired by [factorio-bot](https://github.com/Windfisch/factorio-bot/). It allows scripted execution of a factorio server and multiple clients. 

Goals / Use Cases:
- TAS (Tool Assisted Speedrun) to beat the world record with many bots which share the workload efficiently
- Learning Environment to train Machine Learning algorithms within Factorio
- Playground for Factorio Experiments

## Youtube Videos

- Reference: [Any% World Record gets automation in 7:33](https://www.youtube.com/watch?v=rHvaZMdjnLE&t=455)

- [Factorio Bot 0.1.2: Research logistics with 4 Bots in 15:51](https://youtu.be/iFhcyjfcjx8)
- [Factorio Bot 0.1.1: Research automation with 1 Bot in 8:57](https://youtu.be/1vbWWiSV6Sw)
- [Factorio Bot 0.1.0: Research automation with 1 Bot in 12:33](https://youtu.be/6KXYuVDRZ-I)

## Features
- [x] sets up & starts factorio server & configurable number of clients
- [x] resizes Factorio client windows so the whole screen is equally used (windows only)
- [x] integrated [monaco](https://microsoft.github.io/monaco-editor/) lua editor which allows scripting of bots 
- [x] uses included factorio mod to read factorio instance ...
  - [x] recipes
  - [x] entity prototypes
  - [x] item prototypes
  - [x] entities
  - [x] resources
- Build Graphs of:
  - [ ] Entity Connections with distance based weights (currently broken)
  - [ ] Flow Connections with flow rate per second for each belt side/resource (currently broken)
  - [ ] Bot Task Dependencies with time estimate based weights (currently broken)
- Example LUA Script which can:
  - [ ] Plan some task with time estimation
  - [ ] Find Seed which minimizes execution time
  - [ ] Research Automation in MM:SS
- [ ] (optional) REST API Endpoints with OpenAPI specs  (currently broken)
- [ ] read map contents by chunk for leaflet based Map View (currently broken)
- [x] use whatever mods you want, configured in central location for all factorio instances
- [x] should work on Win/Mac/Linux (not tested on Mac)
- [x] MIT licenced

## Planned Features
- [ ] Create ZIP from Steam Verson of Factorio
- [ ] Indicate Client Actions as Drawn Lines / Rectangle in the Factorio Client

# Quickstart

- Download the [latest release](https://github.com/arturh85/factorio-bot-tauri/releases) for your Operating System
- Download [Factorio](https://www.factorio.com) as .zip or .tar.xz (don't use the headless version!)
- Start the application and select your downloaded factorio archive under `Settings`
- Use the `Start` Button in the top right to start the given number of factorio instances with given seed/map exchange string
- Select the Script you want to run and execute it

## Contribute

See the [Dev Guide](https://arturh85.github.io/factorio-bot-tauri/devguide/) and Send me your Pull Requests :)

## Contact

Email: [arturh@arturh.de](mailto:arturh@arturh.de)
