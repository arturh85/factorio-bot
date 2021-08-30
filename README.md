# Factorio Bot

- [User Guide](https://arturh85.github.io/factorio-bot-tauri/book/)
- [API Docs](https://arturh85.github.io/factorio-bot-tauri/doc/factorio_bot/)
- [Frontend Build Statistics](https://arturh85.github.io/factorio-bot-tauri/stats.html)

# What is it?

Factorio Bot is a bot platform for the game
[Factorio](https://www.factorio.com) inspired by [factorio-bot](https://github.com/Windfisch/factorio-bot/)

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
- [x] resizes Factorio client windows so the whole screen is equally used
- [x] integrated lua editor which allows scripting of bots 
- [x] uses included factorio mod to read factorio instance ...
  - [x] recipes
  - [x] entity prototypes
  - [x] item prototypes
  - [x] placed entities
- Build Graphs of:
  - [ ] Entity Connections with distance based weights (currently broken)
  - [ ] Flow Connections with flow rate per second for each belt side/resource (currently broken)
  - [ ] Bot Task Dependencies with time estimate based weights (currently broken)
- [ ] (optional) REST API Endpoints with OpenAPI specs
- [ ] read map contents by chunk for leaflet based Map View (currently broken)
- [x] use whatever mods you want, configured in central location for all instances
- [x] should work on Win/Mac/Linux (not tested on Mac)
- [x] MIT licenced

# Installation

- Download the [latest release](https://github.com/arturh85/factorio-bot-tauri/releases) for your Operating System
  - on windows you can use the [chocolatey](https://chocolatey.org/) package manager: `choco install factorio-bot --version=0.2.0`
- Download [Factorio](https://www.factorio.com) as .zip or .tar.xz (don't use the headless version!)
- Start the application and select your downloaded factorio archive under `Settings`
- Use the `Start` Button in the top right to start the given number of factorio instances with given seed/map exchange string
- Select the Script you want to run and execute it

# Technologies used

## Frontend: Vite.js + PrimeVue
- [Vite.js](https://vitejs.dev/) is a new modern bundler for javascript which is blazing fast and includes many sensible defaults.
- [Vue.js](https://vuejs.org/) is an incremental frontend framework which is an absolute joy to work with. It has seen very impressive improvements in version 3 including Composition Api, script setup, dynamic css binding and ... .
- [PrimeVue](https://www.primefaces.org/primevue/) is the a component library for Vue 3. Lots of premade components will make your job as application developer easier and more fun.

## Backend: Rust + Tauri
- [Tauri](https://tauri.studio/) is a new modern technology to turn your web apps into a desktop app for multiple platforms (Windows, MacOS, Linux, android and ios soon). Tauri apps have very small file size and tiny memory consumption.

## Setup Development Environment 
- Ready your workspace according to [Tauri Getting Started](https://tauri.studio/en/docs/getting-started/intro/)
- Clone repository `git clone git@github.com:arturh85/factorio-bot-tauri.git`
- Change to frontend/
- `cd frontend/`
- `yarn` or `npm i`

## Development Usage

- `npm start` starts the application while watching for changes 

## Contribute

Send me your Pull Requests :)

## Contact

Email: [arturh@arturh.de](mailto:arturh@arturh.de)
