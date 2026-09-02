# Spec and plan audit — what was designed and never built

**Date:** 2026-09-03
**Question asked:** "can you check our specs/plans, do we have any open items we
missed implementing?"
**Method:** every one of the 13 specs and 12 plans read, then each concrete
commitment checked against the tree at `39cf19f7` by symbol, file, line and
commit. **No build, no test, no Factorio run** — a live four-client run held the
CPU. So every claim below is *presence and wiring of code*, never *green suite*.

---

## Read this first: the checkboxes are worthless

Every plan uses `- [ ]` / `- [x]` syntax. **All twelve plans are 0% ticked** —
575 boxes, none of them checked, including the plans whose work demonstrably
shipped and whose crates you use every day. The convention was never adopted.

```
2026-08-29-axum-server-replaces-rocket.md      total=53  done=0
2026-08-29-planner-scheduling-engine.md        total=43  done=0
2026-08-30-frontend-transport-swap.md          total=137 done=0
2026-08-30-goal-values.md                      total=40  done=0
2026-08-30-management-api-reads-and-mutations  total=42  done=0
2026-08-30-planner-execution-increment.md      total=60  done=0
2026-08-30-planner-goals-and-methods.md        total=43  done=0
2026-08-30-planner-hardening.md                total=32  done=0
2026-08-30-script-execution-jobs-and-sse.md    total=69  done=0
2026-08-30-shared-settings-and-serve-command   total=38  done=0
2026-08-31-shadcn-ui-redesign.md               total=104 done=0
2026-09-01-run-record-enrichment.md            total=75  done=0
```

**A plan's progress in this project can only be read off the code.** Nothing was
inferred from a checkbox in this audit, and nothing should be.

The spec **Status** lines are worse than useless in a different way: they are
mostly *stale in the safe direction* (claiming "not implemented" when the work
landed), which trains a reader to discount them — and then one of them is stale
in the **unsafe** direction and gets believed. See "Status lines that lie".

---

## Verdict counts

| Verdict | Count |
|---|---|
| `landed` | 19 |
| `partially landed` | 4 |
| `never started` | 2 |
| `superseded` / `abandoned` | 0 whole documents (several parts — named inline) |
| **total** | **25** |

---

## The table

### Specs

