import {describe, expect, it} from 'vitest';
import {
    combineRunMatchChecks,
    frameAtTick,
    framesForClient,
    manifestTickRange,
    replayObservedTickRange,
    tickOverlapCheck
} from './frameJoin';
import {REALISTIC_REPLAY} from './replay.fixtures';
import {
    EMPTY_MANIFEST,
    MANIFEST_WITH_UNPARSED_ENTRY,
    OVERLAPPING_MANIFEST,
    UNRELATED_RUN_MANIFEST
} from './frames.fixtures';

describe('replayObservedTickRange', () => {
    it('spans every observed tick in the realistic fixture (100-400)', () => {
        expect(replayObservedTickRange(REALISTIC_REPLAY)).toEqual({start: 100, end: 400});
    });

    it('is null when nothing was ever observed', () => {
        expect(replayObservedTickRange({...REALISTIC_REPLAY, steps: []})).toBeNull();
    });
});

describe('manifestTickRange', () => {
    it('spans the parsed ticks only', () => {
        expect(manifestTickRange(OVERLAPPING_MANIFEST)).toEqual({start: 0, end: 1200});
    });

    it('ignores entries whose filename did not parse', () => {
        // 200 and 300 parse; the null-tick entry must not become NaN or 0.
        expect(manifestTickRange(MANIFEST_WITH_UNPARSED_ENTRY)).toEqual({start: 200, end: 300});
    });

    it('is null when there are no parseable frames at all', () => {
        expect(manifestTickRange(EMPTY_MANIFEST)).toBeNull();
    });
});

describe('tickOverlapCheck -- the honesty-requirement-1 detector', () => {
    it('is consistent when the two ranges overlap', () => {
        const check = tickOverlapCheck(REALISTIC_REPLAY, OVERLAPPING_MANIFEST);
        expect(check.result.kind).toBe('consistent');
    });

    it('is contradicted, naming both ranges, when the two ranges never overlap', () => {
        const check = tickOverlapCheck(REALISTIC_REPLAY, UNRELATED_RUN_MANIFEST);
        expect(check.result.kind).toBe('contradicted');
        if (check.result.kind === 'contradicted') {
            expect(check.result.reason).toContain('100');
            expect(check.result.reason).toContain('400');
            expect(check.result.reason).toContain('10000');
            expect(check.result.reason).toContain('10600');
        }
    });

    it('is inconclusive, not contradicted, when there is nothing to compare', () => {
        const check = tickOverlapCheck(REALISTIC_REPLAY, EMPTY_MANIFEST);
        expect(check.result.kind).toBe('inconclusive');
    });
});

describe('combineRunMatchChecks -- one input among (eventually) two', () => {
    it('a single contradiction wins outright: do not show', () => {
        const verdict = combineRunMatchChecks([tickOverlapCheck(REALISTIC_REPLAY, UNRELATED_RUN_MANIFEST)]);
        expect(verdict.show).toBe(false);
        expect(verdict.reason.length).toBeGreaterThan(0);
    });

    it('consistent checks show frames, but flagged partial -- overlap is not proof', () => {
        const verdict = combineRunMatchChecks([tickOverlapCheck(REALISTIC_REPLAY, OVERLAPPING_MANIFEST)]);
        expect(verdict.show).toBe(true);
        expect(verdict.partial).toBe(true);
    });

    it('a contradiction is definitive, not partial: it is not hedged the way a pass is', () => {
        const verdict = combineRunMatchChecks([tickOverlapCheck(REALISTIC_REPLAY, UNRELATED_RUN_MANIFEST)]);
        expect(verdict.partial).toBe(false);
    });
});

describe('framesForClient', () => {
    it('keeps only the given client, sorted by tick, dropping unparsed entries', () => {
        const frames = framesForClient(MANIFEST_WITH_UNPARSED_ENTRY, 1);
        expect(frames.map((f) => f.tick)).toEqual([200, 300]);
    });

    it('is empty for a client the manifest never mentions', () => {
        expect(framesForClient(OVERLAPPING_MANIFEST, 99)).toEqual([]);
    });
});

describe('frameAtTick -- honesty requirements 2, 3 and 4', () => {
    const frames = framesForClient(OVERLAPPING_MANIFEST, 1); // ticks 0, 300, 1200

    it('requirement 2: before the first capture, there is no frame at all', () => {
        expect(frameAtTick(frames, 0 - 1)).toBeNull();
        // exactly at the earliest capture is a hit, not a miss
        expect(frameAtTick(frames, 0)?.age).toBe(0);
    });

    it('requirement 3: the most recent capture at or before the tick is used, with its true age', () => {
        const result = frameAtTick(frames, 400);
        expect(result).not.toBeNull();
        expect(result?.frame.tick).toBe(300);
        expect(result?.age).toBe(100);
    });

    it('requirement 4: a scrubber sitting inside a dropped-frame gap gets the stale frame before it, not a fabricated one, and the true (large) age reveals the gap', () => {
        // 300 -> 1200 is a 900-tick gap where the mod's normal 300-tick
        // cadence dropped a capture. Sitting inside it must not silently
        // reach across to 1200.
        const result = frameAtTick(frames, 700);
        expect(result?.frame.tick).toBe(300);
        expect(result?.age).toBe(400);
    });

    it('never returns a frame from the future relative to the scrubber', () => {
        const result = frameAtTick(frames, 300);
        expect(result?.frame.tick).toBe(300);
        expect(result?.age).toBe(0);
    });
});
