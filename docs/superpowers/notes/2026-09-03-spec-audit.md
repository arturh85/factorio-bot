# Spec and plan audit — 2026-09-03 (second pass)

**Question asked:** what in `docs/superpowers/plans/` (12 files) and
`docs/superpowers/specs/` (14 files) is genuinely unimplemented, what is done,
and what is now **obsolete**?

**Method.** Every document read against `master` at `4bfccea7`, by symbol, file
and line, across eight parallel readers, each result spot-checked. **No `cargo`
build, no test run, no live game run** — another agent is mid-change in
`crates/core`, `crates/scripting_lua` and `mods/BotBridge`, and a run would have
collided with its verification. So every claim here is *presence and wiring of
code*, never *green suite*.

**This supersedes `notes/2026-09-03-spec-and-plan-audit.md`**, taken at
`39cf19f7`. Overnight work closed its top two ranked items and falsified several
of its findings; corrections are in §5.

---

## 0. Read this first

**The checkboxes are worthless, and there are more of them than last time.**
`- [x]` across all twelve plans: **0 of 736**. (The previous audit counted 575;
the plans grew, the tick rate did not.) Nothing below was inferred from one.

**Nine of the fourteen spec `Status:` lines are false.** They are stale in the
*safe* direction — "approved, not yet implemented" written over shipped code —
which trains a reader to discount the field, and then the dangerous ones get
discounted too. Only `goal-values-design` and `run-record-enrichment-design`
are accurate. The two specs covering the **current milestone** both open with
*"Status: design only. Nothing implemented"* over ~85% shipped code.

**Line numbers in the specs are dead.** `crates/core/src/factorio/rcon.rs` grew
139 lines *during this audit*. Treat any `file:line` in a document older than a
day as archaeology, not as a map.

**Markers inserted this pass** (documents only, no source touched): the whole of
`server-camera-design`, eight sections of `video-capture-design`, six sections
of `run-recording-and-replay-design`, and the Global Constraints of the two
plans that still order `cargo fmt --all`.

---

## 1. The table

### Specs

