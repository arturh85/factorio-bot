# One camera, one renderer: replacing per-client screenshots

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

**Status, corrected 2026-09-02 evening. Read this before the rest.**

**Step 2 is dead.** One of the two experiments named in "What could not be
determined" *was* run, and it killed the graphical-server half permanently: a
`--host` server **renders but cannot do RCON**. It opens two UDP sockets and no
TCP listener, and it silently accepts and ignores `--rcon-port`, `--rcon-bind`,
`--rcon-password` and even `--port`. The control is conclusive — same binary,
same instance, same config, with `--start-server` instead: RCON starts and
authenticates. RCON is this codebase's only control channel, so a server without
it is unreachable. Two further findings would each have sunk it independently:
the host **gets a character** and would be counted as a bot by `rcon_players()`,
and `--host` blocks on a LAN-username GUI modal, failing as a *hang*.

Full evidence: `notes/2026-09-02-graphical-host-probe.md`.

**Step 1 survives, reshaped.** Nothing in "one renderer, one director camera"
required the renderer to be the *server*. The version that lives keeps the
headless `--start-server` for RCON and designates an existing graphical
**client** as the renderer. The `source: "server"` axis simply goes away.

**Step 1 has no code.** Read the rest of this document with the above applied;
its unqualified references to a graphical server describe something that has
been proven impossible.

Overtaken separately: per-client screenshots were **retired** on
2026-09-02 (video is the visual record), so the cost this design set out to
remove is largely gone. What remains of step 1's value is a steered camera,
not a saving.

## The recommendation in one paragraph

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

Split the proposal in two, because it is two changes wearing one name.

1. **One renderer, one camera** — the mod stops registering a camera per
   player and stops letting each peer write its own frames. Every camera
   renders through one named peer (`by_player`), the default camera is a
   *directed* one aimed at the bot with the most recent dispatched action, and
   the aim is recorded per frame. This is a mod change plus a rename of the
   `client` axis in three Rust files and two TypeScript ones. It needs no
   change to how Factorio is launched, and it delivers the frame-attribution
   fix and most of the render-load cut.
2. **A graphical server** — replace `--start-server` with `--host` so the
   process that simulates is also the process that renders. This is possible:
   the binary is the full graphical build and headless is a *mode it selects*,
   not a build it is. But `--host` has never been run here, its RCON support is
   unverified, and putting the renderer inside the simulation process is the
   one change that could make UPS *worse*. It is gated behind a measurement.

Step 1 is worth doing whether or not step 2 ever happens. Step 2 without step 1
would move six cameras into the server process, which is the wrong direction.

---

## 1. What exists today

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

### 1.1 The capture

`mods/BotBridge/control.lua`:

| What | Where |
| --- | --- |
| interval 300 ticks, 1920×1080, JPEG q85, zoom 1 | `:1169`–`:1173` |
| `frames/tick-<%07d>-<camera>.jpg` | `frame_capture_path`, `:1252` |
| `frames/run.json` sidecar, written on **every peer** | `:1892` |
| directory wipe, on **every peer** | `helpers.remove_path`, `:1828` |
| camera registration: `follow` + one `bot-<index>` per known player + `area` | `rcon_frame_capture_start`, `:1813`–`:1875` |
| a late joiner gets its own camera | `frame_capture_on_player_joined`, `:1785` |
| the follow shot | `frame_capture_take_follow`, `:1279`, screenshot at `:1294` |
| the bounding-box shot | `frame_capture_take_area`, `:1352`, screenshot at `:1387` |
| the 300-tick handler | `on_frame_capture_tick`, `:1731`; sole registration at `:2354` |
| arbitrary directed screenshot over RCON, already present | `rcon_screenshot`, `:3272`, dispatched at `:3332` |

Six cameras for a four-bot run: `follow`, `area`, `bot-1`…`bot-4`. Confirmed by
`manifest.json` on `run-1788320177-77989`: `frames: 1217` over `60785` ticks =
202 captures × 6.

### 1.2 The peer scramble is real, and it is on disk right now

`by_player` narrows each screenshot to one peer, and the file lands in *that
peer's* `script-output`. Which peer holds which player is decided by connection
order, not by directory name. The current workspace:

```
workspace/server/script-output/frames    run.json only
workspace/client1/script-output/frames   73 × bot-4
workspace/client2/script-output/frames   73 × bot-2
workspace/client3/script-output/frames   73 × follow, bot-1, area
workspace/client4/script-output/frames   73 × bot-3
```

