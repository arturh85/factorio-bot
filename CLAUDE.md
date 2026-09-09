# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Factorio Bot is a Rust application that orchestrates Factorio game servers and multiple bots via Lua scripting, with a browser frontend served by its own HTTP server. It was a Tauri desktop app until plan 5 (`docs/superpowers/plans/2026-08-30-frontend-transport-swap.md`) removed Tauri entirely; the crate directory is still named `app/src-tauri` because renaming it is a deferred mechanical change, not because Tauri is still there. Use cases include tool-assisted speedruns (TAS), ML training environments, and Factorio experiments.

## THIS PROJECT RUNS SPACE AGE, NOT BASE FACTORIO

Every instance extracts from `workspace/factorio-space-age_linux_2.1.17.tar.xz`,
and a run loads **six** mods:

```
base 2.1.17 · space-age · elevated-rails · quality · recycler · BotBridge 0.0.1
```

**Nothing recorded this until 2026-09-07** (`295056a5` puts the mod set in
provenance, BotBridge's version included). Two sessions worked for a day on
Space Age world-record saves — ten surfaces, biolabs, cargo landing pads — while
calling this install's prototype values "vanilla", with the archive filename in
their own terminal output. **Seeing is not registering**; the sibling of *filing
is not handling* below.

**Why it matters, beyond the label:**

- **Every prototype-derived value in this repo was read off a Space Age
  install** — `energy_usage`, `supply_area_distance`, `maximum_wire_distance`,
  `crafting_categories`, `resource_category`. They are right for what we run.
  They are **not** "vanilla Factorio" values, and ~111 uses of the word
  *vanilla* in `state.rs` and `types.rs` mean *"the shipped fallback"* rather
  than *"base game"*. The fallback tables themselves are sound — checked:
  `VANILLA_CHARACTER_RESOURCE_CATEGORIES` is `["basic-solid"]`, the character's
  own category, which Space Age does not change.
- **It is why product ambiguity is everywhere.** Three of sulfur's four recipes
  (`advanced-carbonic-asteroid-crushing`, `biosulfur`, `sulfur-recycling`) are
  Space Age; in base Factorio *"produce sulfur"* has exactly one answer.
  Petroleum gas is the counter-example worth keeping straight — its four
  producers are all base game, so **that** ambiguity is real regardless of mods.
- **It explains `no resource patch found for 'calcite'` / `'scrap'`**, which has
  scrolled past in every log anyone has read. Those are Space Age resources the
  entity graph looks for and a Nauvis map does not have. Nobody asked why the
  graph wanted calcite.
- The owner's world-record saves are Space Age too, so runs and saves are
  comparable — **that was luck, not design**, and only provenance makes it a
  fact rather than an assumption.

**Do not add or remove a mod without checking `provenance.mods` afterwards (the field is `mods`, an `Option<BTreeMap>`; this file said `mod_set` until 2026-09-08 and an agent went looking for a field that does not exist).**
`None` there means *not captured*; `Some(empty)` means *the game is vanilla* —
collapsing them would make an unrecorded run indistinguishable from a base-game
one, which is the confusion provenance exists to prevent.

## What landed on 2026-09-08, and the trap in each

Six capabilities that did not exist the day before. Each is listed with the
thing that will bite whoever uses it next.

- **`goal.gathered` / `goal.produced` / `goal.extracted` are sayable from Lua.**
  Before this, the planner had eight goal kinds and a script could name six, so
  the oil milestone was unreachable from the only path that can execute it.
  **A goal kind lives in SEVEN places**, not the three that are obvious:
  constructor, `goal_from_lua` arm, `render_goal` arm, `KINDS`, a test inside
  `goal.all`, the `__doc_entry_<kind>` string in `goal/mod.rs`, and the expected
  set in `the_goal_table_offers_exactly_the_new_surface`. **Only the last two
  fail loudly**; the first four have no guard between them, which is how
  `charted` went missing for a day. The list is in `install_goal_constructors`'
  doc. `unlocks` is exposed and is **a claim, not a grant** — nothing checks the
  technology name, and a wrong one makes the plan believe a technology is done.

- **`Site::Beside { of, steps }` puts a second block on one map**, resolved
  before `recover_anchor` for the same reason `Site::Anchored` is. It needs the
  blueprint's **declared pitch**, which **cannot be derived from the entities**:
  `MinerLine`'s centres span 4 tiles of x and it declares 7. A block declaring
  no pitch is refused by name rather than tiled at its extent, because tiling at
  the extent silently eats whatever the author reserved.

- **`tools/fixture_shape.py` decodes every fixture in `scripts/rcontest.lua` in
  about a second** — composition, declared pitch, `span` (centre to centre) and
  `tiles` (tile columns holding an entity centre). **Run it instead of typing a
  fixture'"'"'s shape.** Three quantities had been competing for one label and two
  sessions quoted different ones at each other; this file'"'"'s "5 x 21" for
  `MinerLine` is the `tiles` reading. It deliberately omits power draw — a
  second copy of `consumer_kw` would reproduce the 624/639 confusion.

- **Water is a question about what the ground yields**, not about a tile name.
  `FactorioTile::yields_water` answers `Unknown -> is_water()` else
  `fluid.yields("water")`, so `Dry` outranks the name and `Yields` outranks it
  the other way. **The by-name fallback is load-bearing and was measured**:
  delete it and all four baselines refuse with `PowerPlantNeedsWater` while
  reporting `charted ground covers 17 of 17 probes` — a **plausible lie about
  the map**, not a diagnosable failure, because every archived dump reads
  `Unknown`. Documented as a fallback for old senders, never the definition of
  water: the answer to a missing name is that the sender should declare `fluid`.

- **`Method::hands_over` is the weaker question `converges` was standing in
  for.** `run_steps` sizes every action against `ctx.chain_actor`, but a chain
  only *opens* on a stated holder or on `converges` — so a subtree passing items
  **hand to hand without converging** got a bill in one bot'"'"'s name and no owner,
  and the scheduler split it. `pipe` is one iron plate, so `HandCraft::converges`
  was honestly `false`. Both are read off one `short_ingredients` at thresholds
  2 and 1, so the weaker claim cannot drift from the stronger one containing it.

- **The oil rig plans and the bots die walking to it.** On seed 31337 crude oil
  is **372.5 tiles** away and a fresh map generates to ±320, so it is outside by
  52. Charting is cheap — one ring, 8 surveys, 0 failures, 4,159 ticks, and the
  plan it yields matches the fully-explored offline action count exactly. But
  the survey reveals **32 enemy structures**, and the first live attempt lost
  two bots (a small-biter at tick 27,065, a small-worm-turret at 53,619) after
  246 of 2,295 actions. **Nothing in the planner models a threat.** Whether the
  answer is threat-aware planning, a forward base, or "oil is not a t=0 target
  on this seed" is an open owner decision — one run does not separate them.

**`runtime-api.json` does NOT carry defines VALUES — only `name` and `order`.**
Reading a number out of it gives you a **sort key**. I quoted
`character_guns = 5, character_ammo = 6, character_armor = 7` and
`turret_ammo = 33` from it; measured against the running game the values are
**3, 4, 5 and 1**. Ask the game, or read the mod's own `inventory_type_name`
table, and never read an integer out of the API dump. This is the sibling of
the attribute-versus-method check: the file describes the *shape* of the API,
not its constants.

**One environment note that cost four verification attempts**: a `target/`
shared by two cargo processes tears incremental objects, and **cargo then
considers them fresh**, so `mold: error: undefined symbol: anon.<hash>.llvm.<n>`
survives on an idle box. Contention causes it; stale output perpetuates it.
`cargo clean -p <crate>` fixes it — but never during another writer'"'"'s build.

## Build & Development Commands

**Never `git commit --amend` in this checkout, and never `cargo fmt --all`.**
Both assume you are the only writer and neither checks. An agent amended what it
believed was its own HEAD; another had committed in the intervening seconds, so
it amended *their* commit. It was recoverable — `git reset --soft` back to the
original commit object from the reflog, `git diff` verified empty, then commit
separately — but only because it noticed. `cargo fmt --all` has the same shape
and has already rewritten another agent's live files; `cargo fmt -p <crate>`
still rewrites a whole crate, so use `rustfmt --edition 2024 <file>`. **The
edition flag is not optional**: bare `rustfmt` defaults to Rust 2015 and dies on
every `async fn` in the file, which reads as a compile error in code that builds
fine -- and chained with `&&` it silently skips whatever you meant to run next.
It skipped a crate test run here, in the same breath as telling agents to prefer
`rustfmt <file>`.

The same reasoning covers `git add -A`, a bare `git commit`, `git reset --hard`
and `git stash`: commit with explicit paths (`git commit -m "..." -- <paths>`),
and note that **an explicit path is still a path to the whole file** — naming a
file carefully does not separate two writers editing it.

**And `git checkout -- <file>` belongs in that list, which this section learned
the hard way on 2026-09-08.** A falsification harness restored its mutations
with `git checkout -- <file>` — the natural reach, and it does not restore *the
mutation*, it restores **the last committed state**, silently discarding the
agent's own uncommitted work in all four files it touched. The tell was a build
error naming a method that had existed minutes earlier
(`hands_over is not a member of trait Method`), which reads like a compile
problem rather than a data-loss one. Recovered from the patch scripts, diffstat
verified identical, and committed *before* re-running.

Two durable rules from it: **a mutation sweep must back up by file copy and
restore by copy plus `touch`** (`cp -p` preserves mtime, so cargo re-runs the
*mutated* binary against restored source — a false red), and **commit before a
sweep, not after**, so the worst case is a lost mutation rather than lost work.
`git restore` has exactly the same hazard under a different name.

**Every cargo command needs `nix develop -c`.** `pkg-config` and Lua 5.4 come
from the flake, not from the ambient shell, so a bare `cargo build` dies in
`mlua-sys`' build script with

```
cannot find Lua5.4 using `pkg-config` ... The pkg-config command could not be found.
```

which reads like a missing system package and is not one. Same for the
graphical client: the GL/X11 libraries and `SDL_VIDEODRIVER=x11` are set by the
dev shell, so a client launched outside it fails with `No available video
device` — a message that says nothing about the cause (see the Platform Notes).
Prefix the command (`nix develop -c cargo test --workspace`) or wrap a whole
script (`nix develop -c bash -s <<'EOF' ... EOF`). Direnv is not configured
here, so nothing enters the shell for you.

```bash
# Master Check Tool, runs Rust clippy, tests, build, takes a few minutes, only run in the end to finalise a change
# Verify only -- never rewrites a file, so it is safe on a dirty tree and when
# another agent is working in the same checkout.
just test

# Apply what `just test` reports. Rewrites files workspace-wide -- only run it
# when every uncommitted change in the tree is yours.
just fix

# Start Factorio server with BotBridge mod
just factorio

# Frontend dev server on :8080, proxying /api to a `just serve` on :7492
just start          # or: cd app && pnpm start

# The real thing: axum serving the built SPA and the API on :7492 -- this is
# "the viewer". It builds `factorio-bot`'s `viewer` feature alias (cli, lua,
# restapi -- no repl, no tokio-console), equivalent to
# `cargo run --release --no-default-features --features viewer -- serve --web-root app/dist`.
# Do NOT reach for `--all-features` to get `restapi` -- see the tokio-console
# note below the block.
just serve

# REPL mode (faster build, no GUI, for testing scripting)
cargo repl

# Run frontend tests (vitest, watch mode; `pnpm run test:coverage` for one shot
# plus the enforced coverage gate)
cd app && pnpm test

# Rust tests. `cargo nextest` is NOT installed here -- if you install it, note
# that it runs one process per test, which contains a hang to a single named
# failure instead of killing the run, but does NOT run doctests. That is a
# real trade, not a free upgrade.
cargo test --workspace

# Lint everything
cd app && pnpm lint              # TypeScript + ESLint + Vue type checking
cargo clippy --workspace --all-features --all-targets -- --deny warnings

# Pre-commit check (runs all CI checks locally)
cd app && pnpm run precommit:check

# Production build
cargo build --release           # the binary IS the deliverable now; default
                                 # features (restapi, repl, cli, lua) already
                                 # cover it -- no flag needed, and specifically
                                 # not --all-features (see below)
cd app && pnpm run build:web    # and the SPA it serves

# Build with/without default features
cargo build --all-features
cargo build --no-default-features
```

**Do not build with `--all-features` to "get restapi".** Default features
already include it (`default = ["restapi", "repl", "cli", "lua"]`); the only
feature `--all-features` adds beyond that is `tokio-console`, opt-in debug
instrumentation that binds a fixed TCP port (`127.0.0.1:6669` unless
`TOKIO_CONSOLE_BIND` says otherwise). A viewer build and a game-run build both
started with `--all-features` both try to bind it, so the second one either
loses tokio-console (current behavior: `Context::new` probes the port first
and warns instead of installing the console layer when it is taken -- see
`app/src-tauri/src/context.rs`) or, before that fix existed, panicked the
whole process, because `[profile.release]` sets `panic = "abort"` and the
crash came from `console-subscriber`'s own background thread with no mention
of Factorio. For the viewer specifically, use `just serve` or
`--features viewer` (an alias for `cli,lua,restapi`, deliberately excluding
`tokio-console`) rather than `--all-features`.

**LSP tools**: prefer language-server-backed refactoring tools (rename symbol,
find references, go to definition) over textual search-and-replace — they read
macros and trait implementations correctly, which `grep` does not.

### The shell here is aliased, and the aliases fail in ways that look like data

Three commands do not mean what they say, and each has silently produced a
wrong answer in this repo rather than an error you would notice.

- **`ls` is `eza`.** `ls -t` does not sort by time -- `-t` is eza's
  `--time <FIELD>` and wants a value, so `ls -1t <dir>` dies with
  `invalid value '<dir>' for '--time <FIELD>'`. Worse, under `zsh` a
  command whose glob fails aborts the whole line, so
  `ls -1t runs/*/events.jsonl 2>/dev/null | xargs ...` prints **nothing at
  all** and looks exactly like "no runs matched". Three separate analyses
  returned empty this way before the alias was noticed. Use `command ls`.
- **`cat` is `bat`**, so `cat -v` and `cat -A` fail rather than showing
  non-printing characters.
- **`git diff` is difftastic**, so `git diff | git apply` cannot work. Use
  `git diff --no-ext-diff` when something machine-readable is wanted, or
  `git revert`.

**A backtick in `git commit -m "..."` is COMMAND SUBSTITUTION, and it silently
deletes what it cannot run.** `-m` is passed in double quotes, so the shell
expands backticks before git ever sees the string. A message written as

```
Gating runs on load with `bc`, which is not installed here.
```

lands in the log as `Gating runs on load with , which is not installed here.` —
the word gone, the sentence still grammatical, and `zsh: command not found: bc`
scrolling past in output nobody re-reads. This happened **four times in one
session** to four different commits, each time removing exactly the identifier
the sentence was about, because prose about code is mostly backticked
identifiers. Two of them silently dropped the names of the technologies a
finding was about.

Write the message to a file with a **quoted** heredoc and use `git commit -F`:

```bash
cat > /tmp/msg <<'EOF'
fix(thing): `identifier` survives here
EOF
git commit -F /tmp/msg -- <paths>
```

`<<'EOF'` (quoted delimiter) suppresses every expansion; `<<EOF` does not.

**A pipeline reports the LAST command's exit code, not the interesting one.**
`cargo test --workspace | grep -E "^test result"` exits 0 whenever *grep*
matched something, even with a failing test in the output. That has already
shown a green `[exited with code 0]` over a genuinely red run. Redirect to a
file and test the exit code, or set `pipefail`; never read a pipeline's status
as the tool's status.

The general shape: **an aliased command that fails still exits into a pipe**,
and a pipe that receives nothing reads as an empty result rather than as a
broken command. When an analysis over run records comes back empty, check the
command before concluding anything about the data.

### Evaluate a planner change OFFLINE first -- seconds, not a 20-minute run

Since `db612be9` a plan can be made against a **dumped world**, with no
Factorio, RCON, workspace or settings file:

```bash
# in a live run, from Lua -- writes <scripts>/map.json
world.dump("scripts/map.json")

# then, offline, in ~4 seconds against an 864 MB dump
factorio-bot plan --world workspace/scripts/map.json \
    --goal researched:automation --bots 1,2,3,4 --steps
factorio-bot score-map --world workspace/scripts/map.json --bots 1,2,3,4
```

`score-map` reports resource distances, a walk score, a verdict, and the full
`PlanReport` -- actions, makespan, steps/acts/walks per bot, planned and idle
ticks, roster utilisation. **This is the loop to iterate in.** It has caught a
13:05 planning ceiling, a green-science capability gap and a whole workstream's
result without spending a run.

**Since 2026-09-05 `workspace/scripts/map.json` is the seed-31337 t=0 dump**
(fingerprint `c161fa3f437221d0`, four headless character bots with the
honest freeplay inventory), kept also as `map-31337-t0.json`.

**The baselines this file used to quote here were stale, and quoting them at
all was the mistake.** It said `researched:automation` = 177 actions / 22,044
ticks and `producing:logistic-science-pack:6` = 623 / 71,167. Measured
2026-09-06 against `master`: automation is **176 / 21,784**, confirmed twice
independently. Green has moved more than once and two builds disagreed on the
same day -- 442 / 47,542 from two agents on merged `7ebe707d`, against
451 / 48,829 from a release binary compiled a few commits earlier. Neither is
wrong; **the plan legitimately changes as the planner improves**, which is the
whole point of the work, and a number pinned in prose goes stale silently
while looking authoritative.

So: **re-measure, do not quote from here.** One command, about 4 seconds:

```bash
target/release/factorio-bot plan --world workspace/scripts/map.json \
    --goal researched:automation --bots 1,2,3,4
```

Take the three baselines before and after a planner change on the *same
binary*, and state the commit beside them. A baseline compared across two
builds measures the builds, not the change -- that is exactly the trap above. Before that it was the OLD map's baseline, which lives
on as `map-t0-baseline.json` (fingerprint `dfac0f4caa0a7500`) -- every
offline number in the 2026-09-03/04 record was made on that map while the
live runs were on 31337. Regenerate with
`factorio-bot lua dump_31337.lua --headless --bots 4 --seed 31337 --new`
(the script is `world.dump("map-31337-t0.json")`); a one-bot dump plans the
other three with fabricated empty inventories and reads 8:48 for automation.
Anything overwriting `map.json` -- a seed search, a `--resume-from` dump --
destroys it unless you copy it aside first. That has already happened once.

**Three blind spots, each of which has produced a wrong "the bug is absent":**

- **CORRECTED: `world.dump` DOES refresh buffers now**, so the `Withdraw` path
  is reachable offline. `create_lua_world`
  (`crates/scripting_lua/src/globals/world.rs`) builds a `BufferRefresher` over
  `Planner::refresh_buffers` and hands it to the binding, which asks the game
  what is in those containers before serialising — exactly as `goal.plan` does.
  The readings ride in the dump and `PlanState::from_world` loads them.
  **Still true, and still the trap**: the refresher exists only when there is
  an RCON connection, and the `plan` CLI has none — it refreshes nothing and
  relies entirely on what the dump carried. So a dump taken without a live game,
  or one written before this landed, still has `inventories: []`, and against
  such a dump the `Withdraw` path is unreachable. A real double-spend bug lived
  exactly there and needed hand-injected inventories to reproduce. Check the
  dump for readings before concluding a `Withdraw` bug is absent.
- **A dump is t=0-shaped unless you make it otherwise.** A `--resume-from`
  savepoint restores *saved* inventories and positions, not the live ones at the
  moment of failure. Two separate bugs needed the dump perturbed -- one with bot
  positions, one with inventories -- before they appeared at all.
  **Two ways to make it otherwise without a game (2026-09-09)**: `plan
  --replan 1 --fail <label>` applies the plan's own placements as map facts
  and plans again (`just replan-check`), and `plan --standing-from-run <run>
  --at-tick <T>` puts a finished run's keyframe, bot positions and machine
  recipes on the dump. Both go through `factorio_bot_planner::standing`, and
  both were shown red on the defect they exist for before they were trusted.
  A tick cut is the wrong knife -- an abandoned batch is a dependency cone,
  not a suffix -- and a snapshot without recipes is a world where every
  assembler stands empty. See
  `docs/superpowers/notes/2026-09-09-a-replan-you-can-run-offline.md`.
- **A fresh map has charted almost nothing.** Distances read off an early dump
  measure what has been *seen*, not what exists.

### Ask the RUNNING game directly -- `rcon -s localhost`

`factorio-bot rcon -s localhost -- '<command>'` attaches to an **already
running** instance. **The reply reaches you on STDERR, not stdout, and this
paragraph took three attempts to state correctly.** What is true: the client
logs the body at INFO as `rcon ⮞ <body>` (`crates/core/src/factorio/rcon.rs:148`),
so the answer *is* readable; `FactorioRcon::send` also returns it as
`Result<Option<Vec<String>>>`, and `app/src-tauri/src/cli/rcon.rs:49` discards
that with `rcon.send(command).await.unwrap();`. So the reply is visible in the
log and absent from stdout — which is why one agent read two probes as "the
command did nothing" and why printing it is a one-line change if anyone wants
it on stdout. Note also that a console `/c` cannot see a mod's `storage` — go
through the surface. No MCP server, no schema, no second
process: it is the fastest loop in this project for any question about live
game state, which is exactly the class `world.dump` cannot answer (its
`inventories` is always `[]`).

```bash
factorio-bot rcon -s localhost -- '/c rcon.print(game.tick)'
factorio-bot rcon -s localhost -- '/c rcon.print(serpent.line(remote.interfaces))'
```

Worked example, from a run whose `BatchProgress` counters had been frozen for
690 s with one action in flight:

```
1:char   2:char MINING   3:char MINING   4:char MINING
```

Bot 1 idle while the others mined -- so the game was healthy and **bot 1's
completion signal was lost**, not a lag wait. Thirty seconds, against the
25-minute run that question used to cost.

The mod's interface (`remote.interfaces.botbridge`) includes `world_snapshot`,
`inventory_contents_at`, `find_entities_filtered`, `find_tiles_filtered`,
`player_info`, `player_force`, `players`, `set_recipe`, `savepoint`,
`session_reset` and the `action_start_*` family. **`freeplay` also exposes
`set_chart_distance`** -- relevant to exploration, which this mod otherwise
never does.

**Two rules.** Keep it **read-only during a measured run**: a query is cheap,
but mutating a live game contaminates the measurement, and provenance has no
field that would record it. Anything goes against a savepoint-resumed world.
And do not add `rcon.print` *inside* a mod function the executor calls -- that
output lands in the RCON reply body and the executor reads it as the action's
result. An ad-hoc command on your own connection is a different thing and is
safe.

### Action ids are PER-PLAN, not global

A run replans, and ids are reused across plans: in `run-1788465258-49050`, id 38
is `craft 2 iron-gear-wheel` in the first plan and `craft 1 offshore-pump` in
the third. **44 of 147 ids collide in that one run.**

So **never key an aggregate on `id`, and never join anything on `id` across a
plan boundary.** Doing so once understated a bot's executing time by 8,067 ticks
(it silently dropped 44 settles); doing it again invented three "blocker cleared
in 10,000-40,000 ticks" measurements that were replan boundaries. Aggregate over
**settle events**; identify actions by `(id, dispatch order)` or by position.

### Milestone savepoints make a failure reproducible

Every milestone writes `runs/<run>/savepoints/milestone-N.zip` plus a `.json`.
`--resume-from <run>[:<milestone>]` starts a run on that world (`lua` and
`start` both take it). A resumed run is marked in provenance and `--compare`
refuses to compare it against a fresh one -- correct, and expected.

This is what turned "reproduce the failure by paying for the whole 25-minute
prelude again" into a short server start plus an offline plan.

## Architecture

```
Browser (Vue 3 + Tailwind v4 + reka-ui)
    ↓ (HTTP + SSE, no IPC)
crates/server (axum: /api/v1/*, serves the built SPA)
    ↓
┌─────────────────────────────────────────┐
│ Rust Workspace                          │
│  • crates/core         (orchestration)  │
│  • crates/scripting*   (Lua via mlua)   │
│  • crates/server       (axum HTTP+SPA)  │
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
  - `graph/flow_graph.rs` - Material flow throughput. **Validated against a
    world-record base on 2026-09-07 and now within 1-32% of the game's own
    production statistics.** Everything below is current as of that date; the
    entry that stood here through 2026-09-06 was wrong on three separate
    counts, and the *reasoning* that was wrong is kept at the end because it is
    the transferable part.

    **What is true now.** It has real readers: `throughput_at` and `node_at`
    are called from `crates/planner/src/method/sustain.rs`. It refreshes
    itself: `ensure_current` is called by every public reader and rebuilds --
    clearing first -- whenever `EntityGraph::generation` has moved, and the
    entity graph bumps that on every mutation. Rates are **derived from
    prototypes**: `smelting_output` computes
    `product.amount * crafting_speed / recipe.energy`. Nothing in
    `crates/executor` or `crates/server` mentions it. `condense()`,
    `inner_graph()`, `graphviz_dot()` and `graphviz_dot_condensed()` still have
    no caller outside the file and its own tests.

    **The validation is documentation, not a harness.** The error figures live
    in doc comments and in `docs/superpowers/notes/`; there is no automated
    check that would catch a regression. Against a 6:39:53 Space Age base at
    tick ~1,447,000 (`docs/superpowers/notes/
    2026-09-06-what-the-record-base-knows.md`):

    | item | game /min | model /min | ratio |
    |---|---:|---:|---:|
    | copper-cable | 22,367 | 22,680 | 1.01 |
    | copper-plate | 15,147 | 16,580 | 1.09 |
    | iron-plate | 15,170 | 19,016 | 1.25 |
    | electronic-circuit | 6,694 | 8,820 | 1.32 |

    Four things that only a real base could have shown, each of which
    contradicted an expectation held going in:

    - **We OVER-predict.** The gap was expected in the other direction, on the
      theory that modules and beacons we cannot model would make the real base
      faster. **Falsified where the volume is**: all 1,222 furnaces and all 559
      iron drills read `speed_bonus` and `productivity_bonus` of exactly 0.000,
      and not one of the base's 2,226 modules sits in a furnace or an ore drill.
    - **The dominant term is idleness, and nothing in the graph represents it**
      — +16% to +23%, unbounded, against coverage at ~1.6% and force bonuses at
      −9%. A machine standing still is invisible here; 194 drills were sitting
      at `waiting_for_space_in_destination` during the measurement. **This, not
      modules, is the next piece of work.**
    - **The old hard-coded `1/3.2` would have been exactly 2.0x low on every
      plate**, since 1,196 of the 1,222 furnaces are `steel-furnace` at
      `crafting_speed` 2. The fix had landed but had never been *verified*;
      this is the verification.
    - **A furnace was running every recipe at once.** The furnace arm added a
      full-rate edge per smeltable input, so one furnace on a mixed belt
      reported smelting iron AND copper AND stone at 100% each — stone-brick
      read 7,125/min against the game's 450. Outputs now share the furnace's
      time.

    **And copper-cable's 1.01 is a coincidence, stated as one**: its +12% beacon
    speed and +3% productivity nearly exactly cancel its 18% idle. This repo's
    rule that a match is more suspicious than a 3x, measured for once rather
    than asserted.

    Two standing limits: the model holds **one surface** (Nauvis is 86.5% of the
    base by entity count but 99% of plate and circuit production), and
    `EntityGraph::add`'s whitelist deliberately excludes 763 entities including
    **all 249 beacons**, which are therefore invisible by construction.

    **A denominator artefact worth not repeating**: this base was reported for
    weeks as "the model holds 12% of it". It holds **98.0% of the built base**.
    The 12% divided by *every* entity within 1,000 tiles, 83% of which are ore
    tiles — and ore lives in `EntityGraph::resources`, a `Pos`-keyed map, not in
    `entity_tree`. Comparing against `entity_tree` could never have found them.

    **What this entry got wrong, kept for the shape of the mistakes.** Three
    claims stood here and were each falsified:

    - *"There is no per-update cost to reclaim; the liability is dead code, not
      CPU."* The premise was checked and is still right — `update()`'s two
      original call sites are `output_parser.rs::on_init`, which fires **once**
      when Factorio logs `initial discovery done`, and `snapshot.rs::attach_world`,
      the `--connect` path — but "dead code" did not survive a reader arriving.
    - *"It never refreshes, so a reader gets the world as it was at tick 0,
      silently and with no error."* True when written, and the instruction it
      carried — **treat "add a reader" and "refresh" as one piece of work,
      because a stale answer looks exactly like a current one** — is why the
      refresh landed with the readers. Keep the rule; the defect is fixed.
    - *"The refresh cannot be `update()` again, because `self.inner` is never
      cleared."* Correct diagnosis, and the fix took its advice. The two failure
      modes it named are worth remembering for any graph built from another —
      **a removed entity's node and edges stay forever** when nothing deletes,
      and **a position reused by a different entity keeps the old node**,
      because `node_at` matches on position alone and returns before the
      prototype is consulted.
    - It also placed the `1/3.2` in a function called `furnace_output`. **No
      such function exists**; the constant survives only as an expected value
      in tests, because vanilla's iron recipe genuinely has `energy = 3.2`. A
      grep for it found the tests and was read as finding the defect.
  - `process/` - Factorio process spawning/control
  - `plan/planner.rs` - `Planner`, the Lua runtime's context holder (rcon,
    real_world, plan_world). NOT a planner any more: the task-graph planner it
    was named for was deleted, and goal decomposition now lives in
    `crates/planner`. The name is kept because `run_lua` takes it.

- **crates/planner**: the goal planner. Pure and deterministic — no I/O, no
  async, no wall-clock, ordered collections only, floats via `total_cmp`.
  `Goal` -> `expand()` -> `ActionNetwork` -> `schedule()` -> `Schedule`.
  Methods live in `method/`; `state.rs` overlays a `FactorioSurface` snapshot.

  **`FactorioWorld` was renamed to `FactorioSurface` on 2026-09-06, and the
  name `FactorioWorld` now means something else.** The old type was never a
  world — one `EntityGraph`, one `FlowGraph`, one set of `Pos`-keyed overlays,
  all of them describing a single surface, so a chest at (10, 10) on Nauvis
  and a chest at (10, 10) on a platform were the same key everywhere. The new
  `FactorioWorld` (`crates/core/src/factorio/world.rs`) is the aggregate: a
  `BTreeMap<SurfaceId, Arc<FactorioSurface>>`, one graph per surface, so the
  aliasing is impossible by construction rather than by care. The surface does
  **not** go on `Position` — a coordinate is only comparable within a surface,
  and `p1 - p2` across two has no answer an `f64` can carry.

  Two things to know before touching it. **A second surface is safe now, and
  the refusal that used to forbid it has narrowed** (2026-09-07). The
  game- and force-global fields — recipes, prototypes, `forces` and their
  research, the action id counter — moved off `FactorioSurface` into
  `GameGlobals`; the world owns one `Arc<GameGlobals>` and `insert_surface`
  checks with `Arc::ptr_eq` that every surface holds *that* one. So two
  surfaces cannot disagree about what is researched, hand out the same
  `action_id`, or carry two recipe tables — by construction rather than by
  care, the same argument that put the surface on the container instead of on
  `Position`. `SurfaceNotYetSeparable` (refuse *any* second surface) is gone;
  what is left is `SurfaceGlobalsNotShared`, which refuses a surface carrying
  **its own** globals. **A cloned surface is a fork and cannot be inserted
  back** — `Clone` deep-copies the globals on purpose, so the plan world's
  writes never reach the live model. Build a second surface with
  `FactorioSurface::with_globals`, never by cloning one.

  And **`only_surface()` is the porting seam, not `nauvis()`**: it answers only
  while there is exactly one surface, so a caller that never said which surface
  it meant stops working rather than silently getting Nauvis. `nauvis()` is for
  a caller that genuinely means Nauvis. The mod's Nauvis guard in
  `mods/BotBridge/control.lua` is the matching half upstream — no non-Nauvis
  chunk reaches Rust at all, and what it drops is counted in
  `surface_chunk_drops` rather than discarded silently. The type's own doc
  carries the field-by-field split and names `players` and `benches` as
  genuinely ambiguous; read it rather than this paragraph before changing
  anything. See `docs/superpowers/notes/2026-09-06-surfaces-survey.md`.

  - **`method::connect`** (`connect_steps`, built on `graph::route::route_belt`
    in `crates/core`) routes a `transport-belt` run between two **machines**
    and places the inserter at each end. It takes the two `FactorioEntity`s,
    not two positions, because **the size and the parity of a machine are the
    whole problem**: an inserter tile is derived from the machine's
    *footprint perimeter* (`bounding_box`), and every facing is computed
    between two cell centres of one grid. The first version derived it from
    the four neighbours of the machine's *centre cell* instead, which refuses
    every 3×3 (all four are inside its own footprint) and hands every 2×2 a
    diagonal — and because an even footprint sits on a tile *boundary* (see
    `method::util::tile_alignment`) while every belt tile is a half-integer,
    it refused `NotCardinal` on open ground for every stone furnace. Four
    task reviews passed it; the fixtures had been built at illegal positions
    with a shrunken box, so they fitted the code.
    **It tunnels since 2026-09-09.** Until then this paragraph said "belts
    only": `route_belt` was always called with `max_underground: None` on the
    ground that "neither `FactorioEntity` nor the mod's `rcon_place_entity`
    can express which half" of a pair an entity is. **That had been false at
    every layer for days** — `FactorioEntity::underground_half`,
    `new_underground_belt`, the mod's fifth `rcon_place_entity` argument, the
    executor threading it through, and `route_belt`'s own underground search
    all existed and were proven live by `method::blueprint` on `FurnaceLine`
    — while the comment, the `None` and an `unreachable!()` arm outlived
    their reason. The cost was the belt-fed `sustain` path stuck at one cell
    (`SustainNoRouteForFuel`, "blocked by 4 tiles"). Now: the reach is read
    from the `underground-belt` prototype's `max_underground_distance` (5 on
    this install; `fast-` 7, `turbo-` 11), **in the prototype's unit** —
    entry-to-exit distance, so 5 hides four tiles; `route_belt` used to count
    hidden tiles under the prototype's name, which is the off-by-one a caller
    reading the field would pass straight through. A world with no such
    prototype routes on the surface only. A jump is priced at about sixteen
    belts (`UNDERGROUND_PENALTY`, from the base recipes' iron), so a detour
    is taken before a tunnel and a tree is walked round, not under; only
    launched from a tile already facing its way (a side-fed entry half-loads
    one lane); and followed by a straight tile (an exit emits forward — the
    old search would surface and turn, which places perfectly and moves
    nothing). The ground beneath every existing pair, base world or plan
    overlay, is reserved twice: as a same-axis tunnel bit no new jump may run
    along (the game pairs an input with the first same-type half on its
    line — belt weaving works only across tiers), and as a blocked cell
    nothing else of the plan is built on. A wall wider than the reach refuses
    as `ConnectRefusal::SpanTooLong { needed, max }`, both numbers in the
    prototype's unit. The recipe is **disabled at t=0** (it needs
    `logistics`), so a plan that tunnels carries that research.
    **And the "blocked by 4 tiles" that motivated all this was never a
    wall.** The four tiles were a 1x1 coal chest's own four neighbours — a
    perimeter budget, refused by `first_free_perimeter` before `route_belt`
    ever ran. `sustain:iron-plate:30:36000` needed three sustain-side fixes
    to plan (a standing chest is reused only with one free side per unfed
    burner; a later cell's chest is hauled from the nearest coal chest with a
    side to spare, not from a source 31 tiles off; the drill-first feed order
    is tried on a fork and reversed when it refuses) and one in `connect`: a
    route may not hug the chest it serves (its other sides, two tiles deep,
    are closed to that route). Read the tiles a refusal names before naming
    the mechanism — adjacent-to-the-source is a perimeter, not an obstacle.
    **And a BYSTANDER chest's perimeter is the caller's budget, not
    `connect`'s** (2026-09-09, `run-1788920460-08860`): the plate chest of a
    `sustain` cell had three of its four sides spent by the cell's own coal
    branch *passing* it, and the assembly method's supply link then refused
    with all four neighbours named — zero science, 48 steps abandoned. A
    rule in `connect` keeping one side of every bystander chest was tried
    twice and reverted twice, because the cell's *coal* chests legitimately
    spend every side they have and the router cannot tell the two apart. So
    `sustain` declares the exit (`Offtake::exit`) and lays every run with
    it closed (`connect_steps_reserving`); and **which** side to keep is
    decided by laying the rest of the cell on forks (`choose_exit`), because
    the straight-line exit walled the branch into a loop on the fixture and
    cost the furnace's run a tunnel the force cannot craft. Ask the future,
    do not guess it. **And a kept side is not a way out**: on seed 31337
    every exit the trial kept came back SEALED (`belt_reaches_open_ground`)
    — the cell's four coal runs draw a closed ring round the whole cell,
    double belt walls at x=25.5/34.5 and y=-51.5 — so the chest keeps its
    side and a run out of it would still tunnel. Two things were tried and
    withdrawn, with the numbers: a corridor of up to 8 tiles (sealed at
    every length), and reserving a BFS escape path to the window edge per
    run (every trial then refused — the wall cut the furnace's own coal
    run). What ships instead: `supply_link_steps` routes both ends on forks
    first and takes the one whose run needs fewer undergrounds, so the link
    is laid on the surface from the furnace (bundle 885 actions / 52,891
    ticks) rather than from the chest under two belts (972 / 62,972, with
    `logistics` researched off a hand charge just to craft the pair). The
    ring itself — a haul that loops the cell it serves — is the open item,
    and so is a replan recognising its own half-built link: the live replan
    refused the furnace end because the link's first belt at `[27.5,-43.5]`
    stood while its arm did not.
    **CLOSED 2026-09-09, and the link was the symptom** (`docs/superpowers/
    notes/2026-09-09-a-replan-finishes-what-it-began.md`): nothing tracks
    who placed a belt, and a replan recognised a science cell only by a
    standing MACHINE — the last part to stand, since both need `automation`
    and every arm a circuit off the same take. So a cut batch left chests,
    arms, poles and the whole link standing, `complete_cell` saw no cell,
    sited a fresh one, and its supply chest needed a second link out of a
    plate chest with no side left. Now a cell is recovered from any two of
    its parts on their tiles, and `connect::standing_run` finishes a belt
    chain that already joins the two doors with its two arms. Run 70024's
    replan: refused → 91 actions. Reproduce any replan refusal offline
    before naming the router.
    **AMENDED 2026-09-09 (`docs/superpowers/notes/2026-09-09-the-ring-was-the-other-end.md`):
    that sealing was measured on `a69ae64c`'s layout and the layout has
    moved. On the current one the kept exit opens north-east and a 2-span
    pair crosses what the plan's own link later lays across it. The "span
    of 7" that stopped `run-1788936524-99544` was the SINK -- a science
    cell sited on the shore with its supply chest's one free belt tile in
    the gap between the steam engine and the boiler -- and `SpanTooLong`'s
    number is read off the straight line from the source, so it cannot say
    which end is shut. A sealed sink is now refused by its four walls, and
    `assemble` sites a cell only where a belt can reach its supply chest
    (`supply_chest_is_reachable`, the same `belt_reaches_open_ground`
    question `choose_exit` asks). Reproduce any of these in ten seconds
    with `plan --standing-from-run <run> --at-tick <tick>` before naming
    a ring.**
    **And that fix was green offline and did not hold live**
    (`run-1788923927-04849`, same four tiles, same 48 abandoned). Read off
    the record: `[30.5,-46.5]` — the kept exit — was under a `stone-furnace`
    at `[31,-47]` placed at tick 14,003 by bot 3, plan id 782, a *hand-smelt*
    furnace from a `have copper-plate` subgoal. The reservation was a local
    of `sustain`'s expansion, handed to its own coal runs and to nothing
    else; to every other siting the exit was free grass. Now it is state:
    `PlanState::reserve_ground` makes the tiles `Occupant::Reserved` for
    every `is_area_free` in the plan, and `connect` closes them to every
    route but the one whose own end borders them. **Two lessons, both
    general.** A promise one method makes about the ground binds every
    method that sites on it, so it belongs in the state every siting reads,
    not in a parameter. And **an offline plan against the t=0 dump cannot
    see the replan path** — "a cell ALREADY MAKES copper-plate" is a sentence
    only a replan can say, every baseline here is offline, and the t=0 dump
    has no standing cell for a hand furnace to land beside. A regression
    for anything replan-shaped must start from a world with the cell
    standing (`built_world` in the sustain tests, or `--resume-from`).
    The trigger of that replan was a third defect, and it is the one that
    killed the science cell in **every** measured run: action 643 *take 10
    copper-plate from the cell* failed with `removed 3`, and 48 steps were
    abandoned behind it (237 in the newest run), the science cell among
    them. **Read off `samples.jsonl`, the plan's lag was right and the
    plates were somewhere else**: the furnace at `[27,-46]` had made 10 by
    tick 20,311, exactly the cell's 240-tick rate from its fuelling at
    17,409 — but `sustain`'s offtake arm (`burner-inserter` at
    `[28.5,-46.5]`, *take copper-plate out of the stone-furnace*) had
    carried the first 7 into its chest at `[29.5,-46.5]` and then stopped,
    a burner arm touching only plates with no fuel source (see the burner
    note under Known Issues). `produce`'s take reads the furnace's result
    slot, which held the 3 made since. So a rate-sized lag could not have
    fixed it; a take that finishes in pieces does — see the executor entry.
    It **refuses before placing anything**: every `ConnectRefusal` variant is
    returned before an action is emitted or a single entity lands in the
    plan overlay, because a half-built belt run is worse than none — items
    would sit on it with no bot left to carry them. That promise depends on
    the obstacle grid being a **placement** grid: `enclosure::rasterize` is
    called with the belt's own half-box (0.4), not zero. With zero a tile
    counted as blocked only when an obstacle covered its exact centre, so
    grid-aligned buildings were accidentally safe and **trees and rocks, which
    sit at arbitrary sub-tile positions, were missed entirely** — the route
    was planned through them and the build failed partway.
    Materials are stated both as `Goal::Have` subgoals and as `HasItem`
    preconditions on each `Place`, so the existing shortfall machinery
    refuses before the first belt goes down; the emitted steps are ordinary
    `Step::Act(Action { id, pre, eff, duration, .. })`, the same shape
    `power.rs` emits.
    Inserter facing is computed by `inserter_facing()` in the same module: it
    names the side the inserter *picks up from* (established empirically —
    see the inserter-direction note under Known Issues). **It is the one
    place new code should use, not the only encoding in the tree**:
    `PlanState::delivers_into` derives the same fact from an inserter's
    pickup/drop positions, and `method::assemble` places its cell's inserters
    from fixed offsets with the directions written out. All three agree
    today; a claim that this is "the only place in the planner that computes
    it" was overstated and is now corrected in the function's own doc.
    **It had no caller until 2026-09-06, and that absence is what hid the
    geometry defect through four reviews** — nothing but a fixture ever
    exercised the code, and the fixtures were written alongside it.

    **It has one now, and it worked** (`84c3259a`, the self-fed cell). The
    live caller is `connect_steps_with`, from `method::sustain` (fuel routing,
    refusing as `PlannerError::SustainNoRouteForFuel`), which is in the
    production registry and so reached by `goal.plan`, `score-map` and the
    executor's recovery alike. The plain `connect_steps` wrapper still has only
    test callers — check which of the two you are reading about. A
    belted burner cell ran with **no bot in the loop for 27,249 ticks**, and
    the rate table read **`factory`** at two intervals — the first
    non-`roster-fed` attribution this project has ever produced, against
    every interval of every previous run. 288 actions dispatched, 288
    settled, zero failures, **`connect` refused nothing**, 63 belts and 8
    burner inserters standing. The belts are shown to MOVE, not merely
    stand: the source chest is empty in 88% of samples and the destination
    chest in **100%**.

    **One defect the first caller exposed**, which no unit test could have:
    `connect_steps` hard-coded the electric `inserter`, which is **not
    enabled at t=0** on seed 31337, so its first real caller would have
    refused on the materials bill before geometry was ever reached. It takes
    the prototype as a parameter now. A module with no caller cannot discover
    that its bill is unbuildable.

    See `docs/superpowers/notes/2026-09-05-belt-routing-first-run.md` and the
    self-fed cell's own record.

  - **`Goal::Built` / `goal.built(blueprint, anchor)`** (`method::blueprint`,
    `BuildBlock`) places a **decoded Factorio blueprint by hand**, entity by
    entity, at a fixed anchor -- the opposite of `method::connect`'s
    self-sited belt run. It is proven live up to 179 entities
    (`FurnaceLine`: 87 belt, 48 inserter, 24 furnace, 13 pole, 3 lamp, 2
    splitter, 2 underground-belt), including the underground-belt pair's
    input/output half and its direction, both read back correct off the
    live surface, and a real production curve out of the furnaces it
    built (see `docs/superpowers/notes/2026-09-05-first-block-built.md`).
    Four things by name:
    - **"Proven live" means STANDING, and for the furnaces also SMELTING.
      It does not mean anything MOVED.** 178 of 179 entities were placed
      (one transport-belt was never placed, a pathfinder miss), and all 178
      were read back off the live surface at the right tile facing the
      right way, and the furnaces smelt when fed by hand. But the plate
      count in that run is the sum of the furnaces' own `output_inventory`,
      which rose monotonically -- had the output inserters been emptying
      them it would have flattened -- and it could not have been otherwise:
      `FurnaceLine` carries 13 poles and **no generator at all**, and the
      run cheated in no power source. Power coverage is not power capacity.
      So 87 belts, 48 inserters, 2 splitters and 2 underground belts --
      **138 of 179 entities, 77% of the block** -- are proven to stand and
      have **never been shown to move a single item**.

      **What was missing was one machine, and that is now measured** (peer
      session, 2026-09-06, computed offline from the fixture with no world):
      **13 of 13 poles wired into one component, 48 of 48 inserters inside a
      pole's supply area, 639 kW of demand.** The block is *internally
      complete* — its own poles connect and cover its own consumers. It was
      never a coverage or a distribution problem, and nothing in the planner
      was at fault: **nobody ever gave it a generator.** "13 poles and no
      generator at all" meant exactly what it said.

      **That figure was 624 kW until 2026-09-07**, when `electric_energy_usage`
      began crossing the bridge. The old number counted the 48 inserters and
      priced the block's **three `small-lamp`s at nothing**, because
      `consumer_kw` had no row for a lamp and the unknown-name branch errs
      towards permitting. The 2.4% error is harmless here; **the shape of it is
      not, and it is why `BlockDemand` carries an `unpriced` set: a table's
      silence means "I have never heard of this", not "it draws nothing", and
      those were the same answer.** 17 electric consumers were missing from that
      table (`foundry` 2,500 kW, `electromagnetic-plant` 2,000, `crusher` 540,
      `rocket-silo` 250, `recycler` 180 ...), every one of them headroom that
      was not there.

      So closing it needs **generation**, not power *in* the blueprint: one
      hop from a supply anchor to any one of the block's own poles, which is
      point-to-point and is what `method::power::ensure_powered` does. A
      900 kW plant covers 639 kW with headroom.

      **And 639 kW is the PRE-ELECTRIC number.** Those furnaces burn coal; the
      draw is 48 inserters at 13 kW. Convert them to electric furnaces at
      **180 kW each** and one yellow belt's worth (24 furnaces, see the
      smelting ratios) is **4,320 kW** against a plant that tops out at 1.8 MW
      with two engines. That is the owner's *"a second boiler is usually
      needed after the electricity demands skyrocket once we start using
      electric smelters"* with arithmetic under it, and it says the plant
      ceiling binds on a block that exists rather than on a hypothetical one.
      Design for growing the plant: `docs/superpowers/notes/
      2026-09-06-one-place-that-decides-power.md`.
    - **It has no siting story, and refuses rather than guessing.** The
      block is placed at a fixed offset. Since 2026-09-05 `expand()` scans
      the whole footprint **before emitting anything** and refuses with
      `PlannerError::BlockGroundOccupied`, which names the tile and what is
      on it -- an entity by prototype name, water, terrain, ore, a
      footprint the game already refused, or a **character**, called out
      separately when it is one of this plan's own bots (cleared by
      walking, not by moving the block). Before that the fact arrived from
      `schedule()` as `PlannerError::ChainOwnerInfeasible`, blaming an
      internal scheduling decision for a fact about the ground; four runs
      across three anchors were spent distinguishing hypotheses the named
      tile answers in one line. Choosing a clear anchor, or clearing
      obstacles by script first, are still the two ways past it; siting the
      block automatically is out of scope.
    - **It refuses unrecognised tiles, recipes and module requests by
      name**, rather than silently dropping or misplacing them -- part of
      the same decode/build path, from Task 1's blueprint allowlist work.
      An entity standing on the right tile **facing the wrong way**, or the
      wrong half of an underground pair, is refused by name too: it is not
      "already built", and nothing here can rotate or remove it.
    - **Bands are balanced by entity count and cut across the block's
      LONGER axis.** 179 across 4 bots splits 45/45/45/44. The remainder is
      spread, not dumped: six across four is 2/2/1/1 (it was 2/2/2/0 under
      the old `div_ceil` chunking -- an idle bot). Two numbers this file
      used to give here were wrong: it claimed the split was "as even as an
      integer division allows", which `div_ceil` was not, and it reported a
      6-entity block as "3/4/3/0", which are **step** counts, not bands --
      the bands were 2/2/2/0. The axis was also wrong until 2026-09-05: the
      sort was by x unconditionally, so over `MinerLine` bands 0, 1 and 2
      all occupied x = 3.5 and band 0 spanned the whole height the other
      two were segments of -- three bots interleaved in a one-tile
      corridor, while the spec claimed "a bot never crosses another's
      band".

      **A third number here was wrong until 2026-09-06: `MinerLine` is not
      "4 wide, 20 tall".** Decoded from the fixture itself, it is **5 x 21**
      by position bounding box (x[1.5,5.5], y[0.5,20.5]) with its 13 drills
      in **two columns** at x = 1.5 and x = 5.5 -- so once 3x3 collision
      boxes are counted the block covers 8 tiles of width, not 4. Any
      reasoning about band splits or footprints that used 4x20 was reasoning
      about a block that does not exist.

      **The fixture is the authority, not this prose.** Decode the blueprint
      before quoting its shape; the tests do. Three documented constants in
      this file turned out wrong in a single day -- these dimensions, the
      `R + 1.1` walk margin (measured at 0.6), and the walk speed 0.15
      (measured 0.1413).

- **crates/executor**: runs a `Schedule` across bots over RCON. Per-action
  completion signals (`tokio::sync::watch`, not polling), lag edges modelling
  machine time, pre-flight wait-graph cycle rejection, and recovery tiers in
  `recover.rs`. Issues only legitimate player actions — no `cheat_*` calls.

  **A short take is finished in pieces, not failed** (`run::take_in_pieces`,
  2026-09-09). The mod's `tried to remove 10 copper-plate but removed 3` is a
  fact — the 3 are in the bot's hands — and until then it was a verdict that
  abandoned every dependent step: 48 in `run-1788920460-08860`, 237 in
  `run-1788923927-04849`, both `assembling-machine-1` placements and the
  `set_recipe` among them, in every measured run of the science cell, while
  the source went on making a plate every 240 ticks. Now the bot stands at
  the source and asks for **the remainder only** every 300 ticks of game
  time, succeeds with a note (`took 10 copper-plate in 3 pieces over 1,700
  ticks ...`, in `action_settled.error` like the destination-full note), and
  is reported as `WaitKind::Restock` while it waits. Bounded twice, in game
  ticks: nothing arriving for 1,800 ticks fails as *the source is not
  producing*; 18,000 ticks in total fails as *too slow for the budget*. The
  failure keeps the mod's sentence with the cumulative count, so the record
  still classifies it `partial_transfer` and `recover` still refuses to
  reschedule it — nothing downstream of that wording had to change. A stub
  actuator (`ShortSource`) covers both bounds and the no-clock path; **no
  offline plan can exercise this** — it lives in dispatch, so the proof is a
  run from a standing world (`--resume-from` the run above).

- **crates/scripting_lua**: Lua 5.4 bindings exposing host functions for task queuing, graph queries, and RCON commands
  - `sandbox.rs` - the restricted interpreter every user script runs in
  - `lua_docs.rs` - generates all five files in `docs/lua/src/` — build
    artifacts, gitignored, written by `app/src-tauri/build.rs` (which is the
    CLI crate's build script; the directory name is a leftover). Never edit a
    `.lua` there.
    - `{globals,world,rcon,goal}.lua` come from the `__doc_entry_*` strings in
      `globals/`; edit the Rust strings.
    - `types.lua` comes from `schema_for!` on the roots listed in
      `documented_type_schemas()`, i.e. from the `JsonSchema` derive on
      `crates/core/src/types.rs`, which reads the same serde attributes that
      decide the wire shape. It was hand-written and tracked until it had
      accumulated two outright lies and was missing the type they lied about.
    - The two halves are held together: the roots must be **exactly** the set
      of `` `types.X` `` names the four binding files mention. A `@return`
      naming a type with no root fails the build; a root no binding hands out
      fails it too. So documenting a new return type means adding
      `schema_for!(That)` — and `JsonSchema` is required transitively, so
      everything it contains is described as well.

- **crates/server**: axum HTTP server (replaces the former Rocket `crates/restapi`)
  - `webserver.rs` - router assembly, static SPA serving, `start()` entry point
  - `game/` - the `/api/v1/game/*` query and control handlers
  - OpenAPI generated with utoipa; Swagger UI at `/swagger-ui`, spec at `/openapi.json`
  - Serves the built Vue SPA from the configured `web_root` (index.html fallback); with no web root it serves the API only and redirects `/` to the docs

### Frontend (app/src/)

- Vue 3 with Tailwind v4 and headless `reka-ui` primitives. **PrimeVue and
  PrimeFlex are gone** (plan 6) — there is no component library to register, so
  `main.ts` installs only Pinia and the router. Shared primitives live in
  `components/ui/` in the shadcn style: single-word names, owned in-tree rather
  than imported, and listed in the `vue/multi-word-component-names` ignore list
  in `eslint.config.mjs` — extend that list when adding one.
- Icons come from `@lucide/vue`; `@vueuse/core` supplies the composables.
- Monaco editor for Lua scripting (`components/Editor.vue`), whose workers are
  wired through Vite's native `?worker` imports
- Pinia 4 for state management (`defineStore('id', {...})`, not the removed
  object-with-id form) and Vue Router 5 with hash history
- Vite config lives in `vite.config.mts` — `.mts` because the package is not
  `"type": "module"` and some plugins are ESM-only
- ESLint uses flat config in `app/eslint.config.mjs`; `.eslintrc.js` no longer
  works on ESLint 10. `pnpm lint` runs `tsc`, `vue-tsc` and `eslint` over both
  `.ts` and `.vue` files

The frontend/backend contract is pinned by a **snapshot seam**, and it fails
from both ends on purpose. `app/src/api/openapi.snapshot.json` is generated
from the utoipa spec (`UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p
factorio-bot-server --features lua --test openapi`), and
`app/src/api/openapi.contract.spec.ts` ties every published schema to a
declaration in `app/src/api/types.ts` via `objectContract<T>`. Adding a field
in Rust fails the Rust snapshot test until regenerated, then fails the
TypeScript contract test until mirrored. Neither half can drift quietly.

**But a THIRD place can, and did (2026-09-07).** `vitest` does **not**
type-check — it strips types and runs. So a test *helper* that constructs a
`FactorioEntity` literal can go stale when a field is added, and every test
still passes while `tsc --noEmit` fails. `MapEntities.spec.ts` sat broken on
master exactly that way after `input_inventory` and `transport_lines` landed:
`cargo test --workspace` green, `pnpm test` green, **`pnpm lint` red**.

Two consequences worth internalising. **`pnpm lint` is part of the seam, not a
style pass** — run it before calling a frontend-touching change done. And a
type error in a file `git status` reports as *clean* is not evidence of
anything: a committed file can be committed-broken. Asking "is this file
dirty?" answers a different question from "does this compile", and that
substitution has already produced one confident wrong dismissal here.

### Logging: two systems, on purpose

**Narration is `paris`, on stdout. Diagnostics are `tracing`, on stderr.**

A line the user reads *while the tool runs* is narration: progress, the
spinner, the address it bound, "press Ctrl-C to stop". A line that explains
something to whoever debugs it *later* is a diagnostic. `paris` was kept for
narration deliberately -- it has zero dependencies and does colour, glyphs and
timestamps unaided, so replacing it would add `console` plus a time crate
rather than remove anything.

Two traps here, both of which have already been paid for:

- **A `tracing` event goes nowhere unless a subscriber is installed**, and
  silently -- the macro still compiles and runs. Every `tracing::` call in this
  workspace emitted nothing until `74269106`, including two error paths, which
  went unnoticed because a `paris` line one statement away said something
  similar. The subscriber lives in `app/src-tauri/src/context.rs`.
- **`crates/core/src/lib.rs` has `#[macro_use] pub extern crate paris`**, so
  `info!`/`warn!`/`error!` resolve with no import anywhere in that crate --
  ~148 call sites name no logging system at all. Import `tracing` explicitly at
  each converted site; never grant `tracing` the same global, or a
  half-converted file compiles cleanly with no way to tell which macro a line
  called. `lib.rs` says so in place.

Colour markup (`<bright-blue>{}</>`) is `paris` syntax and renders only through
`paris`. A string moved to `tracing` must have its tags stripped or they print
literally.

### Communication Flow

1. User scripts written in Lua via Monaco editor
2. Scripts schedule goals → planners expand to task graph nodes
3. Executor assigns tasks to bots based on availability/travel time
4. RCON commands sent to Factorio via BotBridge mod
5. Entity/event data streams back to update graphs

### Script Execution (HTTP)

`POST /api/v1/scripts/execute` takes either a `path` under the scripts
directory or inline `code`, answers `202` with a `job_id`, and runs the script
detached (`crates/server/src/manage/execute.rs`, `crates/server/src/jobs.rs`).

- **One script at a time.** The job registry holds a single execution slot.
  While one script runs, a second request is refused with `409`, whose body
  carries `running_job_id` — the id of the job actually holding the slot, so a
  caller can attach to it rather than guess. The slot is taken *last*, after
  the body, the running Factorio instance and the script path have all been
  checked, so a request that was going to fail anyway never holds it.
- **Output comes over SSE, not in the response**, at
  `GET /api/v1/jobs/{id}/events`. A subscriber that attaches mid-run receives
  the backlog of lines already printed and then the live ones, so nothing is
  lost by attaching after the `202`. The stream carries *script* output only,
  not the Factorio process's stdout. `GET /api/v1/jobs/{id}` is the polling
  alternative and returns the job's status.
- **Scripts are sandboxed to the scripts directory.** The endpoint is
  unauthenticated, so this boundary is what stands between an HTTP caller and
  the host. `crates/scripting_lua/src/sandbox.rs` builds the interpreter with
  `table`, `string`, `math` and `coroutine` only — no `io`, `os` or `package`,
  with `require`, `dofile` and `loadfile` removed and `load` restricted to
  text chunks (Lua 5.4's bytecode loader does not validate untrusted input).
  That leaves `include`, `file_read`, `file_write` and `world.draw` as the
  only filesystem access, and each resolves through `resolve_script_path` /
  `resolve_write_path` (`crates/core/src/scripts.rs`), which refuse any path
  that leaves the scripts root.

### Replay and Video

**Per-camera screenshots ("frames") were retired on 2026-09-02 and removed on
2026-09-03.** The mod no longer calls `game.take_screenshot` on a beat, there is
no `frames/` directory, no `/api/v1/frames*` route, and no `ArchivedFrame`.
Video is the visual record. Do not reintroduce a screenshot cadence without
reading `docs/superpowers/notes/2026-09-02-screenshots-retired.md`: one run
wrote 2,164 JPEGs / 947 MB against 290 MB for the same 45 minutes of video, and
`take_screenshot` renders *synchronously inside the game loop* where the video
grabber reads a frame the GPU already drew.

**One diagnostic frame, taken by hand, is a different thing and it works.**
Nothing in the retirement removed `game.take_screenshot`, and the mod still
exposes it: `remote.call('botbridge', 'screenshot', {...})` forwards straight to
it. Verified 2026-09-06 to answer a question four separate measurements could
not — where ore was sitting on a belt, which no binding can read.

```bash
# while a run is holding the game open
factorio-bot rcon -s localhost --settings <settings> -- \
  "/silent-command remote.call('botbridge','screenshot',{player=game.players[1], \
   surface=game.surfaces[1], position={x,y}, resolution={1400,1400}, zoom=3, \
   path='shot.png', show_entity_info=true})"
```

Three things that cost a run each to learn:

- **A headless server cannot render.** The call reaches the game and *creates*
  `script-output/`, then writes nothing. **The empty directory is the tell** —
  there is no error anywhere.
- **A graphical client can**, and the file lands in the **client's**
  `script-output`, never the server's. So this needs `--clients 1` and
  `DISPLAY=:0` (see the Platform Notes); the first such run extracts the client,
  which is minutes.
- **The script keeps the game alive.** When it returns the server dies, so a
  script must hold the game open — a tick-wait loop at the end — for a frame to
  be taken from outside.

This is not a cadence and must not become one — see the retirement above for
why. One frame, on demand, when something is invisible.

Three unrelated things are still called "frame" and must survive a grep:
entity-map **keyframes** (`map.jsonl`, `lib/runMap.ts`), **video frames**
(`crates/core/src/record/video/`, `api/videoClock.ts`), and
`requestAnimationFrame`.

A run produces artefacts joined in the UI by `game.tick`, the only clock they
share.

- **Replay** — the executor serialises what it actually did, reaching the
  browser over the job's SSE stream (`WireEvent::Replay`), not a REST route: it
  belongs to the run that produced it, so it travels with that run's output.
  `app/src/api/replay.ts` narrows it at runtime (`parseReplay`) rather than
  casting.
- **Video** — opt-in per run (`record.start({video = true})`), filmed from a
  graphical client's window by ffmpeg and joined to the plan through
  `ticks.jsonl`, the `(tick, wall_ms)` table the viewer interpolates. It is
  *not* tick-exact: a stalled game keeps writing the last drawn image, which
  looks exactly like a game that was running and idle. The event log and
  `map.jsonl` are the tick-exact record.
- **Samples** — `samples.jsonl`: research, production, power and bot
  inventories, on 300- and 60-tick beats. These ride on the mod's *sampling
  session* (`rcon.sampling_start(run_id)` / `sampling_stop()`), which is what
  the screenshot capture used to ride on. Removing the session would take the
  whole world-state stream with it, silently — that is why it outlived the
  cameras.

**Planned ticks start at zero; observed ticks are absolute `game.tick`.** A run
dispatching its first step at tick 60,551 would otherwise draw the whole plan in
the first 1.4% of the axis. `ReplayScrubber.vue` works in shifted ticks and
converts in exactly one place — `observedOrigin()`.

**The join is checked, never assumed.** `app/src/api/runMatch.ts` compares tick
ranges and run ids and reports `contradicted | confirmed | consistent |
inconclusive`; `api/videoJoin.ts` applies it to a recording. A missing run id is
**unknown, never "no match"** — treating absence as mismatch refuses a good
join; treating it as a match asserts something nobody established.

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

### Headless character bots: `--headless`, and the two modes

**A bot is either a connected graphical client or a server-side `character`
entity, and a run is all of one or all of the other.** The mix is refused by
name, in the mod and in core, before a process is spawned.

```bash
# Iterate: four character bots, no client, world at HEADLESS_SPEED (10x)
just headless factory_stage2.lua
factorio-bot lua <script> --headless --bots 4 --game-speed 10

# Measure or film: clients, 1x, the number you quote
just bench <script>
```

Why it exists: a client costs ~26 s of sprite load each plus a connect wait
bounded at 300 s, and **`--headless` had the script running 12-13 seconds after
launch** with four bots. `--game-speed` scales `game.speed` and the executor's
wall-clock deadlines with it (`Actuator::game_speed` now reads the real value
over RCON instead of assuming 1.0).

Three things that are **not** interchangeable between the modes:

- **A character bot is honestly equipped; a `--clients 0` bot is fabricated.**
  Spawning asks freeplay for its starting items
  (`remote.call("freeplay", "get_created_items")`), so each character holds the
  8 iron plates, furnace, drill and wood a joining player gets. `--clients 0`
  has no players at all, so
  `Planner::initiate_missing_players_with_default_inventory` **synthesises** a
  roster holding wood/furnace/drill and no plates. `goal_smoke.lua` asserts a
  positive makespan for `have("iron-plate", 5)`, which is true only of the
  fabricated roster: on `--headless` the goal is already satisfied and an empty
  plan is the correct answer, not a regression.
- **No video.** It is filmed from a client window and there is none, so
  `record.start{video = true}` warns and records everything else rather than
  failing the run.
- **Provenance says which mode ran**: `bot_mode` (`clients` / `characters`) and
  `game_speed`, written at run start from `<instance>/run-mode.json`.
  `just analyse` treats a difference as a note, not a refusal, but **do not
  compare a headless run's wall timings against a 1x client run**.
  **`HEADLESS_SPEED` is 10, raised from 5 on 2026-09-07 because 5 was never
  chosen** -- it was a hard-coded constant with no recorded justification. The
  measurement that replaced it (same script, same map, four headless bots) is
  in the `justfile` beside the constant: 10x nearly halves iteration wall time
  for a +1.5% tick cost, and 20x is real and usable at +5.7% but opt-in. One
  sample per speed; take more if a decision rests on the tick cost.
- **Trigger technologies: the game fires `mine-entity` itself, the mod
  emulates `craft-item` and `build-entity`, and prerequisites gate both.**
  Factorio 2.0 unlocks 32 technologies by *doing* — `automation-science-pack`
  by crafting one lab, `electronics` by 10 copper plates, `steam-power` by 50
  iron plates. All 32 were enumerated from a live server and each kind was
  tested against a server-side character on 2026-09-05
  (`docs/superpowers/notes/2026-09-05-research-triggers.md`):
  - **`mine-entity` fires with no player at all** — a character bot chopping
    a rock through `action_start_mining`, a fuelled burner drill and a
    powered pumpjack each completed their technology. **Do not emulate it.**
  - **`craft-item` fires for machine output and NOT for a hand craft**: with
    the sweep switched off, a furnace's tenth copper plate earned
    `electronics` by itself within ~400 ticks, while a lab hand-crafted
    through `action_start_crafting` left `automation-science-pack` open for
    4,000 ticks. `emulate_research_triggers` completes it when the force has
    **already** produced what the trigger names, every 60 ticks, only while
    character bots exist, writing a `research_trigger_emulated` event with
    the counts. Two counters, **the larger taken, never the sum**: machine
    production (the force's statistics -- redundant with the game, and only
    ever ahead of it by under 400 ticks) and hand crafts
    (`storage.crafted_tally`), because **a hand craft does not appear in
    production statistics at all**. A 60-tick sweep beats the game's own
    check, which is how the first measurement misread the furnace row --
    switch the sweep off before concluding what the game does alone.
  - **`build-entity` does not either** — `surface.create_entity{force=…}`,
    which is every placement the mod makes, fires nothing with or without
    `raise_built`. Emulated from `storage.built_tally`, filled in
    `on_some_entity_created` for entities on the player force. The one
    shipped use is `space-science-pack` (an asteroid collector), which
    `can_place_entity` refuses on Nauvis even as a ghost, so the stub tests
    are its only proof; no live run can cross it here.
  - **Prerequisites gate the trigger and the act is remembered**: a rock
    mined with `planet-discovery-vulcanus` open earned nothing; a stromatolite
    mined *before* `planet-discovery-gleba` earned `heating-tower` right after
    it. The sweep now skips a technology with an open prerequisite, which is
    what stops `automation-science-pack` completing in the same sweep as, or
    before, the two plate triggers it depends on.
  - **`capture-spawner` and `create-space-platform` are refused by name** in
    the planner (`UnsupportedResearchTrigger { act }`): no action in this
    project performs the act, so an emulation would be a grant.
  `remote.call("botbridge", "set_research_trigger_emulation", false)`
  switches the sweep off for a measurement and records that it did.
- **Eight bots have been run (2026-09-05), nothing above.** Eight characters
  reach automation in one plan in ~5:26 and green in one plan in 19:58 at
  5x on seed 31337 — but green's 693-action plan of 50,665 ticks executed
  in 71,936 (1.42×, against 1.18× for four bots), so the plan is shorter
  and the run is not: contention between bots on the ground is the open
  cost. `PlayerId` is `u8`, so 255 is the arithmetic ceiling; before that,
  the mod polls **every bot every tick** (whole main inventory, sorted
  signature, crafting-queue scan) and every action is its own RCON round
  trip, so bot count is what costs tick rate.

  **Measured 2026-09-06: a bot costs ~16.3 us per tick, and at eight bots that
  is 42% of all tick time.** `tools/measure_tick_cost.sh` with
  `scripts/tickrate.lua`, idle roster, release build, seed 31337:

  | roster | tps | us/tick |
  |---|---|---|
  | 1 | 5,145 | 194 |
  | 4 | 4,147 | 241 |
  | 8 | 3,242 | 308 |

  Fit: **178 us base + 16.3 us per bot per tick**, with the two intervals
  agreeing to 8% (15.6 and 16.8 us/bot), so the relationship is linear rather
  than two points and a hope.

  **Taken on a quiet box, and the number is only worth its conditions**:
  `load_start=1.82`, `load_end=4.01` on 20 cores, with the harness refusing
  outright above load 6 and waiting up to 30 minutes for the box to drop below
  cores/4. **This is one of the few measurements here that a busy tree can
  move** — game speed and tick rate are wall-clock quantities, so there is no
  tick-bounded form of the question. Re-run it only on a quiet box, and state
  the load beside any new figure.

  **The old figure here — "eight bots still held 242 of 300 requested tps" —
  had no baseline**, so it said where eight bots ended up and could not say what
  one costs. Anything quoted as a per-bot cost needs a 1/4/8 sweep behind it.

  Three things that make the number readable, each of which was wrong in an
  earlier attempt: startup is cancelled by **differencing two tick spans**
  (60k and 180k) rather than estimated, because a single-span run was 17.5s
  wall of which ~16s was server start; the probe's own RCON polling is in the
  **base and not the slope**, verified by poll counts being identical across
  rosters (12,000 short / 36,002 long for all three); and the roster is read
  from `rcon.players()` and asserted, never taken from the `--bots` flag.

  **Scale it before acting on it.** 130 us at eight bots is under 1% of a 60 Hz
  tick, so this is invisible at 1x and only bites headless at high speed, where
  the tick budget is whatever the CPU can do. That is the regime this project
  iterates in, so it is worth fixing — as a throughput optimisation for our own
  loop, not as a correctness or playability problem. Parallel *runs* are the cheap axis: two headless
  instances on their own ports and workspaces each held ~220 tps and
  finished in the wall time of one (`docs/superpowers/notes/
  2026-09-05-headless-experiments.md`).

### Important Timing Considerations

- **Archive extraction**: 8-10 minutes per client instance on first setup (macOS DMG extraction)
- **Server startup**: ~12-17 seconds to initialize and be ready for connections
- **Client loading**: ~26 seconds per client to load sprites before connecting
- **Connection wait**: polled every second, bounded by 300 s of *no progress*
  (`CONNECT_STALL_TIMEOUT`) -- see Expected Behavior step 4 below for why the
  bound is on progress rather than on total time
- **Total time**:
  - First run with new clients: 15-20 minutes (due to archive extraction)
  - Subsequent runs: 120-180 seconds for multi-client tests (2-4 clients)

### Running Multi-Client Tests

```bash
# Iterating? Use a DEBUG build. Timing a run? Use --release.
#
# This is not a preference, the two builds behave differently:
#
#   debug    workspace/mods/BotBridge is a SYMLINK to the repo's
#            mods/BotBridge, created or repaired on every setup, so an edit
#            to mods/BotBridge/control.lua is what the next run loads. There
#            is nothing to refresh and no copy to go stale. workspace/mods
#            itself stays a real directory: the other mods live there, and
#            Factorio rewrites mod-list.json and mod-settings.dat in it.
#            Do NOT "delete workspace/mods to re-seed" -- every instance's
#            mods dir is a symlink to it, so deleting it leaves the server
#            with no bridge mod: it hangs at `start waiting` forever after
#            writing a level.zip with no bridge state, which poisons every
#            later run (Factorio only migrates on a version bump and
#            info.json is pinned at 0.0.1).
#            Setup also re-enables BotBridge in workspace/mods/mod-list.json
#            if a previous run dropped or disabled it -- a mod present on
#            disk but absent from an existing mod-list.json is a DISABLED
#            mod, and that failure is completely silent.
#            Every run -- either build, no flag needed, and NOT gated on
#            --verbose any more -- logs one line naming which directory won
#            and what BotBridge is: "Using mods directory <absolute path>
#            (<why>)". That line, not a guess from a traceback, is the
#            authoritative answer to "did my edit ship". It used to be
#            suppressed by `silent` (which every CLI path sets unless you
#            pass --verbose) and so printed on no run at all.
#            Scripts: a name resolves against <workspace>/scripts ONLY, never
#            the CWD (`crates/core/src/scripts.rs::ensure_scripts_dir`, the
#            single entry point; the CWD-probing `scripts_dir` is deleted).
#            A missing OR EMPTY workspace/scripts/ is seeded by COPYING the
#            checkout's scripts/ -- a copy, not a symlink, so a script's
#            file_write / world.dump land in the workspace and not the repo.
#            A populated one is left alone and only warned about when stale,
#            so an edit to scripts/foo.lua in the repo does NOT run until you
#            copy it over. Every run logs "Using scripts directory <absolute
#            path> (<why>)", once, not gated on --verbose, beside the mods line.
#            Data dir is ~/.local/share/factorio-bot-dev/
#   release  mods and scripts are include_dir!-embedded into the binary at
#            COMPILE TIME and extracted once into the workspace. Editing
#            mods/ or scripts/ has NO effect until you rebuild -- and no
#            effect at all on an existing workspace, because extraction is
#            skipped when the directory already exists. There is no symlink
#            here and there must not be: a release binary has no checkout to
#            point at. FACTORIO_BOT_REFRESH_MODS=1 refreshes the workspace
#            copy from the embedded snapshot; it is a no-op in a debug build,
#            which has no snapshot.
#
#            **BUT THE TWO PROFILES SHARE ONE WORKSPACE, and only one of
#            them can repair it.** `link_bridge_mod` is
#            `#[cfg(debug_assertions)]`, so a DEBUG run repoints
#            workspace/mods/BotBridge at its own checkout every setup --
#            correct for that run, and it silently fixes whatever the last
#            run left. A RELEASE run has no repair path at all and skips
#            extraction when workspace/mods exists, so it hands Factorio
#            whatever is there. The dangerous sequence follows:
#              1. a debug run from .worktrees/x points the link into x;
#              2. x is removed when its branch merges -- `git worktree
#                 remove` succeeds cleanly and warns about nothing;
#              3. the next RELEASE run hangs at `start waiting` forever,
#                 having written a level.zip with no bridge state that
#                 poisons the workspace.
#            **The drift is created by the profile that self-heals and paid
#            for by the profile that cannot**, which is why nobody catches
#            it by reasoning about their own habits: the run that creates it
#            is never the run that suffers. Release is what every measured
#            run uses.
#            If you launch from a worktree, restore the link afterwards:
#              ln -sfn <repo>/mods/BotBridge <repo>/workspace/mods/BotBridge
#            The main checkout cannot be removed underneath a run; a
#            worktree can. Happened twice in two days.
#            Data dir is ~/.local/share/factorio-bot/
#
# Editing the mod against a release build costs two confusing runs. Ask.
cargo build --no-default-features --features cli,lua            # debug: iterating
cargo build --release --no-default-features --features cli,lua  # release: timing

# FAST PLANNING LOOP: no graphical client, no 90s connect wait.
# --clients is how many Factorio processes to spawn; --bots is how many bots
# to plan for. They used to be one flag, which made `-c 0` plan for zero bots
# and `-c 1` demand a display. Planning only needs bots.
#
# **It still starts a SERVER.** `--clients 0` means no graphical *client*; the
# run goes through the same "Factorio started, running script..." path as any
# other, so a script under it can talk RCON and mutate a live game. That is
# usually what you want -- it is why `initiate_missing_players_with_default_
# inventory` exists on this path -- but it is NOT the process-free option, and
# reading this heading as "no game at all" has already sent one agent looking
# for a planning path that does not spawn Factorio.
#
# For genuinely process-free planning -- no Factorio, no RCON, no workspace --
# use `plan` against a dumped world (see "Evaluate a planner change OFFLINE
# first" above): `factorio-bot plan --world <dump> --goal ...`.
factorio-bot lua goal_smoke.lua --clients 0 --bots 4

# Run multi-client test with adequate timeout (180s recommended)
#
# The script name is resolved against the workspace scripts directory
# (`<workspace_path>/scripts`), NOT against the current working directory, so
# pass a bare name and run from anywhere. An absolute path does NOT work: the
# leading `/` is stripped and the rest is joined onto the scripts root, so
# `/home/me/scripts/foo.lua` is looked up as `<scripts_root>/home/me/scripts/foo.lua`
# and reported as not found.
timeout 180 target/release/factorio-bot lua <script_name>.lua -c <num_clients>

# Example: Test with 2 clients
timeout 180 target/release/factorio-bot lua multi_client_test.lua -c 2
```

### Expected Behavior

1. **Server starts** and outputs "waiting finished" when ready (~12-17s)
2. **Clients spawn** as graphical Factorio windows (not headless)
3. **Clients load** sprites and resources (~26s per client)
4. **Wait loop** polls `rcon_players()` every 1 second. The bound is
   `CONNECT_STALL_TIMEOUT` (`crates/core/src/process/connect_wait.rs`), **300
   seconds of NO PROGRESS** -- it restarts every time another client appears,
   so a trickle of arrivals cannot trip it. Raised from 90s on 2026-09-03
   after two runs on a loaded machine stalled at 0/4 while every client did
   eventually connect (one reached `Factorio initialised` at 123.5s).
5. **A stall does NOT abort the run -- it proceeds with whoever showed up**,
   logging `Gave up waiting for clients ... The run continues without them`.
   That degrades silently to a one-bot roster and a *different plan*. **Check
   `plan_created.bots == [1,2,3,4]` before comparing a run to anything.** A
   one-bot run misread as a four-bot regression cost a good commit a revert.
6. **Script runs** - clients should be connected by this point
7. **Multi-bot coordination** verified via task graph execution

### Measuring a run, and the traps that have produced wrong answers

**Every wrong conclusion drawn on 2026-09-03 came from a measurement, not from
carelessness.** Read this before quoting a number.

**A measurement an iteration cap or a wall clock can move is a broken
instrument, not a scheduling problem.** Bound a window in **game ticks** and it
is immune to whatever else the box is doing — starvation only makes the wall
clock longer, and the run still covers the ticks you asked for. Two failures on
2026-09-06 came from ignoring this, both mine:

- **`for _ = 1, 5000` polls is not a duration.** On a faster box more game ticks
  pass per RCON round trip, so the same poll count covers more game time. A
  block compared this way was published at **4.6x** and re-measured at **2.6x**
  on a fixed 6,300-tick window.
- **A terminal value against a non-terminal one is not a comparison.** Two runs
  had plateaued — they had stopped producing — while the third was still
  climbing when its cap closed. One number was finished and the other was not.
- **And a shared mutable substrate is the same fault.** The first re-measure
  built three variants on **one map in sequence**, so the second was sited on
  ground the first had changed. One fresh map per variant.

The exception is real and narrow: **anything whose subject IS the wall clock**
— game-speed choice, tick rate, per-bot cost — has no tick-bounded form and does
need a quiet box. That is a small set, and naming it is what stops every other
measurement being pessimised into serial execution.

**Use `just analyse` (`tools/run_analysis.py`).** It reports milestone spans in
game time, per-verb dispatch->settle ticks, `steps/bot`, `planned ticks/bot`,
per-bot failed walks, frozen-position detection, repeated refused destinations,
sample coverage, per-network power and per-machine status. It exists because
the ad-hoc one-liners that produced those wrong answers were unrepeatable.

**The first number is the production curve; the milestone tick is the second,
and the record carries both** (owner rule, 2026-09-05: "prioritize production
rates at given times over raw run time"). `just analyse` opens with cumulative
production and /min at fixed game-time marks (5/10/15/20/25/30 min from
`run_started`, `--marks` to change) plus a plateau detector, and the milestone
spans follow as a peer section; the headline line carries both (`rates: iron
32->57->43 /min at 5/10/15; ... | milestone 3 satisfied at 17:20`). Judge a
`producing:`/rate goal on the curve and a `researched:`/first-event goal on
the tick it flipped. Marks are game time, so a headless run at any
`--game-speed` and a 1x client run are comparable *on rates* (not on wall
time). `--rates-md` /
`tools/rates_table.py` print the record's table -- generate it, do not type
it. **Known limit as of 2026-09-05: production plateaus at the plan's bill.**
In runs 13-15 iron stops at ~670 around minute 15 and red packs at 85, then
nothing grows until the run ends: the cell makes what the plan asked for and no
more, so a later mark measures the bill, not the factory.

**A RISING PRODUCTION CURVE IS NOT EVIDENCE OF A WORKING FACTORY.**
`production.made` counts what a *machine* produced, and a stone furnace a bot
walked to and hand-loaded is a machine. A peer session's 179-entity furnace
line was reported as smelting and had **no generator at all**: its 48 inserters
and 87 belts had never moved an item, and every plate came from a bot carrying
ore and coal in by hand. So the curve alone answers "did output rise", never
"did a factory make it".

**The attribution line under the table is what says who earned it.** Since
2026-09-05 every mark interval reports the roster's busy %, the count of
feeding-verb dispatches (`insert`/`stock`/`charge`/`fuel`/`take`/`mine` -- the
count, because five of those six settle in the tick they dispatch and their
share of *ticks* is ~0), the kW generated and drawn, and how many machine
readings were `working` split into electric and burner producers. From those
comes a verdict per item and interval: `roster-fed`, `factory`, or `unclear` --
and `unclear` is said freely, because a wrong confident label is worse than an
honest one. The headline carries it: `rates: iron 40->73->22 /min at 5/10/15
(mostly roster-fed; no generator until 8:26)`. A plateau now says which kind it
is -- input ran out, or the factory stopped -- from the machine statuses after
it (`no_ingredients`/`no_fuel` versus `no_power`) and whether anybody was still
feeding.

**Our green runs are roster-fed, and the record now says so.** Runs 14, 16, 17
and the headless run all read `roster-fed` at 5 and 10 minutes (and at 15
except in run 17, where it is `unclear`): no
generation at all for the first 8-10 minutes, and after that the 120 kW drawn
went to a lab and a steam engine while every machine that made an item was a
`stone-furnace` or a `burner-mining-drill` a bot hand-loaded. The only intervals
that are not `roster-fed` are the last ones, where `assembling-machine-1` is
working *and* bots are still feeding: `unclear`, honestly. The green milestone's
witness -- 5 packs into the chest in 90 s with every bot idle -- remains the one
place the runs prove automation, and it covers the green cell only. Quote
"N plates/min" only with the attribution beside it.

- **Game time, not wall clock.** `roster ready -> SATISFIED` on the clock
  includes client load and startup. One run read as 24.4 min on the clock and
  21.4 min of game time.
- **`walk_dispatched` has NO `id` field.** Joining it to `walk_settled` on `id`
  makes Python key everything under `None` and attribute every failure to the
  last dispatch. **`walk_settled` carries `bot` directly -- read it.** This
  produced "all twelve failed walks were bot 1's" when bot 1 had *zero*.
- **A verb histogram cannot see waiting.** Verbs that settle in their dispatch
  tick (`place`, `insert`, `fuel`, `take`) contribute 0, and *idle* time appears
  nowhere. "88% of the run is hand-mining" was 88% of the *timed* verbs, while
  walking was 31.8% of the milestone and 39.1% was one bot waiting on a furnace.
  Ask what a number cannot see before acting on it.
- **Runs are only comparable if the map is.** Two runs 13 hours and ~20 commits
  apart, on different maps (one had a resource 100 tiles east the other lacked)
  produced "four bots do double the work of one". Retracted. `factorio-bot lua`
  takes **`--seed`** -- but read the next section before trusting it, because
  **`--seed` alone does nothing at all on a workspace that already has a map.**
- **Step counts, furnace counts and plan sizes move for reasons that are not
  progress.** `steps/bot` has been the number that mattered all along; it sat at
  `{1: 103, 2: 4, 3: 4, 4: 4}` while two whole classes of defect were fixed and
  the headline time did not move.

### Reproducible runs: the seed, and the trap in it

**`--seed` is ignored, silently, whenever `workspace/server/saves/level.zip`
already exists.** This was measured, not assumed. The seed only ever reaches
Factorio as `--map-gen-seed` on a `--create` invocation
(`crates/core/src/process/instance_setup.rs`), and `--create` runs only inside
`if !saves_level_path.exists()`. The single thing that deletes that file is
`recreate_save`, i.e. the `--new` / `-n` flag, which defaults to false. So:

```bash
# WRONG. Prints nothing, generates nothing, runs on whatever map was there.
factorio-bot lua factory_stage2.lua -c 4 --seed 31337

# RIGHT. --new deletes level.zip, so the seeded map is actually generated.
# DESTRUCTIVE: the previous map, and everything built on it, is gone.
factorio-bot lua factory_stage2.lua -c 4 --seed 31337 --new
```

This is worse than not passing a seed, because the run *looks* controlled. It
now prints a loud `--seed was IGNORED` warning that is deliberately **not**
gated on `silent` -- every CLI path sets `silent`, which is exactly how the
`Using mods directory` line came to print on no run at all.

Two further paths accept `--seed` and can never use it: `lua --connect` (never
starts a server) and `--server <host>` (never sets one up).

**A seed reproduces a map ONLY UNDER FIXED SETTINGS.** A map is noise-generated
from the seed *plus* the map-gen settings — resource frequency, size and
richness, water (a per-resource knob like any other), trees, cliffs — so the
same seed under different settings, or on a Factorio version whose defaults
moved, is a different map. A map-exchange string encodes seed and settings
together, which is why it is the complete identity.

Owner, 2026-09-06: *"its not that bad, a seed also uniquely regenerates a map as
long as we keep the default settings."* **Every run this project has made is on
empty settings — Factorio's defaults for the installed version — so the archived
record is reproducible as it stands**, on the seed plus the game version. What
is on disk (checked 2026-09-06):

```
~/.local/share/factorio-bot{,-dev}/AppSettings.toml   map_exchange_string = ""
workspace/server/map-exchange-string.txt              ABSENT
workspace/blocks/server/map-exchange-string.txt       ABSENT
workspace/*/server/map-gen-seed.txt                   31337
workspace/runs/*/provenance.json                      map_exchange_string: null (20/20)
```

**The thing that could break that condition was in the repo, and it was live.**
Until 2026-09-06 both `crates/core/src/data/AppSettings.toml` **and**
`FactorioSettings::default()` shipped a 719-character exchange string dating to
the project's first commit (`0bb136c6`, Feb 2021, Factorio 1.x). It is not
inert: `factorio-bot lua` and `start` take the string only from `--map` and
never from settings — which is why no run ever applied it — but the **REPL**
`start` and the **REST API** `POST /api/v1/instance/start` both fall back to the
setting, and `setup_factorio_instance` then parses it into
`map-gen-settings.json` / `map-settings.json` and hands both to
`factorio --create`. A fresh setup through either path would have been silently
switched off defaults and generated a *different map on seed 31337*, and every
timing taken on it would read as a regression with the seed matching. **Both now
ship empty**, with the reason beside each; the string is in
`crates/core/src/settings.rs`'s git history.

The general shape is worth keeping: `power.rs`'s water constants and the
distance columns in the seed-scan table below are all tuned against **one
settings profile**. Under defaults that is consistent. It only matters the day
someone turns a knob — and the shipped string was exactly a knob turned without
telling anyone. **Quote the seed, the game version, and "default settings"**;
under defaults that triple is sufficient.

The plumbing used to be one-way — `rcon.parse_map_exchange_string` and the mod's
`rcon_parse_map_exchange_string` consumed a string and nothing produced one.
`rcon_map_exchange_string` (mod) / `FactorioRcon::map_exchange_string` (Rust)
now do, and `provenance.map_exchange_string` is asked of the running game at run
start. That **records** the default-settings condition rather than establishing
it. **Not yet live-confirmed**: no run has written a non-null value, so the
first one that does is the confirmation. `None` there still means *not
captured*, never "defaults" and never an empty string. See
`docs/superpowers/notes/2026-09-06-a-map-is-a-seed-plus-settings.md`.

**The benchmark seed is `31337`**, chosen by an owner decision on 2026-09-04:
*"lets do the seed search, like i said we don't need a perfect/optimal one,
just a reasonable one where everything is close to the start."*

It was picked by scanning **16 arbitrary seeds** (`20260903 20260904 1 2 3 42
100 777 1234 4242 9001 12345 31337 65535 99999 123456`) with
`factorio-bot score-map`. **All 16 were `Viable`**; `31337` won on both metrics
at once.

| | previously used (unidentified) | `20260903` | **`31337`** |
|---|---|---|---|
| iron ore | 40.4 | 26.3 | **18.4** |
| copper ore | 58.3 | 44.1 | 54.9 |
| coal | 54.0 | 44.5 | **32.1** |
| stone | 32.7 | 35.4 | 33.3 |
| water | 46.7 | 37.0 | 48.1 |
| walk score | 1550 | 1250 | **1246** |
| planned makespan | 30,077 | 31,764 | **29,000 (8:03)** |

Fingerprint `c161fa3f437221d0`. `20260903` — the seed this file previously
named, chosen for being a date rather than a map — came **9th of 16**.

**The argument this file used to make against choosing a seed still stands, and
is now a caveat rather than a policy.** A map with ore near spawn flatters every
timing and makes it less comparable to the ~9-minute manual solo baseline, which
was not run on an optimised map. So: quote the seed with every number, and do
not read a `31337` time as beating a baseline set elsewhere. What changed is
that a *reproducible* map is worth more than an unbiased one — every result
before 2026-09-04 was measured on a map nobody can regenerate, because `--seed`
was silently ignored until `61ec7364`.

This was a scan, not a search: 16 seeds, ranked, first acceptable winner taken.
It is not an optimum and no one should describe it as one.

`just bench <script>` is that run, and **it was executed for the first time on
2026-09-06** (`run-1788693799-62453`). Two results from that first execution.

**The seed is validated.** The map generated, the run reached its milestone,
the delivered tick rate was 60 of 60 nominal, and there were zero failed and
zero lost actions. The shoreline concern that motivated the caveat -- a map
whose nearest water does not fit a pump/boiler/engine has genuinely refused a
run here (`the nearest water is 66.7 tiles away`) -- did not materialise on
31337.

**And the recipe was measuring one bot.** `--clients` defaults to `1`
(`app/src-tauri/src/cli/lua.rs`) and the recipe passed no client count, so
`just bench` ran a **one-bot roster** while reading as the project's benchmark:
`roster: [1]`, `plan_created.bots = [1]`, automation at 7:22 against the 6:05
four-bot record. Nothing stalled and no client failed to connect -- one client
was all that was ever requested, so the run was correct and the expectation was
not.

Fixed by `BENCH_CLIENTS := "4"` in the justfile, stated there rather than
inherited from a default. **A benchmark must state its roster**, because the
roster is half of what the number means; this repo already warns about the same
confusion from the other direction, where "a one-bot run misread as a four-bot
regression cost a good commit a revert". Check `plan_created.bots` on the next
bench run too -- the recipe now asks for four, and asking is not the same as
getting.

**What a run now records about its map** (`crates/core/src/record/provenance.rs`):
`provenance.json`, written at run *start* rather than at finish. That timing is
the point -- `manifest.json` is written only in `finish()`, and 9 of the 24
archived runs have none, which are precisely the killed runs whose identity
somebody later needs. It carries the seed, the map-exchange string, the game
version, the git commit **and whether the tree was dirty**, the build profile,
the requested roster, and a `map` fingerprint.

That fingerprint (`EntityGraph::resource_fingerprint`) is the only identity
available for a map whose seed nobody wrote down, which is every map used so
far. **Its two answers are not symmetrical: equal digests mean the same map, a
different digest means unknown**, because the resource table holds *charted*
tiles and charting grows as bots explore.

**None of this can be backfilled.** No archived run recorded a seed, and the mod
never read `map_gen_settings`, so the maps behind every timing quoted in the
2026-09-03 notes are permanently unidentifiable.

### Silence is not success

Five separate mechanisms have been found reporting nothing while broken. When
adding any check, ask what a reader sees when it *fails*, and prefer a record
entry over a log line:

- **A STALE BINARY drops a newly-added field in silence, and it reads exactly
  like "the game does not report it"** (2026-09-07). A session added
  `entity.status` to the mod, then read it back as `nil` for every furnace. The
  mod was correct; the **release binary predated the field by 89 minutes**, so
  `FactorioEntity` had nowhere to put it and serde discarded it without a
  word. **The tell is that the field is UNIFORMLY absent rather than sometimes
  absent** — a real "the game does not know" is almost never perfectly
  uniform. Cousin of the stale-mod trap in the debug-vs-release build note
  under "Running Multi-Client Tests" above, and it bites from the opposite
  side: there, the binary is new and the mod is old. Rebuild before concluding
  anything about a field added in the same session.

  **And rebuilding is not always enough — `include_dir!` has no
  `rerun-if-changed`.** A release binary embeds `mods/` at compile time and
  **cargo does not know that editing a `.lua` file should invalidate it**, so a
  rebuild after a mod edit can legitimately produce a binary carrying the *old*
  mod. The field then reads uniformly absent exactly as if the game never sent
  it: the tell fires correctly and points at the wrong cause. Cost an agent a
  false conclusion on 2026-09-07.

  **Interrogate the binary, not the game:**

  ```
  strings -a target/release/factorio-bot | grep get_max_wire_distance
  ```

  Zero hits means the edit is not in there. Touching any `crates/core` source
  file forces the re-embed.

  **And the same family bites a falsification harness, producing a false RED.**
  `shutil.copy2` (and `cp -p`) **preserve mtime**, so restoring a mutated file
  leaves it *older* than the artifact cargo built from the mutation — cargo
  then sees nothing to rebuild and **re-runs the mutated binary against restored
  source**. On 2026-09-07 that reported two entity-graph tests failing over a
  `git status`-clean tree. `touch` the restored file and re-run before believing
  any red that appears after a mutation sweep.

- the `Using mods directory` line was gated behind `if !silent`, which every CLI
  path sets, so the authoritative "did my edit ship" answer printed on no run;
- 19 of 20 walk failures were archived as `kind: "other"` because
  `classify_walk_failure` did not know the wording this build emits;
- archived samples stopped at the last closed milestone while the mod sampled
  the whole run -- one run lost 199,449 ticks, the entire window its cell
  existed in, and it was only recovered from the mod's live file;
- a batch's events are written only **after** `goal.run` returns, so while one
  is in flight the record cannot tell "working" from "stalled". This one bit
  hard: a run that was executing normally -- 44,846 ticks of progress past its
  last recorded event, bot inventories climbing across consecutive samples --
  was read as a two-hour hang from the record's silence and **killed**. It had
  been running 13m45s, and an earlier batch of the same run had already gone
  ~18 minutes silent without incident. `EventKind::BatchProgress` (`83c42346`)
  now beats every 30 s with counters and **no verdict**; `just analyse`
  distinguishes executed / never-dispatched / killed-mid-batch / **UNKNOWN**,
  and says explicitly when absence "is NOT evidence of a stall".
  **9 of 24 archived runs end on a `plan_created` with nothing after it**; only
  the newest can ever be cross-checked, because the mod's live sample file is
  overwritten per run.

### Working alongside other agents

- **`git commit -- <path>` takes the WHOLE file.** If another agent has
  uncommitted work in a file you need, committing yours ships theirs under your
  message. Land one, then the other -- or hand over a patch. Two agents have
  had to stop for this.
- **AN EXPERIMENT WHOSE APPARATUS LIVES ONLY IN A WORKTREE DIES WITH IT.**
  Removing merged worktrees is correct disk hygiene and this file recommends
  it — but on 2026-09-08 it destroyed the reproducibility of three published
  results at once. The `ElectricOreToPlate` block is cited in four notes and a
  `power.rs` doc comment; `git log --all -S ElectricOreToPlate -- '"'"'*.lua'"'"'` is
  **empty**. Its blueprint string and its loading script only ever existed
  inline in an ad-hoc script in a worktree that was later removed, so the block
  cannot now be decoded, re-run, or fixed by anybody, and the numbers taken from
  it cannot be re-derived.

  Worse, the numbers were **wrong in a way nobody could check**: the note records
  `chests loaded by hand: iron-ore x96, coal x50 (each of two)` — both
  commodities into both chests — which is precisely the mixed-lane control
  measured on 2026-09-06 and written up thirty lines above in this file (coal
  drains, ore never moves, one plate). It was read as a fixture defect for a day.

  **So: commit the apparatus, or copy it aside before `git worktree remove`.**
  A blueprint string, the script that loaded it, and the loading *order* are
  part of the result, not scaffolding around it. A worktree's `scratch/` is
  gitignored on purpose — that is a reason to lift what matters out of it, not a
  reason to let it go. Cheap rule: if a note quotes a number, the thing that
  produced the number must be reachable from `master`.

- **Develop in a throwaway worktree** (`git worktree add .worktrees/x HEAD` --
  inside the repo, not `/tmp`) when a live run holds
  `target/debug/factorio-bot`.

  **Be precise about why, because the obvious statement of it is wrong.**
  Linux does not lock a running binary the way Windows does -- you can
  `unlink` it or `rename` over it while it executes, and that succeeds. What
  fails is an **in-place write** to a running ELF image: `ETXTBSY`, reported
  as `Text file busy`. Verified here rather than assumed:

  ```
  cp $(command -v python3) ./t && chmod 755 ./t
  ./t -c 'import time; time.sleep(20)' &
  cp $(command -v python3) ./t   # cp: cannot create regular file './t': Text file busy
  mv other ./t                   # succeeds
  ```

  So a build that links straight to the output path trips over a live run,
  and one that writes a temp and renames does not. Copying the binary aside
  and running the copy sidesteps it either way -- `tools/seed_search.sh`
  honours a `BIN` override for exactly that.
- **A worktree run redirects `workspace/mods/BotBridge` at that worktree's
  copy.** A run launched from `/tmp/x` loaded `/tmp/x/mods/BotBridge` and
  produced no machine samples even though the feature had landed in the main
  checkout. Restore the symlink afterwards and `readlink` it before trusting a
  run's mod-dependent data.

### Known Issues & Workarounds

- **Timeout warnings**: Clients may take 90+ seconds to connect on first run. The warning is informational - the script will still run successfully once clients connect.
- **macOS GUI processes**: Clients must use `Stdio::null()` for stdin/stdout/stderr, otherwise GUI windows fail to render.
- **Lock file conflicts**: Server and clients each need separate `--config` paths pointing to instance-specific `config.ini` files.
- **JSON parsing**: BotBridge's `helpers.table_to_json({})` returns `"{}"` for empty tables, not `"[]"`. The Rust RCON client handles both cases.
- **A graphical client works from the dev shell; you only need `DISPLAY`.**
  `flake.nix` now supplies `SDL_VIDEODRIVER=x11` and the X11/GL libraries, so
  `DISPLAY=:0 workspace/client1/bin/x64/factorio` reaches
  `Initialised OpenGL … Factorio initialised`.

  **If you are debugging this anyway, the error message lies about the cause.**
  `No available video device` says nothing about what is missing, and an
  earlier version of this note diagnosed it as `/run/opengl-driver/lib` being
  masked. That was incomplete. The real chain, each step visible only after
  fixing the one before it:

  1. SDL picks the **wayland** backend on a wayland session (Hyprland here) and
     fails. `SDL_VIDEODRIVER=x11` uses Xwayland and surfaces the next error
     instead of hiding it.
  2. Then `x11 not available` — no X11 **client** libraries in the shell
     (`libXrandr.so.2` first).
  3. Then `Failed loading libGL.so.1`, which is **not** in
     `/run/opengl-driver/lib`: that path holds the Mesa *driver*; `libglvnd`
     holds the GL dispatch library. Both are needed, driver path first, since
     Mesa must match the running kernel.

  Audio still fails (`libasound.so.2`) unless `alsa-lib` is present; that is a
  warning, not a failure, and `flake.nix` includes it anyway to keep the log
  readable.
- **Nothing printed inside an RCON-invoked mod function reaches stdout — so
  `writeout` does not work there either.** Factorio redirects console output to
  the RCON client while a command is in flight, and `writeout` is exactly one
  `print`. So a function reached through `remote.call` from an RCON command
  **cannot put anything into the planner's world model**: its output goes into
  the reply body, which the executor reads as the action's result and nothing
  else ever sees.

  This entry used to say "do not debug with `rcon.print`, use `writeout`
  instead", which is right about `rcon.print` and **wrong about the remedy** —
  both are the same channel in this context. That advice cost a ghost-streaming
  fix that read correctly, passed five tests, and did nothing.

  **Proven by a control, not by inspection** (2026-09-06): a plain non-ghost
  record — a `stone-furnace` at a fixed position — written on the same channel
  from the same loop in the same call also never arrived. One run:

      entries in the RCON reply body:   9
      ghosts standing in the GAME:      9
      ghosts visible to the PLANNER:    0
      non-ghost control record:         0   <- the one that settles it

  That eliminates every hypothesis about the *record* at once — shape,
  `ghost_name`, the emitting filter, the graph's whitelist, the bounding box.
  It is the channel.

  **Event-handler writeouts work fine**, in the same process and the same run:
  the exploration census grows from `on_chunk_generated`. So the channel is
  healthy in general and absent only from RCON context. **If you need something
  an RCON-invoked function creates to reach the world model, have the mod notice
  it from an event it already receives** — do not push harder from inside the
  call. Deferring to `todo_next_tick_other` was tried and also produced nothing;
  why is not established (one unverified candidate: it is drained in an
  `elseif`, so a non-empty `todo_next_tick` starves it).
- **An inserter's `direction` points at the side it PICKS UP from**, not the
  side it drops into. Established empirically (chest / burner-inserter / chest,
  then machine / inserter / chest): `direction = 12` ("west") is what moves
  items *west to east*. Getting this backwards produces a layout that places
  100% correctly, passes every geometry check, and does absolutely nothing --
  the failure is silent because placement and function are separate concerns.
  For a row fed from a belt to its north: input and output inserters are both
  `direction = 0`, feeder and takeoff inserters are `direction = 12`.
- **Two belt-loading rules decide whether a smelter works, and both fail
  silently.** An **inserter drops on the belt's FAR lane** (the one away from
  itself), and **a belt running into the SIDE of another sideloads onto its
  NEAR lane**. Together they are how ore and coal share one belt: load ore from
  the north so it lands on the south lane, and T-junction the coal branch in
  from the north so it lands on the north lane. Get either backwards and both
  commodities land on ONE lane, where the block still places 100% correctly and
  every arm still self-fuels — **and one commodity crowds the other out
  entirely**. Measured 2026-09-06 with ore and coal in a single chest behind a
  single loader: the chest's coal drained 50 → 17 over 2,500 ticks while its
  ore never moved off 99, and the whole block produced one plate. The corrected
  T-junction version consumed 100 of 100 ore and 50 of 50 coal and ran both
  furnaces at 192 ticks/plate, i.e. 100% of a stone furnace's rate. See
  `docs/superpowers/notes/2026-09-06-t-junction-smelter.md`.
- **A smelter line fed from one end must have a SATURATED input belt; a partial
  one starves its far end completely.** Measured 2026-09-06 on six furnaces
  (three a side of one mixed belt) deliberately under-supplied: the two westmost
  took **117 of 150 plates, 78%**, and the two eastmost took 3 between them.
  The starved furnaces were **not** short of fuel — they ended with *full* fuel
  slots and no ore. **Coal balances itself and ore does not, because of buffer
  size**: a fuel slot caps at 5 and then refuses more, so coal rides past the
  near arm to the next takeoff, while a furnace's ore input has no such small
  ceiling and the near arm absorbs everything. So "add more furnaces" is not the
  scaling move — saturating the belt is, and that is **measured, not inferred**:
  the same six furnaces fed by three loaders instead of one went from a
  lowest:highest spread of **1:59 to 38:63, a ratio of 1.66**, with the two rows
  balancing to within 4% (153 vs 147). 48 stone furnaces saturate a yellow
  belt (0.3125 plate/s each against 15 items/s), which is why real designs run a
  full belt into 48 rather than a partial belt into a few. A double-sided line
  does work: an inserter picks from **both** lanes, preferring the far one, so
  two rows meeting opposite lanes first self-correct once a furnace's ore slot
  fills. See `docs/superpowers/notes/2026-09-06-two-rows-off-one-belt.md`.

  **NARROWED 2026-09-07 by `entity.status`: the gradient is real, "not short of
  fuel" is NOT general.** The claim above was inferred from plate counts on one
  block. Read directly instead — 3,058 status samples across the window, on a
  four-furnace chain:

  ```
  furnace 1 (near)  working 91%  no_ingredients  2%  no_fuel  7%
  furnace 2         working 43%  no_ingredients 42%  no_fuel 15%
  furnace 3         working  7%  no_ingredients 62%  no_fuel 31%
  furnace 4 (far)   working  0%  no_ingredients 58%  no_fuel 42%
  ```

  **`no_fuel` climbs 7% → 42% alongside `no_ingredients`**, so the far end
  starves of *everything the belt carries*, not of ore specifically. The
  original reading took its mechanism from the commodity that happened to be
  counted. The buffer-size story survives but is narrower than stated: a fuel
  slot capping at 5 lets coal ride past a **satisfied** furnace, and does
  nothing for a furnace whose arm never gets a turn at all.

  The general lesson is the one this file keeps relearning: **a mechanism
  inferred from the one quantity you were measuring will be about that
  quantity.** The status field cost nothing to read and corrected an inference
  within an hour of landing.
- **RETRACTED 2026-09-06: "ungenerated ground reads as clear".** This entry
  claimed that a siting search picked a tile with a tree on it because the chunk
  was not generated yet, and that "zero entities in a 10-tile disc of a fresh
  map is the tell". **The premise is nonsense — open grass is perfectly
  ordinary on a fresh map**, so an empty disc is evidence of nothing at all, and
  the whole mechanism was built on top of it.
  Measured with the discriminator that was in the bindings all along (`rcon.*`
  asks the GAME, `world.*` asks the MODEL), on a fresh seed-31337 map:

      before any walk    GAME=0  MODEL=0   planner ACCEPTS the anchor
      after the walk     GAME=1  MODEL=0   planner ACCEPTS the anchor
                         (the 1 is the scout itself)

  **There is no tree at (44.5, 0.5).** Nothing appeared after the walk, and the
  planner accepts that anchor on a clean world — so no chunk filled in and no
  writeout gap is needed to explain anything either. What is left standing:
  the refusals only ever happened on a **replan after a partial build**, so the
  occupant was something those runs created; the cause is **unexplained** and is
  deliberately left that way rather than replaced with a second story. And
  walking a scout to the site first does help — but by putting the bots *near
  the site* before the build, which fixes walk routing, not by making ground
  real.
  The general lesson is the one worth keeping: **an inference is not a
  measurement, and the tell was a guess about Factorio terrain that a moment's
  thought would have killed.**

  **CLOSED 2026-09-07, and the answer was siting — every refusal in the chain
  was correct.** Using the new `world.blocked_boxes` binding on its first real
  case:

  ```
  replan REFUSES: a footprint the game already refused a burner-mining-drill
                  at, where it found no entity
  model coverage: charted (64 of 64 tiles)
  model boxes covering the refused tile: 0
  iron-ore in the rectangle:                41
  iron-ore under the drill's own footprint:  0
  ```

  **Siting chose an anchor where one drill had no ore beneath it.** The game
  refused, correctly — a drill cannot stand on ground with nothing to mine. The
  refusal was recorded with **no named blocker**, so it is durable and the
  expiry rule rightly cannot drop it. The replan refused the same footprint,
  also correctly. Nothing in the refusal machinery was ever broken.

  **What misled two sessions for a day was a message, not a mechanism**:
  `occupant_of` consulted `blocking_boxes_within` *before* the refused-footprint
  check, so `Occupant::Terrain` masked `Occupant::Refused`. We went looking for
  terrain because the error said terrain, and there was none — twice.

  **REOPENED AND RE-CLOSED 2026-09-07 — "the defect is in siting" was wrong,
  because siting never ran.** This paragraph previously said `drills_are_fed`
  asks whether the area covers *some* extractable resource. **It does not.** It
  loops over every drill and refuses if any one covers nothing. The check was
  correct the whole time.

  The real chain: `resolve_site` opens with `recover_anchor`, **first and
  unconditionally**. `drills_are_fed` has exactly one non-test call site, in
  `first_obstruction`, which has exactly one, inside `search_site`'s candidate
  loop — so **when recovery hits, none of it runs**. Recovery trusts an anchor
  once *two* of a blueprint's entities stand at the right relative offsets, and
  the fixtures in `scripts/rcontest.lua` are variations of one another
  (`crates/core/tests/recovery_crosstalk_probe.rs`):

  ```
  ElectricSmelter   21 of its 28 entities  match inside FurnaceLine
  SaturatedSmelter  17 of 33               match inside FurnaceLine
  TwoRowSmelter     15 of 27               match inside FurnaceLine
  OreToPlate        14 of 24               match inside MinerLine
  ```

  The chain scripts run several of these on **one map in sequence**, and
  FurnaceLine's 179 entities stood there. **So the anchor was never chosen — it
  was read off a different block**, putting a drill where FurnaceLine's geometry
  wanted it, which is not on ore.

  **Raising the vote threshold does not fix it**: 21 of 28 is 75% of the
  blueprint, and any threshold loose enough to recover a genuinely half-built
  block accepts that. Recovery by geometry is ambiguous whenever two blocks
  share a sub-layout. **The fix is to persist the anchor with the goal rather
  than re-derive it from the ground, which changes `Goal::Built` semantics and
  is an OWNER DECISION, not an overnight one.**

  Landed meanwhile (`06296b0a`): every anchor is screened for ore, not only the
  ones siting chose — `Site::At` and recovered anchors both get `drills_are_fed`
  before a step is emitted, refusing as `BlockDrillUnfed`, which names the drill,
  the tile, and whether the remedy is to move the anchor or clear the half-built
  block.

  **The methodological lesson, which is the durable part: a check can be
  correct, correct per item, and still say nothing about the cases that never
  reach it.** Two sessions reasoned about that function's *logic*; one of them
  mis-stated it, and the mis-statement changed nothing, because the logic was
  never the problem — its one call site two frames up was. **`grep -n
  drills_are_fed` would have shown that in a second**, and was run only after
  the decision to change the function had already been made. Find the call
  sites before reasoning about the body.

  **And a second lesson, about retracting**: the first hypothesis here
  overclaimed, and the retraction of a *later* hypothesis then **overshot in
  the other direction** — an empty `placement_refusals` across 21 archived runs
  was read as "this is a different structure", when it only ever supported
  "unknown". A retraction can be as unsupported as the claim it replaces.
  Retract to *uncertainty*, not to an opposite certainty.
- **`method::blueprint` had NO enclosure guard until 2026-09-06, while
  `method::assemble` has had one — so the method that builds the LARGEST blocks
  was the unguarded one.** `BuildBlock::expand` now calls
  `crate::enclosure::check` against a fork with the whole block created, and
  emits `ActionKind::Evacuate` (pinned to the bystander, linked ahead of every
  placement and the ghost stamp) or refuses by name when no way out exists.
  **The executor's `pre_place` cannot cover this**: it judges only the character
  *doing* the placing, so one bot walling in another is invisible to it, and
  once a bot is enclosed every later placement reads as `already walled in ...
  this placement does not change that` and is allowed. Fixed the way the
  refusal text always promised — "cleared by walking, not by moving the block" —
  and deliberately NOT by making the search avoid characters, which would make
  the anchor depend on where a bot happens to stand and move the block on every
  replan (`a_bystander_in_the_search_path_does_not_move_the_sited_anchor` exists
  to forbid exactly that).

  **It does NOT fix the live failure that found the gap, and that was verified
  by re-running it.** `evacuation steps planned: 0`, same `pending=13`. The
  guard is right to stay silent: `TwoRowSmelter` is an open-ended strip — a belt
  row between two furnace rows — so a bot in the corridor can walk out either
  end, and `enclosure::check` correctly finds no enclosure. **The live failure
  is a walk failure**, not a ring: a walk resolved to a tile inside a furnace's
  collision box and "the pathfinder never answered that there is no way there".
  Only bot 1 reports `walled in`, 14 times, never having moved from `[0.5,
  0.5]`. **A `pocket_tiles=1.0` reading for a bot standing in an open-ended
  three-tile corridor is not credible as a fact about the map** and points at
  the executor's enclosure *window* rather than at the ground — the same window
  `26498dee` resized for a different reason. Open.
- **`player blocks placement in all directions` was the MOD refusing a belt
  the GAME had accepted, and it cost `run-1788914717-24351` its objective.**
  Bot 1 laid 154 belts, then stood on the tile of the 155th. The game said
  yes -- a belt's collision layers (`water_tile, floor, transport_belt, object,
  meltable`) and a character's (`is_object, player, train`) are disjoint, and
  `create_entity` builds a belt under a character and leaves it standing;
  measured live 2026-09-09 -- but `rcon_place_entity`'s post-check scanned the
  raw box for *any* character and answered the actor sentinel anyway. The
  RCON layer then looked for a spot with an **empty two-tile disc five tiles
  out**, which no tile inside a belt run has, and failed the action. 48 of 878
  steps were abandoned behind the two refusals, both `assembling-machine-1`s
  among them, and **the record said nothing**: abandoned steps wrote no
  attempt and read as `pending`, the same as a killed run, so the supervisor
  called the milestone satisfied. `pre_place.rs` had known belts do not
  collide since it was written; the mod's four character arms never asked.

  Now: every character arm in the mod (actor sentinel, bystander transient,
  post-check, push-out, and the pre-check's `character` flag) is gated on
  `prototype_collides_with_character`, read off the two masks' layer names;
  the actor sentinel carries the landing the game's own
  `find_non_colliding_position` ladder offers (` landing=x,y`) and
  `place_entity_timed` walks there before falling back to the compass; a
  walk ending on a belt is no longer refused as unstandable
  (`standing_verdict` subtracts walkable boxes, the enclosure fill's own
  rule); and an abandoned step is `Status::Abandoned` -- its own
  `action_settled` naming the predecessor, `obs.abandoned`, and
  `batch_progress.abandoned` while the batch is still running.

  Two shapes worth keeping. **The message named the actor because the actor
  arm was checked first, not because the actor was the cause** -- the same
  masking as `Occupant::Terrain` over `Occupant::Refused` above; the fix
  began by asking the live game whether a belt goes under a character, which
  took one `rcon -s localhost` and settled a question two layers of code
  disagreed about. And **"nothing was dispatched, so nothing is written" is
  the silence-is-not-success defect in its purest form**: the executor knew
  exactly why 48 steps died and recorded none of it.
- **Both SEARCHING forms of `goal.built` still ignore characters when CHOOSING
  the anchor, which is deliberate and now safe.** Siting treats a character as non-blocking on purpose — a
  bot can walk away, so it should not veto a site — and only the explicit-anchor
  form refuses for a bot on the footprint. That holds until the block is large
  enough to enclose the bots it was sited around: a 27-entity block spanning
  ~16x8 with rows above and below a corridor, sited from the roster centroid,
  produced `the character is already walled in here ... pocket_tiles=1.0`, a
  walk ending inside a furnace's collision box, and **`done=true` beside
  `pending=13`** with 2 of 6 furnaces standing. Pass a `near` hint away from
  spawn as a workaround; the gap itself is open.
- **A burner block can EARN the research that unlocks its electric successor,
  in about 37 seconds of game time.** Both prerequisites of
  `automation-science-pack` are trigger technologies fired by ordinary smelting:
  `steam-power` by 50 iron plates (unlocking boiler/steam-engine/offshore-pump)
  and `electronics` by 10 copper plates (unlocking `inserter`,
  `small-electric-pole`, `electronic-circuit`, `lab`, `copper-cable`). Measured
  2026-09-06: a copper run of `TJunctionSmelter` fired `electronics` at tick
  2,220 and the chain `copper-cable -> iron-gear-wheel -> electronic-circuit ->
  inserter` then crafted from the block's own 16 copper plates — no lab, no
  science pack, no research action in the plan. **Proved with a control that
  fails for the right reason**: five copper plates (below the threshold of ten)
  were cheated in first, so the pre-run craft could only fail on
  `recipe copper-cable is not enabled for this force`, never on materials. See
  `docs/superpowers/notes/2026-09-06-the-block-earns-its-own-upgrade.md`.
- **A burner block works exactly where coal flows THROUGH it.** A burner
  inserter fuels itself from the coal it carries, so an arm on a mixed lane
  never needs fuelling — but an arm that touches only ore, or only plates, has
  no fuel source at all and stops when its hand charge burns out. That is not a
  bug to route around: it is why the electric `inserter` matters, and it puts a
  hard shape on t=0 blocks — the way out is the `electronics` trigger in the
  entry above, not a fuelling route.
- **`only_ghosts = true` validates nothing.** Ghosts do not collide, so a
  blueprint whose entities overlap places exactly as many ghosts as a correct
  one. A ghost-placement count is not evidence that geometry is legal; only a
  real build (`only_ghosts = false`, with the materials present) is.
- **Power *coverage* is not power *capacity*.** A layout can have every
  consumer inside a pole's supply area, be fully connected, and still do
  nothing at all, because generation is short. This does not degrade
  gracefully into "slow" -- an under-supplied network can read as completely
  dead. Solar is the worst offender for tests: output depends on the in-game
  time of day, so the same blueprint runs or does not run depending on when
  the run starts. Check generation against demand (assembling-machine-2 is
  150kW, inserters ~13kW each), and prefer a deterministic source when what
  you are testing is geometry rather than power.
- **`inventory_type` is passed straight to `entity.get_inventory(N)`, and a
  furnace's result slot is 3.** A chest's main inventory and a burner's fuel
  slot are both 1. `remove_from_inventory` had **no call site anywhere in the
  tree** — no script and no planner code — until 2026-09-06, so none of these
  indices had ever been exercised outside `insert_to_inventory`. A wrong index
  answers `cannot remove from nonexisting inventory`, but a removal that moves
  nothing is silent: re-read the source's count afterwards rather than assuming
  it worked.
- **`Option::None` reaches Lua as mlua's null sentinel, which is light
  userdata and therefore TRUTHY.** So `local inv = r.output_inventory or {}`
  does *not* substitute the default, and the next `pairs(inv)` raises
  "table expected, got light userdata". Guard with `type(x) == "table"`.
- **Resource positions are tile centres.** Every real resource entity sits at
  `(-40.5, -48.5)`, never `(-41, -49)`, and the mod's
  `surface.find_entity(name, position)` matches exactly. `EntityGraph` keys
  resources by `Pos(i32, i32)`, which floors, so anything reading a position
  back out of that map must restore the half-tile offset. Getting this wrong
  made mining fail with "no entity to mine" for every ore on every map, while
  every test passed — `test_utils::spawn_ore` builds ore at integer positions,
  the one input for which the lossy round-trip is lossless.

### Critical Bug Fix (Jan 2026): config.ini Creation

**Clients crashed on spawn with "Error: Specified config file doesn't exist"
while setup reported success**, because archive extraction creates an **empty**
`config/` directory and the guard tested the *directory* rather than the file.
`if !config_path.exists()` skipped writing `config.ini` whenever the directory
was already there. Fixed by testing `config_ini_path` instead
(`crates/core/src/process/instance_setup.rs`, `if config_ini_path.exists()`).

**The durable rule: a guard that means "is this artefact present" must name the
artefact, not its container.** Verification on macOS: a successful multi-client
launch shows N Factorio icons in the Dock (one per client, plus the server if
graphical).

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