| Document | What it claims | What is in the code | Verdict |
|---|---|---|---|
| `2026-08-29-multi-agent-planner-design` | "Approved design, not yet implemented" | **Stale.** All six layers exist: `PlanState` (`crates/planner/src/state.rs`, `44e856d5`), `Goal`/`Holder` (`goal.rs:77`/`:8`), `Method`/`Step` (`method/mod.rs:17`), `Action`/`Condition`/`Effect` (`action.rs:17,32,228`), `schedule()` (`schedule.rs:125`), executor (`crates/executor`, tiers at `recover.rs:288-362`, `5e775ede`) | `landed` — 4 named items never built, see below |
| `2026-08-29-webserver-replaces-tauri-gui-design` | "Approved design, not yet implemented" | **Stale.** Rocket gone (`54f18018`), axum `crates/server` with all 16 game routes, utoipa + Swagger UI, job registry + SSE, `ServeDir` + index fallback (`spa.rs:33-47`), Tauri deleted (`555238b3`). Five spec decisions never built | `partially landed` |
| `2026-08-30-goal-values-design` | "approved 2026-08-30" | Full surface: `goal.have/researched/all/plan/holds/refusal/start/run`, pinned in both directions by `the_goal_table_offers_exactly_the_new_surface` (`globals/goal/mod.rs:1449`). Grew two functions past the spec | `landed` |
| `2026-08-31-supervisor-loop-design` | "approved, not yet implemented" | **Stale.** `scripts/supervisor.lua` (464 lines, `456c16f4`); `TERMINAL` at `:121`; Rust tests `include_str!` the shipped file exactly as D5 demanded (`supervisor_lib.rs:15-19`); every named test exists plus ~20 more | `landed` |
| `2026-09-01-per-bot-share-sizing-design` | "design only, not implemented" | **Stale.** All six steps in ~24h: `even_shares` (`have.rs:1812`), `distinct_bots` (`:1753`), guard deleted (`f3a22e29`), all seven new tests present | `landed` |
| `2026-09-01-produced-goal-design` | "approved in outline, not yet implemented" | **Stale.** `cf0d7bff`. `Goal::Produced { item, count, whose, unlocks }` (`goal.rs:96`), effect attached by the producing method (`have.rs:152`), `whose` added beyond the spec | `landed` |
| `2026-09-01-producing-goal-design` | "specified, not implemented" | **True.** `Goal::Producing { item, rate }` at `goal.rs:107`; `have.rs:212` returns `None`; pinned unreachable by `producing_has_no_method_in_this_increment` (`method/mod.rs:1401`); **not exposed in Lua at all** | `never started` |
| `2026-09-01-run-record-enrichment-design` | "approved for planning" | Every section: `samples.rs` (`aaffe826`), `map.rs` (`940fcfb4`), keyframes + divergence, `PlannedStep`/`SatisfiedReason`/`FailureKind` (`159ccc2d`), analysis view (`f3440ab7`). Two deliberate reversals | `landed` |
| `2026-09-01-run-recording-and-replay-design` | "approved in outline, not yet implemented" | **Stale.** Run dirs, JSONL with `#[serde(other)] Unknown`, splits (`a4170bac`), retention (`bedb38a6`), `/api/v1/runs` (`57257a00`), `RunsPage.vue` (`6a75bcf5`). Overshot the design | `landed` |
| `2026-09-02-material-convergence-design` | "design only. Nothing implemented" | **Stale, but only two of four stages.** Stages 0+1 landed (`86c2d28e`): `Step::Owned`, `SharedSmelt`, `worth_converging` (`have.rs:1935`). **Stage 2 and stage 3 absent** | `partially landed` |
| `2026-09-02-mod-side-actions-design` | "design only, not implemented" (+ craft rows marked superseded) | **Stale.** Awaited research landed (`f7b60f1c`); crafts superseded by `d0a5e094`; the mlua tick harness exists (`botbridge_walk_repath.rs:202`). **Steps 1, 2, 3, 5, 6 never started** | `partially landed` |
| `2026-09-02-server-camera-design` | "Design only … the two experiments have not been run" | **Stale in BOTH directions.** One experiment *was* run and killed step 2 (`88294571`); the other never was. Step 1: zero code — no `camera_subject`, no `options.renderer`, `client` axis unrenamed | `never started` (step 1) / step 2 `abandoned` |
| `2026-09-02-video-capture-design` | "Steps 2–5 built … nothing has been run against a live Factorio … Step 1 outstanding, Step 6 gated" | **Stale.** Two live runs; run 30 = 290 MB, 45m16s, `ffmpeg_exit: 0`. Bitrate now measured twice. **Encoder UPS cost still unmeasured; step 6 never started** | `partially landed` |

### Plans