Two consequences, both load-bearing for this design:

- **`ArchivedFrame.bot`'s doc comment is false.** `crates/core/src/record/frames.rs:19`
  says "The `client<N>` the frame came from. Bots and clients are 1:1." The
  listing above shows client 1 holding camera `bot-4`. The field is a *renderer
  id*, and it has never been anything else. `viewsOf`
  (`app/src/lib/runTimeline.ts:319`) exists solely to stop the UI offering the
  18 `(bot, camera)` pairs this scramble never produces.
- **One peer already carries three of the six renders.** client3 held
  `follow` + `bot-1` + `area` because it held player 1, which is the anchor for
  both `follow` (`:1844`, `player_index = 1`) and `area` (`:1360`, lowest
  connected index). The render load is *already* concentrated, unevenly and by
  accident.

### 1.3 The cost that started this

`docs/superpowers/notes/2026-09-02-inventory-shortfall.md`: the run sustained
~53.6 UPS instead of 60, `await_preds` converted a 4032-tick lag edge to 67.2 s
of wall clock, and the take arrived 433 ticks early — 18 plates of 20. The note
attributes the shortfall to "six 1920×1080 JPEGs every 300 ticks".

**That attribution is inference, not measurement.** No A/B was run with capture
off. It is the most plausible cause and the run was otherwise unremarkable, but
this design must not be justified by a number nobody measured — see step 0 of
the implementation plan.

The clock bug itself is already fixed (`run::wait_out_lag` now chases
`game.tick`), so a slow server no longer *breaks* a plan. It still makes every
run take 12% longer in wall clock, which is reason enough.

### 1.4 What reads the frames

| Consumer | File | What it assumes |
| --- | --- | --- |
| archive | `crates/core/src/record/frames.rs:75` `client_dirs` | directories named `client<N>`, `N: u8` |
| archive index | `frames.rs:17` `ArchivedFrame { bot: u8, tick, camera, file }` | one numeric renderer axis |
| live manifest | `crates/server/src/manage/frames.rs` `discover_client_dirs` | same `client<N>` pattern |
| live routes | `frames.rs` | `GET /api/v1/frames`, `GET /api/v1/frames/{client}/{name}` |
| archived routes | `crates/server/src/runs.rs:330`, `:423` | `/runs/{id}/frames`, `/runs/{id}/frames/{bot}/{name}` |
| run-id reconciliation | `app/src/api/frameJoin.ts` `runIdCheck`, `staleClients` | one sidecar per client, all compared |
| view picker | `app/src/lib/runTimeline.ts:319` `viewsOf` | `(bot, camera)` pairs |
| scrubber | `app/src/components/replay/ReplayScrubber.vue:102`–`:159` | a client select and a camera select |

Storage today: `workspace/runs` is **7.4 GB** over 20 runs; the largest single
run (`run-1788325660-10154`, 3142 frames, 157080 ticks) is **2.1 GB**. Average
frame ≈ 470 KB.

---

## 2. Can the server render?

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

**The binary can. The server, as launched today, cannot — and that is a
configuration, not a construction.**

Evidence, all from this checkout:

- **One binary for both roles.** `process_control.rs` resolves the executable
  through `io_utils::get_factorio_binary_path` (`io_utils.rs:273`) for the
  server (`:333`) and for clients (`:426`) alike, and every instance is
  extracted from the same archive by the same
  `setup_factorio_instance`. `is_server` changes exactly two things
  (`instance_setup.rs:436`): it writes `server-settings.json` and it seeds
  `saves/`. Nothing about graphics differs; `config.ini` is rendered from one
  template (`crates/core/src/data/config.ini`) for both.
- **The build is the full one.** `workspace/server-log.txt:1` —
  `Factorio 2.1.17 (build 87315, linux64, **full**, space-age)`. Identical to
  `workspace/client1-log.txt:1`.
- **Headless is chosen at startup, with a display available.**
  `server-log.txt:9` records `DISPLAY=:0`; `:10` says `Running in headless
  mode`. The only difference from the client invocation is the argument list
  (`process_control.rs:362`–`:375`: `--start-server`, `--port`, `--rcon-port`,
  `--rcon-password`, `--server-settings`, `--config`). The client log instead
  reads `Video driver: x11` and `Initialised OpenGL … Mesa 26.1.5`.
