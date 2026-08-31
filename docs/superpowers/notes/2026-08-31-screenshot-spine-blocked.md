# Tick-cadence screenshot spine — why it is not started

Requested: one frame per 5 game-seconds at 1920x1080, cameras follow-bot-1 →
per-bot → auto-frame, flat `tick-NNNNNNN-<camera>.png`, replayed in the web UI
in sync with plan/task state, for screencapture.

**Not started deliberately.** Its first half cannot be validated in this
environment, and building the second half against frames nobody can produce is
the kind of unvalidated artifact this project has spent a long time removing.

## The blocker, measured rather than assumed

**`game.take_screenshot` does nothing headless.** From the vendor's own docs
(`workspace/factorio-api-docs/runtime-api.json`, `LuaGameScript.take_screenshot`):

> Take a screenshot of the game and save it to the `script-output` folder...
> **If Factorio is running headless, this function will do nothing.**

Corroborated on disk: `workspace/client1/script-output/` exists with 420 PNGs
from an earlier graphical run; **`workspace/server/script-output` does not
exist at all.** The headless server has never written one. Docs and disk agree.

So capture needs a **graphical client**. In this environment:

    DISPLAY                     unset
    WAYLAND_DISPLAY             unset
    Xvfb                        not available
    workspace/client*/          no extracted Factorio binary
    /run/opengl-driver/lib      exists (the documented LD_LIBRARY_PATH fix)

Even with the `LD_LIBRARY_PATH` fix now in CLAUDE.md, there is no display, no
virtual framebuffer, and no extracted client — and extraction is 8-10 minutes
per instance, followed by ~26s of sprite loading and a connect wait.

## What IS already known, so nobody re-derives it

See `.superpowers/sdd/screenshot-spine-prep.md` for the full survey. The load-
bearing findings:

- **The capture primitive already exists.** `mods/BotBridge/control.lua:1901`
  `rcon_screenshot(args)` forwards arbitrary args to `game.take_screenshot`,
  registered in the RCON dispatch table. All three camera modes are just
  different `position`/`zoom`/`player` values. No new capture code needed.
- **`by_player` is the per-bot camera mechanism** — "the screenshot will only
  be taken for this player".
- **Frames drop by default.** `force_render` defaults to `false`, and the game
  skips rendering when it falls behind. Set it `true` — but it **is not
  honoured on multiplayer clients catching up to the server**, which our bots
  are. So **the manifest must record which ticks actually produced a file**,
  never compute them from a start tick and a stride. A computed manifest would
  claim frames that do not exist.
- **PNG at 1920x1080 is not viable.** Measured from this project's own
  512x512 captures: 420 files, 270 MB, avg 643 KB. Scaled by area that is
  ~5 MB/frame, so a 60-minute run with 3 cameras is ~11 GB. Use `.jpg` with
  `quality` (natively supported); the destination is a video.
- **Path resolution is confirmed**: `<workspace>/client<N>/script-output/<path>`,
  and `path` creates intermediate directories.
- **Retention has no owner.** 270 MB accumulated with nobody asking. The
  existing tile capture has no cleanup path — `instance_setup.rs` has a
  commented-out delete block. Whatever the spine writes needs a retention rule
  from the start.

## What would unblock it
A working graphical client on the host: a real display, or Xvfb plus
`/run/opengl-driver/lib` ahead of the dev shell's `LD_LIBRARY_PATH`, plus an
extracted client instance. Then the capture half is testable and the serving
half is worth building.