| Document | Boxes | What is in the code | Verdict |
|---|---|---|---|
| `2026-08-29-axum-server-replaces-rocket` | 0/53 | T1–T7 all landed: `529570aa`, `ca7dcd0`, `01a4340`, `b16b6cd`, `8bb9324`, `54f18018`. Zero `.unwrap()` in handler code | `landed` |
| `2026-08-29-planner-scheduling-engine` | 0/43 | T1–T7 landed. Only unbuilt item: `Schedule::critical_path` (**0 hits repo-wide**) | `landed` |
| `2026-08-30-frontend-transport-swap` | 0/137 | All 17 tasks + 1b/3b/3c. Zero `@tauri-apps` or `invoke(` in `app/src`; `555238b3` removed the shell. Six named follow-ups still open | `landed` |
| `2026-08-30-goal-values` | 0/40 | T1–T7 including the two finicky late steps (`lua_docs.rs:581-594` bidirectional; `CircularWait` surfaced at `run.rs:396-403`) | `landed` |
| `2026-08-30-management-api-reads-and-mutations` | 0/42 | All 7 tasks incl. the last: `34d23c18` create/delete over HTTP; `9a588429` `--web-root` + bounded shutdown | `landed` |
| `2026-08-30-planner-execution-increment` | 0/60 | All 8 incl. the destructive T8 (`b52688f7` deleted the task graph). Its open question (game speed) closed by `a4e7b6fc` | `landed` |
| `2026-08-30-planner-goals-and-methods` | 0/43 | T1–T7. Three of its four self-declared deviations still stand; the fourth (`Researched` has no method) is closed | `landed` |
| `2026-08-30-planner-hardening` | 0/32 | T1–T4 landed (`d526d7f8`, `6e3c77f2`, `e128bf4e`, `60866cff`). **T5 landed `c68b00b5` then was deliberately deleted `f3a22e29`** | `landed` |
| `2026-08-30-script-execution-jobs-and-sse` | 0/69 | All 8 incl. T8's doc-generator bug (`d8cee6aa`). Its one parked item (hand-written `types.lua`) closed later | `landed` |
| `2026-08-30-shared-settings-and-serve-command` | 0/38 | All 6. T6 chose the local newtype `ApiQuery<T>` (`extract.rs:29`) over `axum-extra` | `landed` |
| `2026-08-31-shadcn-ui-redesign` | 0/104 | All 12, contiguous `09d7737d`…`f37c5344`. No `primevue`/`primeicons`/`sass` in `package.json` | `landed` |
| `2026-09-01-run-record-enrichment` | 0/75 | **73 of 75 steps** correspond to real code or a real archived run. Two gaps, both deliberate | `landed` |

---

## Detail: everything not fully landed

### `2026-09-02-material-convergence-design` — stages 2 and 3 open

Stages 0 and 1 landed at `86c2d28e` and are **running in production today**.
Stage 3 was declared by its own §13 a **hard gate before this design runs
unattended for hours**. It is not built, and every one of the four facts its §8
rests on is still true:

1. `PlanState` has no container model — no `buffers`, no `buffered()`.
2. `FactorioWorld::on_some_entity_updated` is still `Ok(())` under a `// TODO`
   at `crates/core/src/factorio/world.rs:351`.
3. `FactorioRcon::inventory_contents_at` (`rcon.rs:2204`) has exactly two
   callers — the HTTP handler (`server/src/game/query.rs:228`) and the Lua
   binding (`globals/rcon.rs:713`). **No planning path calls it.**
4. `update_entity_inventory`, `Condition::BufferHas`, `Effect::BufferGain`,
   `Effect::BufferLose` and `Withdraw` — **zero hits repo-wide**.

Stage 2 (`HandOff` + `iron-chest`) is likewise absent: the only `iron-chest` /
`wooden-chest` strings in `crates/` are test fixtures.

**Overtaken in a good way.** Corrections 5 and 6 of
`notes/2026-09-02-convergence-stage-1.md` — "convergence costs 1,200 ticks and
saves none", "G6 leaves room for no convergence at all" — were **invalidated
the same day** by `576d5b57` (a mining claim carries whose timeline it sits on).
`have.rs:1974` records the consequence in place: the slack term halved from
`2 * roster` to `1 * roster`, the unlock subtree went from `{bot 1: 48}` to
`{bot 1: 48, bot 2: 4, bot 3: 4, bot 4: 4}`, and the makespan from **15866 to
12403**. Convergence now pays for itself. The spec's §14 open question 7
("whether the chest is ever needed in practice") is therefore *more* live, not
less — stage 1 works, so stage 2 has to justify itself against a working stage 1.

### `2026-09-02-mod-side-actions-design` — 5 of 7 steps open

