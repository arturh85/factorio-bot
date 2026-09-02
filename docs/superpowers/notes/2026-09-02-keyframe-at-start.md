# Keyframe at run start

## Status

Done. Compiles, formats, clippy-clean (`--all-features --all-targets --deny
warnings`), and `cargo test --workspace` passes (see test summary below).

## Commit

`d812e3d8` — `fix(record): write a keyframe at run start, not just at
milestone boundaries`, on the branch that was checked out at commit time
(`master`; the session's initial gitStatus snapshot said `feat/axum-server`
but the working tree had since moved to `master` before I touched it — I did
not switch branches myself).

Touches only:
- `crates/core/src/record/map.rs`
- `crates/scripting_lua/src/globals/record.rs`

No mod files were touched, so nothing needed re-copying into
`workspace/mods/` and `luac -p` was not required.

## Where the start bounds come from, and why

`record.start()` now calls `rcon.connected_players()` (the same RCON call
`rcon.players()` exposes to Lua and that `wait_for_roster` in
`research_run.lua` already polls) and takes the bounding box of every
connected bot's position, widened by 16 tiles — the same margin
`record.keyframe()` already uses for its placement-bounds keyframes, so the
two are directly comparable rather than arbitrarily different sizes.

Bot position was the deliberate choice over the alternatives:

- **`samples.jsonl`** also carries bot positions, but it's written by the mod
  into the *workspace's* `script-output` and only copied into the run
  directory at `record.finish()` (`ingest_samples`) — it is not something
  `record.start()` can read synchronously as an authoritative "this run's
  bots" source at the moment recording begins, and doing so would mean
  reading a file the mod owns rather than asking the game directly.
- **The whole map, or a fixed area around spawn** would not track where the
  run is actually about to happen (a resumed save might spawn bots far from
  the original spawn), and a keyframe over the wrong region is worse than no
  keyframe: it looks like coverage instead of admitting there isn't any.
