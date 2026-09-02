# Recording the run instead of photographing it

Design only. Nothing here has been implemented and nothing here has been run.
This is the second half of
`docs/superpowers/specs/2026-09-02-server-camera-design.md`, which decided
*what renders*; this decides *what the render is written to*.

## The recommendation in one paragraph

**Both, and the split is not the one the question implies.** Frames stay the
record — they are the only artefact addressable by tick, the join to the plan is
verified rather than assumed, and `runTimeline.ts` already reads their ticks to
size the analysis axis. Video is added as an opt-in second artefact for the
thing frames genuinely cannot do: show what happened *between* two captures five
seconds apart. It is opt-in for the same reason `per_bot = true` is opt-in in
the sibling spec — the default should be the cheap honest one and the extra mode
should have to be asked for — but the direction is reversed: `per_bot` is opt-in
because it is expensive *inside* the game, and video is opt-in because it is
expensive *outside* it (a whole encoder process, a display, and — on the design
that actually steers a camera — a fifth Factorio client that is not a bot).
Video is never the source of a tick. A `<video>` element knows only seconds, and
the conversion from seconds to ticks is a measurement this design has to make
and record, not a multiplication it is allowed to perform.

---

## 1. Three things in the brief that are wrong

These are corrections to the "already established" list, not quibbles: two of
them change the design.

### 1.1 The mod cannot read wall-clock time at all

> "The obvious design is a sidecar the mod appends (`{tick, wall_ms}` every N
> ticks)."

**The mod cannot write that sidecar.** Factorio's control stage has no clock.
`os` is not in the control-stage sandbox, and a search of the entire 2.1.17
runtime API (`workspace/factorio-api-docs/runtime-api.json`, 157 classes, every
method and attribute description) for real-time / wall / UTC / epoch /
millisecond returns exactly one relevant class:

- `LuaProfiler` (`LuaHelpers::create_profiler`), which is explicitly designed to
  refuse this: *"Since performance is non-deterministic, these objects don't
  allow reading the raw time values from Lua."* It can only be **used** where a
  `LocalisedString` is accepted. `helpers.write_file`'s `data` parameter is a
  `LocalisedString`, so a profiler's rendered duration *can* physically reach
  disk — as prose, in an unspecified, locale- and magnitude-dependent format,
  measuring elapsed-since-created rather than an epoch, on an object that
  "cannot be serialized" and therefore cannot live in `storage` across a save.

Building a clock on that is building a clock on a debug print. **Do not.**

The correct source is the one this project already has and does not use:
**every RCON reply is already stamped with `game.tick` from inside the game.**
`FactorioRcon` (`crates/core/src/factorio/rcon.rs:711`–`:719`) keeps the last
one in an `AtomicU64` and documents exactly this:

> Every `remote_call_timed` reply arrives stamped with `game.tick` from inside
> the game, so the game tells us what time it is on every command we send.

So the host is *already* receiving a stream of ticks, on a machine whose
monotonic clock is the same one that will spawn ffmpeg. The clock sidecar is
therefore written host-side, needs **no mod change and no new RCON function**,
and — crucially — has no cross-process clock skew to reason about, because there
is only one clock. This is strictly better than the mod-side design that is not
available anyway.

This also inherits the lesson of `docs/superpowers/notes/2026-09-02-wall-ms.md`,
where `Event::wall_ms` was removed because it was stamped when a *batch* was
flushed rather than when the event happened. The failure there was a timestamp
taken at the wrong moment for the thing it labelled. The pairing below is
immune to that specific error by construction: the tick and the wall reading are
two halves of *one* observation, taken microseconds apart, and neither is
attached to a third event that happened somewhere else.

### 1.2 Frames *are* in the analysis path, in one place

> "`samples.jsonl` and `map.jsonl` are the ANALYSIS artefacts; frames have never
> once been used to diagnose a run. So moving frames to video should cost the
> analysis path nothing — verify that claim against the code."

Verified, and it is *nearly* true. Nothing reads a frame's **pixels** for
analysis. But `app/src/lib/runTimeline.ts` reads a frame's **tick**, and not
only for the picture panel:

```
tickSources(splits, frames, lanes) -> {all, drawn}   // runTimeline.ts:79
```

Frame ticks are pushed into **both** `all` and `drawn`, and `drawn` is what
`tickBounds` uses to decide where the axis starts — deliberately, per its own
comment: a milestone opens before capture produces anything, and spanning that
gap "spends axis width on a stretch with no frame and no lane bar". `leadInTicks`
reads the same source. So a run that captures no frames does not merely lose the
picture panel; **its analysis axis silently starts and ends somewhere else**,
computed from splits and lanes alone. Nothing errors, nothing is marked, the
axis is just different.

Consequence for this design: if video ever replaces frames for a run, the
video's tick-sample range must be fed into `tickSources` as a `drawn`
contributor in the frames' place. That is a real, small, easy-to-forget change,
and it is item 9.5 below.

### 1.3 "Keyframe interval buys scrubbing precision" — it buys latency

> "Specify the encode settings and say what scrubbing precision they buy."