| Document | Verdict | Justification (file:line) |
|---|---|---|
| `2026-08-29-multi-agent-planner-design` | **DONE** (4 named items never built) | All six layers ship: `PlanState::from_world` (`crates/planner/src/state.rs:946`), `Goal`/`Holder` (`goal.rs:76`/`:8`), `Method`/registry (`method/mod.rs:207,286`), `Action`/`Condition`/`Effect` (`action.rs:17,32,354`), `schedule()` (`schedule.rs:248`), executor with per-action `watch` signals (`run.rs:64-67`) and three recovery tiers (`recover.rs:288,320,348,362`). Unbuilt: `Holder::Chest`/`TakeFromChest`/`Consolidate`, `Schedule::critical_path` (0 hits), `Goal::Built`, `Action::pinned` from Lua. |
| `2026-08-29-webserver-replaces-tauri-gui-design` | **PARTIAL**, status stale | Swap complete — no Tauri in `app/src-tauri/src/`, `app/src/api/client.ts` + `http.ts:21` replace `invoke`. ~12 decisions implemented differently or dropped (§4.B). Genuinely open: the `crates/cli` and `restapi`→`server` renames, both cosmetic. |
| `2026-08-30-goal-values-design` | **DONE** | Surface installed at `globals/goal/value.rs:26`, pinned by "exactly the new surface" (`globals/goal/mod.rs:1596`). Grew three functions past the spec. Its whole "Out of scope, deliberately" list has since shipped. |
| `2026-08-31-supervisor-loop-design` | **DONE**, and outgrown | `scripts/supervisor.lua` (929 lines) + `supervisor_lib.rs` (1706 lines, 51 tests) loading the shipped file via `include_str!` *inside the sandbox* (`supervisor_lib.rs:16-19,30`) — stricter than D5 asked. D3 deliberately tightened; premise 3 now false (§4). **History, not a work item.** |
| `2026-09-01-per-bot-share-sizing-design` | **DONE** | `even_shares` (`method/have.rs:2210`), `distinct_bots` (`:2151`), the `BotsNotInterchangeable` guard deleted, all seven tests present, plus a `seats` cap the spec predates. §4's residual-hazard analysis is now inverted (§4). |
| `2026-09-01-produced-goal-design` | **DONE** | `Goal::Produced { item, count, whose, unlocks }` (`goal.rs:96-104`); `demand()` (`have.rs:107`), `attach_unlock` (`:130`), `holds` → `None` (`:218`). Gained `whose` beyond spec. Deliberately **not** exposed in Lua. |
| `2026-09-01-producing-goal-design` | **PARTIAL** (was filed "never started") | Layer 3 done, Layer 2 done *differently*, **Layer 1 absent**. `Goal::Producing` claimed by two methods (`have.rs:1992,1997`), answered by `holds` (`:227`), reachable as `goal.producing` (`globals/goal/value.rs:70`), used live. Missing: the ratio solver — no matrix, no rationals, just `ceil(per_minute * ticks_per_item / 3600)` (`produce.rs:150`). |
| `2026-09-01-run-record-enrichment-design` | **PARTIAL** (~90%) | Five real gaps (§3, G1–G5). Its central promise — `plan_empty` explains milestone 4 — is no longer how the code works. All frames passages obsolete. |
| `2026-09-01-run-recording-and-replay-design` | **PARTIAL / part OBSOLETE** | Recorder, JSONL, splits, retention (`retention.rs:12`), `/api/v1/runs*`, viewer: all shipped. **Every frames passage is dead** (`15c85c1f`) — six markers inserted. Never built: manifest identity fields, a `/runs/:id` route, streamed events. |
| `2026-09-02-material-convergence-design` | **PARTIAL** — stages 0, 1, 3 done; **stage 2 never started** | Stage 0: `Step::Owned` (`method/mod.rs:51`), `split_probe` (`:277`), `UnownedHandover` (`error.rs:208`). Stage 1: `SharedSmelt` (`have.rs:2448`), `worth_converging` (`:2333`) — **never measured live**. Stage 3: `buffered()` (`state.rs:1329`), `BufferHas` (`action.rs:183`), `Withdraw` (`have.rs:874`) — **never executed**. Stage 2 (`HandOff`): 0 hits. |
| `2026-09-02-mod-side-actions-design` | **PARTIAL** — steps 1, 3, 5, 6 open | Step 0 done differently (four mlua `on_tick` harnesses in `crates/core/tests/`), step 4 done end to end, craft rows superseded by `storage.craft_actions` (`control.lua:3171`). Step 2 partial. Your two claims confirmed — §2. |
| `2026-09-02-server-camera-design` | **OBSOLETE IN FULL** | Step 2 killed by the `--host` probe; step 1 never had a line of code and now has no subject — `record/frames.rs`, `manage/frames.rs`, `ArchivedFrame`, `EventKind::Frame`, `frameJoin.ts` all deleted in `15c85c1f`. `botbridge_sampling_session.rs:283-311` pins that the 300-tick beat can no longer reach `take_screenshot`. Markers under the title and all 11 headings. |
| `2026-09-02-video-capture-design` | **PARTIAL, mostly DONE** | Steps 2–5 built, wired, and run live (45m16s / 290 MB, `ffmpeg_exit: 0`). Clock (`video/clock.rs`), calibration (`recorder.rs:229,633`), encode line (`ffmpeg.rs:77-127`), routes (`manage/video.rs:77,93,121`), viewer (`videoClock.ts`, `ReplayScrubber.vue`). Only step 6 (steering) unbuilt — **and its gate is closed, not open**. Eight markers. |
| `2026-09-03-starter-factory-design` | **PARTIAL** (~85%) | Stage 1 done and witnessed; stage 2 built but never produced; stage 3 unreachable by the shipped cell shape. **Self-contradictory** — §4.1. Per-stage detail below. |

### Plans

