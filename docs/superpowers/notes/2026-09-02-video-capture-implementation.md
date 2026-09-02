# Video capture v0, built

Implements `docs/superpowers/specs/2026-09-02-video-capture-design.md` steps
2–5: the clock, the v0 capture (an existing client's window, unsteered), the
serving and contract seam, and the viewer. **Nothing here steers a camera,
spawns a spectator peer, or depends on `--host`** — step 6 stays gated, as does
the director camera in the sibling spec.

Nothing below has been run against a live Factorio. See "What is unverified".

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

## The one thing that is NOT wired: `record.start({video = ...})`

The Lua entry point lives in `crates/scripting_lua/src/globals/record.rs`, which
another agent owned for the duration of this change, so it was **not edited**.
Everything below it is built and tested; the trigger is not connected, so no run
records video yet.

The patch is small. In `create_lua_record_with_slot`:

```rust
// `start` currently takes no arguments. mlua accepts an absent argument as
// `None`, so this stays backward compatible with every existing script.
lua.create_async_function(move |_lua, options: Option<LuaTable>| {
    // ... existing body, unchanged, through `recorder.record(RunStarted)` ...

    if let Some(video) = video_options(options.as_ref())? {
        // Never fatal: `VideoRecorder::start` returns Ok with
        // `status: failed` for every capture failure, and the run goes on
        // with frames. Only an unwritable video directory is an Err.
        let capture = VideoRecorder::start(
            &workspace, &run_id, video, rcon.clone(), Some(opened_at),
        )
        .await
        .map_err(record_error)?;
        recorder.attach_video(capture);
    }
    *slot.lock() = Some(recorder);
    Ok(run_id)
})
```

with

```rust
/// `video = true` -> defaults; `video = {resolution = "1080p", fps = 15,
/// client = 1}` -> overrides. An unknown resolution is an error at start, not
/// a silent fallback: a run that quietly recorded at the wrong size is worse
/// than one that refused to start.
fn video_options(options: Option<&LuaTable>) -> LuaResult<Option<VideoOptions>> {
    let Some(table) = options else { return Ok(None) };
    match table.get::<LuaValue>("video")? {
        LuaValue::Nil | LuaValue::Boolean(false) => Ok(None),
        LuaValue::Boolean(true) => Ok(Some(VideoOptions::default())),
        LuaValue::Table(video) => {
            let mut chosen = VideoOptions::default();
            if let Some(name) = video.get::<Option<String>>("resolution")? {
                chosen.resolution = Resolution::parse(&name).map_err(record_error)?;
            }
            if let Some(fps) = video.get::<Option<u32>>("fps")? { chosen.fps = fps; }
            if let Some(client) = video.get::<Option<u8>>("client")? { chosen.client = client; }
            Ok(Some(chosen))
        }
        other => Err(record_error(format!(
            "record.start: video must be a boolean or a table, got {}", other.type_name()
        ))),
    }
}
```

and, in `record.finish`, **before** `recorder.finish(...)`:

```rust
// `finish` archives the recording but cannot stop it -- it is not async. A
// recording that was never stopped is archived with `status: "recording"`,
// which the viewer reports as a defect rather than showing as complete.
recorder.stop_video(Some(tick)).await.map_err(record_error)?;
```

Note the lock discipline: the slot is a `parking_lot::Mutex`, so the recorder
has to be taken out of it (or the guard dropped) around the `.await`.

`VideoOptions.pid` is left `None` by this patch, which means window resolution
falls back to the name search — see the next section.

## What is unverified without a live capture

Everything that needs an X server, a running Factorio client, or ffmpeg actually
grabbing pixels. Specifically:

- **Whether `xdotool search --pid` finds anything**, i.e. whether SDL sets
  `_NET_WM_PID` under Xwayland for this build. The spec flags this as unresolved
  (§11.2) and the code probes it and falls back: the pid search is tried first,
  then a name search. Until the caller supplies a pid (see above), only the name
  search runs — and **a name search cannot tell two Factorio clients apart**, so
  on a multi-client run it will find several windows and refuse, which is the
  designed behaviour but is not the behaviour anyone wants. Resolving §11.2 is
  the highest-value next experiment: `xdotool search --pid <client pid> --name
  Factorio` against a running client, one line of output or none.
- **Whether the observed geometry ever matches the request** under Hyprland.
  The code records the observed one either way and warns on a mismatch; nobody
  has seen the warning fire or not fire.
- **Whether ffmpeg's x11grab actually captures this window**, whether an
  occluded or unfocused Xwayland window grabs correctly (§11.3), and whether the
  one-frame probe correctly distinguishes an ffmpeg without x11grab. All three
  are exercised only by a real run.
- **Every bitrate and file size.** No Factorio window has been encoded on this
  machine. Step 1 of the spec's plan has not been run and is still the thing to
  do before trusting any size trade-off.
- **The UPS cost of the encoder.** Unmeasured, as is the screenshot cost it
  would be compared against. Do not claim video is cheaper than screenshots.
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