- **Bot positions** are the one thing guaranteed to exist the instant
  `record.start()` runs: every connected bot has a character with a real
  position, and `record.start()` is only ever called from a script that has
  already confirmed a roster exists (see `research_run.lua`'s
  `wait_for_roster`). An empty roster (a script that didn't wait) writes no
  keyframe, mirroring `record.keyframe()`'s own "nothing to draw yet"
  no-op — not an error.

Implementation: added `map::bounds_around(positions, margin)` (pure, unit
tested: empty → `None`, one position → a square of the margin, several
positions → covers all of them plus the margin) as the run-start counterpart
to `RunRecorder::placed_bounds`. The game/model snapshot-and-diverge logic
that used to live inline in `record.keyframe()`'s Lua closure was pulled out
into `keyframe_snapshot()` and is now called from both `record.keyframe()`
and the new code in `record.start()`, so the two keyframe producers cannot
drift apart on what counts as "relevant" (`keyframe_relevant`) or how a
divergence is computed.

A genuine RCON/model failure at this point is left to raise rather than
being swallowed to `false`: by the time this code runs, `frame_capture_start`
(the line right above it) has already proven the game is reachable, so a
failure past that point is a real defect worth surfacing, not an
unremarkable timing gap the way "nothing placed yet" is for
`record.keyframe()`.

## What one keyframe costs, and whether it delays anything

One keyframe = one `connected_players` RCON round trip (already paid
elsewhere in every script that waits for a roster) + one
`find_entities_filtered` over the bounding box (the same call
`record.keyframe()` already makes at every milestone boundary) + one
in-memory `EntityGraph::snapshot_within` query (two quad-tree lookups, no
I/O) + one `divergence_between` set comparison + one synchronous JSON-line
write+flush to `map.jsonl`.

At run start this box is normally *smaller* than a later milestone-boundary
keyframe's box: it is centred on wherever the bots spawned rather than on
however far their placements have since spread, so the population inside it
(and thus the divergence computation and the RCON reply size) is typically
lower, not higher, than the existing per-milestone keyframes this file
already writes without complaint.

This happens once, synchronously, inside `record.start()`, before the driving
script does anything else — it adds one RCON round trip's worth of latency
(the same order of magnitude as the `frame_capture_start` call two lines
above it, i.e. well under the per-tick or per-action timescales the rest of
the run cares about) to a call that already makes one RCON round trip. It
does not run per-tick and does not touch the executor's hot path.

## The harder half: can/should the divergence list learn about terrain now?

Conclusion: **no, not without a separate lane, and I did not build one.**

`EntityGraph::blocking_boxes_within` (the tree `crates/planner/src/state.rs`
now consults for placement legality) is fed by `blocked_tree`, which does
hold trees, cliffs and water — but it stores them as bare `Rect`s tagged only
with an `is_minable` bool. There is no name, no entity type, nothing to build
an `EntitySnapshot` from. That's not an oversight; `blocked_tree` exists to
answer one question ("is this rectangle occupied at all") as cheaply and
generically as possible, and erasing identity is how it stays cheap.

The keyframe's `Divergence` mechanism (`divergence_between`) is built on
`EntitySnapshot` equality — name, position, direction — matched between the
`game` list (named, individual entities from `find_entities_filtered`) and
the `model` list (also named individual entities, from `snapshot_within`).
Feeding `blocking_boxes_within`'s nameless boxes into that comparison
would not produce meaningful per-tree divergences; it would produce a
*guaranteed* one for every tree, because a box with no name (or a
fabricated placeholder name) can never equality-match a real
`iron-ore`/`tree-01`/etc. `EntitySnapshot` the game side reports. That is
strictly worse than the status quo, not better: it wouldn't surface real
terrain *disagreements* (missing trees, a tree the model doesn't know
about) — it would surface every tree, every time, as a permanent "only in
model" or "only in game" entry, indistinguishable from an actual bug.

That's on top of the volume problem already on record: ~10,510 tree records
against 2,681 resource records per run. Even with an identity story that
worked, admitting trees at that ratio would make the list ~79% trees — the
exact "nobody will read it" outcome the task says to avoid. So the answer is
"not without a separate terrain lane" on two independent grounds (no
matchable identity, and volume), not just one.

**What a real terrain lane would need**, if built later:

1. **A different comparison, not name-equality.** Terrain has stable
   identity in the game (`find_entities_filtered` returns real names and
   positions for trees) but the model's cheap source (`blocked_tree`) does
   not carry it. Either give `blocked_tree` (or a sibling structure) real
   per-entity identity for the terrain classes worth tracking, or compare at
   a coarser grain than per-entity — e.g. "count of blocking boxes in this
   region" vs. "count of terrain entities the game reports in this region",
   flagging a *mismatch in count or footprint*, not a per-entity list.
2. **A separate field on `MapKind::Keyframe`**, not an addition to
   `divergence`/`game`/`model` — so an old reader that doesn't know about it
   keeps working (`#[serde(default)]`) and a new reader can choose to ignore
   it without wading through thousands of tree lines to find the handful of
   built-entity divergences that currently matter.
3. **A volume-bounded shape** — a count, a hash, or a small list of
   *disagreements* only (boxes present on one side and not the other),
   never a full per-tree dump — so the 79%-trees outcome cannot recur even
   by accident.

None of that is built here. `keyframe_relevant` is unchanged, and
`blocking_boxes_within` is not wired into the divergence list.

## Test summary

`cargo fmt --all -- --check` clean after `cargo fmt --all`;
`cargo clippy --workspace --all-features --all-targets -- --deny warnings`
clean; `cargo test --workspace --quiet` — all suites pass (200 in
`factorio-bot-core`'s lib tests including 3 new `bounds_around` cases, 32 in
`factorio-bot-scripting-lua`, and every other crate's suite; 0 failures, a
handful of pre-existing `ignored`). No test exercises `record.start()`'s new
RCON-backed keyframe write end-to-end, matching the existing precedent that
`record.keyframe()` itself was never unit-tested for the same reason (both
need a live game or a mocked RCON reply for `find_entities_filtered`, which
the harness doesn't have); the new logic's only pure, testable surface
(`map::bounds_around`) has its own tests, and `keyframe_snapshot` is a
straight, behavior-preserving extraction of code `record.keyframe()` already
ran unexercised at this level.
