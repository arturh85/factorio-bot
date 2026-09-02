# The video capture that died in one second and was noticed 37 minutes later

The first live capture ran the whole ladder run and produced a `video.mp4` of
zero bytes. The immediate cause is fixed in `8482d560`: `record_args` passed
`-movflags +frag_keyframe+empty_moov+default_base_is_moof`, and
`default_base_is_moof` is not a value ffmpeg 9.0.1 knows — the flag is
`default_base_moof`. Reproduced here against a lavfi source, since the clients
are gone:

```
=== -movflags +frag_keyframe+empty_moov+default_base_is_moof ===
[mov muxer] [Eval] Undefined constant or missing '(' in 'default_base_is_moof'
[mov muxer] Unable to parse "movflags" option value "default_base_is_moof"
[mov muxer] Error setting option movflags to value +frag_keyframe+empty_moov+default_base_is_moof.
[out#0/mp4] Could not write header (incorrect codec parameters ?): Invalid argument
exit=234 bytes=0
=== -movflags +frag_keyframe+empty_moov+default_base_moof ===
exit=0 bytes=11764
```

Note the spec still carries the wrong spelling at
`docs/superpowers/specs/2026-09-02-video-capture-design.md:383`. It is outside
this change's boundary and is left for its owner.

This note is about the other half: **nothing noticed.**

## What the run actually left behind

Read off disk after the run finished, not from memory:

```json
{ "status": "stopped", "reason": null, "ffmpeg_exit": 234,
  "calibration": [], "rate_ok": null }
```

and `ticks.jsonl`, 4,294 lines over 37 minutes, ending

```
{"t":137825,"w":2222956,"k":"sample"}
{"t":137825,"w":2223159,"k":"stop"}
```

Every artefact is individually well-formed and the set of them is a lie. The
exit status *was* captured — the initial diagnosis that `ffmpeg_exit` stayed
null was a mid-run reading, and the field is filled at stop. What was missing
is worse than a missing field:

- `"status": "stopped"` for a process that exited **234**. The shutdown path
  asked whether the child had been waited on successfully and never asked what
  it said.
- `"reason": null` beside a known-bad exit. The verdict existed; the
  explanation was thrown away. ffmpeg printed the four lines above and every
  one of them went into a `Stdio::piped()` stderr that had no reader anywhere
  in the process. The only way to learn why was to run ffmpeg again by hand.
- A tick log that runs the full length of the run and closes tidily, describing
  a video of zero bytes. Nothing in it is false and the whole of it is
  misleading.
- The child sat in the process table as a zombie for 37 minutes, because
  nothing ever called `wait` on it.

## Why the mechanism that was supposed to catch this did not

