# Factorio Bot Developer Guide


[![User Guide](https://img.shields.io/badge/user-guide-green)](https://arturh85.github.io/factorio-bot-tauri/userguide/)
[![LUA API Docs](https://img.shields.io/badge/lua-apidocs-blue)](https://arturh85.github.io/factorio-bot-tauri/lua/)
[![MIT License](https://img.shields.io/github/license/arturh85/factorio-bot-tauri)](https://github.com/arturh85/factorio-bot-tauri/blob/master/LICENSE.txt)
[![GitHub issues](https://img.shields.io/github/issues/arturh85/factorio-bot-tauri)](https://github.com/arturh85/factorio-bot-tauri/issues)
[![codecov](https://codecov.io/gh/arturh85/factorio-bot-tauri/branch/master/graph/badge.svg?token=Q4I23JAT9A)](https://codecov.io/gh/arturh85/factorio-bot-tauri)
[![actions](https://github.com/arturh85/factorio-bot-tauri/actions/workflows/test.yml/badge.svg)](https://github.com/arturh85/factorio-bot-tauri/actions)
[![Github Repo](https://img.shields.io/badge/repo-github-blueviolet)](https://github.com/arturh85/factorio-bot-tauri)

## Intro

You are welcome to clone the [Github repository](https://github.com/arturh85/factorio-bot-tauri) and extend the ability of the platform.
This guide is intended to write down steps needed to develop this application before i forget it myself.

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
- Change directory to app/
- `cd app/`
- `yarn` or `npx yarn`

### Commands

- `cd app/; npm start` starts the application while watching for changes
- `cargo repl` starts the repl version of the application which removes most features and builds quicker
- `cargo nextest` starts rust test runnner
- `cargo release` increments the version numbers, updates changelog and pushes release
- `cd docs/userguide; mdbook serve` serves userguide locally


### Required tools
- `cargo install tauri-cli`

### Optional tools
- `cargo install mdbook`
- `cargo install mdbook-mermaid`
- `cargo install cargo-release`
- `cargo install git-cliff`
- `cargo install cargo-nextest`