| Document | Verdict | Justification |
|---|---|---|
| `2026-08-29-axum-server-replaces-rocket` | **DONE** | All 7 tasks. `crates/restapi` and every Rocket/okapi dep gone; 17 game routes (`crates/server/src/game/mod.rs:39-57`). **Its Global Constraints are now dangerous** — §4.A. |
| `2026-08-29-planner-scheduling-engine` | **DONE** | Every `Produces:` line resolves: `ids.rs`, the nine overlay methods (`state.rs:1414…:2822`), `ActionNetwork`'s eleven methods (`network.rs`), `schedule()` (`schedule.rs:248`), all four properties in `tests/scheduling.rs:145,187,292,388`, `render.rs:11,33,80`. Only unbuilt item: `Schedule::critical_path`. |
| `2026-08-30-frontend-transport-swap` | **DONE** | No `tauri` in `Cargo.lock`, no `tauri.conf.json`, one comment-only `invoke` (`app/src/api/http.ts:2`); `flake.nix` webkit/gtk/soup gone. Only the deferred `crates/cli` rename open. Six doc claims contradicted — §4.B. |
| `2026-08-30-goal-values` | **DONE** | T1–T7; module split as specified; migration complete — no live `goal.schedule`/`execute` in `scripts/`. |
| `2026-08-30-management-api-reads-and-mutations` | **DONE** | All 7 tasks plus the deferred `POST /api/v1/instance/start` (`manage/instance.rs:70`); `--web-root` (`cli/serve.rs:23`); bounded shutdown (`webserver.rs:129-169`). |
| `2026-08-30-planner-execution-increment` | **DONE** | Including destructive T8 — `graph/task_graph.rs`, `plan/plan_builder.rs`, `plan/execute.rs`, `gantt_mermaid.rs`, `globals/plan.rs` all gone; `roll_best_seed` tombstoned (`cli/roll_seed.rs:7-15`); the `world.draw` sandbox escape fixed (`globals/world.rs:231`). T7's *named* Lua surface was replaced wholesale (§4). |
| `2026-08-30-planner-goals-and-methods` | **DONE** | All methods and helpers present; `MAX_EXPANSION_DEPTH = 32` (`method/mod.rs:355`). Two plan-named symbols superseded by better ones (`free_area_near`, the derived coal bill). |
| `2026-08-30-planner-hardening` | **DONE**, T5 built then deliberately reverted | T1–T4 landed (`goal.rs:62`, `network.rs:75,181-232`, `util.rs:171`, `have.rs:412-420`). T5's `BotsNotInterchangeable` added `c68b00b5`, deleted `f3a22e29`. **Do not re-add** — `recover.rs:332-347` depends on it being gone. |
| `2026-08-30-script-execution-jobs-and-sse` | **DONE** | All 8 tasks; `jobs.rs` registry; the exact 400→503→404→409 ordering (`manage/execute.rs:113-215`); SSE `output`/`finished`/`lagged`. |
| `2026-08-30-shared-settings-and-serve-command` | **DONE** | All 6 tasks: `paths.rs`, `app_settings.rs:11-23`, `serve`, `ApiQuery<T>` (`extract.rs:29`). |
| `2026-08-31-shadcn-ui-redesign` | **DONE** | No `primevue`/`primeicons`/`sass`; zero PrimeFlex classes in non-spec source; every planned `components/ui/` primitive present; the eslint ignore list matches the code exactly (`eslint.config.mjs:82`). |
| `2026-09-01-run-record-enrichment` | **PARTIAL (~90%)** | Landed beyond what it asked (`runDiff.ts`'s `walksOf`, `planEpochs`, `inventoryAtFailure`). Five gaps (§3). Four of its instructions are now wrong — §4.C. |

### Starter factory, per stage

| Stage | Verdict | Evidence |
|---|---|---|
| **1 — burner smelting cell** | **DONE and WITNESSED** | Run 37: `WITNESSED: iron-plate in 1 watched machine(s) went 0 -> 1 … in 480 of 2400 ticks, 466 polls`, every bot idle. 480 ticks against a predicted 432 — model and game agree within 11%. |
| **2 — powered red-science cell** | **PARTIAL. Built, recipes set, never produced.** | Run 4: the game accepted all eight placements. Run 10: all nine actions settled `success`, **including both `set_recipe` calls** — the first time `set_recipe` ever executed in a game. Then it died in the smelting work that charges the chests, before any witness ran. **Nine actions succeeded, zero packs exist.** |
| **3 — green science** | **NEVER STARTED, and structurally out of reach** | `assembly_spec` (`assemble.rs:258-296`) needs a 2-ingredient recipe with *exactly one* single-ingredient intermediate. `logistic-science-pack` = 1 transport-belt (2 ingredients) + 1 inserter (3) — `intermediate_for` returns `None` for both, so it falls to `_ => return None`. **The blocker is the cell shape, not the ratio solver.** |

---

## 2. Your three prior beliefs, checked

**Confirmed, with corrected line numbers:**

* `mods/BotBridge/control.lua:874` (you said 945) — `if player.connected and player.character then -- TODO FIXME`, still wrapping the *entire* per-player body, so a characterless bot freezes the walking **and** mining followers with no verdict. None of `on_player_died`, `on_pre_player_left_game`, `on_player_respawned`, `on_player_controller_changed` is registered.
* `crates/core/src/factorio/rcon.rs:40` (you said 39) — `ACTION_RESULT_DEADLINE: Duration = Duration::from_secs(360)`, consumed purely on `Instant::elapsed` (`sleep_for_action_result_until`, `rcon.rs:2015-2046`). **The tick-denominated version needs no mod change**: `Actuator::game_tick` already exists (`crates/executor/src/actuator.rs:279`, used by `run::wait_out_lag` at `run.rs:588`).
* Stage 1 done and witnessed; stage 2 builds the cell and sets both recipes but has never produced a pack; stage 3 not started. **All three confirmed.**
* The §8.4 / §14 contradiction is real. **Confirmed and recorded** — §4.1.

**Two corrections:**

* Mod-side step 1 is not untouched: `on_player_cancelled_crafting` **is** registered (`control.lua:2079`, registered `:2301`) — one of its four bullets. The other three are open.
* Steps **5 and 6 are also open**, and step 2 is only partial. It is four-and-a-half steps outstanding, not two.

---

## 3. What is genuinely left, ranked

Ranked by leverage on *an automated red-science cell a witness can see produce*,
not by ease.

### 1. Fire the stage-2 witness — a run that survives to the chest charge
**No design work; this is the milestone.** Everything upstream is verified in a
real game: eight placements accepted (run 4), nine actions including both
`set_recipe`s (run 10). What has never happened is the 30-iron / 15-copper smelt
that fills the chests. `scripts/factory_stage2.lua:190-198` already carries the
witness call.
**Where:** `scripts/factory_stage2.lua`, unchanged. **Size:** a run, not a diff.
**Unblocks:** the milestone itself, and items 8 and 9 below as a side effect.

### 2. Actions that never settle — mod step 1 + the Rust half of step 3
**The failure with a measured cost, and the one implicated in the most recent
dead run.** Run 11 sat 13 minutes with 104 actions planned and **zero
dispatched**; the only activity was two `too far away, moving first!` warnings
**11.6 minutes apart** — that is `ACTION_RESULT_DEADLINE` expiring twice with
nothing settling in between. One bot losing its character costs six minutes of a
forty-five-minute run, and the gate at `control.lua:874` guarantees no verdict
ever arrives.
**Where:** `mods/BotBridge/control.lua` (invert the gate at `:874`, add
`settle_player_actions`, register `on_player_died` / `on_pre_player_left_game`,
make `on_player_left_game` at `:1961` call it) — **~100–150 lines, mod only, wire
format unchanged.** The Rust half is `sleep_for_action_result_until`
(`rcon.rs:2015`) taking a tick budget off `self.game_tick()` — **~40–60 lines,
lands independently.**
**Unblocks:** item 1, and the whole 360-second-freeze class.
*(Both files are owned by another agent right now.)*

### 3. The owner decision on the anchor contradiction — §8.4 vs §14
**Not a bug; a design question nobody has answered, and it decides what "stage 2
done" means.** The shipped cell is 7 parts hand-charged for
`CELL_CHARGE_TICKS = 9_000` ticks ≈ 2.5 minutes (`assemble.rs:112`), and
**nothing detects that it has stopped.** §14 promises something else entirely:
*"`Researched("logistic-science-pack")` completes without a bot hand-crafting 75
packs"* — 15 packs per charge against 75 needed. Three options, none designed:
a pole line from the plant to the ore patch (no routing primitive); a second
ore-anchored stage-1 cell plus haulage (no belt primitive); or accept
hand-charged forever and amend §14 to say so.
**Where:** a decision, then `crates/planner/src/method/assemble.rs`.
**Unblocks:** an honest definition of the milestone. **The wood objection to the
pole-line option has evaporated** — `Chop` (`have.rs:1196`, `b0e3e12e`) makes
wood from trees, so poles are no longer capped at eight.

### 4. Amend the nine lying status lines and the two milestone spec headers
**Ten minutes, and it is the cheapest item on this list by an order of
magnitude.** Both specs covering the current milestone open with *"Status:
design only. Nothing implemented"* over ~85% shipped code. The previous audit
said the same thing yesterday and nothing was edited; the specs' own
`git log` shows no touch since 2026-09-02.
**Where:** the `Status:` line of nine specs, plus §14's stage-2 part list.
**Unblocks:** every future reader, human or agent, from re-planning finished work
and re-deriving the contradiction from scratch.

### 5. Boiler autonomy
§7's `iron-chest` + `burner-inserter` fuel buffer was **not built**. What shipped
is `boiler_coal()` (`assemble.rs:890-901`), a one-shot bot insert capped at
`COAL_STACK = 50`, because a boiler's fuel inventory is one slot. So the cell
still needs a bot visit every few minutes — the exact thing §7 claimed to
eliminate. **Note the spec's arithmetic is wrong**: one stack is ~13 minutes at
the real 249 kW, not the 5.4 it computes off a phantom 621 kW.
**Where:** `crates/planner/src/method/assemble.rs`. **Size:** small.
**Unblocks:** removes a whole class of "the cell stopped and nobody knows why".

### 6. A pin test for `scripts/factory_stage2.lua`
`supervisor.lua`, `research_run.lua` and `factory_stage1.lua` each have a Rust
harness that `include_str!`s the shipped file and drives it
(`supervisor_lib.rs:16`, `research_run_lib.rs:17`, `factory_stage1_lib.rs:18`).
**The stage-2 driver — the script that runs the current milestone — has none.**
A change to `supervisor.witness`'s contract or to `goal.producing` breaks it
silently, and only a live run finds out. Given that a live run currently costs
45 minutes and dies for unrelated reasons, that is an expensive way to learn.
**Where:** a new `crates/scripting_lua/src/factory_stage2_lib.rs` + one line in
`lib.rs`. **Size:** ~150 lines, modelled on its sibling.

### 7. Three stale doc comments inside the milestone's hot file
`assemble.rs`'s `POLE_OFFSET` doc still asserts *"this planner cannot make wood"*
and *"four bots is four wood and eight poles for the whole run"*;
`Cell::brings_pole` repeats it; `CELL_CHARGE_TICKS`'s doc misquotes its own
script (says the witness waits 2,400 ticks; `factory_stage2.lua:185` sets 3,600
— 2,400 is *stage 1's* window). The first two are false since `b0e3e12e`, and
they will be quoted against item 3.
**Where:** `crates/planner/src/method/assemble.rs`. **Size:** trivial.

### 8. Execute `Withdraw` once, and measure `SharedSmelt`
Both landed, both wired, **neither has ever met a game** in the way that matters:
`Withdraw` has never been executed, and `SharedSmelt`'s justification (milestone
6's 33/1/1/1 distribution) has never been re-measured. Item 1's smelting step is
exactly the workload both were built for, so one run settles both — read
`plan_created` from it.
**Size:** zero code; one reading.

### 9. Run-record gaps
`G1` map `removed` lines (`map.rs:36-45` says the `take_removal` seam does not
exist); `G2` the periodic keyframe — spec says *"every milestone boundary **and
every 9,000 ticks**"*, and there is no cadence constant anywhere, so a run with
one long milestone gets one keyframe; `G3` `ResearchSample.eta_ticks`, hard-coded
`nil` (`control.lua:1651`); `G4` manifest carries no seed/bots/versions/git-sha
(`record/mod.rs:681-704`), which is why `runsStore.ts:50` has to reconstruct the
roster from samples; `G5` no `/runs/:id` route — you can deep-link the analysis
view but not the run.
**Size:** G2 and G4 are small and independently useful; G1 needs an actuator seam.

### 10. Green science needs a new cell shape, not a matrix
`assembly_spec`'s "exactly two ingredients, exactly one single-ingredient
intermediate, the other not craftable" rule rejects `logistic-science-pack`
outright. **That rule is the work item.** D1/D2's `Ax = b` solver and rationals
buy nothing until it changes.
**Where:** `crates/planner/src/method/assemble.rs:258-296`. **Size:** large.

### 11. Everything else
Mod step 2 (the `4711`/`4712` id leak is ~50 lines; the 400–600-line unification
is mostly spent), step 5 (approach into the mod — **now seven `move_player`
sites, not the five the spec says**), step 6; encoder UPS cost (**already
measured**: 57.12 vs 57.08, −0.07% — noise); the `crates/cli` and
`restapi`→`server` renames; `enable_autostart` and `enable_restapi`, two
published settings with no reader; tree keyboard nav.

---

## 4. What the documents assert that the code contradicts

This is the part worth acting on. Ordered by how badly following the document
would hurt.

### A. Actively destructive instructions

1. **`cargo fmt --all` is ordered by two plans and banned by CLAUDE.md.**
   `plans/2026-08-29-axum-server-replaces-rocket.md:21` (*"`cargo fmt --all`
   before every commit"*) plus six verification commands at `:301, :513, :631,
   :819, :1014, :1204, :1289`; and
   `plans/2026-08-30-frontend-transport-swap.md:674, :877, :3566`. CLAUDE.md
   bans it because it has **already rewritten another agent's live files** in
   this shared checkout. The other five plans get this right and say so
   explicitly. Markers inserted; the commands themselves left in place.
2. **"Rust edition 2021"** in every plan preamble. Every crate is
   `edition = "2024"`. This is not cosmetic: CLAUDE.md requires
   `rustfmt --edition 2024 <file>` precisely because a bare/2021 `rustfmt` dies
   on every `async fn` and reads as a compile error in code that builds fine.
3. **`management-api` Task 6 Step 5 re-introduces a fixed bug.** It prescribes
   wrapping the shutdown await in `tokio::time::timeout(Duration::from_secs(10), …)`.
   `webserver.rs:117-128` records that this exact implementation was tried and
   kills the server 10 s after start even with no shutdown signal, and is now
   guarded by `server_survives_past_the_grace_period_with_no_shutdown_signal`.
4. **`run-record-enrichment` Task 2 Step 4's `power_totals` code is wrong.** It
   sums `pole.electric_network_statistics`' `input_counts`/`output_counts`.
   `control.lua:1417-1500` rejects both halves against `runtime-api.json`: those
   are **cumulative joules**, and for electric networks `input_counts` is
   **demand, not generation** — the inverse of the item convention the plan
   copied. Implementing it literally ships power numbers wrong by orders of
   magnitude, plausibly.
5. **`producing-goal-design` D4 names the wrong goal, and following it produces a
   replan loop.** D4 (`:84-87`) says each entity in the layout becomes a
   `Goal::Produced`. `produce.rs:554-557` says in place: *"`Goal::Have` and not
   `Goal::Produced`, **which is where the 2026-09-01 design named the wrong
   goal**: `Produced` deliberately ignores inventory … so a bill written with it
   re-crafts machines the bot is already carrying, every replan."*
6. **`axum` plan: "`tower-http` MUST be pinned to `0.6`, not `0.7`".** False
   today — `crates/server/Cargo.toml:18` declares `"0.7"` and `Cargo.lock` holds
   exactly one copy. The constraint would send someone to downgrade for nothing.
7. **`planner-scheduling-engine` T3 prescribes `petgraph` as a direct
   dependency.** The planner manifest has none; it uses core's re-export
   (`network.rs:6-7`). Copying the plan's manifest block produces a wrong crate.

### B. Load-bearing claims that are simply false

1. **THE SELF-CONTRADICTION — confirmed, and it is why stage 2 is not a factory.**
   §8.4 (`:724-728`): *"stage 2's [origin] **is anchored to the supplying pole**,
   exactly as `lab_site` is."* §14 (`:1103-1104`): *"three furnaces, **two
   drills**, ~12 inserters"*, reinforced by §7 (`:561`) and the §6.1 table's
   `electric-mining-drill | 2`. These cannot both hold, because §8.1 (`:596`)
   makes the cell *"a fixed cell, stamped at a sited origin and rotated as a
   rigid body"* — every part at a fixed offset from the origin. If the origin is
   the pole, and the pole is at the plant, and `power.rs` sites the plant on a
   shoreline, then a drill at a fixed offset must stand on ore **beside the
   water**. That is a map accident, not a design.
   **The code broke the tie unilaterally: §8.4 won.** `BuildAssemblyCell::expand`
   (`assemble.rs:1359`) sites from `nearest_supply_anchor` (`state.rs:2066`), and
   the drills and furnaces were **deleted from the design** — their job replaced
   by bots hand-charging two chests. The spec still describes the version that
   lost. *(Already caught in `notes/2026-09-03-red-science-layout.md:166-173`,
   which the spec does not link to.)*
2. **`multi-agent-planner` §353-356: *"Completion is observed, never assumed."***
   It is assumed. `run.rs:462` calls `wait_out_lag`, which sleeps against a tick
   budget (`run.rs:588-620`), then `RconActuator::remove` fires.
   `inventory_contents_at` has **zero callers in `crates/executor`**. Crafting
   *is* observed, but by mod-side action completion (`rcon.rs:2433`), not by
   inventory polling.
3. **`mod-side-actions` §1.1/§1.2: research *"nobody waits for it"*, completion
   *"none bound to an action"*, Rust *"returns the moment the tech is queued"*.**
   All false. `research_actions()` is keyed by technology name with an array of
   ids (`control.lua:3121`), `settle_research_actions` (`:2241`) fires on
   `on_research_finished` (`:2219`), and `research_timed` (`rcon.rs:1744`) waits.
   The spec bolted a "Superseded" postscript onto the *craft* rows and left the
   research rows looking current.
4. **`mod-side-actions` §1.2 and §6: *"the mod teleports on a stuck leg"*.** It
   does not, since `98895500`. The branch (`control.lua:976-1000`) says: *"**The
   recovery does not live here, and it never should have.**"* The only two
   `player.teleport` calls left are ghost-revive and blueprint paths. §6 rejects
   a fast/teleport alternative *on the strength of* that teleport, so its
   conclusion now rests on a false premise.
5. **`supervisor-loop` premise 3: *"`Goal::Producing` has no method."*** The test
   it cites (`producing_has_no_method_in_this_increment`) no longer exists
   anywhere. Two methods claim it. The whole "Future path" section is stale.
6. **`supervisor-loop` D3: an empty plan is satisfaction.** No longer the rule —
   the goal must *also* be confirmed to hold (`supervisor.lua:4-5`), pinned by
   `an_empty_plan_for_an_unheld_goal_is_refused_rather_than_reported_satisfied`.
   D3 as written is now the wrong rule.
7. **`per-bot-share-sizing` §4: *"A share's chain has no owner"*, citing a test
   that asserts `owner_of(chain) == None`.** That same test now asserts
   `Some(BotId(1))` (`method/mod.rs:1991-1995`). A live four-bot run crashed
   twice on the hazard the section argued was safe.
8. **`run-record-enrichment` spec §3.4: `MilestoneSatisfied` gains reason
   `PlanEmpty`, *"the one [change] that would have explained milestone 4"*.**
   `record/mod.rs:457-467`: *"**No live writer emits this any more**, as of
   `ee623717`."* The ambiguity was resolved by a different mechanism — that
   branch now halts `stuck`. A reader will look for `reason: "plan_empty"` in new
   runs forever.
9. **`starter-factory` §6.1's ≈621 kW stage-2 power table is wrong for what was
   built.** `cell_demand_kw` (`assemble.rs:659-664`) is `2×75 + 3×13` = **189 kW**;
   with the lab, 249 kW on 900 — **28% load, not 69%**. Every number downstream
   in §6.2 and §7 is scaled off the phantom figure.
10. **`starter-factory` §14 is circular in its own numbers.** Electric drills need
    the `electric-mining-drill` research = 25 red packs (§5:389), and stage 2 is
    the thing that makes red packs. The code dodges it only by having no drills.
11. **`webserver` spec error shape `{error, detail}`.** Actual is
    `{message, code, running_job_id?}` (`error.rs:24-36`). Every client written
    from the spec reads the wrong keys. Same class: SSE events are
    `output`/`finished`/`lagged`/`replay`, not `line`/`status`/`done`.
12. **`planner-execution-increment` T7's seven named Lua functions.** Only
    `goal.have` and `goal.researched` survive; `goal.schedule`, `goal.graphviz`,
    `goal.execute`, `goal.progress`, `goal.wait` have **zero hits**. The
    `multi-agent-planner` spec's Lua block (`plan.goal`, `plan.solve`,
    `schedule:gantt()`, `plan.execute`) is worse — **a script written from it runs
    zero lines.**
13. **`frontend-transport-swap` follow-up #7 describes a mechanism that does not
    exist.** Six passages (`:7, :88, :137, :3351, :3512, :3966`) say TS
    generation *moved* to `crates/server/build.rs`. There is no
    `crates/server/build.rs`; `typescriptify` has zero hits workspace-wide. Task
    13 reversed the decision and the summary sections were never updated.
14. **`shadcn-ui-redesign`'s route arithmetic** ("five routes", four places).
    `router.ts:12-53` declares eight. And `GanttChart.vue`, which the plan says
    stays a stub, no longer exists — it became `ReplayPanel.vue`.
15. **Every frames passage in three documents.** `/api/v1/frames`,
    `ArchivedFrame`, `EventKind::Frame`, `frameJoin.ts`, `frames/index.json`,
    `frames/run.json`, "Bots and clients are 1:1", "Frames stay the record".
    All deleted in `15c85c1f`. Markers inserted.

### C. One contradiction inside the code itself

`crates/core/src/record/video/mod.rs:10-19` still carries the heading
**"Frames stay the record; video is opt-in"** and argues at length that the
stronger record must not be retired for the weaker one — a decision that was
reversed six commits later. Lines `:25` and `:32` still reference
`frames/run.json`. **Not touched here** (`crates/` is another agent's), but it is
the same failure mode as the specs, sitting inside the crate that owns the
feature.

---

## 5. Corrections to the previous audit (`2026-09-03-spec-and-plan-audit.md`)

Its ranked list has moved substantially in one day. Do not act from it.

* **Its #1, "material convergence stage 3 — buffer visibility", is DONE.**
  `PlanState::buffered` (`state.rs:1329`), `Condition::BufferHas`
  (`action.rs:183`), `Effect::BufferLose` (`:393`), `Withdraw` (`have.rs:874`,
  registered `:1983`), and the game is actually asked at plan time by
  `Planner::refresh_buffers` (`crates/core/src/plan/planner.rs:178`), proven live
  in run 35. It claimed *"zero hits repo-wide"* for these symbols. **The
  `Withdraw` method has still never executed**, which is the honest residue.
* **Its #2, "`Goal::Producing` — unreachable from Lua", is DONE.** Two methods
  claim it, `goal.producing` is a shipped constructor used by two live scripts,
  and it is documented (`__doc_entry_producing`). The field is `per_minute: u32`,
  not the spec's `rate: f64` — changed deliberately for determinism
  (`goal.rs:118-128`).
* **Its #3, mod-side steps 1 and 3, stands** — and is now correctly #2 here.
* **Its "server-camera step 1 is dormant correctness debt"** is void: the feature
  it was debt against no longer exists.
* **Its "encoder UPS cost never measured"** is closed: 57.12 vs 57.08 UPS,
  −0.07%. Noise, and it was never the real argument — the disk figure was.
* **Its "the resolution knob is inert"** is closed by `95839c66`: the window is
  opened at 720p via `config.ini` rather than resized with `xdotool` afterwards.
* **Its "`per-bot-share-sizing` landed"** — correct, but it missed that the
  spec's §4 hazard analysis was *inverted* by the same work.
* **Its checkbox count (575)** is now 736. Same zero.
* **What it got right and is worth repeating:** the checkboxes are worthless; the
  status lines are worse; and `f3a22e29`'s deletion of `BotsNotInterchangeable`
  must not be undone.

**One thing it flagged that has since been fixed, and is worth knowing because
you named it:** CLAUDE.md's remedy for a stale mod — *"delete `workspace/mods` to
re-seed"* — did break runs, and `e15faf5a` fixed both the code (a debug build now
**symlinks** `workspace/mods/BotBridge` to the checkout) and the document
(master's CLAUDE.md:397 now says *"Do NOT 'delete workspace/mods to re-seed'"*).
It also un-gated the `Using mods directory …` line, which had been suppressed on
every CLI path — the one check the docs told you to trust printed on no run at
all.

---

## 6. Now unnecessary — do not spend time on these

* **The entire `server-camera-design`**, all seven steps. Step 0's A/B
  measurement can never be run; steps 1–5 have nothing left to change; step 2 is
  disproven; step 6 depended on `--host`.
* **Convergence stage 2 (`HandOff` + the `iron-chest` handover).** Its own §14.7
  said stage 1's measured distribution decides it; stage 1 shipped; and
  `iron-chest` is now used for *cell inputs*, not bot-to-bot handover. **Cancel it
  explicitly** — it has now appeared as "open work" in two consecutive audits.
* **The `Ax = b` recipe matrix and exact rationals (D1/D2).** Stage 3 is blocked
  on the cell *shape*, not on ratio arithmetic. Building the solver first buys
  nothing.
* **`Schedule::critical_path`.** Zero hits and nothing wants it — `render.rs`,
  `executor/replay.rs` and the frontend all consume `steps`. Delete it from the
  spec rather than building it.
* **`Holder::Chest` / `TakeFromChest` / `Consolidate` as a general mechanism.**
  Superseded by `Step::Owned` + `SharedSmelt` + `Withdraw`.
* **"`Craft` occupies its bot exclusively" as a free optimisation.** Now
  **closed-negative**: `network.rs:136-147` drops cross-chain edges on the
  strength of "two actions on one bot cannot overlap in time". Overlapping craft
  with walking would invalidate edge inference.
* **`PLATES_PER_COAL`** (superseded by the derived coal bill, `have.rs:274-291`),
  **`BotsNotInterchangeable`** (deleted by decision), **`Effect::MoveTo`** (zero
  hits; re-adding reopens the `Walk`-in-`ActionKind` hole).
* **Moving TS generation to `crates/server/build.rs`, `embed-spa`/`rust-embed`,
  `POST /scripts/run` + `/scripts/eval`, giving `instance/start` a job id, and
  moving autostart to server startup.** All superseded or explicitly reversed
  with reasoning in place (`App.vue:74-83`).
* **`starter-factory` §8.4's "one free-rect query per candidate origin"** — the
  cell turned out to be 7 small parts, not the ten-machine object the cost
  argument was sized for. `fit()` (`assemble.rs:612`) checks every part
  individually and nobody has complained.
* **`starter-factory` §15.8 (big-pole wire reach 30 vs 32)** — retracted as
  unverifiable, and nothing uses a big pole.
* **`mod-side-actions` §8.5 and §8.6** — two of its seven "could not be determined
  without running the game" questions were answered by landed code, not by a run.
  Strike them.
* **Two published fields with no reader**: `GuiSettings.enable_restapi`
  (`app_settings.rs:14`, in the OpenAPI snapshot and `models/settings.ts:23`,
  gating nothing) and `MapKind::Removed` / `ResearchSample.eta_ticks` (both
  permanently null on the wire). Either build the writer or drop the shape.
* **The `openapi:snapshot` pnpm script** (`app/package.json:13`) — a second source
  of truth that curls a possibly-stale running server. The authoritative path is
  `UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p factorio-bot-server --test openapi`.

**One thing the mod-side spec flags that is still live and must not be dropped:**
§5's warning about `on_mined_entity` — still scoped to `event.player_index` and
still decrementing by the real `event.buffer` (`control.lua:1198`). A naive
"find the action whose entity matches" sweep in step 2 would reintroduce the bug
it fixed.

---

## 7. What could not be determined

* **Nothing here is green-suite evidence.** No cargo command was run, by
  instruction. Every claim is that a symbol exists and is wired.
* **Whether run 11's 13-minute zero-dispatch stall is fully explained by
  `ACTION_RESULT_DEADLINE`.** The signature fits exactly (two warnings 11.6
  minutes apart = two 360-second expiries), but `morning-summary.md:196-256`
  leaves the stall itself open, and several candidates were ruled out.
* **Whether the `4711`/`4712` magic action ids actually accumulate in
  `world.actions` at runtime.** `output_parser.rs:226-286` inserts every parsed
  id unconditionally and nothing in Rust filters either; no note re-confirms it
  since the second id was added.
* **Whether stage 2's hand-charged cell is intended to be the final answer.**
  That is §4.B.1, and it is a decision, not a finding.
