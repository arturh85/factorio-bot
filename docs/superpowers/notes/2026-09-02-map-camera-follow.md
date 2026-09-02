# Making the run map's camera follow the bots — 2026-09-02

## Status

Done, frontend-only. Builds on `311467c8` (the SVG map) and keeps its central
property: the framing is a pure function and the numbers are asserted
directly, not eyeballed.

`pnpm lint` clean. `pnpm run test:coverage` green: **982 passed, 0 failed**
across 53 files, gate cleared at statements 97.13 / branches 93.44 /
functions 96 / lines 97.74 (thresholds 90 / 80 / 90 / 90).
`app/src/lib/mapCamera.ts` is at 100% of all four. 22 new tests in
`mapCamera.spec.ts`; `MapPanel.spec.ts` went from 29 to 38.

No Rust was built and no Factorio was launched — a live four-client run was in
progress on this machine throughout.

## The bug, precisely

`MapPanel` framed the `bounds` prop, which is `boundsAt(map, cursor)`: the
bounds recorded on the latest `keyframe` in `map.jsonl`. Those describe
**entities**. A bot that walks past the built area walks out of the picture,
and since the map is the panel that says "something is happening", the viewer
then watches a still frame while the run is at its busiest.

The naive fix — frame a tight box around everything, recomputed every cursor
tick — trades one bad picture for a worse one. At the default play rate of 300
ticks per step a bot covers roughly 45 tiles between frames, so a tight box
moves on every single step and the whole map slides under a stationary base.
Both halves had to be designed together: *what* must be visible, and *when the
camera is allowed to move*.

## The policy

`app/src/lib/mapCamera.ts`, pure, `cameraFrame(input) -> CameraFrame | null`.
Two functions with distinct jobs:

- **`contentBox`** — what the current mode is responsible for keeping on
  screen. In `fit` that is the keyframe bounds unioned with every bot position
  *and every trail point*; in `follow` it is the bots and their trails alone,
  falling back to the world when a run has no `bots` samples yet.
- **`cameraFrame`** — whether to move. It is handed the frame currently on
  screen and returns a `reason` (`initial | mode-changed | kept | escaped |
  shrank`) alongside the bounds, which is what makes the *policy* assertable
  and not just the geometry.

Constants, all exported and all in tiles:

| | | why |
|---|---|---|
| `CAMERA_MARGIN` | 8 | how close content may get to the edge before the camera moves |
| `CAMERA_LEAD` | 32 | how much room a re-frame leaves |
| `CAMERA_GRID` | 16 | frame edges snap outward to this (half a chunk) |
| `CAMERA_MIN_SPAN` | 96 | the smallest frame |
| `CAMERA_SHRINK_SLACK` | 2 | how stale a too-large frame gets before tightening |
| `CAMERA_STRAIN_SPAN` | 256 | past this, `fit` reports that it is stretched |

The frame is always **square**, because the viewport is square and
`projectionFor` letterboxes anything that is not — and then "does the content
fit inside the previous frame" would be answering about a rectangle that is
not the visible one.

### Trails are content, not decoration

A trail leading off-frame is the same failure as a bot leading off-frame: it
is the part of the picture that says where the bot came from. Including trails
also turned out to *stabilise* the camera, because a trail spans the last
1,800 ticks of travel and its extent changes gradually where a bot's position
jumps.

## How it stopped jittering: three mechanisms, and only one of them is obvious

1. **Grid snapping.** Frame edges are snapped outward to 16 tiles, so two
   content boxes inside the same cell produce *the same frame*. Sub-cell
   motion cannot move the picture at all, hysteresis or no hysteresis.
2. **A dead zone.** The frame is kept until content comes within
   `CAMERA_MARGIN` (8 tiles) of an edge. Everything inside that is free
   movement.
3. **Lead, which is the one that actually mattered.** The first draft re-framed
   to the content plus the same 8-tile margin that had just been violated —
   which puts the bot straight back on the edge it just reached, so the camera
   stepped forward again a few tiles later. Re-framing pads by `CAMERA_LEAD`
   = 32 instead, four times the trigger, so a bot must cross 24 tiles of dead
   zone before the picture moves again. **The twitch is caused by where the
   camera moves to, not by how often it is asked.** Measured, not assumed, on
   the 100-step walk of 2 tiles per step in the spec: re-framing to the
   trigger margin moves the camera **9** times, the 32-tile lead moves it
   **4**, and the bot is on screen for all 101 steps either way. (My estimate
   before measuring was "about 20" — grid snapping was already absorbing more
   of it than I credited, which is worth recording as a reminder that these
   three mechanisms overlap rather than add up.) The test asserts `toBe(4)`:
   the function is pure over a fixed walk, so the count is the policy, not a
   tolerance.

