import {describe, expect, it} from 'vitest';
import {parseReplay, parseReplayJson, replayAxisCeiling} from './replay';
import {REALISTIC_REPLAY, REALISTIC_REPLAY_JSON, REFUSED_REPLAY_JSON} from './replay.fixtures';

describe('parseReplay against the tracked, Rust-generated documents', () => {
    // `replay.fixtures.ts` already calls `parseReplay` on both tracked
    // snapshot files at import time; if either disagreed with the declared
    // `Replay` shape, importing the fixtures module here would itself throw
    // before any test body ran. These two tests exist to name that check
    // explicitly and keep it from being an implicit side effect nobody reads.
    it('parses the realistic tracked snapshot without narrowing anything away', () => {
        expect(REALISTIC_REPLAY.planned_makespan).toBe(250);
        expect(REALISTIC_REPLAY.steps.length).toBeGreaterThan(0);
        expect(REALISTIC_REPLAY.refused).toBeNull();
        // A status the brief's own sketch did not mention, and the reason a
        // hand-written fixture would not have caught it: the tracked document
        // carries `Running`, not just the four terminal-looking statuses.
        expect(REALISTIC_REPLAY.steps.some((s) => s.status === 'Running')).toBe(true);
        expect(REALISTIC_REPLAY.unmatched_walks.length).toBeGreaterThan(0);
    });

    it('rejects a document whose status is not one of the five real values', () => {
        const bad = JSON.parse(REALISTIC_REPLAY_JSON);
        bad.steps[0].status = 'Bogus';
        expect(() => parseReplay(bad)).toThrow(/steps\[0\]\.status/);
    });

    it('rejects a document whose evidence uses the externally-tagged shape the brief wrongly sketched', () => {
        const bad = JSON.parse(REALISTIC_REPLAY_JSON);
        bad.steps[0].evidence = {Believed: {why: 'wrong shape'}};
        expect(() => parseReplay(bad)).toThrow(/evidence\.kind/);
    });

    it('rejects a document with a non-numeric, non-null observed tick', () => {
        const bad = JSON.parse(REALISTIC_REPLAY_JSON);
        bad.steps[0].observed_start_tick = '100';
        expect(() => parseReplay(bad)).toThrow(/observed_start_tick/);
    });
});

describe('parseReplayJson', () => {
    it('parses a well-formed document into a Replay', () => {
        const result = parseReplayJson(REALISTIC_REPLAY_JSON);
        expect('replay' in result).toBe(true);
        if ('replay' in result) {
            expect(result.replay.planned_makespan).toBe(250);
            expect(result.replay.refused).toBeNull();
        }
    });

    it('parses a refused document, keeping refused as the reason string', () => {
        const result = parseReplayJson(REFUSED_REPLAY_JSON);
        expect('replay' in result).toBe(true);
        if ('replay' in result) {
            expect(result.replay.refused).not.toBeNull();
            expect(typeof result.replay.refused).toBe('string');
        }
    });

    it('reports an error rather than throwing on invalid JSON', () => {
        const result = parseReplayJson('{not json');
        expect('error' in result).toBe(true);
    });

    it('reports an error, naming the field, on a document missing what a view depends on', () => {
        const result = parseReplayJson(JSON.stringify({
            planned_makespan: 10,
            refused: null,
            steps: [],
            unmatched_walks: []
        }));
        expect('error' in result).toBe(false);

        const missingMakespan = parseReplayJson(JSON.stringify({
            refused: null,
            steps: [],
            unmatched_walks: []
        }));
        expect('error' in missingMakespan).toBe(true);
        if ('error' in missingMakespan) {
            expect(missingMakespan.error).toContain('planned_makespan');
        }
    });

    it('reports an error rather than treating a JSON array as a document', () => {
        const result = parseReplayJson('[]');
        expect('error' in result).toBe(true);
    });
});

describe('replayAxisCeiling', () => {
    it('is the planned makespan when nothing observed runs past it', () => {
        const replay = {...REALISTIC_REPLAY, steps: []};
        expect(replayAxisCeiling(replay)).toBe(250);
    });

    it('extends past the planned makespan when an observed tick runs later', () => {
        expect(replayAxisCeiling(REALISTIC_REPLAY))
            .toBeGreaterThan(REALISTIC_REPLAY.planned_makespan);
    });

    it('never lets a null observed tick pull the ceiling down or throw', () => {
        expect(() => replayAxisCeiling(REALISTIC_REPLAY)).not.toThrow();
    });
});