| Step | State | Evidence |
|---|---|---|
| 0 mlua tick harness | landed differently | `crates/core/tests/botbridge_walk_repath.rs:202` drives the real `control.lua` over `on_tick` |
| 1 settle on disconnect / death / nil character | **never started** | `mods/BotBridge/control.lua:945` still reads `if player.connected and player.character then -- TODO FIXME`, wrapping the whole per-player body. None of `on_player_died`, `on_pre_player_left_game`, `on_player_respawned`, `on_player_controller_changed` is registered |
| 2 `storage.actions` registry, sub-action id namespace | **never started, and moved backwards** | No `storage.actions`. `4711` still hardcoded (`control.lua:1171`) and a **second** magic id was added — `PLACEMENT_STEP_ASIDE_ACTION_ID = 4712` (`control.lua:456`), citing 4711 as precedent. Neither is known to Rust (`grep 4711\|4712 crates/` → nothing) |
| 3 tick deadlines, machine failure codes, `action_progress` | **never started** | `ACTION_RESULT_DEADLINE = Duration::from_secs(360)` (`rcon.rs:39`); `classify_failure` still string-matches prose (`globals/record.rs:360-398`) |
| 4 awaited research | **landed** | `f7b60f1c`; `settle_research_actions` (`control.lua:2792`) |
| 5 approach moves into the mod | **never started — highest value** | All five `move_player` laundering sites the spec said would be deleted survive: `rcon.rs:1635, 1717, 2385, 2579, 2652`; the 8-point probe at `:2445` |
| 6 `action_status(ids)` reconciler | **never started** | `Dispatch::NoVerdict` still inferred from silence |
| craft rows | **superseded** | Replaced by `d0a5e094` — `storage.craft_actions[player][recipe]`, counted-not-positional (`control.lua:3796`) |

Today's work **strengthens** step 5 rather than overtaking it: `98895500`
replaced the walk teleport with a re-path loop driven from `on_tick`
(`control.lua:1093`), which closes the spec's §8 open question 7 ("nothing in
this repo requests a path from `on_tick` yet") *in favour of the design*. §4's
"the teleport record must survive" is now partly moot — `teleport_writeout`
(`control.lua:550`) fires only for blocked ghost revives and blueprints.

### `2026-09-02-server-camera-design` — the reshape is written down, barely

**Direct answer: it is not lost, but it exists in one paragraph, in a note about
a different subject, and nothing in `specs/` was amended.**

`notes/2026-09-02-graphical-host-probe.md:337-346`:

> Step 1 — one renderer, one director camera — is unaffected and still stands.
> … keep the headless `--start-server` … and designate one of the existing
> graphical *client* peers as the renderer. That is step 1 exactly as written,
> with `options.renderer` naming a client instead of the server. The
> `source: String` axis proposed in §4.2 loses its `"server"` case and can stay
> a client id.

A reader of the spec alone is still told step 2 is "gated behind a measurement",
when it is dead. **That is the one status line in the corpus that lies in the
dangerous direction.**

Step 1 has zero code: no `camera_subject` (0 hits), no `options.renderer`
(0 hits), `frame_capture_take_follow` still passes a per-camera `by_player`
(`control.lua:1597`), and the `client`/`bot` axis is unrenamed
(`record/frames.rs:28` `pub bot: u8` vs `server/manage/frames.rs:43`
`pub client: u8`, route still `/api/v1/frames/{client}/{name}`). `a7392aff`
fixed the *doc comment* that lied about it; the field kept its name.
`EventKind::Frame` is still defined and **still never emitted** — its only three
occurrences are in `#[cfg(test)]` blocks. The spec said it should either become
the home for per-frame aim or be deleted; neither happened.

`dafe8f82` (retire the screenshot cameras, keep them as a switch) **overtook but
did not supersede** step 1: it turned six cameras off by default on measured
disk cost, and every defect step 1 was for still applies verbatim to any run
that passes `frames = true` — which `notes/2026-09-02-screenshots-retired.md`
explicitly recommends for tick-exactness and 1080p legibility. Step 1 is now
dormant correctness debt on an opt-in path.