- **The API refuses to render headless, and the disk agrees.**
  `runtime-api.json`, `LuaGameScript::take_screenshot`: "If Factorio is running
  headless, this function will do nothing." And
  `workspace/server/script-output/frames/` exists and contains **only
  `run.json`** — the server ran the wipe and the sidecar write (both are
  unrestricted `helpers` calls) and produced not one image.

So `--clients 0` gives a run with no pictures at all, necessarily.

### 2.1 What making it render would cost

The full binary contains a **graphical hosting path**. From `strings` on
`workspace/server/bin/x64/factorio`:

- option names `host` and `host-interactive`;
- the errors `--host specified in headless mode` and `--host-interactive
  specified in headless mode` — i.e. these options are *rejected* in headless
  mode, which is only meaningful if they are the non-headless way to host;
- the description string `Start hosting a multiplayer game`;
- the symbol
  `CommandLineMultiplayer::hostCommandLineMultiplayerGame(const cxxopts::ParseResult&, const Path&, Path, bool)`,
  which is a command-line host entry point distinct from the GUI's
  `AppManager::hostGame(...)`.

So the shape of step 2 is: replace `--start-server <save>` with `--host <save>`
in `process_control.rs:362`, keep `--server-settings` and `--config`, and give
the process a `DISPLAY`. The costs:

