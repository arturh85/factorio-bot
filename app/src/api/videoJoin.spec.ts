import {describe, expect, it} from 'vitest';
import {combineRunMatchChecks, runIdCheck, videoDefects, videoTickRangeCheck} from './videoJoin';
import {
    CLEAN_MANIFEST,
    NO_VIDEO_MANIFEST,
    SKEWED_MANIFEST,
    UNSTOPPED_MANIFEST
} from './video.fixtures';
import {REALISTIC_REPLAY} from './replay.fixtures';
import {parseReplay} from './replay';
import {VideoManifest} from './types';

const replay = parseReplay(REALISTIC_REPLAY)!;

describe('videoTickRangeCheck', () => {
    it('is consistent when the ranges overlap, never confirmed', () => {
        // Two different runs can easily share a tick range, so overlap is never
        // proof -- only non-overlap is a claim this check can stand behind.
        const check = videoTickRangeCheck(replay, CLEAN_MANIFEST);
        expect(check.result.kind).toBe('consistent');
    });

    it('contradicts a recording whose clock never overlaps the replay', () => {
        const elsewhere: VideoManifest = {...CLEAN_MANIFEST, tick_range: {from: 90000, to: 91000}};
        const check = videoTickRangeCheck(replay, elsewhere);
        expect(check.result.kind).toBe('contradicted');
        expect(combineRunMatchChecks([check]).show).toBe(false);
    });

    it('is inconclusive when the recording observed nothing', () => {
        expect(videoTickRangeCheck(replay, NO_VIDEO_MANIFEST).result.kind).toBe('inconclusive');
    });

    it('reuses the frame side\'s run-id check verbatim', () => {
        // Not a second, differently-broken comparison: the same function, so
        // whatever is true of frames stays true of video.
        expect(runIdCheck('run-1', 'run-1').result.kind).toBe('confirmed');
        expect(runIdCheck(null, 'run-1').result.kind).toBe('inconclusive');
        expect(runIdCheck('run-1', 'run-2').result.kind).toBe('contradicted');
    });
});

describe('videoDefects', () => {
    it('finds nothing wrong with a clean recording', () => {
        expect(videoDefects(CLEAN_MANIFEST)).toEqual([]);
    });

    it('says nothing at all about a run that never recorded video', () => {
        // "No video" is the default, not a defect.
        expect(videoDefects(NO_VIDEO_MANIFEST)).toEqual([]);
    });

    it('reports a recorder that outlived its run, and its unchecked rate', () => {
        const defects = videoDefects(UNSTOPPED_MANIFEST);
        expect(defects.map((d) => d.kind)).toEqual(['unstopped', 'rate-unverified']);
        expect(defects[0].message).toContain('never stopped');
    });

    it('reports a rate that was checked and failed', () => {
        const defects = videoDefects(SKEWED_MANIFEST);
        expect(defects.map((d) => d.kind)).toEqual(['rate-unverified']);
        expect(defects[0].message).toContain('different rate');
    });

    it('reports a window manager that ignored the requested size', () => {
        // A warning on the run, not a failure: the video is still usable and
        // still joins on ticks.
        const tiled: VideoManifest = {
            ...CLEAN_MANIFEST,
            video: {...CLEAN_MANIFEST.video!, width: 1278, height: 715}
        };
        const defects = videoDefects(tiled);
        expect(defects.map((d) => d.kind)).toEqual(['geometry']);
        expect(defects[0].message).toContain('1278x715');
        expect(defects[0].message).toContain('1280x720');
    });

    it('does not complain about the geometry of a recording that never started', () => {
        const failed: VideoManifest = {
            ...CLEAN_MANIFEST,
            video: {
                ...CLEAN_MANIFEST.video!,
                status: 'failed',
                reason: 'no DISPLAY: nothing is rendering to capture',
                width: 0,
                height: 0,
                calibration: [],
                rate_ok: null
            }
        };
        const defects = videoDefects(failed);
        expect(defects.map((d) => d.kind)).toEqual(['failed']);
        expect(defects[0].message).toContain('no DISPLAY');
    });

    it('reports a recording cut short by the disk guard', () => {
        const short: VideoManifest = {
            ...CLEAN_MANIFEST,
            video: {
                ...CLEAN_MANIFEST.video!,
                status: 'stopped_low_disk',
                reason: 'only 1500 MiB free on the workspace filesystem'
            }
        };
        expect(videoDefects(short).map((d) => d.kind)).toEqual(['low-disk']);
    });
});
