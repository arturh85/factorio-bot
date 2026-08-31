/**
 * Fixtures for the frame/replay join -- see `frameJoin.ts` and
 * `ReplayScrubber.vue`.
 *
 * Unlike `replay.fixtures.ts`, there is no tracked, Rust-generated snapshot
 * of a `FramesManifest` to import: `GET /api/v1/frames` is always a live
 * directory listing (`crates/server/src/manage/frames.rs`), never a pinned
 * document. So these are hand-written object literals -- but typed directly
 * as `FramesManifest`/`FrameEntry`, never built from a JSON string and cast.
 * A cast (`as FramesManifest`) would compile without checking a single field;
 * a direct literal is checked by `tsc` against the interface at every field,
 * which is the same protection `parseReplay` buys for the JSON-imported
 * replay fixtures, achieved here by simply not going through JSON at all.
 *
 * Ticks below are deliberately chosen to align with `REALISTIC_REPLAY`
 * (`replay.fixtures.ts`), whose observed ticks span 100-400: see that
 * fixture's `crates/executor/tests/replay.snapshot.json` for the source.
 */

import {FramesManifest} from './types';

/** Overlaps `REALISTIC_REPLAY`'s observed range (100-400): a plausible match. */
export const OVERLAPPING_MANIFEST: FramesManifest = {
    clients: [1],
    // No sidecar anywhere in these fixtures, so every client's run is
    // unknown -- which is what `run: null` above says at the manifest level.
    client_runs: [{client: 1, run: null}],
    run: null,
    frames: [
        {client: 1, tick: 0, camera: 'front', name: 'tick-0000000000-front.jpg', bytes: 111},
        {client: 1, tick: 300, camera: 'front', name: 'tick-0000000300-front.jpg', bytes: 222},
        // A dropped capture between 300 and 1200 (the mod's normal cadence is
        // 300 ticks): a real, visible gap, not a rendering artefact.
        {client: 1, tick: 1200, camera: 'front', name: 'tick-0000001200-front.jpg', bytes: 333}
    ]
};

/**
 * Entirely outside `REALISTIC_REPLAY`'s observed range (100-400): job 7's
 * frames next to job 3's plan, in the brief's own words. Ranges: replay
 * 100-400, manifest 10000-10600 -- disjoint.
 */
export const UNRELATED_RUN_MANIFEST: FramesManifest = {
    clients: [1],
    // No sidecar anywhere in these fixtures, so every client's run is
    // unknown -- which is what `run: null` above says at the manifest level.
    client_runs: [{client: 1, run: null}],
    run: null,
    frames: [
        {client: 1, tick: 10000, camera: 'front', name: 'tick-0000010000-front.jpg', bytes: 111},
        {client: 1, tick: 10300, camera: 'front', name: 'tick-0000010300-front.jpg', bytes: 222},
        {client: 1, tick: 10600, camera: 'front', name: 'tick-0000010600-front.jpg', bytes: 333}
    ]
};

/** One entry whose filename did not parse, alongside two that did. */
export const MANIFEST_WITH_UNPARSED_ENTRY: FramesManifest = {
    clients: [1],
    // No sidecar anywhere in these fixtures, so every client's run is
    // unknown -- which is what `run: null` above says at the manifest level.
    client_runs: [{client: 1, run: null}],
    run: null,
    frames: [
        {client: 1, tick: 200, camera: 'front', name: 'tick-0000000200-front.jpg', bytes: 111},
        {client: 1, tick: null, camera: null, name: 'not-a-frame-name.jpg', bytes: 999},
        {client: 1, tick: 300, camera: 'front', name: 'tick-0000000300-front.jpg', bytes: 222}
    ]
};

/** No capture has ever run: `clients` itself is empty. */
export const EMPTY_MANIFEST: FramesManifest = {clients: [],
// No sidecar anywhere in these fixtures, so every client's run is
// unknown -- which is what `run: null` above says at the manifest level.
client_runs: [],
    run: null, frames: []};

/**
 * Overlaps `REALISTIC_REPLAY`'s observed range (100-400), but its first
 * capture is at tick 200 -- so scrubbing to an earlier tick than that has
 * genuinely no frame yet, rather than every position in the whole fixture
 * happening to have one.
 */
export const OVERLAPPING_MANIFEST_LATE_START: FramesManifest = {
    clients: [1],
    // No sidecar anywhere in these fixtures, so every client's run is
    // unknown -- which is what `run: null` above says at the manifest level.
    client_runs: [{client: 1, run: null}],
    run: null,
    frames: [
        {client: 1, tick: 200, camera: 'front', name: 'tick-0000000200-front.jpg', bytes: 111},
        {client: 1, tick: 350, camera: 'front', name: 'tick-0000000350-front.jpg', bytes: 222}
    ]
};

/**
 * Several cameras for one client, using the ids the capture actually emits:
 * `follow`, `bot-<player_index>` and `area`.
 *
 * `bot-1` is hyphenated on purpose. A camera id containing a hyphen is the
 * case the filename parser splits on the *first* hyphen after the digits for —
 * a last-hyphen split would read `tick-0000300-bot-1.jpg` as camera `1`, which
 * does not error and is not obviously wrong on inspection.
 *
 * `area` deliberately has a frame at a tick the others do not: cameras are
 * independent captures and a per-camera gap is a real thing the manifest must
 * be able to express.
 *
 * Ticks are **absolute `game.tick`**, matching `REALISTIC_REPLAY`, whose
 * observed ticks start at 100 — so these land at 0 and 300 on the scrubber's
 * shifted axis. A fixture at 0 and 300 absolute would quietly model frames and
 * plan as sharing an origin, which a real run disproves: planned 0-868 against
 * observed 60551-61528.
 */
export const MULTI_CAMERA_MANIFEST: FramesManifest = {
    clients: [1],
    // No sidecar anywhere in these fixtures, so every client's run is
    // unknown -- which is what `run: null` above says at the manifest level.
    client_runs: [{client: 1, run: null}],
    run: null,
    frames: [
        {client: 1, tick: 100, camera: 'follow', name: 'tick-0000000100-follow.jpg', bytes: 111},
        {client: 1, tick: 100, camera: 'bot-1', name: 'tick-0000000100-bot-1.jpg', bytes: 222},
        {client: 1, tick: 100, camera: 'area', name: 'tick-0000000100-area.jpg', bytes: 333},
        {client: 1, tick: 400, camera: 'follow', name: 'tick-0000000400-follow.jpg', bytes: 444},
        {client: 1, tick: 400, camera: 'bot-1', name: 'tick-0000000400-bot-1.jpg', bytes: 555}
    ]
};
