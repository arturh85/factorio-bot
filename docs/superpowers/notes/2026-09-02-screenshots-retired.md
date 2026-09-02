# Screenshots retired: video is the visual record

Decided by the project owner on 2026-09-02 and implemented the same day. This
note records what changed, what carries the timeline axis now, what a run with
no frames reads as, and — the part a decision note usually skips — **what was
lost**, because it is not nothing.

## The numbers

One run, `run-1788365280-15443`:

| | count | on disk | resolution |
|---|---|---|---|
| screenshots | 2,164 JPEGs | **947 MB** | 1920x1080 |
| video | 45 min | **290 MB** | 700x854 |

**3.3x the disk, and the disk is the cheaper half of the bill.**
`game.take_screenshot` renders *synchronously inside the game loop* — six
cameras per capture, every 300 ticks (`FRAME_CAPTURE_INTERVAL` in
`mods/BotBridge/control.lua`). The video is grabbed from a frame the GPU has
already drawn, so its cost is an encoder process outside the simulation rather
than update-loop time inside it.

The per-camera figure, for whoever turns capture back on: 0.72 MB per frame at
JPEG quality 85 and 1920x1080, 12 frames a minute, so **~520 MB an hour per
camera**. The camera count with everything enabled is `2 + one per player the
game knows of`, so each additional bot adds another ~520 MB an hour.

## Off by default, not removed — and the reason is not sentiment

The capture code is correct; only its cost is not worth paying. But there is a
harder reason, and it is why deletion was never actually on the table:

**The mod's world-state samplers ride on the capture session.** `sample_force`
is the last line of `on_frame_capture_tick`, and both it and `sample_bots` (the
60-tick beat) return early when `storage.frame_capture` is nil. So
`samples.jsonl` — research, production, power, per-bot inventories, everything
the Research / Production / Inventory panels read — exists *only while a capture
session is running*. Deleting the `frame_capture_start` call, or making it
conditional on wanting pictures, would have taken the entire world-state stream
out with the screenshots, silently, and the first symptom would have been three
empty panels with no error anywhere.

So the session still starts on every recorded run. It just registers **no
camera**.

`crates/core/tests/botbridge_frame_capture.rs::the_world_state_samplers_still_run_when_no_camera_was_asked_for`
is the negative control for exactly that, and it drives the real `control.lua`.

### The switch

`rcon_frame_capture_start(run_id, cameras)`:

| `cameras` | effect |
|---|---|
| absent / `false` | **no camera at all — the default** |
| `true` | every camera: `follow`, `bot-N` per player, `area` (what it used to do) |
| `{ids...}` | exactly those, from the same catalogue |

Reachable from Lua as `rcon.frame_capture_start(run_id, cameras)` and, the way
a run actually asks, as `record.start({frames = true})` or
`record.start({frames = {"follow"}})` — the sibling of the existing
`{video = ...}` option.

Three rules the plumbing follows so a run cannot end up capturing because a
flag defaulted wrong:

- **No fallback anywhere.** There is no `cameras or <something>` in the mod, no
  `FrameCameras::default()` that resolves to anything but `None`, and no
  `Option<FrameCameras>` that a forgotten argument could fill in with the
  expensive answer. `FrameCameras::None` sends *no extra argument at all*, so
  the default call is byte-identical on the wire to what it always sent.
- **An unknown camera id raises, it is never dropped.** A list quietly reduced
  to nothing produces a run that looks configured, renders nothing, and leaves
  exactly the same empty directory a deliberately camera-less run does. The
  refusal happens *before* the wipe, so a call that is going to be refused
  cannot first destroy the previous run's frames.
- **A bot joining mid-run gets a camera only on an all-cameras run.**
  `frame_capture_on_player_joined` exists so a late joiner is not invisible; on
  a run that registered none it would have switched rendering back on one bot at
  a time with nothing reporting it. The *request* decides, which is why
  `all_cameras` is remembered in `storage.frame_capture` rather than guessed
  from the camera list.

## What carries the timeline axis now

`app/src/lib/runTimeline.ts`'s `tickSources` pushes ticks into a `drawn` set,
and `tickBounds`/`leadInTicks` use it to decide where the axis starts. Frame
ticks were the main contributor. With frames off by default, **the video's
`tick_range` carries it**, through the branch §9.5 of
`docs/superpowers/specs/2026-09-02-video-capture-design.md` added:

```ts
if (videoRange !== null && frames.length === 0) {
    all.push(videoRange.from, videoRange.to);
    drawn.push(videoRange.from);
}
```

`app/src/store/runsStore.ts` feeds it `this.video?.tick_range ?? null`.

The narrowing (`frames.length === 0`) stays, and is now the *exception* rather
than the rule it was written as. It stays because a run that opts back into
frames still has to line up against every run recorded before the retirement:
video contributing *beside* frames would move an axis that frames already
decide, and two runs' axes would stop lining up for a reason nothing reports.
That is the direction the first attempt at this fix got wrong, and it is pinned
from both sides in `runTimeline.spec.ts`.

Verified, not assumed — mutation results:

| mutation | tests that go red |
|---|---|
| video no longer pushed into `drawn` | 3, all video-only axis tests |
| guard dropped, video contributes beside frames | 1, `leaves the axis exactly where the frames put it when a run has both` |
| `axisFrom`'s "never cut more than you keep" removed | 4, including the new late-recording case |

Two runs with **neither** frames nor video fall back to lane bars, which are
drawn and therefore an honest axis start; with no lanes either, `drawn` is empty
and no trim is applied at all. Neither case moves silently.

**One loose end, not fixed here** (`app/src/api/` is outside this change's
ownership): `RunsPage.vue` computes its own `videoRange` from
`clockTickRange(videoClock)` while `runsStore` computes the axis from
`video.tick_range`. Both derive from the same samples, but they are two
independent derivations of one range, and if they ever disagree the "recording
starts at tick N" button seeks to a tick outside the axis.

## What a zero-frame run reads as

**"None were captured"**, everywhere, and never "capture failed" or a missing
file:

- `manifest.json` → `frames: 0`. Already correct: `archive_frames` copies zero
  files and reports the count.
- `frames/index.json` → `[]`, written unconditionally. Now pinned by
  `a_run_that_captured_no_frames_reports_an_empty_index_not_a_missing_file` and
  `a_workspace_with_no_clients_still_writes_the_empty_index` — a missing index
  beside `frames: 0` would be indistinguishable from a capture that failed.
- `frames/run.json` is **still written** even when no camera was registered, and
  that is deliberate. `archive_frames` only walks a client directory whose
  sidecar names this run; an unclaimed directory is skipped as somebody else's
  leftovers. So the sidecar is what turns "this run captured nothing" into a
  report rather than a shrug.
- `app/src/api/frameJoin.ts` — checked, unchanged, already honest. With zero
  frames `manifestTickRange` is `null`, so `tickOverlapCheck` answers
  **`inconclusive`** ("not enough observed or frame ticks to compare ranges"),
  which is the right one: there is nothing to compare, and that is neither a
  contradiction nor a confirmation. `runIdCheck` still answers `confirmed` from
  the sidecar, which is a true statement about *which run the directory belongs
  to* and is what it has always meant. `camerasForClient` and `framesForClient`
  answer `[]`, `frameAtTick` answers `null` — all documented in that file as the
  honest answers for a camera that produced nothing.
- `RunsPage.vue` says so in words. It used to render a `<select>` with no
  options above the line "This bot and camera captured no frames in this run.",
  naming a bot and a camera nobody had chosen — a panel that read as broken. It
  now shows one sentence and no picker. A *failed* frames listing still wins
  over that message, because "none were captured" and "we could not find out"
  are different answers.

## What is lost

This trade is not free, and the honest list is short but real:

1. **Tick-exactness.** A frame's filename carries `game.tick`, written by the
   game at the moment of capture. A video frame's tick is *derived*, by
   interpolating between clock samples the host recorded — accurate, but a
   computation, not a measurement. For any question of the form "what exactly
   was on screen at tick N", a frame answers and a video approximates.
2. **Resolution.** 1920x1080 against 700x854. Reading an alert icon, an item
   count in a machine's tooltip, or which of two adjacent inserters is facing
   the wrong way is a question the still can answer and the video frame cannot.
   This is the single biggest loss and it is a real one.
3. **A video cannot say "nothing was captured here".** `frameAtTick` returning
   `null` is a rendered state, and a dropped frame is a visible gap. While the
   game stalls, the recorder keeps writing frames of the last drawn image, which
   looks exactly like a game that was running and idle. Only the tick sidecar
   distinguishes those, and only because it refuses to interpolate across a gap.
4. **Per-bot vantage points.** `bot-N` cameras gave a picture of each bot
   individually. The video films one client's window.

What is gained, beyond the disk and the UPS: ~84x the temporal density (15 fps
against one frame per 5 seconds), so every "what happened between those two
frames" question that previously had no answer has one, and continuous camera
motion, which a 300-tick screenshot sequence structurally cannot do.

If a question needs (1) or (2), turn one camera back on for that run:
`record.start({frames = {"follow"}, video = true})` costs ~520 MB an hour rather
than the ~3 GB the retired default was costing on a four-bot run.

## Files

- `mods/BotBridge/control.lua` — `frame_capture_catalogue`,
  `frame_capture_select`, the `cameras` argument, the joiner rule.
- `crates/core/src/factorio/rcon.rs` — `FrameCameras`, `frame_capture_args`.
- `crates/scripting_lua/src/globals/rcon.rs` — `frame_cameras_from_lua`, the
  `rcon.frame_capture_start` binding and its doc entry.
- `crates/scripting_lua/src/globals/record.rs` — `frame_options`,
  `record.start({frames = ...})`.
- `crates/core/src/record/frames.rs` — the zero-frame archive pins.
- `app/src/lib/runTimeline.ts`, `app/src/pages/RunsPage.vue` — the viewer.
- `crates/core/tests/botbridge_frame_capture.rs`,
  `app/src/pages/RunsPage.spec.ts` — new tests.
- `scripts/camera_check.lua`, `scripts/run_id_check.lua` — live probes, now
  passing `true` explicitly, since their whole subject is frames on disk.

A run only picks up mod edits with `FACTORIO_BOT_REFRESH_MODS=1`.