- **A display on the machine that runs the server.** Today only clients need
  one. CI and headless boxes lose frames entirely (they already do — no client,
  no frames — but the failure moves from "no clients requested" to "server
  cannot start").
- **~26 s of sprite loading added to server startup**, on top of the current
  ~12–17 s.
- **Render work inside the simulation process.** This is the risk. The current
  shortfall is a *peer* problem — a Factorio server runs no faster than its
  slowest peer — and moving the render into the server converts a sync stall
  into direct competition with the update loop. It may be better. It may be
  worse. It is not knowable from here.
- **Unverified RCON.** Nothing in the binary's strings gates RCON on
  `--start-server` (`--rcon-port or --rcon-bind need to be specified with
  --rcon-password to enable RCON` is the only constraint found), but nothing
  confirms `--host` starts `RemoteCommandProcessor` either. If it does not, step
  2 is dead and step 1 still stands.

### 2.2 Why step 2 does not remove the clients

**Bots are players, and players are peers.** `rcon_players`
(`control.lua:2787`) returns `game.players` entries that are `connected` and
have a `character`; the executor addresses bots by player index. Nothing in this
project creates server-side characters. So four bots still means four peers.
`--host` gives the *host* a player, so a four-bot run needs three clients
instead of four — a real saving, but not the FLE saving.

Deleting the graphical clients outright, as the Factorio Learning Environment
did (`docs/superpowers/notes/2026-09-02-prior-art-research.md:642`), requires
server-side character creation and retires the screenshots. That is a different
project and it is not this one.

---

## 3. What the camera API actually offers

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

From `workspace/factorio-api-docs/runtime-api.json` (2.1.17, runtime stage),
`LuaGameScript::take_screenshot` — the only class that provides it, alongside
`take_technology_screenshot` and `set_wait_for_screenshots_to_finish`. Every
parameter is optional.

| Parameter | Documented meaning | Bearing on this design |
| --- | --- | --- |
| `position` (`MapPosition`) | "the screenshot will be centered on this position. Otherwise … on `player`" | the whole steering mechanism. Already used by `frame_capture_take_area` (`:1391`). |
| `zoom` (`double`, default 1) | "the map zoom" | 32 px/tile at zoom 1; the `area` camera already solves for a fit (`:1378`–`:1384`). |
| `surface` (`SurfaceIdentification`) | "the screenshot will be taken on this surface" | must be explicit once no player anchors it. |
| `player` (`PlayerIdentification`) | "the player to focus on. **Defaults to the local player.**" | with `position` set, this is only a focus hint. On a `--host` process the local player is the host's. |
| `by_player` (`PlayerIdentification`) | "the screenshot will only be taken for this player" | **the writer selector.** Not a rendering parameter — it decides which peer's `script-output` gets the file. |
| `resolution` (`TilePosition`) | max 16384²; recommended max 4096² | 1920×1080 today. |
| `quality` (`int32`) | JPEG percentage | 85 today. |
| `force_render` | "the game won't drop frames… **not honored on multiplayer clients that are catching up to server**" | why gaps are real and must never be interpolated. |
| `allow_in_replay` (default false) | — | why a re-simulated tick cannot double-write. |
| `show_gui`, `show_entity_info`, `hide_clouds`, `hide_fog`, `daytime`, `anti_alias`, `water_tick`, `show_cursor_building_preview` | — | `hide_clouds`/`hide_fog`/`daytime` are worth taking for a directed camera: a fixed `daytime` makes frames comparable across runs. |

**Can a screenshot be taken with no associated player at all?** In the
parameter sense, yes: omit both `player` and `by_player`, supply `surface` and
`position`, and the shot is fully determined. In the practical sense, no — with
no `by_player`, *every* graphical peer renders and writes its own copy into its
own `script-output`, which is precisely the duplication the current code's
comment at `:1296`–`:1305` avoids. There is no "server" value for `by_player`
(it takes a `PlayerIdentification`, and the `0` sentinel exists only on
`helpers.write_file`'s `for_player`).

**What a server-only capture therefore looks like.** `on_nth_tick` still fires
on every peer — it must, because the gate lives in replicated `storage` and
every peer has to agree on whether a capture tick is a capture tick
(`:1703`–`:1730`). What changes is that every `take_screenshot` names the *same*
`by_player`: the renderer's player. On a `--host` server that is the host's own
player. On the step-1 design it is whichever connected player the capture picked
as renderer. Either way exactly one peer writes, and the filename means one
thing.

The sidecar is a separate mechanism and already has a server-only form:
`helpers.write_file(path, data, append, for_player)` with `for_player = 0`
writes "only to the server's output if present" — which is how
`samples.jsonl` is already server-only (`control.lua:1218`, `:1833`). The
frames sidecar at `:1892` passes no `for_player`, which is why every peer has
one.

---

## 4. What changes

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

### 4.1 Mod

1. **`rcon_frame_capture_start(run_id, options)`** — `options` is an optional
   table: `{ renderer = <player_index|nil>, cameras = {"director"}, interval =
   300, resolution = {1920,1080}, per_bot = false }`. Absent options keep
   today's numbers. The camera list replaces the `2 + one per player`
   construction at `:1843`–`:1868`.
2. **A `director` camera kind.** Aims at the current subject (§5) and falls
   back to the `area` framing when there is no subject.
3. **`storage.camera_subject`**, set inside the existing
   `rcon_action_start_*` entry points (`:2357` walk, `:2387` mine, `:2924`
   craft, and the rest of that family), holding `{player_index, position,
   tick}`. Set from data those functions already receive; **no new RCON traffic
   and no executor change**.
4. **Renderer selection.** `frame_capture_connected_players()[1]` (`:1263`,
   already ordered and already the `area` anchor) unless `options.renderer`
   names one. Recorded in `storage.frame_capture.renderer` so it cannot drift
   between ticks.
5. **The sidecar becomes renderer-only** — `helpers.write_file(..., false,
   renderer_index)` at `:1892`, or `0` once the server renders. The wipe at
   `:1828` stays unrestricted: every peer must still clear its own leftovers.
6. **`frame_capture_on_player_joined`** (`:1785`) adds a camera only when
   `per_bot` is on.

### 4.2 Rust

- `FactorioRcon::frame_capture_start` (`crates/core/src/factorio/rcon.rs:987`)
  gains the options argument; the Lua binding and its `__doc_entry_*` string
  (`crates/scripting_lua/src/globals/rcon.rs:196`–`:228`) and the recorder's
  call site (`crates/scripting_lua/src/globals/record.rs:463`) follow.
- The renderer axis stops pretending to be a bot. `ArchivedFrame` and
  `FrameEntry` get `source: String` (`"server"`, `"client3"`), and the `bot` /
  `client` fields become `Option<u8>` retained only so archived runs keep
  deserialising (§7).
- `client_dirs` (`record/frames.rs:75`) and `discover_client_dirs`
  (`server/src/manage/frames.rs`) enumerate `server` as well as `client<N>`.
- Routes: `/api/v1/frames/{source}/{name}` and
  `/api/v1/runs/{id}/frames/{source}/{name}`, `source` a bounded string
  (`server` or `client<1..255>`) validated before it reaches
  `resolve_script_path` — that guard stays exactly as it is, it is the only
  thing between an unauthenticated caller and the filesystem.

### 4.3 Frontend

- `openapi.snapshot.json` regenerates; `app/src/api/types.ts` mirrors; the
  contract spec fails from both ends until both are done. That is the seam
  working.
- `viewsOf` (`runTimeline.ts:319`) keeps its `(source, camera)` shape but will
  normally return one entry; `ReplayScrubber.vue` hides a picker with one
  option rather than gaining a special case.
- `staleClients` / `client_runs` keep working unchanged: with one renderer there
  is one sidecar, so agreement is trivial and `runIdCheck` still `confirmed`s.

---

## 5. What the camera follows

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

**Decision: `director` — the bot with the most recent dispatched action,
framed together with that action's target; falling back to the all-bot bounding
box when no action has been dispatched. One camera by default. The aim is
recorded per frame.**

Framing rule, in order:

1. If `storage.camera_subject` is set and its player is connected and on one
   surface with its target: frame the box through the bot's position and the
   target position, plus the existing 16-tile margin, fit-zoomed, clamped to
   zoom 1 and refusing below 0.05 exactly as `frame_capture_take_area` already
   does (`:1378`–`:1386`).
2. Else if any bot is connected: today's `area` framing, unchanged.
3. Else: **no file**, as now. The absence is the record.

**The aim is written down, not inferred.** Every capture appends one
`samples.jsonl` line — `{kind="camera", tick, camera, subject_player, x, y,
zoom, rule}` — through the existing server-only `write_sample` (`:1217`). A
picture whose subject is only implied is a picture that can lie; this makes the
subject a measurement on the same clock as everything else. (`EventKind::Frame`
in `crates/core/src/record/mod.rs:823` is currently defined and never emitted —
it appears only in tests. Either it becomes the home for this, or it should be
deleted.)

### Why this one

- It needs no new RCON round trips and no executor change: the mod already
  receives the bot and the position on every `rcon_action_start_*` call.
- Its claim is checkable against the record: `EventKind::ActionDispatched`
  (`record/mod.rs:96`) carries `bot` and `target` on the same tick axis, so a
  frame's recorded subject can be cross-checked against the plan afterwards.
  Two independent sources agreeing is worth more than one source asserting.
- It answers the question a viewer actually has — "what was happening" — which
  the `area` camera deliberately refuses to answer (`:1325`–`:1329`: "nothing in
  this mod defines action, so such a camera would be pointing at a thing it had
  invented"). That objection is retired precisely because the plan *does* define
  action and now tells the mod.

### Alternatives rejected

- **Bounding box of all bots only.** Kept as the fallback, rejected as the
  default: it degenerates. Four bots working a 400-tile spread produce a frame
  where every bot is four pixels, and below zoom 0.05 it writes nothing at all.
  It is the honest shot and an unwatchable one.
- **Milestone target area.** The milestone is the right unit for a *chapter*,
  not for a frame: milestone 4 spans 60 000 ticks and most of the map. It would
  be a fixed camera with extra steps, and it needs a plumbed-through target the
  mod does not have.
- **Fixed overview.** Cheapest and most stable, and it makes every run look
  identical. It cannot show a bot mining. Worth keeping available as an
  `options.cameras` entry for time-lapse purposes; wrong as a default.
- **Executor-pushed camera (`camera_look_at` over RCON per dispatch).** Same
  shot as the chosen design, but one extra round trip per action and a new
  coupling that fails silently: a dispatch path that forgets to call it leaves
  the camera on a stale subject with nothing to say so. The mod-side hook sits
  in the function that *is* the dispatch, so it cannot be forgotten separately.
- **Multiple directed cameras (one per bot, server-rendered).** This is the
  current cost with a new address. If per-bot views are wanted they are the
  opt-in of §6, not the default.

### The cost of this choice

The subject changes whenever another bot is dispatched, so with four busy bots
consecutive frames can cut between them. That is honest — each frame shows a
real bot doing a real thing — but it is not a smooth video, and a video built
from it will jump. Two mitigations, both optional and neither required for the
first version: hold a subject for a minimum number of captures, or run
`director` and `area` together as two cameras (still a third of today's six).

---

## 6. What is lost

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

**Per-bot point of view.** Today every bot has a camera, so any tick can be
inspected from any bot's position. With one directed camera, at a given tick you
see one bot and whatever else is in frame. Specifically:

- **You cannot answer "what was bot 2 doing at tick 40 000" from pictures** if
  bot 2 was not the subject. The record still answers it —
  `events.jsonl` lanes (`record/lanes.rs:32`), `samples.jsonl` per-bot
  inventories every 60 ticks, `map.jsonl` placements — but not visually.
- **No multi-angle comparison.** Two bots that interfered with each other
  cannot be watched side by side.
- **A stuck bot is less obvious.** A bot that stands still for 20 000 ticks is
  currently visible as 66 identical frames on its own camera. Under the new
  design it is visible as *absence from the subject record*, which is a
  different and less immediate signal.

**Nothing in the viewer breaks.** `viewsOf` was written for a scrambled,
variable set of views and handles one view as easily as eighteen;
`ReplayScrubber` derives its selectors from the manifest rather than from a
list it knows (`:128`–`:139`), so a camera that stops being produced simply
stops being offered. The analysis view reads `events`/`samples`/`splits` and
never reads a frame's bot. The loss is analytic, not structural.

**Therefore: hybrid, and say so.** `per_bot = true` in the capture options
re-registers today's `bot-<N>` cameras, rendered through their own peers exactly
as now. Default off. This keeps the expensive mode available for the run where
somebody needs it, at the price of one branch in the camera-registration code —
which already exists as a loop. Recommending the hybrid over the pure
single-camera design is deliberate: the per-bot cameras were built for a reason
and the reason has not disappeared, it has only stopped being worth 6× the
render cost on every run.

---

## 7. Migration

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

**Nothing on disk is converted.** 7.4 GB of archived runs, up to 3142 frames
each, stay exactly as they are.

- `frames/index.json` in archived runs has `{bot, tick, camera, file}` and no
  `source`. Read-side shim: `#[serde(default)] source: Option<String>`, and a
  reader that resolves `source.unwrap_or_else(|| format!("client{bot}"))`. Old
  runs keep their four buckets and their six cameras; the picker keeps offering
  them, because they really do exist.
- New runs write `source` and (for a server renderer) `bot: null`. `bot` is
  never reused for a renderer that is not a client, and never set to `0` as a
  sentinel — a numeric axis that sometimes means "server" is how the current
  false doc comment happened.
- The **live** endpoints have no history to preserve: they report the current
  workspace, which is wiped by the next `frame_capture_start`.
- Retention is unchanged (`DEFAULT_KEEP = 20`, `.keep` marker). One camera at
  the same interval and resolution is ~1/6 the bytes, so the same 20-run budget
  covers roughly six times the history. Do not simultaneously raise `keep`;
  measure first.
- The frame *name* does not change. `tick-<%07d>-<camera>.jpg` and both
  parsers (`record/frames.rs:parse_frame_name`, shared by the server) stay
  exactly as they are, including the hyphenated-camera-id handling — `director`
  needs it no more than `bot-1` did, but old archives still contain `bot-1`.

---

## 8. Video or frames

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

**Frames stay the artefact. Video is generated on demand and is not stored.**

Factorio has no video API; frames plus `ffmpeg` is the only route, so the
question is only which one is the record.

Frames win because they are addressable by tick. The entire join to the plan —
`frameAtTick`, `ReplayScrubber`'s shifted-tick axis, the `observedOrigin()`
conversion — is "which picture belongs beside this tick", and a video answers
that only through a frame index you would have to keep anyway. A video also
cannot represent a dropped frame honestly: `force_render` is not honoured on a
catching-up client, gaps are real, and a constant-rate encode silently makes a
gap look like time passing normally.

If a video is wanted, the honest encode uses the concat demuxer with per-frame
durations derived from *tick* deltas, so a gap lasts as long as it really did:

```
# durations from the tick deltas, not a fixed -framerate
ffmpeg -f concat -safe 0 -i frames.txt -vsync vfr -c:v libx264 -crf 23 run.mp4
```

A glob-sequenced `-pattern_type glob -i 'tick-*-director.jpg'` is the tempting
one-liner and it is wrong for exactly this reason.

For the runs already recorded, one camera has to be picked before any of this
applies — they contain up to six interleaved, and a glob over all of them
produces a video that cuts between bots at 12 fps. Filter to a single camera
first. No back-conversion is proposed: those runs are frames, they remain
frames, and they remain joinable.

---

## 9. Implementation plan

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

Each step is independently reviewable and independently revertable. Steps 1–5
do not touch how Factorio is launched.

**Step 0 — measure, before changing anything.** Three runs of the same script
with `--clients 4`: capture off, capture with one camera, capture with today's
six. Measure UPS the way the executor now does — poll `FactorioRcon::game_tick`
against the wall clock — and record all three numbers in a note. This is the
only thing that establishes the premise. If one camera does not move UPS, the
render load was never the problem and everything below is a tidiness change
rather than a performance one; that is worth knowing before doing it.

**Step 1 — one renderer, same cameras.** Mod only: pick the renderer once at
`frame_capture_start`, pass it as `by_player` for every camera, make the sidecar
renderer-only. Frames all land in one directory. Nothing downstream needs to
change (the enumerators already handle "some clients have no frames"), and
`viewsOf` collapses on its own. Reviewable as: "did the scramble disappear".

**Step 2 — the `source` axis.** Rename the renderer axis in
`ArchivedFrame`/`FrameEntry`, add the `server` directory to both enumerators,
change both route pairs, regenerate the OpenAPI snapshot, mirror in
`types.ts`. Add the archived-run compatibility shim and a test that reads a
real pre-change `index.json` from `workspace/runs`. Reviewable as: "do old runs
still render in the viewer".

**Step 3 — capture options.** `rcon_frame_capture_start(run_id, options)`
through `FactorioRcon`, the Lua binding and its doc string, and the recorder.
Defaults reproduce step 1 exactly. Reviewable as: "does omitting options change
nothing".

**Step 4 — the `director` camera.** `storage.camera_subject` in the
`rcon_action_start_*` family, the framing rule, the `samples.jsonl` camera
line. Make `{"director"}` the default camera list. Reviewable as: "does the
recorded subject match `events.jsonl`'s `ActionDispatched` for the same tick" —
which is a test that can be written against an archived run.

**Step 5 — per-bot opt-in.** `per_bot = true` restores today's cameras. Pin it
with a test so the expensive mode cannot rot.

**Step 6 — the graphical host, gated on step 0's numbers and on the two
experiments below.** Swap `--start-server` for `--host` in
`process_control.rs:362`, keep everything else, require a `DISPLAY`, and make
the failure mode explicit: if the server cannot render, say so at startup
rather than producing a run with no pictures. Re-run step 0's measurement in
this configuration before keeping it.

---

## 10. What could not be determined without running the game

> **OBSOLETE (2026-09-03):** the per-camera screenshot feature this describes was removed end to end in `15c85c1f`; video capture (`crates/core/src/record/video/`) replaces it.

1. **Whether `--host` accepts `--rcon-port` / `--rcon-password` /
   `--server-settings` and starts the RCON listener.** The strings show `--host`
   exists, is described as "Start hosting a multiplayer game", is rejected in
   headless mode, and has its own command-line entry point. They show no gate
   tying RCON to `--start-server`. That is suggestive and not sufficient. One
   command answers the first half: `workspace/server/bin/x64/factorio --help`.
   One run answers the rest.
2. **Whether `--host` is non-interactive enough to be spawned and managed the
   way the server is today** — whether it goes straight into the game or shows a
   host-game dialog first, and what `--host-interactive` is for.
3. **Whether a `--host` process auto-pauses without peers.** The binary carries
   an `AutoPauseWithoutPeersTag`; a host with its own player probably counts as
   a peer, but "probably" is not a design.
4. **The actual UPS cost of rendering, and where it is cheapest.** Whether one
   camera restores 60 UPS; whether rendering inside the server process is better
   or worse than rendering on a client peer. This is step 0 and it gates
   everything.
5. **Whether the host's own player joins with a character**, and therefore
   whether `rcon_players` (`control.lua:2790`, which requires `connected and
   character`) counts it as a bot. If it does, a four-bot run needs three
   clients; if it does not, it needs four and the host is a pure camera.
6. **Whether `take_screenshot` renders terrain the anchoring player has never
   charted.** The `area` camera already depends on this working at moderate
   distances and appears to; a `director` camera aimed at a far-off bot pushes
   it further. Unverified either way.
7. **The measured frame cost at other settings.** 1280×720 is 2.25× fewer
   pixels and a 600-tick interval is half the captures; both are cheaper levers
   than any of this and neither has been measured.
