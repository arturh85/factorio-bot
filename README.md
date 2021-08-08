# Factorio Bot

- [Book](https://arturh85.github.io/factorio-bot-tauri/book/)
- [API Docs](https://arturh85.github.io/factorio-bot-tauri/doc/)
- [Frontend Build Statistics](https://arturh85.github.io/factorio-bot-tauri/stats.html)

# What is it?

Factorio Bot is a bot platform for the game
[Factorio](https://www.factorio.com) inspired by [factorio-bot](https://github.com/Windfisch/factorio-bot/)

Goals / Use Cases:
- TAS (Tool Assisted Speedrun) to beat the world record with enough bots to share the workload efficiently
- Learning Environment to train Machine Learning algorithms within Factorio
- Playground for Factorio Experiments

## Youtube Videos

- Reference: [Any% World Record gets automation in 7:33](https://www.youtube.com/watch?v=rHvaZMdjnLE&t=455)

- [Factorio Bot 0.1.2: Research logistics with 4 Bots in 15:51](https://youtu.be/iFhcyjfcjx8)
- [Factorio Bot 0.1.1: Research automation with 1 Bot in 8:57](https://youtu.be/1vbWWiSV6Sw)
- [Factorio Bot 0.1.0: Research automation with 1 Bot in 12:33](https://youtu.be/6KXYuVDRZ-I)

## Features
- [x] extract factorio .zip/tar.xz and symlink bridge mod
- [x] start Factorio server and/or one or multiple clients (unrestricted) 
- [x] read factorio recipes/entity prototypes/item prototypes/graphics
- [ ] read map contents by chunk for leaflet based Map View
- Build Graphs of:
  - [ ] Entity Connections with distance based weights
  - [ ] Flow Connections with flow rate per second for each belt side/resource
  - [ ] Bot Task Dependencies with time estimate based weights 
- [ ] Use whatever mods you want
- [x] should work on Win/Mac/Linux, not tested on Mac
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