Step 0 (A/B the UPS cost of capture off / one camera / six) was never run.
`morning-report.md:326` says so: "the '~7 UPS from six cameras' figure is
inference."

### `2026-09-02-video-capture-design` — step 1 half-done, step 6 never started

- **Bitrate: measured, twice.** ~143 KB/s at 706x854@15fps hand-verified, then
  **6.4 MB/min** on a real run. Both replace the spec's §2 estimates.
- **Encoder UPS cost: never measured** — and the spec's own §11.6 rule ("do not
  claim video is cheaper in UPS than screenshots until both numbers exist") was
  never satisfied, yet the screenshots were retired anyway (on disk size and the
  synchronous-render argument, which are sound but are not that number). **This
  is the one place a decision outran its own stated gate.**
- **Step 6 (steering): never started**, and its cheapest route is dead —
  `centered_on` has 0 hits, and its §11.1 prerequisite was resolved negative by
  the host probe. The only surviving route is a fifth dedicated client peer,
  unpriced.
- **The resolution knob is inert here.** `Resolution::parse` accepts
  `720p`/`1080p` and `WindowOps::resize` shells `xdotool windowsize`, but
  Hyprland refused the resize — every recording so far is 706x854 / 700x854.
- **Spec text that is actively wrong instructions, not cosmetic:** `:383` still
  says `default_base_is_moof` (the token that produced the zero-byte run;
  corrected in code by `8482d560`); §5 specifies both `-nostdin` and a `q` on
  stdin, which cannot both hold; §5 still specifies `-vf scale=1280:-2`,
  contradicting its own approved decision section.

### `2026-08-29-webserver-replaces-tauri-gui-design` — five decisions never built

1. **`app/src-tauri` → `crates/cli` rename** — deferred twice, never done.
2. **Feature rename `restapi` → `server`** — no plan ever picked it up.
   `app/src-tauri/Cargo.toml:79` still reads `default = ["restapi", "repl",
   "cli", "lua"]`.
3. **`embed-spa` / `rust-embed`** — 0 hits.
4. **Per-job output line caps** — the 50-*job* cap exists
   (`server/src/state.rs:12`); a job's transcript is deliberately uncapped and
   the code says so at `state.rs:9-11`.
5. **Autostart moving to server startup** — *reversed*, not merely skipped. The
   frontend plan's T11 step to remove the client-side trigger was undone;
   `App.vue:83` still fires it, with a rationale at `:74-82` noting nothing
   under `crates/` reads `enable_autostart`. Confirmed: the only Rust hits are
   `app_settings.rs` persistence and its tests.

Superseded, with named replacements: `crates/server/build.rs` generating
`types.ts` → deleted outright (`fa148a28`) and replaced by the OpenAPI snapshot
seam; `POST /scripts/run` + `/scripts/eval` → one `/scripts/execute`; error
shape `{error, detail}` → `{message, code}`.

### `2026-08-29-multi-agent-planner-design` — four named items never built

1. **`Holder::Chest(Position)`, `TakeFromChest`, `Consolidate`.** `Holder` is
   exactly `Anyone | Bot | Share` (`goal.rs:12,15,62`); the only `Consolidate`
   mentions in `crates/` are two prose comments. **Partly obsolete**: the
   convergence problem it existed for was solved differently by `Step::Owned` +
   `SharedSmelt`, and `Researched`'s one-inventory requirement by `c470388b`.
   What survives of it *is* material-convergence stage 2.
2. **`Schedule::critical_path`** — 0 hits repo-wide; `Schedule` is
   `{ steps, makespan }`.
3. **`Action::pinned` reachable from Lua as a TAS escape hatch** — the field
   exists (`action.rs:437`) and the scheduler honours it (`schedule.rs:254`),
   but no `goal.*` binding sets it. Lua-side occurrences are `pinned: None` in
   fixtures only.
4. **`Goal::Built { blueprint, at }`** — never defined, deliberately deferred
   twice, still out of scope.

### Other real gaps, small