The design nominates one detector for "ffmpeg dies silently": `-progress`
`out_time_us` failing to advance for 10 s (§8, hazard table line "ffmpeg dies
silently"). The implementation is faithful to it, and it was structurally
incapable of firing here, because the whole liveness block is guarded on

```rust
if shared.progress_seen.load(Ordering::Relaxed) { ... }
```

A child that dies **before its first progress line** never sets that flag, so
the guard is false forever and the detector is inert. The design's detector
watches an encoder that stops working. It cannot see an encoder that never
started working, which is the mode ffmpeg fails in when it rejects an argument:
it exits at option parsing, in well under a second, having emitted no progress
at all.

"Watch the output stream go quiet" is a liveness check on a *running* thing. It
is not a substitute for asking the operating system whether the thing is
running.

## What changed

All of it in `crates/core/src/record/video/`.

**Detection: ask the child.** The tick sampler already wakes every 500 ms, so it
now calls `try_wait` on the encoder each time round (`encoder_exit`). A death is
observed within one sampling interval instead of at the end of the run, and the
same call reaps the zombie. `try_wait` polled from the sampler rather than a
task parked in `child.wait()`, because `stop` needs the same handle and a task
holding that lock for the whole run would deadlock the stop.

The progress-stall detector stays — it catches a live-but-frozen encoder, which
the exit check cannot see — but it now ends the recording rather than annotating
it: it kills the child, on the grounds that the fragmented container means
everything written so far is already playable, and an encoder that thawed later
would go on appending to a file whose manifest has already declared it dead.

**`video.json` is rewritten at the moment of death.** The record moved into
`Shared`, so the background task that discovers the death writes the file there
and then. Previously `stop` was the only writer, which is precisely why a
manifest could claim `recording` for 37 minutes. A caller reading the directory
mid-run now gets the truth.

**stderr is read.** `read_stderr` keeps the last 20 lines, sends every line to
`tracing` (diagnostics, stderr), and narrates the first 10 through `paris`
(stdout), where the person watching the run can act on it. The tail is quoted
into `reason`, so `video.json` carries ffmpeg's own words. This is also a
robustness fix independent of any message: a pipe nobody reads is a pipe that
fills, and at 64 KiB the child blocks on it.

`attach_child` is the single place that decides which of ffmpeg's pipes get
read, and the tests drive the real one rather than a parallel fixture — stderr
went unread for as long as this file existed, and a second wiring path is how
that comes back.

**Two verdicts became pure functions**, so they can be pinned directly:

- `stop_verdict(clean, code, tail)` — ffmpeg answers a `q` with 0 (measured
  against 9.0.1, not assumed, because the branch turns on it), so a clean wait
  returning any other code is a death inside the last sampling interval, not a
  clean stop. This is the function that turns the run's actual `"stopped"` /
  `234` / `null` into `died` with a reason.
- `zero_byte_verdict(status, existing, tail)` — **look at the artefact, not
  only at the exit status.** A `video.mp4` of zero bytes is not a recording
  however the encoder exited, and the file is right there to be measured. A
  capture that never started keeps its own reason: "there was no DISPLAY" beats
  "wrote no bytes".

## The clock: it stops with the encoder

Decided rather than defaulted. Once the encoder is declared dead, the sampler
appends a `gap` line naming the death and **stops**.

Sampling on would not have been *wrong* — the ticks are true observations of a
game that is still running. But this table has exactly one purpose, joining a
video, and `spawn_tasks` only starts it when there is a video to join. A clock
that outlives its video reads as a healthy recording: 4,294 lines closing with a
tidy `stop` is what a good run looks like, and that is what this one produced.
The run's own record is `events.jsonl` and `samples.jsonl`, neither of which is
touched by any of this, so nothing true is lost — and 2 RCON commands a second
stop being spent on a join that can never happen.

The file now ends with a gap that says why, which is a shape a reader can only
interpret one way.

## The design gap: the probe proved the wrong thing

The spec deliberately refuses a version-string probe and requires a real
one-frame trial grab. That instinct is right and it still let this through,
because the probe grabbed its frame to `-f null`:

```
-f x11grab -framerate 1 -video_size ... -window_id ... -i :0 -frames:v 1 -f null -
```

There is no `-c:v`, no `-x264-params` and no `-movflags` in that command. A null
muxer builds no container, so it validates no container flag. The probe proved
x11grab worked — which was never in doubt on this machine — and proved nothing
whatever about the command that actually runs. It could not have failed for the
reason the recording failed.

So the probe now writes a real one-frame fragmented MP4 to a temporary file and
deletes it. The mechanism that makes it stay honest is not the new arguments but
the factoring: `output_args` is one function, called by both `record_args` and
`probe_args`, and
`the_probe_exercises_the_same_output_chain_as_the_recording` asserts the whole
chain appears in both **contiguously and in order** — a subset check would let a
future flag be added to one and not the other, which is the same hole in a
smaller shape. Any argument added to the recording is exercised by the probe
automatically.

Cost, measured above: one frame, ~12 KB, well under a second, against a run of
45 minutes. The probe also now fails a probe that exits 0 having written zero
bytes, since an exit status alone is not evidence that anything was written —
that being the exact shape of the failure.

## Left open, deliberately

`calibration: []` and `rate_ok: null` were correct behaviour: there were no
frames, so there was nothing to calibrate. But it means a video would have been
*servable* with a clock nobody had checked, and `rate_ok: null` means "unknown",
which a consumer can easily read as "fine". The right fix is on the consuming
side — the viewer refusing an unverified clock the way `videoClock.ts` already
refuses to interpolate across a stall, or the server declining to serve a
recording of zero bytes — and both `crates/server/` and `app/src/` are outside
this change's boundary. Adding a predicate to `VideoRecord` that nothing calls
would have been API for its own sake. Recorded here for whoever owns those.

## Evidence

Red first, against the code as shipped:

```
---- the_probe_exercises_the_same_output_chain_as_the_recording ----
the probe never touches the muxer that fails: ["-hide_banner", "-nostdin",
"-loglevel", "error", "-f", "x11grab", "-framerate", "1", "-video_size",
"1280x720", "-window_id", "0x2c00007", "-i", ":0", "-frames:v", "1", "-f",
"null", "-"]

---- an_encoder_that_dies_before_reporting_any_progress_is_noticed ----
the sampler kept polling a dead encoder; that is the 158 KB of ticks.jsonl
```

The third red test failed for the *wrong reason* and was rewritten. It asserted
that the last line of `ticks.jsonl` is a `gap`, and that passed accidentally:
the fixture's tick source answered nothing, so every poll wrote a gap. The
failure output said so —
`reason: Some("the game did not answer a tick query")` — and the test now uses a
source that keeps answering, so the death is the only gap in the file and the
count is asserted.

The encoder in these tests is `/bin/sh`, not ffmpeg: what is under test is the
recorder noticing a dead child, which is true of any child, and the tests must
not depend on ffmpeg being installed.

Then each part of the fix was mutated in turn. Seven mutations, seven killed:

| mutation | tests that went red |
| --- | --- |
| sampler never asks whether the child is alive | `an_encoder_that_dies_before_reporting_any_progress_is_noticed`, `the_clock_stops_at_the_gap_that_names_the_death` |
| stderr pipe opened and never read | `an_encoder_that_dies_before_reporting_any_progress_is_noticed`, `ffmpegs_stderr_is_read_rather_than_discarded` |
| probe goes back to a null muxer | `the_probe_exercises_the_same_output_chain_as_the_recording`, `the_probe_grabs_exactly_one_frame_into_a_real_file` |
| death recorded in memory but not written to `video.json` | `an_encoder_that_dies_before_reporting_any_progress_is_noticed`, `an_encoder_that_stops_advancing_is_declared_dead_and_stopped` |
| clock keeps ticking after the encoder dies | `an_encoder_that_dies_before_reporting_any_progress_is_noticed`, `the_clock_stops_at_the_gap_that_names_the_death` |
| any clean wait is a clean stop (the shipped bug) | `an_encoder_that_exited_234_is_never_called_stopped` |
| a zero-byte recording is not questioned | `a_recording_of_no_bytes_is_a_death_unless_it_never_started` |

**One test in this change cannot go red and is labelled as such.**
`a_zero_byte_recording_is_never_reported_as_stopped_cleanly` survived the
zero-byte mutation, and that is what it is for: it is a negative control pinning
that the zero-byte rule does *not* paint over a better explanation. It does not
pin the rule itself — `a_recording_of_no_bytes_is_a_death_unless_it_never_started`
does — and the docstring says so in place.
