/**
 * Deciding whether an artefact and a replay describe the same run.
 *
 * **Nothing here converts a tick to a time.** A replay's ticks and a
 * recording's ticks are both `game.tick`, the same number from the same
 * source, so every comparison in this file is a direct numeric one. If a change
 * here ever introduces seconds, that is a bug, not a feature. Seconds live in
 * `videoClock.ts` and nowhere else.
 *
 * This was `frameJoin.ts` until the per-camera screenshots it was written for
 * were retired (2026-09-02). What it holds now is the part that was never about
 * frames: a `RunMatchCheck` takes ticks and opaque strings, so the rules that
 * decide a match are stated once and every artefact is judged by them. Copying
 * them per artefact is how two halves of a system come to disagree about what a
 * match is.
 */

import {Replay, Ticks} from './replay';

export interface TickRange {
    start: Ticks;
    end: Ticks;
}

/**
 * The span of every *observed* tick in the replay -- deliberately not the
 * planned range. Planned ticks are a schedule's intent, not a measurement,
 * and this range exists specifically to be compared against another
 * measurement. `null` when nothing was ever observed, which a caller must
 * treat as "cannot judge", not as "range is empty" (an empty range would
 * still be a range with a start and end at the same point).
 */
export function replayObservedTickRange(replay: Replay): TickRange | null {
    let start: number | null = null;
    let end: number | null = null;
    for (const step of replay.steps) {
        for (const tick of [step.observed_start_tick, step.observed_end_tick]) {
            if (tick === null) {
                continue;
            }
            if (start === null || tick < start) {
                start = tick;
            }
            if (end === null || tick > end) {
                end = tick;
            }
        }
    }
    return start === null || end === null ? null : {start, end};
}

/**
 * What one check concluded. `contradicted` is the only result strong enough
 * to act on alone: it is proof (given the ranges are correct) that the two
 * documents are unrelated. `consistent` is deliberately weak -- it means only
 * "this check found no reason to reject", never "this check confirms a
 * match" -- and `inconclusive` means the check had nothing to compare (e.g.
 * an unattempted run has no observed ticks at all).
 */
export type RunMatchResult =
    | {kind: 'contradicted'; reason: string}
    /**
     * The run is *established*, not merely un-contradicted. Only an identity
     * comparison can produce this — a range overlap never can, however wide.
     * Kept distinct from `consistent` because showing "this might be from a
     * different run" when the run is known is under-claiming, which is its own
     * form of saying something untrue.
     */
    | {kind: 'confirmed'}
    | {kind: 'consistent'}
    | {kind: 'inconclusive'; reason: string};

export interface RunMatchCheck {
    name: string;
    result: RunMatchResult;
}

/** What to do, given every run-match check that ran. */
export interface RunMatchVerdict {
    show: boolean;
    reason: string;
    /**
     * `true` means "shown, but unverified": no check found a mismatch, which
     * is not the same as a check having confirmed a match. Must be surfaced
     * in the UI, not just in this type.
     * `false` on a `show: false` verdict, because a contradiction IS
     * definitive: it is not hedged the way a pass is.
     */
    partial: boolean;
}

/**
 * Combines every run-match check into one verdict. A single `contradicted`
 * result wins outright and is definitive, not partial. Otherwise the verdict
 * permits showing the artefact, but flags `partial: true`: a range check
 * cannot *prove* a match, since two different runs can share a tick range, so
 * the absence of a detected mismatch must never read as a guarantee.
 *
 * Adding a further authoritative check is a matter of passing another
 * `RunMatchCheck` into the list this function already takes -- this function's
 * shape does not change.
 */
export function combineRunMatchChecks(checks: RunMatchCheck[]): RunMatchVerdict {
    // A contradiction wins outright, and is checked first: a confirmation and
    // a contradiction together mean two checks disagree about reality, and the
    // safe reading of that is the one that refuses to show anything.
    for (const check of checks) {
        if (check.result.kind === 'contradicted') {
            return {show: false, reason: check.result.reason, partial: false};
        }
    }
    // A confirmation retires the caveat, because the caveat is no longer true.
    for (const check of checks) {
        if (check.result.kind === 'confirmed') {
            return {
                show: true,
                reason: 'the artefact carries this run\'s identifier',
                partial: false
            };
        }
    }
    return {
        show: true,
        reason: 'no check found a mismatch, but tick overlap alone cannot prove this artefact belongs to this run',
        partial: true
    };
}

/**
 * Compares an artefact's opaque run identifier against the job the replay came
 * from. The only check here that can *establish* a match rather than fail to
 * refute one.
 *
 * `null` on either side is **inconclusive, never a mismatch**. An artefact may
 * legitimately be produced without an id, and a missing sidecar says the
 * question cannot be answered — treating that as a contradiction would refuse a
 * perfectly good join, and treating it as a match would assert something nobody
 * established. Both are wrong; saying nothing is not.
 *
 * The comparison is equality on an opaque string and nothing else. Neither the
 * mod, the server, nor this function parses it.
 */
export function runIdCheck(artefactRun: string | null, jobId: string | null): RunMatchCheck {
    const name = 'run identifier';
    if (artefactRun === null || jobId === null) {
        return {
            name,
            result: {
                kind: 'inconclusive',
                reason:
                    artefactRun === null
                        ? 'this artefact carries no run identifier'
                        : 'this replay has no job to compare against'
            }
        };
    }
    if (artefactRun === jobId) {
        return {name, result: {kind: 'confirmed'}};
    }
    return {
        name,
        result: {
            kind: 'contradicted',
            reason: `this artefact was captured for run ${artefactRun}, not run ${jobId}`
        }
    };
}
