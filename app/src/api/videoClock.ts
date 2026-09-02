/**
 * Converting between `game.tick` and a position in the recording.
 *
 * **This is the only file in the frontend permitted to contain seconds.**
 * `frameJoin.ts`'s header says "If a change here ever introduces seconds, that
 * is a bug, not a feature" — that stays true; seconds are quarantined here
 * instead, the same discipline `observedOrigin()` already enforces for the
 * shifted-tick axis.
 *
 * The four rules below are in priority order and the second one is the reason
 * this module exists:
 *
 * 1. **Before the first sample or after the last: `null`.** Not clamped to 0 or
 *    to the duration. A tick outside the recording's span has no position in it,
 *    and clamping would park the scrubber on a frame showing a different moment.
 * 2. **Across a gap, or across more than {@link MAX_INTERP_GAP_MS}: `null`.**
 *    Not an interpolated guess. This is the single most important rule in the
 *    design. A missing JPEG announces itself; a stalled game does not — the
 *    recorder keeps writing frames of the last drawn image, so the video at that
 *    timestamp shows a frozen picture that looks exactly like a legitimate
 *    frame. Only the clock can tell those apart, and only if it refuses.
 * 3. **Otherwise** linear interpolation between the two nearest samples, offset
 *    by `calibration[0]`.
 * 4. **If the rate check did not pass, nothing is `certain`.** The offset may be
 *    right at the start and wrong by the end, and nothing in the clock can say
 *    where the error accumulated. `rate_ok: null` counts as not passing:
 *    unknown is not verified.
 */

import {Calibration, VideoManifest, VideoTicksResponse} from './types';
import {Ticks} from './replay';

/**
 * The widest span the clock will interpolate across, in host milliseconds.
 *
 * Two seconds is roughly 120 ticks at 60 UPS — far wider than the 500 ms floor
 * cadence, so an ordinary sampled stretch never trips it, and narrow enough that
 * an autosave or a peer catching up does.
 */
export const MAX_INTERP_GAP_MS = 2000;

/** One usable `(tick, wall_ms)` observation. */
interface ClockPoint {
    tick: number;
    wallMs: number;
}

/**
 * The clock, prepared for lookup.
 *
 * `bridges[i]` says whether the span between `points[i]` and `points[i + 1]` may
 * be interpolated. It is computed once, when the clock is parsed, so that every
 * lookup asks the same question of the same data rather than re-deriving it.
 */
export interface VideoClock {
    points: ClockPoint[];
    bridges: boolean[];
    calibration: Calibration[];
    /** `true` only when the recorder checked the rate and it held. */
    rateVerified: boolean;
}

export interface VideoPosition {
    seconds: number;
    /**
     * `false` when the recording's own rate was never verified. The position is
     * the best available, and it is not a measurement anybody stands behind.
     */
    certain: boolean;
}

export interface TickPosition {
    tick: Ticks;
    certain: boolean;
}

/**
 * Builds a clock from what the two endpoints answered.
 *
 * `null` when there is nothing to build one from: no recording, no calibration,
 * or fewer than two observations. A clock with one point can place exactly one
 * tick and interpolate nothing, which is indistinguishable in practice from no
 * clock — and pretending otherwise would offer a scrubber that answers `null`
 * everywhere for a reason nobody could see.
 */
export function parseVideoClock(
    manifest: VideoManifest | null,
    ticks: VideoTicksResponse | null
): VideoClock | null {
    const record = manifest?.video ?? null;
    if (record === null || ticks === null) return null;
    if (record.calibration.length === 0) return null;

    const points: ClockPoint[] = [];
    // Gaps are read from the *file order* of the samples, not from their
    // timestamps: a gap line says "the recorder looked here and the game did not
    // answer", which is a fact about the position in the stream.
    const gapBefore: boolean[] = [];
    let sawGapSinceLastPoint = false;
    for (const sample of ticks.samples) {
        if (sample.k === 'gap' || sample.t === undefined || sample.t === null) {
            sawGapSinceLastPoint = true;
            continue;
        }
        points.push({tick: sample.t, wallMs: sample.w});
        gapBefore.push(sawGapSinceLastPoint);
        sawGapSinceLastPoint = false;
    }
    if (points.length < 2) return null;

    const bridges: boolean[] = [];
    for (let i = 0; i + 1 < points.length; i += 1) {
        const from = points[i];
        const to = points[i + 1];
        bridges.push(
            !gapBefore[i + 1] &&
            to.wallMs - from.wallMs <= MAX_INTERP_GAP_MS &&
            // A clock that goes backwards is a clock nobody should read across.
            // It cannot happen — `game.tick` only rises — which is exactly why
            // seeing it means something else is wrong.
            to.tick >= from.tick &&
            to.wallMs >= from.wallMs
        );
    }

    return {
        points,
        bridges,
        calibration: record.calibration,
        rateVerified: record.rate_ok === true
    };
}

