# Video capture's unverified prerequisites, settled live

Probed against run 28's four running clients, which cost nothing because they
were already up. Everything the video spec marked "cannot determine without a
live capture" in the window-discovery chain is now determined.

## `_NET_WM_PID` is set under Xwayland — the primary path works

```
pid 3764406 -> windows: NONE          <- the headless server, correctly nothing
pid 3769396 -> windows: 10485815
pid 3769400 -> windows: 14680119
pid 3769402 -> windows: 12582967
pid 3769403 -> windows: 8388663
```

One window per graphical client, none for the headless server. `xdotool search
--pid` is sound and the spec's §11.2 risk does not exist.

## Geometry readback works

```
xwininfo -id 10485815
  Window id: 0xa00037 "Factorio: Space Age 2.1.17"
  Absolute upper-left X: 2170   Y: 942
  Width: 706   Height: 854
  Map State: IsViewable
```

Note the size: **706x854**, not 1280x720 and not even landscape. So the window
*must* be resized before capture — the observed-geometry readback is not a
belt-and-braces check, it is the only thing that would have caught this.

## A real capture succeeded

```
ffmpeg -f x11grab -window_id 10485815 -i :0 -frames:v 1 probe.png
  -> 950,304 bytes
```

x11grab against an Xwayland-hosted Factorio window works, from the dev shell.

## The name-search fallback: works, and still cannot be trusted

My first attempt reported that `xdotool search --name` found nothing. **That was
my own error** — I passed `-i` where the pattern belonged, so `-i` *was* the
pattern. With correct syntax it returns every client:

```
xdotool search --name 'Factorio'  ->  8388663 14680119 12582967 10485815
```

Which is exactly the implementation's stated concern: the fallback cannot tell
two Factorio clients apart, so on a multi-client run it finds four and refuses.
It is a real fallback for the single-client case and useless for ours. Since the
pid path is now proven, the fallback should be treated as the rare path it is,
not as reassurance.

## What is still unmeasured

Nothing here touches the two numbers that matter for the *decision* to record
video: no bitrate or file size has been measured, and neither has the encoder's
UPS cost or the screenshot cost it would be compared against. **Do not claim
video is cheaper than screenshots.** Those need a capture during a real run.
The Lua wiring now exists
(`docs/superpowers/notes/2026-09-02-video-capture-implementation.md`), so the
run is the only thing missing.
