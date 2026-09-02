import {describe, expect, it} from 'vitest';
import {
    MAX_INTERP_GAP_MS,
    clockTickRange,
    parseVideoClock,
    tickToVideoSeconds,
    videoSecondsToTick
} from './videoClock';
import {
    CLEAN_MANIFEST,
    CLEAN_TICKS,
    NO_TICKS,
    NO_VIDEO_MANIFEST,
    SKEWED_MANIFEST,
    STALLED_MANIFEST,
    STALLED_TICKS,
    UNSTOPPED_MANIFEST,
    ticksOf
} from './video.fixtures';

const clean = parseVideoClock(CLEAN_MANIFEST, CLEAN_TICKS)!;

describe('parseVideoClock', () => {
    it('has nothing to build from when the run recorded no video', () => {
        expect(parseVideoClock(NO_VIDEO_MANIFEST, NO_TICKS)).toBeNull();
        expect(parseVideoClock(null, CLEAN_TICKS)).toBeNull();
        expect(parseVideoClock(CLEAN_MANIFEST, null)).toBeNull();
    });

    it('refuses a clock with fewer than two observations', () => {
        // One point can place exactly one tick and interpolate nothing, which
        // is indistinguishable in practice from no clock at all.
        expect(parseVideoClock(CLEAN_MANIFEST, ticksOf([[100, 500]]))).toBeNull();
    });

    it('drops gap lines from the points but remembers where they were', () => {
        const stalled = parseVideoClock(STALLED_MANIFEST, STALLED_TICKS)!;
        expect(stalled.points).toHaveLength(4);
        // Only the span the gap sits inside is unbridgeable.
        expect(stalled.bridges).toEqual([true, false, true]);
    });

    it('reports the span it can place', () => {
        expect(clockTickRange(clean)).toEqual({from: 100, to: 250});
    });
});

describe('rule 1: outside the sampled range is null, never clamped', () => {
    it('answers null before the first sample', () => {
        expect(tickToVideoSeconds(clean, 99)).toBeNull();
    });

    it('answers null after the last sample', () => {
        expect(tickToVideoSeconds(clean, 251)).toBeNull();
    });

    it('does not clamp to second zero, which shows a different moment', () => {
        // The failure this rule exists to prevent: a scrubber parked on frame 0
        // for every tick before the recording started, looking like a picture of
        // that tick.
        expect(tickToVideoSeconds(clean, 0)).toBeNull();
    });
});

describe('rule 2: never interpolate across a stall', () => {
    const stalled = parseVideoClock(STALLED_MANIFEST, STALLED_TICKS)!;

    it('answers null for a tick bracketed by a gap line', () => {
        // 145 sits between the observation at 130 and the one at 160, with the
        // gap line in between. The video at the interpolated timestamp shows a
        // frozen image that looks exactly like a legitimate frame.
        expect(tickToVideoSeconds(stalled, 145)).toBeNull();
    });

    it('still answers on either side of the stall', () => {
        expect(tickToVideoSeconds(stalled, 115)).not.toBeNull();
        expect(tickToVideoSeconds(stalled, 175)).not.toBeNull();
    });

    it('answers an exactly-observed tick even beside a stall', () => {
        // 160 is not a guess: a sample says the game was at tick 160 then.
        expect(tickToVideoSeconds(stalled, 160)?.seconds).toBeCloseTo(5.0, 6);
    });

    it('refuses a span wider than MAX_INTERP_GAP_MS even with no gap line', () => {
        // No `gap` line here at all -- just two ordinary samples far apart,
        // which is what a starved sampler leaves behind.
        const wide = parseVideoClock(
            CLEAN_MANIFEST,
            ticksOf([
                [100, 500],
                [400, 500 + MAX_INTERP_GAP_MS + 1]
            ])
        )!;
        expect(wide.bridges).toEqual([false]);
        expect(tickToVideoSeconds(wide, 250)).toBeNull();
        // The endpoints themselves are still observations, so they still answer.
        expect(tickToVideoSeconds(wide, 100)).not.toBeNull();
    });
});

describe('rule 3: linear interpolation, offset by calibration[0]', () => {
    it('places an exactly-sampled tick at the measured offset', () => {
        // Host 500 ms is the encoder's first frame, so tick 100 is second 0.
        expect(tickToVideoSeconds(clean, 100)).toEqual({seconds: 0, certain: true});
        // Host 3000 ms, 2500 ms after the first frame.
        expect(tickToVideoSeconds(clean, 250)).toEqual({seconds: 2.5, certain: true});
    });

    it('interpolates between two samples', () => {
        // Halfway from tick 100 (w=500) to tick 130 (w=1000).
        expect(tickToVideoSeconds(clean, 115)?.seconds).toBeCloseTo(0.25, 6);
    });

    it('never returns a negative position', () => {
        // A recording whose encoder started after the clock did: the ticks
        // before its first frame have no picture, and second 0 is not one.
        const late = parseVideoClock(
            {
                ...CLEAN_MANIFEST,
                video: {...CLEAN_MANIFEST.video!, calibration: [{host_wall_ms: 2000, out_time_ms: 0}]}
            },
            CLEAN_TICKS
        )!;
        expect(tickToVideoSeconds(late, 100)).toBeNull();
        expect(tickToVideoSeconds(late, 250)?.seconds).toBeCloseTo(1.0, 6);
    });

    it('names the earliest moment of a stopped game rather than dividing by zero', () => {
        const paused = parseVideoClock(
            CLEAN_MANIFEST,
            ticksOf([
                [100, 500],
                [100, 1000],
                [130, 1500]
            ])
        )!;
        expect(paused.points).toHaveLength(3);
        expect(tickToVideoSeconds(paused, 100)?.seconds).toBeCloseTo(0, 6);
    });
});