- **`2026-09-01-run-record-enrichment` task 2 step 5** — settle-triggered
  `rcon_sample_bots()` was built and then removed; `control.lua:2300-2307`
  records why (nothing called it; the 60-tick beat bounds staleness at one
  second). Re-adding it needs an executor change, not a mod change.
- **`MapKind::Removed`** is declared, published in OpenAPI and mirrored in
  `types.ts:535`, but its only constructor is a round-trip test
  (`map.rs:395`). Harmless until bots deconstruct.
- **Frontend follow-ups with the backend already built**: create/delete-script
  UI (`client.ts:89,94` wrap the routes, no component calls them), RCON reply
  discarded (`manage/rcon.rs:36` returns `NO_CONTENT`), no job-history panel.
- **Tree keyboard navigation** — the shadcn plan's own named follow-up;
  `TreeView.vue` rows are buttons with no arrow-key handler.
- **`roll_best_seed`** — resolved by a third option neither the plan nor the
  spec offered: gated with a refusal (`cli/roll_seed.rs:1-40`). "Resurrect seed
  rolling on `Schedule::makespan`" is unbuilt design, honestly recorded.
- **A run has no shareable URL** — `/runs` is list-plus-detail in one page,
  pinned by `router.spec.ts`. A decision, not an oversight, but it costs
  deep-linking.

---

## Status lines that lie

**Stale in the safe direction (claim "not implemented", the work landed):**
`multi-agent-planner`, `webserver-replaces-tauri-gui`, `supervisor-loop`,
`per-bot-share-sizing`, `produced-goal`, `run-recording-and-replay`,
`material-convergence` ("Nothing implemented" — stages 0+1 landed),
`mod-side-actions` ("not implemented" — research landed, crafts superseded),
`video-capture` ("nothing has been run against a live Factorio" — two live runs,
one 45 minutes long).

**Stale in the dangerous direction:** `server-camera-design` — "the two
experiments named in *What could not be determined* have not been run." One
**was** run, and it killed step 2 permanently. Anyone reading the spec without
the host-probe note would plan work that is known impossible.

**Accurate:** `producing-goal` ("specified, not implemented"), `goal-values`,
`run-record-enrichment-design`.

**Actively wrong instructions (not status, but worse):** the three
`video-capture-design` items above — `default_base_is_moof`, `-nostdin` with a
`q` on stdin, `-vf scale=1280:-2`. Each is corrected in code and only in code,
so the spec would mislead anyone rebuilding from it.

---

## Ranked: what is genuinely still open

Ranked by consequence, not size.

### 1. Material convergence stage 3 — buffer visibility. **The gate is open and the thing it gates is running.**

This is the only item where a design was declared a *hard gate before this runs
unattended for hours* and then the unattended run started anyway. Stage 1 is
live in production right now; its own implementation note says so in terms:
"Stage 3 is a hard gate before this runs unattended … It cannot stand for a long
unattended run."

The failure mode is not degradation, it is a stall. Items left in a furnace are
invisible to `PlanState`, so a replan re-mines them — but the ore is gone from
the ground, so each iteration is slower than the last, into
`scripts/supervisor.lua`'s `stall_limit = 3`. Stage 1 widened the exposure
window from "seconds, one bot, adjacent in one chain" to "however long the taker
takes to walk over". `scripts/research_run.lua` runs `max_iterations = 10`
across seven milestones unattended — exactly the shape §8 warned about.

The half that matters most is also the half that is independently valuable and
independently testable: **`Withdraw`**. The spec says so — it can land before
any of the rest, and it is the honest answer to "the consumer never arrived":
the next plan walks over and picks the items up. That is the smallest change
with the largest effect on this list.

### 2. `Goal::Producing` — the goal that means "build a factory".

The largest open design in the corpus, and correctly labelled. Without it the
planner models **manual labour**: `Have` is satisfied by hand-mining and
hand-crafting, which is why a rocket is not reachable at any horizon. It is not
merely unimplemented, it is **unreachable** — `Goal::Producing` has no Lua
constructor at all, so no script can ask for it even to watch it fail.