Browsers do *accurate* seeking on `HTMLMediaElement.currentTime`: they seek to
the keyframe at or before the target and decode forward to the exact frame. So
**scrubbing precision is the frame period, 1/fps, regardless of the GOP.** What
the keyframe interval buys is seek *latency* and file size: a 10-second GOP
means up to 10 seconds of frames decoded per seek, which on a scrubber being
dragged is a stutter, not an inaccuracy.

Both numbers still matter and both are specified in §5 — but they answer
different questions, and picking a short GOP "for precision" would be paying
bitrate for something you already had.

Everything else in the established list checks out. In particular the tick /
wall-clock warning is exactly right, and §4 is the answer to it.

---

## 2. What a video actually buys, and what it costs

**Buys.**

- **Temporal density.** The current capture is one frame per 300 ticks — one
  picture every 5 seconds of a run that is mostly a bot walking somewhere. At
  15 fps a video has ~84× more moments. Every "what happened between those two
  frames" question that currently has no answer gets one.
- **Near-zero in-game cost.** `game.take_screenshot` renders synchronously
  between update and render, and is paid per camera per capture. An external
  grabber reads the frame the GPU already drew. (Caveat in "costs" — it is not
  free, it is merely not paid in the update loop.)
- **Continuous camera motion.** A `take_screenshot` camera can only be aimed at
  the instant it fires. A viewport camera can be *following* — `LuaPlayer::
  centered_on` tracks an entity between ticks — so a video can pan with a
  walking bot, which a 300-tick screenshot sequence structurally cannot.

**Costs, honestly.**

- **It is not a storage win.** Reference run `run-1788325660-10154`: 3142
  frames, 157,080 ticks, **2.1 GB**, six cameras at 300 ticks. Reduced to *one*
  camera that is 524 frames ≈ **246 MB**. That run is ~44 minutes at 60 UPS,
  ~49 at 53. A 1080p30 H.264 encode of Factorio at a plausible 3 Mbps is
  **~1.1 GB** — 4.5× *worse* than one-camera screenshots. Only by dropping to
  720p15 at a plausible ~700 kbps does it land at **~250 MB**, i.e. par. These
  bitrates are estimates from the codec and content type; **nobody has encoded a
  Factorio window on this machine**, and §12 step 1 is to measure before
  believing any of them.
- **CPU, on the machine running the simulation.** "The GPU already drew it" is
  true of the *capture*; the *encode* is new work. `libx264 -preset veryfast` at
  720p15 is a fraction of a core, at 1080p30 closer to a whole one. That core is
  contended with Factorio's update loop and up to four graphical clients on the
  same box. The sibling spec's step 0 measurement exists because nobody has
  measured the screenshot cost either; this design inherits that discipline and
  must not claim a UPS win it has not measured. It is *plausible* that video is
  cheaper in UPS than six screenshots. It is not established.
- **The camera peer probably cannot be a bot** (§6). That is either a fifth
  Factorio process or a dependence on `--host`, which is unresolved.
- **A video cannot say "nothing was captured here".** This is the deep one and
  it drives §4 and §9.4. `frameAtTick` returning `null` is a rendered state;
  frames drop and the gap is visible as literal empty space on the axis. A video
  has no null: while the game stalls, the recorder keeps writing frames of the
  last drawn image, and the result looks exactly like a game that was running
  and doing nothing. **Only the tick sidecar can distinguish those two, and only
  if it refuses to interpolate across the gap.**

---

## 3. Where things live

```
<workspace>/video/                     # host-written, wiped per run
  run.json                             # {"run": "<run id>"} — same rule as frames/run.json
  video.mp4                            # fMP4, see §5
  ticks.jsonl                          # the clock, see §4
  video.json                           # encoder manifest + calibration + status

<run_dir>/video/                       # archived copy, same three files
```

`run.json` sits **inside** `video/`, for exactly the reason
`crates/server/src/manage/frames.rs` gives for `frames/run.json`: a per-run wipe
clears the identifier together with the thing it identifies. A sidecar one level
up survives the wipe and goes on describing a file that no longer exists.

`<workspace>/video/` rather than `<workspace>/server/script-output/video/`
because the *host* writes it, not the game. `script-output` is the game's
outbox; putting a host artefact there would imply the game produced it, and the
next person to debug a missing video would go looking in the mod.

Archiving is `archive_video(workspace, run_dir, run_id)` in a new
`crates/core/src/record/video.rs`, structurally a sibling of `archive_frames`
and obeying the same law it already enforces at `frames.rs:119`: **copy nothing
unless `run.json` names this run.** Called from `RunRecorder::finish`
(`crates/core/src/record/mod.rs:680`), beside the `archive_frames` call.

---

## 4. The clock

### 4.1 The pairing

The recorder samples `(tick, wall_ms)` pairs host-side. Two sources, merged:

1. **Free pairs.** Every `remote_call_timed` reply already carries a tick
   (`rcon.rs:810`–`:820`). The recorder subscribes to those. During execution
   this is dense — every action dispatch and every observation contributes.
2. **A floor.** When the executor is quiet, a deliberate `FactorioRcon::
   game_tick()` poll (`GAME_TICK_QUERY`, a plain `/silent-command`, no mod
   needed) at a fixed cadence guarantees the table never goes sparse.

`wall_ms` is milliseconds since the recorder's own `Instant` epoch, which is
also the epoch `video.json` calibrates the video's PTS against (§4.4).