A fourth rule handles the other direction. Once a bot comes home, a frame
stretched around its journey is wasted viewport, but tightening on any slack
at all would make the camera track content down tile by tile. It only shrinks
when the frame is more than twice the span it needs. That cannot oscillate,
and the reason is structural rather than tuned: the frame it shrinks *to*
already contains the content plus the full 32-tile lead, so a grow cannot
immediately follow — there is a test for exactly that sequence.

## The control, and why there is one

`fit` and `follow`, as two buttons above the map, `fit` by default.

They answer different questions and no single frame answers both. "Where is
everything?" wants the base in shot even while a bot prospects 300 tiles away.
"What is that bot doing?" wants the bot big, and then the base is a
distraction whose only contribution is to shrink the subject. Picking one for
the viewer means being wrong half the time; a blended compromise is wrong
always. So the panel offers both, and defaults to the one that cannot hide
anything.

Keyboard reachability is the same standard the tooltips already meet: they are
plain `<button>`s in a labelled `role="group"` with `aria-pressed`, so Tab
reaches each and Enter and Space activate them with no key handler in this
component. Deliberately **not** `role="radiogroup"`: that pattern owes the user
arrow-key roving focus, and a two-item segmented control gets the same job done
without a custom focus manager to get wrong. The pressed state changes border
weight as well as colour, so it does not depend on telling two greys apart.
Icons are `Maximize` and `Crosshair` from `@lucide/vue`, both `aria-hidden`
beside real text labels.

### And a nudge instead of a silent switch

`fit` refuses to hide anything, so a bot 400 tiles out produces a 464-tile
frame — about 1 px per tile. Rather than pretend that is fine, or silently
switch modes on the viewer's behalf, the frame reports `strained` past 256
tiles and the panel says so and points at the other mode. Being told why the
picture went small is worth more than having it quietly changed.

## Alternatives rejected

- **Pure all-bot bounding box** — the sibling spec
  (`docs/superpowers/specs/2026-09-02-server-camera-design.md`, §5) already
  rejected this for the mod's camera, and it is just as wrong here: one
  wandering bot collapses everything else. `CAMERA_MIN_SPAN` fixes only the
  opposite degeneracy (bots huddled on one tile). It survives as *half* of
  `follow` mode, which is a choice the viewer makes knowingly.
- **A capped fit frame that drops distant content.** Rejected outright: the
  complaint is that bots leave the frame, and a cap is a rule for making bots
  leave the frame. `follow` is the honest way to exclude something — the
  viewer asked.
- **Smooth interpolation towards a target frame.** Would look better in
  motion and is wrong for this panel: the map is scrubbed as often as it is
  played, and an animating camera means a frame that is briefly *incorrect*
  for the tick under the cursor. Also unassertable in jsdom without faking
  timers, which would have moved the policy back out of reach of the tests.
- **Recomputing per cursor tick with an EMA / low-pass filter.** Smooths the
  motion but never stops it, and makes the frame a function of history depth
  rather than of state — two viewers scrubbing to the same tick from different
  directions would see different pictures.
- **Putting the frame in `runsStore` as a getter.** The store's getters are
  pure functions of `(records, cursor)`; the camera is deliberately stateful
  (it must know what is on screen to decline to move) and is a per-panel view
  preference besides. `RunAnalysisPage` mounts a second `MapPanel` that should
  not share a camera with the first. The store needed no change.
- **Persisting the mode in `localStorage`.** Tempting, and one line of
  `@vueuse/core`. Left out: the choice belongs to a run being watched rather
  than to the browser, and it would have put shared mutable state into every
  test that mounts the panel.

## What I could not verify

- **No browser.** Everything here is asserted through jsdom by parsing the
  group transform back into numbers (`projectionOf`, `onScreen`) and checking
  world positions land inside 0..480. That is a genuine assertion about the
  projection, but it is not the same as seeing the panel at a real width in
  `RunsPage`'s layout, and the CSS for the new buttons was never rendered by
  an engine.
- **Never watched against a live run.** The 45-tiles-per-step figure comes
  from the store's default `rate` of 300 and Factorio's character speed, not
  from watching the cursor move over a recorded run. The tuning of
  `CAMERA_LEAD` rests on the synthetic 100-step walk in the spec.
- **Focus behaviour** carries the same caveat as the tooltips: jsdom's
  `trigger('click')` and `trigger('focus')` do not exercise real focus
  management, so "Tab reaches the buttons" rests on them being native
  `<button>`s rather than on a browser having tabbed to one.
- **`follow` with bots on genuinely opposite corners** of a large map is
  framed honestly and will be small. There is no run in the archive where that
  happens, so how bad it looks is unmeasured — the `strained` hint deliberately
  says nothing in follow mode, because there is no better mode to point at.