/** Host milliseconds to a position in the recording, via `calibration[0]`. */
function wallMsToSeconds(clock: VideoClock, wallMs: number): number | null {
    const zero = clock.calibration[0];
    const outMs = wallMs - zero.host_wall_ms + zero.out_time_ms;
    // Before the encoder's first frame. The recording has no picture of this
    // moment, and second 0 is a picture of a different one.
    if (outMs < 0) return null;
    return outMs / 1000;
}

/** A position in the recording back to host milliseconds. */
function secondsToWallMs(clock: VideoClock, seconds: number): number {
    const zero = clock.calibration[0];
    return seconds * 1000 - zero.out_time_ms + zero.host_wall_ms;
}

/**
 * Finds the index `i` such that `key(points[i]) <= value <= key(points[i+1])`,
 * or `null` when `value` falls outside the whole span.
 */
function bracket(
    points: ClockPoint[],
    value: number,
    key: (point: ClockPoint) => number
): number | null {
    if (points.length < 2) return null;
    if (value < key(points[0]) || value > key(points[points.length - 1])) return null;
    for (let i = 0; i + 1 < points.length; i += 1) {
        if (value >= key(points[i]) && value <= key(points[i + 1])) return i;
    }
    return null;
}

/**
 * Where in the recording `tick` is, or `null` when the recording cannot say.
 *
 * `tick` is absolute `game.tick` — the same clock the frame filenames are
 * written in. A caller working on the shifted replay axis must add
 * `observedOrigin()` first, in the one place that conversion already happens.
 */
export function tickToVideoSeconds(clock: VideoClock, tick: Ticks): VideoPosition | null {
    const index = bracket(clock.points, tick, (point) => point.tick);
    if (index === null) return null;

    const from = clock.points[index];
    const to = clock.points[index + 1];

    // An exact hit needs no interpolation, so it is answered even when the span
    // beside it is not bridgeable: this sample *is* an observation of this tick.
    const exact = from.tick === tick ? from : to.tick === tick ? to : null;
    if (exact !== null) return position(clock, exact.wallMs);

    if (!clock.bridges[index]) return null;
    // A span whose two ends share a tick is a stopped game: every moment in it
    // is that tick, and the earliest is the honest one to name.
    if (to.tick === from.tick) return position(clock, from.wallMs);

    const fraction = (tick - from.tick) / (to.tick - from.tick);
    return position(clock, from.wallMs + fraction * (to.wallMs - from.wallMs));
}

function position(clock: VideoClock, wallMs: number): VideoPosition | null {
    const seconds = wallMsToSeconds(clock, wallMs);
    return seconds === null ? null : {seconds, certain: clock.rateVerified};
}

/**
 * The reverse direction — the user scrubs the video, the timeline follows.
 *
 * Obeys the same three rules. A position inside a stall answers `null` rather
 * than the tick either side of it: the picture at that second is real, and it is
 * not a picture of any tick this clock observed.
 */
export function videoSecondsToTick(clock: VideoClock, seconds: number): TickPosition | null {
    const wallMs = secondsToWallMs(clock, seconds);
    const index = bracket(clock.points, wallMs, (point) => point.wallMs);
    if (index === null) return null;

    const from = clock.points[index];
    const to = clock.points[index + 1];
    if (from.wallMs === wallMs) return {tick: from.tick, certain: clock.rateVerified};
    if (to.wallMs === wallMs) return {tick: to.tick, certain: clock.rateVerified};
    if (!clock.bridges[index]) return null;
    if (to.wallMs === from.wallMs) return {tick: from.tick, certain: clock.rateVerified};

    const fraction = (wallMs - from.wallMs) / (to.wallMs - from.wallMs);
    return {
        tick: Math.round(from.tick + fraction * (to.tick - from.tick)),
        certain: clock.rateVerified
    };
}

/** The span of ticks the recording can place at all. `null` for an empty clock. */
export function clockTickRange(clock: VideoClock): {from: number; to: number} | null {
    if (clock.points.length === 0) return null;
    return {
        from: clock.points[0].tick,
        to: clock.points[clock.points.length - 1].tick
    };
}