**`wall_ms` is stamped as the midpoint of send and receive**, not at receipt.
The tick is the value inside the game at the moment the command executed, which
is somewhere between the two; taking receipt time gives a systematic *late*
bias of one full round trip, while the midpoint bounds the error at half a round
trip and centres it on zero. On loopback that is sub-millisecond either way, but
the midpoint costs one extra `Instant::now()` and removes a bias that would
otherwise be invisible and always in the same direction.

### 4.2 Format

`ticks.jsonl`, one JSON object per line, append-only, ascending in both columns:

```json
{"t": 60551, "w": 0}
{"t": 60581, "w": 508}
{"t": 60612, "w": 1013}
{"t": 60640, "w": 1519}
```

`t` is `game.tick`, `w` is `wall_ms`. Short keys because there are thousands of
lines and this file's only job is to be a table. JSONL rather than one JSON
array so a killed recorder leaves a readable file with a torn last line rather
than an unparseable one with no closing bracket — same reason `events.jsonl`,
`samples.jsonl` and `map.jsonl` are already JSONL.

Two extra line kinds, both mandatory for honesty:

```json
{"t": 60551, "w": 0, "k": "start"}
{"w": 41207, "k": "gap", "reason": "no tick observed for 4.1 s"}
{"t": 121336, "w": 60408, "k": "stop"}
```

A `gap` line is written by the recorder whenever the floor poll fails or is
starved. It is the video's equivalent of a missing frame file, and §4.5 makes
the viewer refuse to interpolate across it.

### 4.3 Cadence, and the interpolation error it buys

Between two samples `(t0,w0)` and `(t1,w1)` the viewer interpolates linearly.
If the instantaneous rate stays within `[u_min, u_max]` ticks/second across the
interval, the worst-case error of that interpolation is

```
|e|max = Δtick · (1/u_min − 1/u_max) / 4
```

(the extremum of the chord-versus-curve difference, achieved by a bang-bang
rate profile that switches at the midpoint of the interval).

With the range this project has actually seen — 53.6 UPS sustained under
capture, 60 nominal — take `u_min = 50`, `u_max = 60`, so
`1/u_min − 1/u_max = 3.33 ms/tick`:

| floor cadence | Δtick between samples | worst-case error | vs. a 15 fps frame (66.7 ms) |
| --- | --- | --- | --- |
| 0.2 Hz | ~300 | 250 ms | 3.7 frames |
| 1 Hz | ~60 | 50 ms | 0.75 frames |
| **2 Hz** | **~30** | **25 ms** | **0.37 frames** |
| 4 Hz | ~15 | 12.5 ms | 0.19 frames |

**Chosen: a 2 Hz floor.** It puts the clock's worst-case error at ~25 ms, safely
under one video frame period at any frame rate this design would use, so the
clock is not the limiting error — the video's own frame period is. Going to 4 Hz
halves an error that is already not the bottleneck, at double the command rate
on a server whose command queue is shared with the executor. Going to 1 Hz is
also defensible and would be fine at 15 fps; 2 Hz is chosen because it stays
sub-frame at 30 fps too, so raising the frame rate later does not silently
invalidate the clock.

Note that during execution the free pairs make the real cadence far denser than
2 Hz — the floor exists for the stretches where the executor is waiting on a lag
edge and sending nothing, which is precisely when UPS is most stable and the
interpolation most trustworthy anyway.

**The bound is conditional and the condition can fail.** If the game stalls
outright — an autosave, a client hitching, a peer catching up — `u_min → 0` and
the formula diverges. There is no cadence that fixes this, which is why the
gap rule below is not optional.

### 4.4 Calibrating the video's own zero

The sidecar is in the recorder's monotonic clock. The video's PTS starts at
zero when ffmpeg captured its first frame, which is some unknown time after the
recorder spawned it — process start, X11 connection, window lookup, first grab.
Guessing that offset is the mistake that would make every seek wrong by a
constant nobody can see.

So it is **measured, twice**. ffmpeg is run with `-progress pipe:1`, which emits
`out_time_us=` continuously. The recorder reads that pipe and records, in
`video.json`:

```json
{
  "run": "run-1788325660-10154",
  "file": "video.mp4",
  "width": 1280, "height": 720, "fps": 15,
  "calibration": [
    {"host_wall_ms": 812,   "out_time_ms": 0},
    {"host_wall_ms": 60408, "out_time_ms": 59598}
  ],
  "status": "stopped",
  "ffmpeg_exit": 0
}
```

The first pair fixes the offset. The second pair, taken at stop, **also fixes
the rate** — and the rate is a check, not a parameter. If
`(out₁−out₀)/(wall₁−wall₀)` is not 1.000 within a small tolerance, the capture
dropped or duplicated frames and the video's clock is not the host's clock. That
is a detectable, reportable failure rather than a silent skew, and the viewer
must surface it (§9.4). A one-point calibration cannot detect it at all, which
is the whole reason there are two.

### 4.5 How the viewer seeks a tick to a timestamp

One function, in one new file, and seconds exist nowhere else — the same
discipline `observedOrigin()` already enforces for the shifted-tick axis:

```ts
// app/src/api/videoClock.ts
export function tickToVideoSeconds(
    clock: VideoClock, tick: Ticks
): {seconds: number; certain: boolean} | null
```