describe('rule 4: an unverified rate makes nothing certain', () => {
    it('marks every answer uncertain when the rate check failed', () => {
        const skewed = parseVideoClock(SKEWED_MANIFEST, CLEAN_TICKS)!;
        expect(skewed.rateVerified).toBe(false);
        expect(tickToVideoSeconds(skewed, 115)?.certain).toBe(false);
        expect(tickToVideoSeconds(skewed, 100)?.certain).toBe(false);
    });

    it('treats an unknown rate as unverified, not as fine', () => {
        // A recorder that outlived its run never took a second calibration
        // pair, so its rate was never checked. "We did not look" is not "it is
        // correct".
        const unstopped = parseVideoClock(UNSTOPPED_MANIFEST, CLEAN_TICKS)!;
        expect(unstopped.rateVerified).toBe(false);
        expect(tickToVideoSeconds(unstopped, 115)?.certain).toBe(false);
    });
});

describe('videoSecondsToTick', () => {
    it('inverts an exact sample', () => {
        expect(videoSecondsToTick(clean, 0)).toEqual({tick: 100, certain: true});
        expect(videoSecondsToTick(clean, 2.5)).toEqual({tick: 250, certain: true});
    });

    it('inverts an interpolated position', () => {
        expect(videoSecondsToTick(clean, 0.25)).toEqual({tick: 115, certain: true});
    });

    it('answers null outside the recording', () => {
        expect(videoSecondsToTick(clean, -1)).toBeNull();
        expect(videoSecondsToTick(clean, 99)).toBeNull();
    });

    it('answers null inside a stall', () => {
        // The picture at this second is real. It is not a picture of any tick
        // this clock observed, and naming one would be fabricating continuity
        // in the other direction.
        const stalled = parseVideoClock(STALLED_MANIFEST, STALLED_TICKS)!;
        expect(videoSecondsToTick(stalled, 2.0)).toBeNull();
    });

    it('round-trips a tick through seconds and back', () => {
        for (const tick of [100, 115, 130, 200, 250]) {
            const seconds = tickToVideoSeconds(clean, tick)!.seconds;
            expect(videoSecondsToTick(clean, seconds)!.tick).toBe(tick);
        }
    });
});

/**
 * The measurement behind the 2 Hz floor.
 *
 * Between two samples the viewer interpolates linearly. If the instantaneous
 * rate stays within `[u_min, u_max]` the worst case of that interpolation is
 * `Δtick · (1/u_min − 1/u_max) / 4`, achieved by a bang-bang rate profile that
 * switches at the midpoint of the interval -- so that is exactly the profile
 * simulated here rather than a smooth one, which would flatter the result.
 *
 * The claim being pinned is not "the error is small" but **"the clock is not the
 * limiting error"**: it has to stay under one video frame period, so that the
 * error you cannot remove (the frame period) dominates the one you chose.
 */
describe('the interpolation error the chosen cadence buys', () => {
    const U_MIN = 50;
    const U_MAX = 60;
    const FRAME_PERIOD_MS_15FPS = 1000 / 15;

    /**
     * The worst error, in host milliseconds, of linearly interpolating one
     * sampling interval of `intervalMs` while the game runs at `U_MAX` for the
     * first half and `U_MIN` for the second.
     */
    function worstErrorMs(intervalMs: number): number {
        const half = intervalMs / 2;
        const midTick = Math.round((U_MAX * half) / 1000);
        const endTick = midTick + Math.round((U_MIN * half) / 1000);
        const clock = parseVideoClock(
            CLEAN_MANIFEST,
            ticksOf([
                [0, 500],
                [endTick, 500 + intervalMs]
            ])
        )!;

        let worst = 0;
        for (let tick = 0; tick <= endTick; tick += 1) {
            // The true host time this tick happened at, under the profile above.
            const trueMs =
                tick <= midTick
                    ? (tick / U_MAX) * 1000
                    : half + ((tick - midTick) / U_MIN) * 1000;
            const estimated = tickToVideoSeconds(clock, tick);
            if (estimated === null) continue;
            worst = Math.max(worst, Math.abs(estimated.seconds * 1000 - trueMs));
        }
        return worst;
    }

    it('stays well under one 15 fps frame period at the 2 Hz floor', () => {
        const error = worstErrorMs(500);
        // ~28 ms measured; the closed form says 30 ticks x 3.33 ms/tick / 4 =
        // 25 ms, and the difference is the tick quantisation this simulation
        // has and the formula does not.
        expect(error).toBeLessThan(35);
        expect(error).toBeLessThan(FRAME_PERIOD_MS_15FPS / 2);
    });

    it('would exceed a frame period at a cadence only four times slower', () => {
        // The comparison that makes the cadence a choice rather than a default.
        // 1900 ms is the slowest interval the clock will still interpolate at
        // all, and there the error is already the *dominant* one rather than a
        // negligible one -- which is what "the video's frame period is the
        // limiting error" stops being true of.
        expect(worstErrorMs(1900)).toBeGreaterThan(FRAME_PERIOD_MS_15FPS);
    });

    it('refuses rather than interpolates once the interval passes the gap limit', () => {
        // Past MAX_INTERP_GAP_MS the answer is not a worse estimate, it is *no*
        // estimate. Only the two endpoints still answer, because they are
        // observations rather than guesses.
        const clock = parseVideoClock(
            CLEAN_MANIFEST,
            ticksOf([
                [0, 500],
                [120, 500 + MAX_INTERP_GAP_MS + 1]
            ])
        )!;
        const answered = [];
        for (let tick = 0; tick <= 120; tick += 1) {
            if (tickToVideoSeconds(clock, tick) !== null) answered.push(tick);
        }
        expect(answered).toEqual([0, 120]);
    });
});
