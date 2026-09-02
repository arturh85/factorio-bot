# Video capture v0, built

Implements `docs/superpowers/specs/2026-09-02-video-capture-design.md` steps
2–5: the clock, the v0 capture (an existing client's window, unsteered), the
serving and contract seam, and the viewer. **Nothing here steers a camera,
spawns a spectator peer, or depends on `--host`** — step 6 stays gated, as does
the director camera in the sibling spec.

`record.start({video = true})` is wired (see "The trigger, and how the client is
chosen"). No run has recorded yet: nothing below has captured a frame from a
live Factorio, and every number in the spec's §2 is still an estimate. See
"What is unverified".

## What exists

| Piece | Where |
| --- | --- |
| the clock (`ticks.jsonl`, parse/write, gap lines) | `crates/core/src/record/video/clock.rs` |
| window resolution, sizing, observed geometry | `crates/core/src/record/video/window.rs` |
| encoder command line and progress parsing | `crates/core/src/record/video/ffmpeg.rs` |
| the recorder: child, sampler, liveness, disk guard | `crates/core/src/record/video/recorder.rs` |
| `VideoManifest`/`VideoRecord`, `archive_video`, `read_video_dir` | `crates/core/src/record/video/mod.rs` |
| `attach_video` / `stop_video`, `archive_video` at finish | `crates/core/src/record/mod.rs` |
| `GET /api/v1/video`, `/video/file`, `/video/ticks` | `crates/server/src/manage/video.rs` |
| `/runs/{id}/video`, `/video/file`, `/video/ticks` | `crates/server/src/runs.rs` |
| tick ↔ seconds, all four rules | `app/src/api/videoClock.ts` |
| the join and the recording's own defects | `app/src/api/videoJoin.ts` |
| the `<video>` and its three states | `app/src/components/replay/ReplayScrubber.vue` |
| the axis contributor (§9.5) | `app/src/lib/runTimeline.ts` |

Tests: 41 in `crates/core` (`record::video::*`), 11 HTTP-level in
`crates/server/tests/manage_video.rs`, and 25 + 11 + 4 + 9 + 6 + 2 in the
frontend across `videoClock.spec.ts`, `videoJoin.spec.ts`, `runTimeline.spec.ts`,
`ReplayScrubber.spec.ts`, `runsStore.spec.ts`, `replayStore.spec.ts` and
`client.spec.ts`. Full suites green; frontend coverage 97.0 % statements /
93.3 % branches against gates of 90 / 80.

## The clock, and its measured error

`ticks.jsonl` is written **host-side**, as the spec's §1.1 requires — the mod
cannot write it, and it does not have to: `FactorioRcon::game_tick()` is a plain
`/silent-command` and both halves of a `(tick, wall_ms)` pair are read on one
machine against one monotonic clock.

`wall_ms` is stamped at the **midpoint of send and receive**
(`recorder.rs::sample_ticks`), not at receipt, so the systematic one-round-trip
late bias is removed rather than being merely small.

**Measured interpolation error**, simulating the bang-bang UPS profile between
50 and 60 UPS that maximises the chord-versus-curve difference
(`videoClock.spec.ts`, "the interpolation error the chosen cadence buys"):

| floor cadence | worst error | vs. a 15 fps frame (66.7 ms) |
| --- | --- | --- |
| 4 Hz | 15.7 ms | 0.24 frames |
| **2 Hz (chosen)** | **17.9 ms** | **0.27 frames** |
| 1 Hz | 45.5 ms | 0.68 frames |
| ~0.5 Hz | 81.4 ms | 1.2 frames |

The spec's closed form predicts 25 ms at 2 Hz; the simulation measures 17.9 ms
because integer ticks quantise where the rate switch lands, which the formula
does not model. Either way the claim that matters holds: **at 2 Hz the clock is
not the limiting error** — the video's own frame period is, at 15 fps *and* at
30 fps, so raising the frame rate later does not silently invalidate the clock.
At ~0.5 Hz it becomes the dominant error, which is why the cadence is a choice.

Past `MAX_INTERP_GAP_MS` (2000 ms) the clock stops answering rather than
answering worse: only the two observed endpoints still resolve, and everything
between them is `null`.

## Corrections to the spec found while building

1. **§5's `-nostdin` and §8's "send `q` on ffmpeg's stdin" cannot both hold.**
   `-nostdin` is precisely the flag that makes ffmpeg ignore stdin, so the clean
   stop would never have worked. Resolved in favour of the stop path: the
   recording command drops `-nostdin` (the child's stdin is a private pipe, not
   an inherited terminal, so the flag buys nothing there) and the one-frame
   probe keeps it, where it is true. Pinned by
   `the_recording_command_leaves_stdin_open_so_it_can_be_stopped_cleanly`.

2. **§5's `-vf scale=1280:-2` contradicts the approved decision section.** The
   decision says to reach 720p by sizing the window and grabbing 1:1, "never by
   downscaling in ffmpeg". There is no `scale` filter; `-video_size` carries the
   size `xwininfo` **observed**, which is also what `video.json` records.
   `requested_width`/`requested_height` sit beside `width`/`height` so a tiling
   compositor's adjustment is reportable as the warning it is.

3. **§8's ladder loses its SIGTERM rung.** `q` → 5 s → SIGKILL. Sending SIGTERM
   needs a `libc` dependency `crates/core` does not have, and adding one was
   outside this change's file boundary. The fragmented container is what made
   the ladder degrade rather than fail in the first place, so the property it
   protected — a killed recorder still leaves a playable file — is unchanged.
   Documented in place at `VideoRecorder::shut_down_encoder`.

4. **The disk guard shells out to `df -kP`** for the same reason: no `statvfs`
   dependency. `None` (tool missing, unparseable) is *unknown* and does not stop
   a recording — refusing to record because a shell tool was absent would be a
   worse failure than filling a disk that may not even be short.

5. **§9.5's fix needed narrowing.** Feeding the video's tick range into
   `tickSources`'s `drawn` set unconditionally would move the axis of every run
   that captured *both*, because the recording starts before the first frame.
   The contributor therefore applies **in the frames' place** — only when there
   are no placeable frames — which fixes the silent case (video, no frames,
   axis computed from splits and lanes alone) without introducing a new one.
   Both directions are pinned in `runTimeline.spec.ts`.

6. **`TickSample.k` is always written**, where the spec's example lines omit it
   for an ordinary sample. This struct is also the HTTP wire shape, and a
   sometimes-absent key forces `types.ts` to declare it optional, at which point
   "absent" and "null" become a distinction the client has to make and this
   format never intends. A line *without* `k` still parses, so every document
   the spec shows is readable. Cost: ~11 bytes a line, ~60 KB over a 45-minute
   run, against a recording measured in hundreds of megabytes.

## The trigger, and how the client is chosen

`record.start()` takes an optional table, and `record.finish` stops the encoder
before it archives:

```lua
record.start({video = true})                     -- 720p, 15 fps, client 1
record.start({video = {resolution = "1080p"}})   -- the opt-in, for a final run
record.start({video = {client = 2, fps = 30}})   -- film a different client
```

Three things about the wiring are decisions, not mechanics:

1. **The options are read before anything is created.** `video_options` runs as
   the first statement of `record.start`, ahead of the already-running check,
   the run id and the RCON call. A typo in the options therefore costs a Lua
   error and nothing else — no run directory for a run that never started.
   Pinned by `a_bad_video_option_is_refused_before_anything_is_created`, which
   asserts on the *message*: it reports the bad resolution rather than "already
   running", which it could only do by having read the options first.
2. **An unknown resolution raises.** `Resolution::parse` refuses anything but
   `"720p"` / `"1080p"`, and so does a `video` that is neither a boolean nor a
   table. A run that quietly recorded at the wrong size is worse than one that
   refused to start.
3. **`record.finish` had to become async.** It is the only place that can stop
   the encoder — `RunRecorder::finish` archives the recording but cannot stop
   it, being sync — and a recording archived while still running is archived
   saying `status: "recording"`, which the design makes the proof that nobody
   stopped it. The slot is a `parking_lot::Mutex`, so the recorder is taken out
   of it in a scope that ends before the `.await`; a guard held across the await
   is not `Send` and would deadlock the next caller.

**Which client gets filmed: 1, by default, configurable.** Not arbitrary:

- client 1 is the only client a run is *guaranteed* to have — any run with a
  graphical client at all has that one, so the default means the same thing on a
  one-bot run and a four-bot one;
- it is registered with the mod first (`whoami("client1")`, in order), so it is
  the lowest player index and the bot most scripts drive first;
- it is the same client on every run, so two recordings are comparable without
  reading their manifests first.

None of that makes client 1 *special* — which is exactly why `client = N` is an
option rather than a constant. v0 does not steer the camera either way: the
video shows whatever that client shows.

## How the pid reaches the recorder

The window search can only tell four identical Factorio windows apart if it has
a pid (`docs/superpowers/notes/2026-09-02-video-prerequisites-settled.md`), and
`VideoOptions.pid` was `None` on every path. It is now **discovered**, in
`window.rs::find_client_pid`, rather than threaded down from the spawn site:

```
<workspace>/client<N>/            the directory client N is set up in and runs from
  -> /proc/<pid>/exe               the binary the kernel says a process is executing
  -> the one pid whose exe lives under that directory
```

`VideoOptions.pid` is still honoured when a caller sets it; discovery only fills
in the `None`.

Why discovery and not bookkeeping. The pid at the spawn site is the more direct
fact, but it is available on exactly one path — the process that spawned the
clients — and reaching the Lua binding from there means a new argument on
`run_lua`, `run_script`, `run_script_file`, the CLI and the HTTP executor, i.e.
six files across four crates, of which five are outside this change's boundary.
The workspace and the client number are already in hand at
`VideoRecorder::start`, they are true whether or not this process spawned the
clients, and `/proc/<pid>/exe` is a link the kernel maintains and a process
cannot rewrite. **If the pid is ever wanted for something other than the window
search, thread it properly** — this is the cheap correct answer to one question,
not a general mechanism.

Three rules it enforces, each with a test:

- **The headless server is never a client.** It runs from `<workspace>/server/`,
  and filming it would film nothing: it renders no window at all. (The live
  probe agrees — `xdotool search --pid` returned nothing for the server.)
- **`client1` is not `client10`.** Matching is `Path::starts_with`, which
  compares whole components; a string prefix test would silently film the wrong
  peer on a run big enough to have a tenth client.
- **Two processes from one instance directory are refused, not ranked.** `.lock`
  should make it impossible; if it happens anyway, picking one films an unknown
  peer — the same rule the window search follows for two matching windows.

A failed lookup is a warning, never a failure: the search falls back to the
window name, which is a real answer on a single-client run.

**Validated against the live four-client run, before it exited.** Reading
`/proc/*/exe` while run 28 was up returned exactly:

```
3764406  workspace/server/bin/x64/factorio     <- server, correctly not a client
3769396  workspace/client1/bin/x64/factorio
3769400  workspace/client2/bin/x64/factorio
3769402  workspace/client3/bin/x64/factorio
3769403  workspace/client4/bin/x64/factorio
```

which is *the same pid list*, in the same order, that the window probe in the
prerequisites note resolved to one window each. So the two halves of the chain
have now been observed against the same live run — separately. They have still
never been run end to end.

## What is unverified without a live capture

Everything that needs an X server, a running Factorio client, or ffmpeg actually
grabbing pixels. Specifically:

- ~~**Whether `xdotool search --pid` finds anything.**~~ Settled: SDL does set
  `_NET_WM_PID` under Xwayland here, one window per graphical client and none
  for the headless server. §11.2's risk does not exist. What is *still*
  unverified is the pid search running **from inside the recorder** — the
  discovery and the search have each been observed against the same live run,
  never joined.
- **Whether the observed geometry ever matches the request** under Hyprland. It
  will not: the one window measured was **706x854**, not landscape at all, so
  the resize is load-bearing rather than belt-and-braces and the mismatch
  warning should be expected to fire. Whether `xdotool windowsize` moves a
  tiling-compositor window at all is the open half — the code records what
  `xwininfo` observes either way, which is the only reason this is a warning and
  not a silent wrong-size recording.
- **Whether ffmpeg's x11grab actually captures this window**, whether an
  occluded or unfocused Xwayland window grabs correctly (§11.3), and whether the
  one-frame probe correctly distinguishes an ffmpeg without x11grab. All three
  are exercised only by a real run.
- **Every bitrate and file size.** No Factorio window has been encoded on this
  machine. Step 1 of the spec's plan has not been run and is still the thing to
  do before trusting any size trade-off.
- **The UPS cost of the encoder.** Unmeasured, as is the screenshot cost it
  would be compared against. Do not claim video is cheaper than screenshots.
- **The whole trigger, end to end.** No `record.start({video = true})` has run
  against a live game. The Lua path, the pid discovery, the window search, the
  resize, the probe and the encoder have never executed in sequence.
- **The `-progress` liveness path and the second calibration pair.** The
  plumbing is there and the arithmetic is unit-tested, but no ffmpeg has ever
  written into that pipe here, so the parse has never met real output.
- **The seek coalescing in `ReplayScrubber.vue`.** The states are tested in
  jsdom; the `currentTime` assignment and the `@seeked` retry are not, because
  jsdom does not implement media playback. (`eslint` caught that `onSeeked` was
  initially not wired to the element at all — it is now.)
- **`archive_video` on a real run.** Unit-tested against seeded directories,
  including the stale-run decoy, but never against a run that actually recorded.

## Contract seam

Both ends were made to fail before they were made to pass. Adding the routes
turned `the_committed_openapi_snapshot_matches_the_published_spec` red with
"GET /api/v1/video is published but missing from the snapshot" plus seven
missing schemas; regenerating with `UPDATE_OPENAPI_SNAPSHOT=1` fixed the Rust
end, and the TypeScript end then required `types.ts` and
`openapi.contract.spec.ts` to mirror it (287 contract assertions, up from 241).

One deliberate asymmetry: `videoUrl()` appends `?run=<id>` and the server never
reads it. It is a cache key, not a parameter — the live recording is one file at
one URL that the next run overwrites — so it is *not* published as a query
parameter, because publishing an argument the handler ignores would put a lie in
a generated client. The reason is written into the OPERATIONS table.

## Caching, which differs between the two file routes on purpose

`/api/v1/video/file` is `Cache-Control: no-cache`; `/api/v1/runs/{id}/video/file`
is `immutable`. A frame is immutable because a tick never recurs, but the live
recording is overwritten in place by the next run — caching it the way frames
are cached would show the previous run's video beside this run's timeline with
nothing saying so.