Rules, in order:

1. **Before the first sample or after the last: `null`.** Not clamped to 0 or to
   the duration. A tick outside the recording's span has no position in it, and
   clamping would put the scrubber at a frame that shows a different moment.
   This is `frameAtTick`'s `null` and it renders as a state, not as a picture.
2. **Bracketed by two samples separated by a `gap` line, or by more than
   `MAX_INTERP_GAP` (recommend 2000 ms): `null`.** Not an interpolated guess.
   Across a stall the linear estimate is unbounded-wrong and, worse, the video
   at that timestamp shows a frozen image that looks like a legitimate frame.
   This is the single most important rule in the design: it is the only thing
   standing between the viewer and fabricated continuity, and unlike a missing
   JPEG it will not announce itself.
3. **Otherwise** linear interpolation between the two nearest samples, offset by
   `calibration[0]`, `certain: true`.
4. If `video.json`'s rate check failed, every result is `certain: false`
   regardless — the offset may be right at the start and wrong by the end, and
   nothing in the clock can say where the error was accumulated.

The inverse, `videoSecondsToTick`, exists for the reverse direction (the user
scrubs the video, the timeline follows) and obeys the same three rules.

---

## 5. Encode and random access

```
ffmpeg -hide_banner -nostdin
  -f x11grab -framerate 15 -window_id 0x<id> -i :0
  -vf scale=1280:-2 -pix_fmt yuv420p
  -c:v libx264 -preset veryfast -crf 28 -tune zerolatency
  -x264-params keyint=30:min-keyint=30:scenecut=0
  -movflags +frag_keyframe+empty_moov+default_base_is_moof
  -progress pipe:1
  <workspace>/video/video.mp4
```

Each choice, and what it is for:

- **`-window_id`** — capture that window only, not the screen. Factorio runs as
  an Xwayland client (`SDL_VIDEODRIVER=x11`) so it has a real X11 window id;
  §11.2 covers finding it and §10.2 covers it not existing.
- **`-framerate 15`** — the density/size trade. 15 fps is 84× the current
  capture density at roughly the current one-camera storage cost (§2). 30 fps
  doubles the bytes for a smoothness nobody has asked for in a diagnostic
  artefact.
- **`scale=1280:-2`** — encode at 720p even if the window is 1080p. Most of the
  cost is pixels and Factorio at 720p is entirely legible. `-2` keeps the height
  even, which yuv420p requires.
- **`-pix_fmt yuv420p`** and H.264 — the only combination every browser plays.
  A `<video>` that decodes on one machine and not another is worse than no
  video.
- **`keyint=30` (2 s at 15 fps), `scenecut=0`** — a *uniform* GOP. Seek latency
  is bounded at 30 decoded frames, and uniformity means that bound is the same
  everywhere rather than dependent on how busy the screen was. As §1.3 says,
  this buys latency, not precision.
- **`-preset veryfast`** — the encode shares a CPU with the simulation.
  `ultrafast` is the fallback if step 1's measurement shows contention;
  hardware VAAPI encode would be better still and is unverified (§11.4).
- **fMP4 (`frag_keyframe+empty_moov`), *not* `+faststart`** — this is a
  correctness requirement, not a preference. A plain MP4 writes its `moov` atom
  at the end; a recorder that is SIGKILLed, OOM-killed, or stopped by a crashed
  host leaves a file with no index that **no player will open at all**. A
  fragmented MP4 is playable and seekable up to the last written fragment. A
  recording that dies mid-run must still be watchable up to where it died, and
  `+faststart` (a second pass over a *complete* file) cannot offer that.

**Scrubbing precision: 1/15 s = 66.7 ms ≈ 4 ticks at 60 UPS.** The clock's
contribution (25 ms, §4.3) is smaller, so the limiting factor is the frame
period — which is the right way round: the error you cannot remove should
dominate the one you chose.

**Serving it needs a server change.** `get_frame`
(`crates/server/src/manage/frames.rs:352`) does `std::fs::read` into a `Vec<u8>`
and returns a plain 200. A `<video>` served that way cannot seek and loads the
whole file into server memory per request. The video route must delegate to
`tower_http::services::ServeFile`, which handles `Range`/206 — the dependency is
already in `crates/server/Cargo.toml` with the `fs` feature, used by
`crates/server/src/spa.rs`. The `resolve_script_path` guard runs **before**
`ServeFile` is constructed, exactly as it does today; that guard is the only
thing between an unauthenticated caller and the filesystem and nothing about
this change may route around it.

---

## 6. What the camera looks at, and why it cannot be a bot

A screenshot camera is aimed by `take_screenshot`'s `position`. **A video camera
is a viewport**, so steering it is a different API surface entirely:

| Mechanism | API | Note |
| --- | --- | --- |
| free camera | `player.set_controller{type = defines.controllers.spectator}` then `player.teleport(pos)` | full control, discrete jumps |
| follow camera | `player.centered_on = <entity>` | *"the player will be switched to remote view and centered on the given entity"* — tracks continuously between ticks |
| zoom | `player.zoom = z` | baseline 1, same scale as `take_screenshot`'s |
| clean frame | `player.game_view_settings.{show_controller_gui, show_minimap, show_alert_gui, show_crafting_queue, ...} = false` | otherwise the video is 20% toolbar |

