/**
 * Fixtures for the video clock and the video/run join.
 *
 * Hand-written object literals typed directly as `VideoManifest` /
 * `VideoTicksResponse`, never JSON cast into shape, for the reason
 * `frames.fixtures.ts` gives: a cast compiles without checking a single field,
 * a literal is checked by `tsc` at every one.
 *
 * Ticks align with `REALISTIC_REPLAY` (`replay.fixtures.ts`), whose observed
 * ticks span 100-400, so a fixture that is meant to join actually does.
 */

import {TickSample, VideoManifest, VideoRecord, VideoTicksResponse} from './types';

/** A recording that stopped cleanly and whose rate checked out. */
export const CLEAN_RECORD: VideoRecord = {
    run: 'run-1',
    file: 'video.mp4',
    width: 1280,
    height: 720,
    requested_width: 1280,
    requested_height: 720,
    fps: 15,
    status: 'stopped',
    reason: null,
    ffmpeg_exit: 0,
    // The encoder's first frame landed 500 ms after the recorder's epoch, so
    // host 500 ms is video second 0.
    calibration: [
        {host_wall_ms: 500, out_time_ms: 0},
        {host_wall_ms: 5500, out_time_ms: 5000}
    ],
    rate_ok: true,
    window_id: '0x2c00007'
};

/** Six observations at the 2 Hz floor, 30 ticks apart: a steady 60 UPS. */
export const CLEAN_TICKS: VideoTicksResponse = {
    samples: [
        {t: 100, w: 500, k: 'start'},
        {t: 130, w: 1000, k: 'sample'},
        {t: 160, w: 1500, k: 'sample'},
        {t: 190, w: 2000, k: 'sample'},
        {t: 220, w: 2500, k: 'sample'},
        {t: 250, w: 3000, k: 'stop'}
    ],
    skipped: 0
};

export const CLEAN_MANIFEST: VideoManifest = {
    run: 'run-1',
    video: CLEAN_RECORD,
    bytes: 4096,
    samples: CLEAN_TICKS.samples.length,
    skipped: 0,
    tick_range: {from: 100, to: 250}
};

/**
 * The same recording with a stall in the middle: the recorder looked at
 * w=1500 and the game did not answer, then picked up again 4 s later.
 */
export const STALLED_TICKS: VideoTicksResponse = {
    samples: [
        {t: 100, w: 500, k: 'start'},
        {t: 130, w: 1000, k: 'sample'},
        {w: 1500, k: 'gap', reason: 'the game did not answer a tick query'},
        {t: 160, w: 5500, k: 'sample'},
        {t: 190, w: 6000, k: 'stop'}
    ],
    skipped: 0
};

export const STALLED_MANIFEST: VideoManifest = {
    ...CLEAN_MANIFEST,
    samples: STALLED_TICKS.samples.length,
    tick_range: {from: 100, to: 190}
};

/** A recording whose two calibration pairs disagree about the rate. */
export const SKEWED_MANIFEST: VideoManifest = {
    ...CLEAN_MANIFEST,
    video: {
        ...CLEAN_RECORD,
        calibration: [
            {host_wall_ms: 500, out_time_ms: 0},
            {host_wall_ms: 5500, out_time_ms: 4200}
        ],
        rate_ok: false
    }
};

/** A recorder that outlived its run: still `recording`, never calibrated twice. */
export const UNSTOPPED_MANIFEST: VideoManifest = {
    ...CLEAN_MANIFEST,
    video: {
        ...CLEAN_RECORD,
        status: 'recording',
        ffmpeg_exit: null,
        calibration: [{host_wall_ms: 500, out_time_ms: 0}],
        rate_ok: null
    }
};

/** A run that never asked for video. The default, and much the commonest. */
export const NO_VIDEO_MANIFEST: VideoManifest = {
    run: null,
    video: null,
    bytes: null,
    samples: 0,
    skipped: 0,
    tick_range: null
};

export const NO_TICKS: VideoTicksResponse = {samples: [], skipped: 0};

/** Builds a clock from `(tick, wall_ms)` pairs, for the arithmetic tests. */
export function ticksOf(pairs: ReadonlyArray<readonly [number, number]>): VideoTicksResponse {
    const samples: TickSample[] = pairs.map(([t, w]) => ({t, w, k: 'sample'}));
    return {samples, skipped: 0};
}
