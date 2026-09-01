# Prior art: programmatic Factorio agents

Research date: 2026-09-02. Web sources only — no code in this repo was read or
changed for this note.

**Reading conventions.** "Read" means I fetched the page or file and the claim
is on it. "Inferred" means I am joining two sources or reasoning from the API;
treat it as a hypothesis to check, not a fact. Where a project is dead or its
docs contradict its code, it says so.

**Version relevance.** Factorio 2.0 (Oct 2024) renamed the mod-global table
`global` -> `storage` and moved prototype access off `LuaGameScript`, among many
other breaks — see the porting guide at
<https://github.com/tburrows13/factorio-2.0-mod-porting-guide> and the changelog
preview at <https://forums.factorio.com/viewtopic.php?t=115737>. Any project
pinned to 0.18/1.1 is describing an interface that no longer loads. That kills
most of the older TAS work outright and is flagged per project below.

---

## 1. The Factorio Learning Environment (and academic work)

Primary sources:

- Paper: <https://arxiv.org/abs/2503.09617>, HTML full text
  <https://arxiv.org/html/2503.09617v1>
- Code: <https://github.com/JackHopkins/factorio-learning-environment>
- Release notes: <https://jackhopkins.github.io/factorio-learning-environment/versions/0.3.0.html>
  and <https://jackhopkins.github.io/factorio-learning-environment/versions/0.2.0.html>
- Generated architecture docs: <https://deepwiki.com/JackHopkins/factorio-learning-environment>
- Claude Code bridge: <https://github.com/JackHopkins/claude-code-plays-factorio>

### Interface

Read: **a Python client and a Lua "server" talking synchronously over RCON/TCP**
(`factorio-rcon-py`, port 27015, 60 s timeout), every command prefixed `/sc`
(silent-command). Lua replies come back as strings via `rcon.print` and are
parsed by a `_lua2python()` helper. Source:
<https://deepwiki.com/JackHopkins/factorio-learning-environment/3-lua-environment-and-tools>.

So: **the same transport this project uses**. The interesting differences are
above and below that transport, not in it.

Read: tools are directories under `fle/env/tools/agent/<tool>/` each containing
a `client.py` (Python surface) and a `server.lua` (in-game implementation). At
startup a `LuaScriptManager` pushes every `server.lua` into the game over RCON
once, and binds every `client.py` into a per-agent `FactorioNamespace`. Source:
<https://deepwiki.com/JackHopkins/factorio-learning-environment/3.2-tool-development-guide>.
The consequence worth noting: **per-action RCON payloads are tiny function
calls, not shipped Lua bodies**, because the bodies were installed once.

### Action granularity

Read (paper §API): ~23 methods, split into queries (`get_entities`,
`production_stats`, `nearest`, `inspect_inventory`), mutations (`place_entity`,
`rotate_entity`, `craft_item`, `set_recipe`, `connect_entities`) and inventory
ops (`insert_item`, `harvest_resource`, `extract_item`), plus `can_place_entity`
and `sleep`. Granularity is roughly "one legitimate player action", i.e. the
same altitude as this project's executor actions.

The agent does not emit actions directly. It **writes Python** that calls those
tools, in a persistent REPL namespace that survives across steps, so it can
define helper functions and keep typed objects around as episodic memory. Read
(v0.3 notes): frontier models *rarely* use that ability and mostly call
primitives — a candid negative result.

`connect_entities` is the one that does real work. Read
(`fle/env/tools/agent/connect_entities/client.py`): it takes a prioritised list
of candidate connection-point pairs from per-entity "resolvers", then
pathfinds at several probe radii (1.5 / 1 / 0.5 / 0.25), with type-specific
parameters (pipes 0.5 with collision-box extension, belts 0.5 with fallback
repositioning, poles 4.0 for long hops), retries twice on specific Lua indexing
errors, special-cases refinery/chemical-plant fluid boxes by routing a "modified
straight line" through underground pipes, and returns a typed `BeltGroup` /
`PipeGroup` / `ElectricityGroup`. It also has a **dry-run mode** returning
`{"number_of_entities_required": N, "number_of_entities_available": M}`.

### Real time

This is the part most directly transferable.