`Goal::Produced` (`cf0d7bff`) removed the reason to defer it: construction needs
machines *made and placed*, `Produced` already means "cause this to come into
existence", so what remains is somewhere to put the results. Not urgent this
week; it is the item that decides whether the project's stated vision is
reachable at all.

### 3. Mod-side actions steps 1 and 3 — actions that never settle.

Ranked above the more glamorous step 5 because it has a **live symptom with a
measured cost**. `control.lua:945` still gates the entire per-player tick body
on `player.connected and player.character` with a `-- TODO FIXME` beside it, and
nothing settles an action when that gate closes. The deadline is a 360-second
wall-clock sleep (`rcon.rs:39`), so one bot losing its character costs six
minutes of a run that takes forty-five. `notes/2026-09-02-actions-that-never-settle.md`
traces exactly this. Step 3's tick-denominated deadlines are the same fix seen
from the other end, and would also retire `classify_failure`'s prose
string-matching (`globals/record.rs:360-398`) — which is re-parsing an English
sentence to recover a machine fact the mod already knew.

---

Then, in order:

4. **Mod-side step 5 — move the approach into the mod.** Five `move_player`
   laundering sites (`rcon.rs:1635, 1717, 2385, 2579, 2652`) plus an 8-point
   probe that costs an RCON round trip per point. Today's re-path work made the
   case *stronger*, not weaker: the mod already re-paths from `on_tick`, which
   was the spec's one unproven prerequisite.
5. **Encoder UPS cost.** Not because video is at risk, but because a decision
   (retiring screenshots) outran its own stated evidence gate. Cheap to close.
6. **Server-camera step 1.** Dormant correctness debt: frame attribution is
   scrambled and the `client`/`bot` axis is misnamed on the opt-in frames path.
   The *reshape* should be moved out of a note about a different subject and
   into the spec, and step 2 should be struck through there — that costs
   minutes and stops someone planning impossible work.
7. **Material convergence stage 2 (the chest).** Its own §14 says stage 1's
   measured distribution is the input to deciding whether it is needed. Stage 1
   now works and pays for itself, so this decision is *ready to be taken* — and
   the answer may well be no.
8. **Server-side autostart.** Small, but it is the one item where a plan's own
   step was actively reversed rather than skipped, so it will not resolve itself.
9. **Video step 6 (steering), the inert resolution knob, `Schedule::critical_path`,
   `Action::pinned` from Lua, `MapKind::Removed`, the three frontend follow-ups
   with backends already built, tree keyboard nav, the `crates/cli` and
   `restapi`→`server` renames.** All real, all cosmetic-to-moderate, none
   blocking anything.

**Overtaken, do not spend time on:** `Holder::Chest`/`TakeFromChest` as a
*general* mechanism (superseded by `Step::Owned` + `SharedSmelt`);
`BotsNotInterchangeable` (deleted on purpose, and its premise could not be
reproduced); the graphical `--host` server (proven impossible); the mod-side
craft redesign (superseded by `d0a5e094`); convergence corrections 5 and 6
(invalidated by `576d5b57` the same day).

---

## What I could not determine

- **Nothing here is green-suite evidence.** No cargo command was run. Every
  claim is that a symbol exists and is wired, never that it compiles or passes.
- **Whether anyone intended server-camera step 1 to be dropped.** No document
  says it is cancelled; the host-probe note says it "still stands". I read that
  as unfinished, not abandoned, but that is a judgement about intent and the
  project owner should confirm it.
- **Whether the magic action ids `4711`/`4712` actually accumulate in
  `world.actions` at runtime.** The mechanism is present and the spec asserts it
  was observed, but no note re-confirms it since the placement step-aside added
  the second id.
- **Whether `Craft` overlapping with walking was ever implemented.** The
  execution-increment plan called single-occupancy a "documented follow-up"; no
  plan re-proposed it and I found no evidence either way.