`centered_on` is the genuinely new capability: it makes the camera *follow*, and
following is the thing a 300-tick screenshot sequence cannot do at any interval.
Combined with the sibling spec's `storage.camera_subject` — set inside the
`rcon_action_start_*` family, already carrying `{player_index, position, tick}`
with no new RCON traffic — the director becomes `player.centered_on =
game.players[subject].character`, one assignment per dispatch.

**But the peer whose window is captured cannot also be a bot.** `rcon_players`
(`control.lua:2787`) returns players that are `connected` **and have a
`character`**, and the executor addresses bots by that index. A spectator
controller has no character. So putting a client into spectator to steer its
camera removes it from the bot roster mid-run, which is not a camera change, it
is a bot disappearing. The options:

- **A fifth client, spectator, camera only.** Works with today's launch path, no
  unresolved prerequisites. Costs one more Factorio process: ~26 s of sprite
  loading, a GPU-rendering peer, and a peer the server must keep in sync (a
  Factorio server runs no faster than its slowest peer — the same mechanism the
  sibling spec blames for the current shortfall, now applied to a peer that
  exists purely to be filmed).
- **The `--host` process.** If §11.1 resolves in favour of `--host`, the host's
  own player is the camera and there is no extra peer at all — and if the host's
  player has no character it is *already* not a bot, so spectator costs nothing.
  This is the design that wants to exist. It is gated.
- **Film an existing bot client, unsteered (v0).** Capture client 1's window as
  it is. The camera follows bot 1 because that is what bot 1's client shows.
  Honest, fixed, uninteresting, and requires no new process and no unresolved
  question. This is the version that can be built today, and §12 builds it
  first precisely so the clock, the encode and the viewer are proven before the
  steering question is opened.

---

## 7. Coexistence, not replacement

**Frames remain the record. Video is `video = true` in the capture options.**

The sibling spec's §4.1 already introduces
`rcon_frame_capture_start(run_id, options)` with `per_bot = false` as an opt-in.
This design adds a *host-side* option in the same spirit but not in that table —
video is not a mod concern and must not be plumbed through RCON. It belongs on
`record.start()`:

```lua
record.start({video = true})    -- or {video = {fps = 15, client = 1}}
```

Consistent with `per_bot` on the principle (extra capability is asked for, never
default) while differing on the mechanism (the mod cannot start ffmpeg and
should not be told about it).

Why not replacement:

- **The join is verified for frames and can only be checked for video.** A
  frame's tick is *in its filename*, written by the game. A video's tick is a
  derived quantity from a table the host built. Those are different epistemic
  categories and the stronger one should not be retired for the weaker one.
- **A video cannot render "no capture here".** §2. Frames can, and do, and the
  viewer has four honesty requirements built on it.
- **`tickBounds` reads frame ticks** (§1.2). Removing frames moves the analysis
  axis silently.
- **Video needs a display and an encoder; frames need a display.** A run on a
  box with neither loses both, but a run with a display and no working ffmpeg
  still gets frames. Keeping the artefact with fewer prerequisites as the
  default is the same reasoning that keeps `--start-server` until `--host` is
  proven.

The two are joined, not alternatives: they share `game.tick`, so a frame and a
video position for the same tick are the same moment, and the viewer can show
both.

---

## 8. Lifecycle and run binding

**Start.** `record.start()` (`crates/scripting_lua/src/globals/record.rs:510`)
already mints the run id and calls `frame_capture_start(run_id)` — its own doc
says "one call, so the log and the frames cannot disagree about which run they
belong to". Video joins that same call, after the frame capture returns its
opening tick:

1. mint `run_id`
2. `frame_capture_start(run_id)` → `opened_at` (the game's own tick)
3. **wipe `<workspace>/video/`, write `video/run.json` = `{"run": run_id}`**
4. **resolve the window id (§11.2), spawn ffmpeg, start the tick sampler with
   its epoch at `opened_at`**
5. `RunRecorder::start`, `RunStarted` at `opened_at`

Step 4 failing is **not** fatal to the run: the run continues with frames only,
`video.json` is written with `"status": "failed"` and the reason, and a
`tracing::error!` plus one `paris` narration line say so at the moment it
happens. A run that silently lost its video and only reveals it when somebody
opens the viewer a day later is the failure this avoids.

**Stop.** `record.finish(outcome)`:

1. take the second calibration pair from the `-progress` stream
2. write `"k": "stop"` to `ticks.jsonl`
3. send `q` on ffmpeg's stdin, wait up to 5 s for a clean exit
4. SIGTERM, wait 2 s; SIGKILL
5. write `video.json` with `status` (`stopped` / `killed` / `died`) and
   `ffmpeg_exit`
6. `archive_video` beside the existing `archive_frames` call
   (`record/mod.rs:680`)

Because the container is fMP4, steps 3–4 degrade rather than fail: even the
SIGKILL path leaves a playable file.

**Tying to the run, and detecting a recorder that outlived or died.** Three
independent mechanisms, none of which requires the other to work:

- **`video/run.json`, wiped and rewritten at start.** `archive_video` copies
  only if it names this run — `archive_frames`'s existing rule, which exists
  because "the decoy case: a stale directory left by an earlier run" already
  shipped once as a bug (`frames.rs:209`). An orphaned recording from a previous
  run therefore cannot be archived into this run's directory, and cannot appear
  in this run's manifest.
- **`video.json.status`.** Written `recording` at start, rewritten at stop.
  **A `video.json` still saying `recording` in an archived run is, by itself,
  proof that the recorder was never stopped** — the run finished and nobody
  told the encoder. The viewer reports that as a defect rather than showing the
  video as if it were complete.
- **Progress liveness.** The recorder reads `out_time_us` from ffmpeg's
  `-progress` pipe. If it stops advancing for more than 10 s while the run is
  live, the recorder writes a `gap` line into `ticks.jsonl`, sets
  `status: "died"`, and narrates. This is what catches ffmpeg dying silently
  mid-run — which it does, e.g. when the window disappears — as distinct from
  ffmpeg exiting, which the `Child` handle already catches.

Note the pre-existing wrinkle this design does not fix and must not deepen: the
frames sidecar's run id is `mint_run_id()`'s `run-<secs>-<nanos>`
(`record.rs:1178`) while `ReplayScrubber.vue` compares it against the HTTP
**job** id. Whether those are ever equal is outside this spec; `videoJoin` reuses
`runIdCheck` verbatim so that whatever is true of frames stays true of video,
rather than inventing a second, differently-broken comparison.

---

## 9. What breaks in the viewer, concretely

### 9.1 The contract seam, both ends
`crates/server/src/manage/video.rs` (new) publishes `VideoManifest` and
`TickSample`; `cargo test -p factorio-bot-server --features lua --test openapi`
fails until `app/src/api/openapi.snapshot.json` is regenerated
(`UPDATE_OPENAPI_SNAPSHOT=1`), then `app/src/api/openapi.contract.spec.ts` fails
until `app/src/api/types.ts` mirrors it. That is the seam working.

### 9.2 New, not modified
- `app/src/api/videoClock.ts` — `tickToVideoSeconds`, `videoSecondsToTick`,
  `parseVideoClock`. **The only file in the frontend permitted to contain
  seconds.** `frameJoin.ts`'s header says "If a change here ever introduces
  seconds, that is a bug, not a feature" — that stays true; this is where they
  are quarantined instead.
- `app/src/api/videoJoin.ts` — reuses `runIdCheck` and `combineRunMatchChecks`
  from `frameJoin.ts` unchanged (they take strings and `RunMatchCheck`s, not
  frames), adds `videoTickRangeCheck` built from the sidecar's first and last
  samples. Do not copy `combineRunMatchChecks`; a second copy is how two halves
  of a system come to disagree about what a match is.

### 9.3 `app/src/api/client.ts`
`videoManifest()`, `videoUrl()`, `getRunVideo(id)`, `runVideoUrl(id)`.
Note `frameUrl` addresses by `(client, name)` because a later run can reuse a
tick under the same filename; the video is one file per run, so `(runId)` is
enough — but the live one at `<workspace>/video/video.mp4` **is** overwritten
per run and must be cache-busted by run id, not served `immutable` the way
frames are (`frames.rs:381`).

### 9.4 `app/src/components/replay/ReplayScrubber.vue`
The most invasive change, and the one with the most ways to be quietly wrong.

- `current = frameAtTick(clientFrames, tick + origin)` gains a sibling
  `videoAt = tickToVideoSeconds(clock, tick + origin)`. **Both must use the same
  `origin`** — `observedOrigin()` is described in CLAUDE.md as converting "in
  exactly one place" and adding a second conversion site here would break that
  invariant for a saving of nothing.
- A `<video ref>` whose `currentTime` is assigned from `videoAt`. Assigning on
  every slider `input` produces a seek storm; the assignment must be guarded on
  the element's `seeking` flag and coalesced (`requestAnimationFrame` or a short
  debounce), with the *last* requested tick applied after a seek completes so
  the final position is right even if intermediate ones were dropped.
- **Three new honesty states, replacing the ones that do not transfer.**
  `frame-current` / `frame-stale` have no video analogue: a video always has a
  frame at every timestamp, so "stale" is meaningless and its absence must not
  read as "current". Required:
  - `video-out-of-range` — the tick is before the first or after the last
    sample (rule 1).
  - `video-clock-unknown` — the tick falls in a gap (rule 2). **This must dim
    or cover the video element, not merely annotate it.** The frame underneath
    is a real frame of a real moment, and that moment is not this tick; leaving
    it visible with a caption is exactly the fabricated continuity requirement 4
    forbids for frames.
  - `video-clock-unverified` — the rate check failed (rule 4), shown for the
    whole run.