Read (`fle/env/gym_env/environment.py`): the environment **unpauses the game and
sets a configured speed before each step, then pauses again after the action**
when `pause_after_action` is set. Read
(<https://deepwiki.com/JackHopkins/factorio-learning-environment/3.1-lua-script-manager>):
`FactorioInstance` exposes `set_speed()`, `get_speed()`, `pause()`, `unpause()`,
`set_speed_and_unpause()`, with a default speed multiplier of **10**.

Inferred (high confidence): these map onto `game.speed` and `game.tick_paused`.
`game.speed = 10` raising the UPS ceiling to 600 is documented behaviour —
<https://wiki.factorio.com/Time>.

So FLE turns Factorio into an approximately turn-based environment: the world
only advances while an action is executing, and when it does advance it runs
10x. Waiting is explicit, via `sleep(seconds)`, and the response carries elapsed
*game* time separately from wall time.

Read (paper): ~218 ops/sec average, 603 peak for simple ops, and only
**25–48 ops/sec for `connect_entities`** because of pathfinding. That is the
cost profile of RCON round trips plus in-game work, measured — useful as a
sanity baseline for this project's own throughput.

### Movement: mod-side loop, not RCON per step

Read (`fle/env/tools/agent/move_to/server.lua`): two modes.
`storage.fast` teleports the character waypoint to waypoint. Otherwise the mod
sets `player.walking_state` and registers `script.on_nth_tick(5, ...)`, which
re-aims the direction each cycle, advances the waypoint when within 1 tile, and
stops when the queue empties. The pathfinder only produces waypoints
(`storage.paths`); it never moves anything.

Read (`move_to/client.py`): in fast mode Python executes and then sleeps for the
real-world equivalent of elapsed ticks. In slow mode it **long-polls**
`get_walking_queue_length(player_index)` every 0.5 s until it reads `"0"`.

That is the answer to the architectural question in area 3, from the most
serious project in the space: **the control loop is mod-side; RCON is used to
start it and to poll a scalar for completion.** They did not attempt per-tick
RCON steering, and they kept a teleport escape hatch for when fidelity does not
matter.

### Error handling and backtracking

Read (paper): typed exceptions with context, written to stderr; runtime
assertions for self-verification; a wall-clock cap that kills runaway programs.

Read
(<https://deepwiki.com/JackHopkins/factorio-learning-environment/5.5-reflection-and-backtracking>):
before each program the environment captures entity positions, inventories,
research, production state and resource deposits. On error it **restores that
snapshot**, hands the agent the error plus the history of attempts, and lets it
resynthesise — up to 3 attempts by default, and "improved programs are always
executed from the pre-error game state". Explicitly *not* save files;
entity-level snapshots. Read (v0.2 notes): worth **+6%** on lab-play.

The Claude Code bridge exposes this as version control:
`commit(tag, message)` and `restore(ref)` alongside `execute(code)`, plus
read-only `fle://status`, `fle://entities/{x}/{y}/{radius}`, `fle://inventory`,
`fle://recipe/{name}`, `fle://metrics`, `fle://render/{x}/{y}` resources.
Source: <https://github.com/JackHopkins/claude-code-plays-factorio>.

### Evaluation

Read (paper): **Production Score** `PS(t) = Σ V(i)·(P_i(t) − C_i(t))` over item
values and throughputs, plus discrete **milestones** for first-production of an
item type or a completed research. Lab-play is **24 fixed tasks** with all
research unlocked and stocked inventories, targets of **16 items/min** (solid)
or 250/min (fluid), a budget of **128 API calls**, and — the key bit — a
**60-second holdout period during which the agent does nothing and throughput is
measured**. Open-play is unbounded factory growth on a generated map.

Results: best model in the paper completed **7/24** lab tasks; agents "lack
strong spatial reasoning"; in open-play they reach electric mining and some
science but fail at electronic-circuit automation. v0.3 notes add that agents
fail to keep a consistent mental model of the layout and **exploit hand-crafting
instead of automating**, a local optimum the scoring initially rewarded.

### Multi-agent

Read (v0.2): built on Factorio's native multiplayer; broadcast and peer-to-peer
`send_message`; per-agent custom instructions enabling partial observability.
The load-bearing sentence: *"agents fully yield to each other when planning and
taking actions in order to minimize coordination challenges"* — i.e. **turns are
serialised**, and even so "agents struggle to fully account for each others'
actions, leading to novel errors potentially difficult to recover from".

That is a direct data point against this project's current shape: the strongest
Factorio multi-agent work available deliberately refuses concurrency, and still
reports coordination failures.

Read (v0.3): the client dependency is gone — a **headless renderer** produces
pixel observations instead of screenshots from a graphical client — and the eval
API conforms to the OpenAI Gym interface. Inferred: characters are therefore
created server-side rather than by connecting graphical clients.

### Other academic work

- **Towards Automatic Design of Factorio Blueprints** (Patterson, Espasa, Chang,
  Hoffmann, ModRef 2023) — <https://arxiv.org/abs/2310.01505>, full text
  <https://arxiv.org/html/2310.01505>. Covered in §4.
- **The Factory Must Grow: Automation in Factorio** (Reid et al., GECCO 2021) —
  <https://arxiv.org/abs/2102.04871>. Defines the *logistic transport belt
  problem* as integer programming on 3x3 / 6x6 / 12x12 grids with obstacles,
  benchmarks meta-heuristics, and — relevant here — built "an interface to allow
  optimizers in any programming language to interact with Factorio". Read
  (abstract/summary only). The grid sizes tell you how small the tractable
  instances are.
- **Develop AI Agents for System Engineering in Factorio** —
  <https://arxiv.org/abs/2502.01492>. I could not retrieve the full text (the
  PDF exceeded the fetch limit) and I am **not** characterising it from the
  title. Flagged as unread.

### What this project is missing, from area 1

Pause-and-step; a speed multiplier; snapshot/restore as the first recovery
tier; a fixed task suite with holdout verification and an action budget; a
routing primitive with a dry-run materials check; and the empirical finding that
serialising multi-agent turns is the *starting* position, not a fallback.

---

## 2. The TAS community

### Factorio-TAS-Generator (the live line)

- Original, **archived 2024-08-30**: <https://github.com/MortenTobiasNielsen/Factorio-TAS-Generator>
- Maintained fork: <https://github.com/theis999/Factorio-TAS-Generator>

Read: a desktop tool that holds a run as **a list of steps in a CSV-like text
file**, shown in a grid, and *compiles it into a Factorio mod* that forces the
character through the steps using the Lua API. There is no external process at
runtime — the run is the mod. The fork shows ~1153 commits and recent activity.

Read (README): steps deviate from human play in specific, declared ways — no
cursor is required, rocks yield fixed amounts (47 coal / 47 stone) instead of
random, construction robots are unavailable. That is a deliberate determinism
budget: they removed the RNG that would make a run unreproducible.

I could **not** confirm the per-step field schema or the per-tick execution
mechanism from the README; treat "executes one step per tick when in reach" as
unverified. The older gotyoke run below states it explicitly for its own mod.

### TAS Step Planner

<https://github.com/theis999/TAS_step_planner> /
<https://mods.factorio.com/mod/TAS_step_planner>

Read: an in-game **recorder**. It captures building, rotating, mining, ctrl-click
transfers, recipe changes, research, crafting, walking and equipment changes,
and exports them in Factorio-TAS-Generator step format for paste-in. It cannot
capture pick-up/drop or queued research (API limits). Successor to the older
"TAS Helper". ~35 commits.

The workflow — *play the thing by hand, export the actions, edit them as data,
compile, replay* — is the loop this project does not have. Its Lua scripting
surface is authored blind.

### TAS State Printer

<https://mods.factorio.com/mod/TAS_state_printer>

Read: prints game state **every few ticks** so the developer can "pin-point when
changes start to cascade". Factorio **1.1**, last updated ~3 years ago, 68
downloads. Dead as code; the *idea* is the deliverable: a periodic canonical
state dump whose diff between two runs localises divergence to a tick.

This is the TAS community's answer to "verify they actually happened", and it is
a diff, not an assertion.

### gotyoke Any% TAS

<https://github.com/gotyoke/Factorio-AnyPct-TAS>,
<https://mods.factorio.com/mod/AnyPctTAS>

Read: **Factorio 0.18.17, hard requirement** — dead for 2.0. Executes a task
every single tick from a generated task list; tasks fire "as soon as the
character is within reach, which minimizes wasted ticks". The list was generated
by an unreleased Python script from a custom shorthand grammar; the emitted list
is ~9,400 lines. 1h21m20s, April 2020, and reportedly the only TAS at the time
that actually launched a rocket.

The honest finding: **it has no error recovery.** The README describes a single
upstream patch to coal-mining yields breaking the whole run. Verification is
"the run either completes or it does not", and desync handling is "re-author".
Do not copy this shape.

### FactorioReplay

<https://github.com/Kizby/FactorioReplay>, live at
<https://kizby.github.io/FactorioReplay/>

Read: a dependency-free JS parser/generator for Factorio's **`replay.dat` input
format**. Drag a save onto the page to see the recorded action stream. It
includes a "Replay Framework" for *programmatically authoring* a run — JS code
driving a `Player` class through crafting and movement with timing — and
emitting a replay file Factorio will play back. A separate **checksum stripper**
removes verification frames so Factorio will play a file that would otherwise
refuse on desync. ~117 commits, MIT, actively accepting patches. Version support
not stated — **check before relying on it**.

Context on why checksums matter: <https://wiki.factorio.com/Replay_system> and
<https://wiki.factorio.com/Desynchronization>. A replay is "another client in a
multiplayer game which receives input actions from a file instead of the
network", so replays are exact only if the mod set and script state are
identical; script changes that touch game state break replay. Also read
(<https://forums.factorio.com/viewtopic.php?p=694877>): **the headless build
cannot record replays**, which limits this for a headless-first design.

### What this project is missing, from area 2

A run as *editable data* rather than as imperative Lua; an in-game recorder to
author it; a periodic state digest whose diff answers "where did this run
diverge from the last one"; and an explicit, written-down determinism budget
(which RNG sources the run is allowed to depend on).

---

## 3. Open-source bots and agents

Ordered by how much there is to learn, not by stars.

### chebykinn/factorio-planning-agent — the most interesting small project

<https://github.com/chebykinn/factorio-planning-agent>

Read: Claude Agent SDK sessions run by a bun daemon; the daemon talks to a
Factorio **dedicated server over RCON**; each in-game chat opens a session with
persistent memory across restarts. Requires Factorio 2.0. MIT.

Three ideas worth more than the repo's 3 stars and 6 commits:

1. **Ghost entities are the plan.** Plans live as JSON in
   `agent-workspace/state/` with metadata, an `as_of_tick`, and a `ghosts` array
   carrying per-belt-lane intent, machine recipes and train schedules. Saving the
   file deploys it. The plan representation and the in-game representation are
   the same objects.
2. **The world journals back into the files.** The mod records every build,
   removal and rotation — by player, by construction bots, from chat — and the
   daemon files each change into the owning state file or into
   `state/world/region_X_Y.json` mirrors for unclaimed ground. The plan is never
   stale in the "nobody told the planner" sense.
3. **Staleness is enforced, not hoped for.** 16-tile dirty-cell tracking; a
   layout declares the tick it was planned against, and **re-application is
   refused if the target area changed since**.

Also present: `trace_belt`, `trace_entity`, `get_rates`, `get_power` as
agent-facing analysis tools, a lane analyser with belt rules derived empirically
from the engine, and an "engine-derived rail router".

Inferred: this is experimental and probably unmaintained (6 commits). Read the
design, do not depend on the code.

### kovan/factorio-play-api

<https://github.com/kovan/factorio-play-api>

Read: a mod exposing `/agent <command> key=value ...` over RCON — walk, mine,
build, craft, inventory, entity interaction, research, scanning. One-shot
commands, queued asynchronously, **not** blocking. Results come back through two
files rather than the RCON reply: `agent-gamestate.json` rewritten every second
(position, health, crafting queue, research progress) and `agent-response.txt`
with `ok`/`error`/`event` plus payloads.

**2 commits, 0 stars.** Read as a sketch, not a project. But the split — small
commands in over RCON, bulk state out through `script-output` files — is a real
design and it is the one this project has already half-adopted for frames.

### moeru-ai/airi-factorio

<https://github.com/moeru-ai/airi-factorio>

Read: TypeScript (TypeScriptToLua) mod `autorio` symlinked into the mod folder
with hot reload via `tstl-plugin-reload-factorio-mod`; a WebSocket transport
(`WS_SERVER_HOST`, `FACTORIO_WS_HOST`); a `factorio-rcon-api` RESTful RCON
wrapper for actions; and **YOLO object detection on screenshots**
(`factorio-yolo-v0`) feeding an LLM. ~76 commits, MIT, active. Factorio version
not stated.

Blunt assessment: **the vision half is not worth copying.** Detecting entities
from pixels when `LuaSurface.find_entities_filtered` returns them exactly is
paying for noise. The two things worth taking are the TypeScriptToLua toolchain
with **mod hot-reload** (this repo's CLAUDE.md documents a nasty stale-mod trap
that hot reload would blunt) and the observation that a WebSocket out of the mod
is an option RCON's request/response shape does not give you.

### lvshrd/factorio-agent

<https://github.com/lvshrd/factorio-agent>

Read: OpenAI Agents SDK, `@function_tool`-decorated Python in
`api/agent_tools.py`, RCON transport, targets Factorio 2.0.32. Actions: move,
place, gather, set up mining, manage inventory. Observation is position /
inventory / nearby resources. No blueprint handling. 15 stars. Known issue the
author states: hitting the agent's max-turns limit; achievements must be
disabled by RCON first.

Nothing architecturally new. It is the "thin RCON tool wrapper around an LLM"
baseline, and it is useful mainly as evidence that this baseline is where
everyone starts and that it does not get far.

### tylerstraub/Factorio_Hivemind-MOD

<https://github.com/tylerstraub/Factorio_Hivemind-MOD/>

Read: a **2.0+** state exporter built as a Lua `remote` interface named
`hivemind`, with `get_attack_events_after(tick)`, `get_chat_messages_after(tick)`
and `get_storage_snapshot(tick)`; everything serialised with
`helpers.table_to_json` and returned through `rcon.print`. Data is bucketed by
tick; old buckets are pruned when new ones are written; the retention window is a
mod setting (default 36,000 ticks ≈ 10 min).

The performance rules it states are the reusable part: event handlers must run
in **under 1 ms even under load**, export must be **on demand and non-blocking**,
and no large table traversal may happen in a per-tick handler. Also: use
`/silent-command` + `rcon.print` so output never reaches player consoles.

Note the resonance with this repo's own hard-won rule that `rcon.print` in the
mod corrupts an action's RCON reply — Hivemind uses `rcon.print` *as* the return
channel, which only works because its calls are queries, not actions. The
general lesson is that **a single RCON reply channel cannot serve both action
results and telemetry**, and the fix is either a separate query verb or files.

### Discord/ops bots

<https://github.com/Lucman00/Factorio_bot> and
<https://github.com/TimMcGilly/FactorioBot> are server-management chat bots.
Not relevant; listed so nobody re-finds them.

### The architectural verdict for area 3

Three designs exist in the wild:

| Design | Examples | Cost per action | Failure mode |
|---|---|---|---|
| Whole run compiled into a mod, executed `on_tick` | Factorio-TAS-Generator, gotyoke | zero at runtime | no recovery at all; edit-recompile loop |
| Mod-side control loop, RCON to start + poll a scalar | FLE `move_to` | one round trip to start, one per poll | polling latency; loop state lives in `storage` |
| RCON round trip per primitive action | this project, lvshrd, kovan | one round trip each, ≥1 tick | latency floor; every action is a distinct failure point |

Nobody credible is doing per-tick steering over RCON. The strongest project
(FLE) uses the middle row for anything durative and reserves round trips for
discrete acts. **Inferred, but with confidence:** for "gather 20 iron ore with
four bots", the walk-and-mine loop belongs in the mod, and the executor should
be starting four loops and awaiting four completion signals rather than driving
four cursors.

---

## 4. Layout, blueprints, ratios

### Constraint-model blueprint synthesis (St Andrews, 2023)

<https://arxiv.org/abs/2310.01505>, full text <https://arxiv.org/html/2310.01505>

Read: modelled in **Essence Prime**, compiled by **Savile Row** to CSP instances
for backend solvers. Three-stage decomposition because flow rates cannot be
expressed in one model:

1. recipes — how many assemblers, which recipes, how many inserters;
2. bin-packing — place assemblers and inserters in the area;
3. layout — route conveyors/inserters from sources to destinations, tracking
   which item travels each path.

Results: outputs are **placeable and functional** — they build in Factorio and
produce the intended item.

Limitations, stated by the authors: ~**100 tiles of area takes several minutes,
and beyond that runtimes become intractable**; each tile carries only one item
type, forfeiting belt-density tricks; inserter starvation is invisible because
flow is not modelled; and when a stage fails, the back-and-forth between stages
is "detrimental to performance".

Verdict: real, honest, and small. Good for a fixed-size module. Useless for a
base. If this project ever wants generated modules, this is the paper to
implement, and the 100-tile ceiling is the number to design around.

### Factorio-SAT

<https://github.com/R-O-C-K-E-T/Factorio-SAT>

Read: SAT encoding where each tile carries booleans for input direction, output
direction, underground state, splitter side, splitter id and colour; constraints
forbid crossing belts, enforce colour consistency along a belt, and enforce
input/output coverage. Generates belt **balancers** (up to 6x6), underground
placements and interchanges, and has **found balancers shorter than previously
known designs**. ~774 stars, active. Stated limits: generation is exponential in
splitter count; 8x7, 7x5 and some others remain unsolved; the assumption that
balancer inputs/outputs are splitter-covered may cost 1 tile of optimality.
See also <https://forums.factorio.com/viewtopic.php?t=102232> and the write-up
<https://gianlucaventurini.com/posts/2024/factorio-sat>.

### VeriFactory

<https://github.com/alegnani/verifactory>

Read: **Rust**, z3. Converts a blueprint into a flow-network graph, simplifies
it, minimises edge throughputs, encodes as logical formulae, and *proves*
properties instead of simulating input combinations: belt balancing, equal input
drain, throughput-unlimited, universal balancer. **A 64x64 balancer verifies in
under a second.** Limits stated by the author: no dual-lane belts, no inserters
or assemblers, no custom property language, minimal docs, GPL-3.0, and the
author calls the code quality low.

This is the closest thing in the ecosystem to what this project needs and does
not have — a *checker*, in Rust, that answers "does this layout do what I meant"
without building it. Its scope is currently too narrow to use directly.

### factorio-draftsman

<https://github.com/redruin1/factorio-draftsman>,
docs <https://factorio-draftsman.readthedocs.io/>

Read: a complete, mod-aware Python library for building and editing blueprint
strings, with **collision sets, collision layers, world-space AABBs** and a
stated goal of being "Factorio-safe" — if the import would error in the game, it
errors in the library. Actively released.

This is the direct answer to the trap already recorded in this repo's
CLAUDE.md — *"`only_ghosts = true` validates nothing"*, because ghosts do not
collide. Draftsman validates geometry **offline against real prototype collision
boxes**, before anything is sent anywhere. There is no Rust equivalent I found;
`MForster/factorio-rust-tools`
(<https://github.com/MForster/factorio-rust-tools>) dumps prototype data from a
Factorio install and is the plausible feed for one.

### Ratio calculators

- **FactorioLab** — <https://factoriolab.github.io/>,
  <https://github.com/factoriolab/factoriolab>. Angular/Redux/TypeScript,
  multi-game, actively maintained. Ratios, machine counts, module/beacon effects.
- **Helmod** — <https://mods.factorio.com/mod/helmod>. In-game planner,
  mod-compatible, multiplayer-safe.
- **Foreman 2** — the desktop planner; superseded in practice by FactorioLab.

Blunt: **these are ratio solvers and they stop where this project's problem
starts.** They tell you 1.2 furnaces per drill. None of them emit a placeable
layout. Do not spend time integrating one; the arithmetic is a data problem, and
this repo's planner already does the recipe expansion.

### Generators that do emit placements

- **Buildasaurus/Factorio-Blueprint-Generator** —
  <https://github.com/Buildasaurus/Factorio-Blueprint-Generator>. Python + web
  UI + OpenAPI, ~236 commits, 14 stars. Intent, in the author's words, is to
  "integrate with solvers, so the solver provides the recipes to execute, the
  number of factories needed, the capacity between factory types" and this lays
  it out. Algorithm not stated in the README — **unverified**, and small.
- **yaanisk/factorio-blueprinter**, **gianluca-venturini/factorio-tools**,
  **kevinburke/factorio-layout-optimizer** (OR-Tools, minimises travel distance
  between base blocks) — surfaced by search, not individually verified.

### Visualisation

- **teoxoy/factorio-blueprint-editor** —
  <https://github.com/teoxoy/factorio-blueprint-editor>, live at
  <https://fbe.teoxoy.com>. Full in-browser blueprint editor on PIXI.js/WebGL.
  **Explicitly unmaintained**, but it is the reference implementation for
  rendering Factorio entities in a browser and it is MIT-ish open source.
- **piebro/factorio-blueprint-visualizer** —
  <https://github.com/piebro/factorio-blueprint-visualizer>. SVG, resolution
  independent, artistic rather than faithful.

Relevant because this project's timeline UI currently shows *screenshots*, which
cost a graphical client. An SVG/canvas render of the entity map is cheaper,
scrubbable, diffable, and works headless.

---

## 5. Things that surprised me

### Clusterio: the orchestration layer already exists

<https://github.com/clusterio/clusterio>, plugin docs
<https://github.com/clusterio/clusterio/blob/master/docs/writing-plugins.md>

Read: a controller/host architecture managing many Factorio **instances** across
machines, with a web UI and a CLI, a JavaScript plugin system with lifecycle
hooks, RCON command sending, and a `send_json` API for pushing JSON out of the
game on named channels that plugins subscribe to. Ships Global Chat, Research
Sync, Statistics Exporter, Subspace Storage, Player Auth, Inventory Sync.

Read (<https://forums.factorio.com/viewtopic.php?t=83407>): the 2.0 design uses
**a single RCON packet per tick that both reads (updates, chat, commands,
tracked events) and writes (tile/entity updates, event queue)**.

That last sentence is the throughput design this project does not have: instead
of N round trips per tick for N bots, **one batched round trip per tick carrying
a queue of actions out and a queue of events in**. Clusterio has been running
that in production for years across a fleet.

I am not suggesting adopting Clusterio — it solves cross-server item transfer,
not agents. I am suggesting the batching pattern and the `send_json` channel
idea are proven and cheap to copy.

### Two output channels, not one

Recurring across kovan, Hivemind, Clusterio and this repo's own frame pipeline:
**small commands go in over RCON; bulk state comes out through
`script-output` files or a push channel.** `helpers.write_file` is the mechanism
(<https://forums.factorio.com/viewtopic.php?t=102992>). This project already
reads `script-output/frames/`; extending the same directory to periodic state
dumps is a smaller change than adding another RCON query path, and it sidesteps
the reply-channel collision documented in this repo's CLAUDE.md.

### Determinism is available but expensive

- The headless build **cannot record replays**
  (<https://forums.factorio.com/viewtopic.php?p=694877>), so the ordinary
  determinism artefact is unavailable to a headless-first design.
- Replays desync when script state changes
  (<https://wiki.factorio.com/Replay_system>), and the historical bug reports —
  e.g. <https://forums.factorio.com/viewtopic.php?t=60661> (console commands),
  <https://forums.factorio.com/viewtopic.php?t=101294> (production statistics
  CRC) — show that a mod that pokes the world is exactly the thing replays
  cannot survive. An RCON-driven bot *is* that mod.

So: byte-exact replay is not a realistic verification strategy here. The
achievable substitute is the TAS State Printer approach — a **canonical periodic
digest** that is diffed, tolerant of irrelevant divergence, and precise about
where the relevant divergence started.

### The determinism budget as a design artefact

Factorio-TAS-Generator's README (§2) is quietly the best idea in the TAS space:
it *names* the randomness it removed (fixed rock yields) and the fidelity it
gave up (no cursor, no construction robots). This project has scattered
equivalents in CLAUDE.md (inserter direction, tile-centre resource positions,
`Option::None` truthiness) but no single statement of "here is what a run is
allowed to depend on".

### Speed and pause are free and nobody here is using them

`game.speed = 10` is a one-line console command with documented behaviour
(<https://wiki.factorio.com/Time>), and FLE ships it as the default. If this
project's 4-bot iron-ore test takes 3 minutes of wall time, most of that is the
world running at 1x for no reason.

---

## Ideas worth stealing

Ranked. Each entry: what it changes here, rough cost, what it replaces.

### 1. Make the world turn-based: pause between actions, and run at `game.speed = 10`

**What changes.** The executor gains explicit control of the clock:
`game.tick_paused = true` while it decides, `game.speed = N` + unpause while an
action runs, pause again on completion. Every action's "did it happen" check
becomes a read on a stopped world instead of a race with one.
**Cost.** Small — two RCON verbs in BotBridge, a clock owner in
`crates/executor`, and a config knob. Days, not weeks.
**Replaces / invalidates.** Most of the tuning pressure on lag edges and
recovery tiers, which currently exist partly to absorb timing jitter. Also
invalidates any wall-clock timeout in the executor: timeouts must become
tick-based. **Conflicts with graphical clients** — pausing a server with
connected clients is a different experience, and speed 10 will stress them.
That dependency is why idea 2 sits next to this one.
**Source.** `fle/env/gym_env/environment.py` (unpause → step → pause);
`set_speed`/`pause` on `FactorioInstance`, default multiplier 10
(<https://deepwiki.com/JackHopkins/factorio-learning-environment/3.1-lua-script-manager>);
<https://wiki.factorio.com/Time>.

### 2. Delete the graphical clients: create character entities server-side

**What changes.** Instead of spawning N graphical Factorio processes (26 s sprite
load each, up to 90 s connect wait, per this repo's own CLAUDE.md), the mod
creates N `character` entities on the headless server and drives them through
`LuaControl` — `walking_state`, `mining_state`, `begin_crafting`, `mine_entity`,
`insert`, `teleport`
(<https://lua-api.factorio.com/latest/classes/LuaControl.html>). A multi-bot test
starts in seconds.
**Cost.** Large and front-loaded. The mod must own character lifecycle, and
anything that assumed a `LuaPlayer` must move to `LuaControl`. Note `build_from_cursor` is LuaPlayer-only; building already goes through `surface.create_entity`
in practice.
**Replaces / invalidates.** The client-spawning half of
`crates/core/src/process/instance_setup.rs`, the config.ini-per-client
machinery, the 90-second connect poll, and **the screenshot frame pipeline** —
mod screenshots require a client. Replace frames with a server-side entity-map
render (see idea 9), which is what FLE did.
**Risk / unverified.** The forum thread
<https://forums.factorio.com/viewtopic.php?t=86603> confirms character creation
and that walking persists, but discusses movement only; it does **not** confirm
crafting/mining on an unassociated character. `LuaControl` says those methods
exist on the shared interface. **Prototype this before committing** — one
throwaway Lua script that creates a character, walks it to ore, calls
`mine_entity`, and reads the inventory back settles it in an afternoon.
**Source.** FLE v0.3 "no longer depends on the Factorio game client"
(<https://jackhopkins.github.io/factorio-learning-environment/versions/0.3.0.html>).

### 3. A fixed task suite with holdout verification and an action budget

**What changes.** "Gather 20 iron ore with four bots" stops being an anecdote and
becomes task #1 of a suite with a stated start state, a stated success predicate,
a **budget of executor actions**, and — for anything durative — a **holdout
window in which the executor does nothing and the outcome is measured from
production statistics**. Runs report pass/fail plus a production score, so a
change is testable.
**Cost.** Small-to-medium and almost entirely additive: a task manifest format,
a runner, and a results table. No architectural change.
**Replaces / invalidates.** Ad-hoc smoke scripts in `/scripts/`. Makes every
subsequent idea on this list measurable — which is why it ranks above the ones
that are technically more interesting.
**Source.** FLE lab-play: 24 tasks, all research unlocked, stocked inventory,
16 items/min solid target, **128 API calls**, **60-second holdout**
(<https://arxiv.org/html/2503.09617v1>).

### 4. Snapshot / restore as recovery tier zero

**What changes.** Before dispatching a step, capture entity positions,
inventories, research and production state. On failure, restore and re-plan from
the pre-error state instead of recovering forward from a half-built mess. Expose
it to Lua as `commit(tag)` / `restore(ref)`.
**Cost.** Medium. The snapshot is the hard part: entity-level, deterministic
ordering, bounded size. This project already has `EntityGraph` and world-state
samples, so much of the reader exists.
**Replaces / invalidates.** The forward-recovery tiers in
`crates/executor/src/recover.rs` for the class of failure where the *plan* was
wrong, as opposed to a transient. Keep the tiers for transients.
**Evidence it is worth it.** +6% on FLE lab-play, and "improved programs are
always executed from the pre-error game state"
(<https://deepwiki.com/JackHopkins/factorio-learning-environment/5.5-reflection-and-backtracking>,
<https://jackhopkins.github.io/factorio-learning-environment/versions/0.2.0.html>).
Note explicitly: **they did not use Factorio save files.** Entity snapshots.

### 5. Move durative actions into the mod; RCON starts them and polls a scalar

**What changes.** `walk_to`, `mine_until`, `craft_n` become mod-side loops
registered on `on_nth_tick`, holding their queue in `storage`. The executor sends
one RCON call to start, then polls one integer ("steps remaining") — or better,
lets the mod push completion out through the existing `script-output` channel.
Four bots walking becomes four `storage` queues, not four cursors driven from
Rust.
**Cost.** Medium, and it is the change that most directly targets the four-bot
reliability problem. Mod work plus a narrowing of the executor's action vocab.
**Replaces / invalidates.** The RCON-round-trip-per-movement-step design. The
existing `tokio::sync::watch` per-action completion signals stay — they just get
fed by a poll or a push instead of by the Rust side counting.
**Source.** FLE `move_to/server.lua` (`script.on_nth_tick(5, ...)` re-aiming
`walking_state`) and `move_to/client.py` (polls `get_walking_queue_length` until
`"0"`, with a teleporting `fast` mode as an escape hatch).

### 6. Validate geometry offline against real prototype collision boxes

**What changes.** Before any placement leaves Rust, check it against prototype
collision boxes and collision layers loaded from the Factorio install. Overlaps
are caught in a unit test, not by a silent no-op factory.
**Cost.** Medium. `MForster/factorio-rust-tools`
(<https://github.com/MForster/factorio-rust-tools>) can dump prototype data;
draftsman (<https://github.com/redruin1/factorio-draftsman>) is the reference
semantics to port. There is no Rust library for this — you would be writing it.
**Replaces / invalidates.** Ghost-placement counting as evidence, which this
repo's CLAUDE.md already records as worthless (*"`only_ghosts = true` validates
nothing"*). Also makes the inserter-direction trap testable, since direction is
part of the placement record.

### 7. A `connect_entities` primitive with a dry-run materials check

**What changes.** The planner stops emitting individual belt tiles and emits
"connect A to B with yellow belt", which resolves connection points, pathfinds
at several probe radii, special-cases fluid boxes via underground pipes, and
returns a typed group object. Dry-run mode returns *required vs available*
entity counts, so a plan can be rejected before a single belt is placed.
**Cost.** Medium-large — this is the single most complex tool in FLE and the
slowest (25–48 ops/sec vs 218 average), which tells you it is real work.
**Replaces / invalidates.** Per-tile belt planning in `crates/planner/method/`,
and the class of run that fails halfway through a belt run for want of 3 more
belts.
**Source.** `fle/env/tools/agent/connect_entities/client.py`; timings from
<https://arxiv.org/html/2503.09617v1>.

### 8. A periodic canonical state digest, diffed between runs

**What changes.** Every N ticks the mod writes a canonically-ordered digest of
the state that matters (bot positions and inventories, entity counts by type,
production totals, research). Two runs of the same script produce two digest
streams, and a diff names the first tick at which they diverge. This slots into
the existing tick-joined replay/frames timeline for free.
**Cost.** Small. A Lua writer plus a Rust differ. The canonicalisation
discipline this repo already applies in `crates/planner` (ordered collections,
`total_cmp`) is exactly what the digest needs.
**Replaces / invalidates.** Guessing from logs why a run that worked yesterday
does not today. Complements, does not replace, the replay event log — the log
says what was *intended*, the digest says what the world *was*.
**Source.** TAS State Printer (<https://mods.factorio.com/mod/TAS_state_printer>)
— Factorio 1.1, abandoned, idea intact. Also
<https://github.com/theis999/Factorio-TAS-Generator>, which names it as the
desync-debugging tool of choice.

### 9. Render the entity map server-side instead of screenshotting

**What changes.** The timeline's visual track becomes an SVG/canvas render of
the entity map at a tick, generated from data this project already collects,
rather than a JPEG from a graphical client. Scrubbing gets cheap, frames get
diffable, and the visual track survives idea 2.
**Cost.** Medium, all in the frontend plus a serialiser.
`teoxoy/factorio-blueprint-editor` (<https://github.com/teoxoy/factorio-blueprint-editor>,
unmaintained) is the reference for faithful rendering;
`piebro/factorio-blueprint-visualizer` for the cheap SVG version.
**Replaces / invalidates.** `mods/BotBridge`'s screenshot capture, the
`frames/` directory layout, the per-client run-id sidecar logic in
`app/src/api/frameJoin.ts`, and the "area camera below zoom 0.05" special case.
That is a lot of working code to retire, which is why this ranks below the
things that fix correctness. **Only do this as a consequence of idea 2.**

### 10. Batch RCON: one packet per tick carrying an action queue out and an event queue in

**What changes.** Instead of N independent RCON round trips per tick for N bots,
the executor accumulates actions and sends one packet per tick; the mod applies
them in order and returns the tick's events in the reply. Latency stops scaling
with bot count.
**Cost.** Medium, and it touches the executor's whole dispatch path.
**Replaces / invalidates.** The one-action-one-round-trip contract, and with it
the per-action error attribution — a batch needs per-action result slots in the
reply, which is a protocol change in BotBridge.
**Do this only if measurement says round trips are the bottleneck.** With ideas
1 and 5 in place they very likely are not.
**Source.** Clusterio 2.0 design
(<https://forums.factorio.com/viewtopic.php?t=83407>): "a single RCON packet per
tick to both read … and write-update".

### 11. Record runs in-game and edit them as data

**What changes.** A mod-side recorder captures a human's actions (build, rotate,
mine, transfer, recipe change, research, craft, walk) and emits them as a step
list the executor can replay and the planner can be checked against. Authoring a
100-step sequence becomes playing it once.
**Cost.** Medium. The recorder is straightforward event handlers; the value
depends on having a data step format, which this project does not yet have (Lua
scripts are imperative).
**Replaces / invalidates.** Hand-writing long Lua scripts to reproduce a
scenario.
**Source.** <https://github.com/theis999/TAS_step_planner>. Note its stated
gaps: pick-up/drop and queued research are not capturable via the API.

### 12. Ghost entities as the plan representation, with dirty-cell staleness

**What changes.** A plan is deployed as ghosts, annotated with the tick it was
planned against; the mod journals every world change (including construction-bot
and human changes) back into the plan's owning record; re-applying a plan whose
target cells changed since `as_of_tick` is **refused**. The plan and the world
share a representation, so "is the plan still valid" is a query, not a belief.
**Cost.** Large, and it is a re-architecture of `crates/planner`'s output type,
not an addition.
**Replaces / invalidates.** `Schedule` as an opaque action list; the
plan_world/real_world split in `crates/core/src/plan/planner.rs` would need
rethinking around a single ghost-annotated world.
**Maturity warning.** The one project doing this has **6 commits and 3 stars**
(<https://github.com/chebykinn/factorio-planning-agent>). The design is worth
reading; the code is not worth depending on. Treat as a direction, not a plan.

### Explicitly not worth stealing

- **Vision/CV pipelines** (`moeru-ai/airi-factorio` YOLO detection). The Lua API
  returns entities exactly; detecting them from pixels adds noise and a model
  dependency for nothing. Their TypeScriptToLua **mod hot-reload** is the only
  part worth a look, and it would blunt the stale-`workspace/mods` trap this
  repo documents.
- **Ratio calculators** (FactorioLab, Helmod, Foreman). They stop where this
  project's hard problem starts, and this project's planner already expands
  recipes.
- **Byte-exact replay verification.** The headless build cannot record replays
  (<https://forums.factorio.com/viewtopic.php?p=694877>), and any mod that
  drives the world is precisely the thing replays cannot survive
  (<https://wiki.factorio.com/Replay_system>). Use idea 8 instead.
- **gotyoke's TAS shape** (<https://github.com/gotyoke/Factorio-AnyPct-TAS>) —
  0.18-only, dead, and its own README records a single upstream yield change
  breaking the entire run. Zero recovery is not a design.
- **Full CSP blueprint synthesis as a general layout engine**
  (<https://arxiv.org/abs/2310.01505>) — intractable past ~100 tiles, by the
  authors' own measurement. Fine for a fixed-size module; not a base builder.
- **Thin LLM-over-RCON wrappers** (`lvshrd/factorio-agent`,
  `kovan/factorio-play-api`) — this project is already strictly past them.