- The existing `run-match-caveat` text is frame-specific ("matched to this run
  by overlapping tick ranges only") and needs a video-side twin, or the same
  banner needs to cover both artefacts.

### 9.5 `app/src/lib/runTimeline.ts` — the silent one
`tickSources` (`:79`) pushes frame ticks into `drawn`, and `tickBounds` /
`leadInTicks` use it. A run with video and no frames loses that contributor and
its axis moves with no error anywhere. Fix: `tickSources` takes the video's
sample tick range and contributes it to `drawn` in the frames' place. Test it
with a fixture that has video and zero frames — otherwise this regression is
invisible until somebody notices two runs' axes do not line up.

`viewsOf`, `botsOf`, `camerasOf`, `frameAt` are unaffected: they take frames and
return empty for a video-only run, and `RunAnalysisPage.vue` already renders
nothing for an empty view list.

### 9.6 `app/src/store/runsStore.ts` / `replayStore.ts`
`runsStore` gains `video`, `videoError` alongside `frames`/`frameError`, and
`getRunVideo` joins the enrichment fetches (which already tolerate one
enrichment failing without losing the run — `:84`). `replayStore` gains the live
manifest alongside `manifest`, with the same "a failed fetch must not lose the
replay" handling at `:99`.

### 9.7 Server
- `crates/server/src/manage/video.rs` — `GET /api/v1/video` (manifest),
  `GET /api/v1/video/file` (ServeFile, ranges), `GET /api/v1/video/ticks`.
- `crates/server/src/runs.rs` — `/runs/{id}/video`, `/runs/{id}/video/file`,
  `/runs/{id}/video/ticks`.
- `crates/core/src/record/video.rs` — `ArchivedVideo`, `archive_video`,
  `parse_tick_samples`. The tick-sample parser lives in `core` for the same
  reason `parse_frame_name` does (`frames.rs:41`): the archive and the live
  endpoint need the same answer, and two parsers for one format is how two
  halves of a system come to disagree.

---

## 10. Failure modes

| Failure | Detected by | Behaviour |
| --- | --- | --- |
| **No display** (`DISPLAY` unset, or headless CI) | recorder, before spawning ffmpeg | `video.json` `status: "failed"`, reason `"no DISPLAY"`, narrated at start. Run proceeds with frames. Never a silent no-op — today a headless server already produces zero frames and the only evidence is an empty directory much later. |
| **Window not found** (client not up yet, `xdotool` finds nothing or finds several) | window resolution (§11.2) | Retry with backoff for up to 30 s (a client takes ~26 s to load sprites), then fail as above. **Finding *more than one* match is a failure, not a coin flip** — picking arbitrarily films an unknown peer, and the filename would not say which. |
| **Window resized mid-run** | ffmpeg (x11grab has a fixed frame size) | ffmpeg either errors out or continues capturing the original geometry, cropping or padding — *which of the two is version-dependent and unverified here*. Either way the progress-liveness check (§8) catches a death, and the size is recorded in `video.json` so a mismatch against the file's actual dimensions is checkable after the fact. Mitigation: `full-screen=false` and a fixed window; do not let anything resize it. |
| **Window occluded or minimised** | not detectable from outside | Two compounding hazards: `halt-rendering-when-minimized=true` is Factorio's default, so a minimised window *stops rendering* and the video freezes on a live game; and an occluded X11 window may grab whatever is drawn over it unless the compositor redirects it. Under Hyprland's Xwayland these windows are separately buffered and should be safe — **unverified, §11.3**. The clock keeps advancing through this, so the video would show a frozen or wrong image at a timestamp the clock says is fine. This is the one failure the clock cannot catch, and it is the strongest argument for the camera peer being a dedicated process nobody touches. |
| **ffmpeg dies silently** | `-progress` `out_time_us` stops advancing >10 s | `gap` line, `status: "died"`, narrated. fMP4 means the file so far is playable. |
| **ffmpeg never starts** (no ffmpeg on PATH, or one without x11grab) | spawn probe at start | The dev shell has x11grab (§11.4) but the *binary's* PATH at run time is not guaranteed to be the dev shell's — the stock nixpkgs ffmpeg has no x11grab and would fail only at the first grab. Probe once at start with a 1-frame trial grab, not with a version string; fail as "no display" does. Do not discover it from a broken file at the end of a 45-minute run. |
| **Disk fills** | recorder, polling free space on the workspace filesystem | At <2 GB free: stop the recorder cleanly, `status: "stopped_low_disk"`, narrate, **let the run continue**. The run's `events.jsonl` and `samples.jsonl` are the artefacts that must survive; the video is the one that may be sacrificed, and it is the one filling the disk. Retention (`DEFAULT_KEEP = 20`, `.keep`) already reaps old runs and now reaps videos with them — do not raise `keep` in the same change, per the sibling spec §7. |
| **Recorder outlives its run** | `video/run.json` mismatch at archive; `status: "recording"` in an archived run | Not archived; reported as a defect in the viewer rather than shown as a complete video. |
| **Rate skew** (dropped/duplicated capture frames) | two-point calibration (§4.4) | Every clock answer marked `certain: false`; `video-clock-unverified` shown for the run. |

---

## 11. Prerequisites and what could not be determined without running

Marked as the sibling spec marks its own: these are gates, not assumptions.

1. **`--host` gives a renderable *and* RCON-able server.** Unresolved, inherited
   verbatim from the sibling spec §10.1–2. The version of this design with no
   extra peer (§6) depends on it entirely. **The v0 in §12 does not**, which is
   why v0 is first.
2. **Finding the window id.** `xdotool` and `xwininfo` are now in the dev shell,
   but neither has been pointed at a running Factorio client — no Factorio was
   launched for this spec. The open question is not the tools, it is whether
   `xdotool search --pid <pid>` finds anything: that needs the client to set
   `_NET_WM_PID`, which SDL normally does but which nobody has confirmed for
   this build under Xwayland. `xwininfo -root -tree` plus a name match is the
   fallback and is worse — it cannot tell two Factorio windows apart, which is
   exactly the case a four-client run presents.
3. **Whether an occluded or unfocused Xwayland window grabs correctly.**
   Unverified. This is the difference between a usable artefact and a video of
   somebody else's terminal.
4. **~~x11grab~~ — settled, no longer a prerequisite.** `flake.nix` (committed,
   `be39d55f`) supplies `ffmpeg-full`, `xdotool` and `xwininfo`, and inside
   `nix develop` the shell reports `D  x11grab   X11 screen capture, using
   XCB`. `ffmpeg-full` came from the **binary cache** — fetched, not compiled —
   so its large closure costs disk, not build time. The stock nixpkgs `ffmpeg`
   (devices `alsa, fbdev, kmsgrab, lavfi, oss, pulse, v4l2`, no x11grab) is what
   an *outside*-the-shell PATH still gets, which matters only for the runtime
   probe in §10: the recorder must not assume its own PATH is the dev shell's.
   **Still unverified:** whether VAAPI H.264 encode works on this Mesa 26.1.5
   stack, which would move the encode off the CPU entirely.

   Related build note, not part of this design but it will bite whoever
   implements it: anything cargo-based must run as `nix develop -c <cmd>`.
   `pkg-config` and Lua 5.4 come from the flake, and a bare `cargo` fails with
   "cannot find Lua5.4 using pkg-config".
5. **Every bitrate and file size in §2 and §5 is an estimate.** No Factorio
   window has been encoded on this machine. Step 1 of §12 exists to replace
   them with measurements before any of the trade-offs built on them are
   trusted.
6. **The UPS cost of the encode.** The sibling spec's step 0 has not been run, so
   the screenshot cost is inference; this design adds an encoder to the same box
   and its cost is likewise unmeasured. **Do not claim video is cheaper in UPS
   than screenshots until both numbers exist.** It is plausible. It is not
   known.
7. **Whether `LuaPlayer::centered_on` works from script without radar or
   remote-view coverage** in 2.1, and whether it renders terrain the anchoring
   player has never charted — the same open question the sibling spec raises for
   a far-aimed `take_screenshot` (§10.6), now applying to a viewport.
8. **Whether a spectator-controller peer still counts as a peer for the
   server's slowest-peer rule**, and therefore what a dedicated camera client
   costs the simulation. A fifth peer that renders and is filmed is exactly the
   kind of thing that could cost more UPS than the six screenshots it replaced.

---

## 12. Implementation plan

Each step independently reviewable and revertable. Steps 1–5 need no change to
how Factorio is launched and no resolution of §11.1.

**Step 1 — measure, before building anything.** On one existing client window,
by hand: grab 60 s at 720p15 and at 1080p30, record the file sizes, the encoder
CPU, and (with the sibling spec's step-0 method — poll `FactorioRcon::game_tick`
against the wall clock) the UPS with and without the encoder running. Write the
numbers into a note. Every trade-off in §2 and §5 is currently an estimate and
this is what makes them data. If the encoder costs more UPS than it saves, this
design is a narrative feature, not a performance one — worth knowing first.

**Step 2 — the clock, alone, with no video at all.** The tick sampler, the free
pairs from `remote_call_timed`, the 2 Hz floor, `ticks.jsonl`,
`archive_video`'s run-id rule, `parse_tick_samples` in `core`. Reviewable as:
"does `ticks.jsonl` from a real run interpolate to the ticks of that run's
frames" — a *checkable* test, because the frames carry the true tick in their
filenames and their file mtimes are host wall clock. The clock can be validated
against an artefact that already exists, before anything depends on it.

**Step 3 — v0 capture: an existing client's window, unsteered.** Window
resolution, the ffmpeg child, `-progress` liveness, the two calibration points,
`video.json` and its `status`, all failure paths in §10. Frames continue
unchanged. Reviewable as: "kill ffmpeg with -9 mid-run — is the file still
playable, and does the run still finish".

**Step 4 — serving and the contract.** `manage/video.rs`, the archived routes,
`ServeFile` with ranges, the OpenAPI snapshot, `types.ts`. Reviewable as: "does
`curl -r 0-99` return a 206".

**Step 5 — the viewer.** `videoClock.ts` with all four rules, `videoJoin.ts`
reusing `runIdCheck`, the `<video>` in `ReplayScrubber.vue` with its three new
states and its seek coalescing, and — do not skip this — `tickSources` taking
the video's range (§9.5) with a video-only fixture pinning it. Reviewable as:
"scrub into a stall and confirm the video is covered, not captioned".

**Step 6 — steering, gated on §11.1 and §11.7.** A dedicated spectator camera
peer (or the `--host` player), `centered_on` driven by the sibling spec's
`storage.camera_subject`, `game_view_settings` cleaned up, and the aim written
to `samples.jsonl` on the same tick axis exactly as the sibling spec's §5
requires — a camera whose subject is only implied is a camera that can lie, and
that is as true of a video as of a screenshot. Re-run step 1's measurement in
this configuration before keeping it.
