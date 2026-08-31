/**
 * Joins a replay's tick clock to a frame manifest's tick clock. Both are
 * `game.tick`, the same number from the same source -- there is no scaling
 * and no time conversion anywhere in this file. If a change here ever
 * introduces seconds, that is a bug, not a feature.
 *
 * This module holds two independent pieces of logic:
 *
 * - **Run matching** (`tickOverlapCheck` / `combineRunMatchChecks`): honesty
 *   requirement 1. Nothing ties a frame directory to a run id today, so a
 *   replay from job 3 can be viewed beside frames from job 7 -- their ticks
 *   join numerically without complaint even though they describe unrelated
 *   runs. Overlapping tick ranges is the only signal available, and it is a
 *   weak one: two *different* runs can easily share a tick range, so overlap
 *   is never treated as proof, only non-overlap is. `RunMatchCheck` is kept
 *   as one element of a list precisely so a future, authoritative check (an
 *   explicit run id from a `run.json` sidecar) can be added beside this one
 *   without reshaping `combineRunMatchChecks`'s contract.
 * - **Frame selection** (`framesForClient` / `frameAtTick`): honesty
 *   requirements 2-4. Which capture, if any, belongs beside a given tick, and
 *   how old it is -- never a future frame, never a fabricated one spanning a
 *   gap.
 */

import {Replay, Ticks} from './replay';
import {FrameEntry, FramesManifest} from './types';

export interface TickRange {
    start: Ticks;
    end: Ticks;
}

/**
 * The span of every *observed* tick in the replay -- deliberately not the
 * planned range. Planned ticks are a schedule's intent, not a measurement,
 * and this range exists specifically to be compared against another
 * measurement (a frame's tick, parsed from what the game itself wrote in the
 * filename). `null` when nothing was ever observed, which a caller must
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
 * The span of every parseable frame tick in the manifest. Entries whose
 * filename did not parse (`tick: null`) are excluded, same as
 * {@link framesForClient} -- they have no position on a tick axis. `null`
 * when there are no parseable frames at all.
 */
export function manifestTickRange(manifest: FramesManifest): TickRange | null {
    let start: number | null = null;
    let end: number | null = null;
    for (const frame of manifest.frames) {
        if (frame.tick === null) {
            continue;
        }
        if (start === null || frame.tick < start) {
            start = frame.tick;
        }
        if (end === null || frame.tick > end) {
            end = frame.tick;
        }
    }
    return start === null || end === null ? null : {start, end};
}

function rangesOverlap(a: TickRange, b: TickRange): boolean {
    return a.start <= b.end && b.start <= a.end;
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
     * Kept distinct from `consistent` because showing "these might be from a
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

/**
 * Honesty requirement 1's detector. Compares the replay's observed tick range
 * against the manifest's frame tick range; contradicts only when they
 * provably never overlap, which -- unlike overlap itself -- is a claim this
 * function can actually stand behind.
 */
export function tickOverlapCheck(replay: Replay, manifest: FramesManifest): RunMatchCheck {
    const name = 'tick range overlap';
    const replayRange = replayObservedTickRange(replay);
    const frameRange = manifestTickRange(manifest);

    if (replayRange === null || frameRange === null) {
        return {
            name,
            result: {kind: 'inconclusive', reason: 'not enough observed or frame ticks to compare ranges'}
        };
    }

    if (!rangesOverlap(replayRange, frameRange)) {
        return {
            name,
            result: {
                kind: 'contradicted',
                reason:
                    `the replay's observed ticks (${replayRange.start}-${replayRange.end}) never overlap ` +
                    `the frames' ticks (${frameRange.start}-${frameRange.end}) -- these frames are from a ` +
                    'different run'
            }
        };
    }

    return {name, result: {kind: 'consistent'}};
}

/** What to do, given every run-match check that ran. */
export interface RunMatchVerdict {
    show: boolean;
    reason: string;
    /**
     * `true` means "shown, but unverified": no check found a mismatch, which
     * is not the same as a check having confirmed a match. Must be surfaced
     * in the UI, not just in this type -- see `ReplayScrubber.vue`.
     * `false` on a `show: false` verdict, because a contradiction IS
     * definitive: it is not hedged the way a pass is.
     */
    partial: boolean;
}

/**
 * Combines every run-match check into one verdict. A single `contradicted`
 * result wins outright and is definitive, not partial. Otherwise the verdict
 * permits showing frames, but flags `partial: true`: nothing in this list can
 * *prove* a match, since two different runs can share a tick range, so the
 * absence of a detected mismatch must never read as a guarantee.
 *
 * Adding a second, authoritative check later (an explicit run id from a
 * `run.json` sidecar) is a matter of passing another `RunMatchCheck` into the
 * list this function already takes -- this function's shape does not change.
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
                reason: 'the frames carry this run\'s identifier',
                partial: false
            };
        }
    }
    return {
        show: true,
        reason: 'no check found a mismatch, but tick overlap alone cannot prove these frames belong to this run',
        partial: true
    };
}

/**
 * Compares the manifest's opaque run identifier against the job the replay came
 * from. The only check here that can *establish* a match rather than fail to
 * refute one.
 *
 * `null` on either side is **inconclusive, never a mismatch**. A capture may
 * legitimately be started without an id, and a missing sidecar says the
 * question cannot be answered — treating that as a contradiction would refuse a
 * perfectly good join, and treating it as a match would assert something nobody
 * established. Both are wrong; saying nothing is not.
 *
 * The comparison is equality on an opaque string and nothing else. Neither the
 * mod, the server, nor this function parses it.
 */
export function runIdCheck(manifestRun: string | null, jobId: string | null): RunMatchCheck {
    const name = 'run identifier';
    if (manifestRun === null || jobId === null) {
        return {
            name,
            result: {
                kind: 'inconclusive',
                reason:
                    manifestRun === null
                        ? 'these frames carry no run identifier'
                        : 'this replay has no job to compare against'
            }
        };
    }
    if (manifestRun === jobId) {
        return {name, result: {kind: 'confirmed'}};
    }
    return {
        name,
        result: {
            kind: 'contradicted',
            reason: `these frames were captured for run ${manifestRun}, not run ${jobId}`
        }
    };
}

/**
 * One client's frames, tick-sorted ascending, unparsed entries (`tick: null`)
 * dropped -- they have no position on a tick axis. Callers that report a
 * frame *count* must still account for what this function drops; see
 * `ReplayScrubber.vue`'s handling of unparsed entries.
 */
export function framesForClient(manifest: FramesManifest, client: number): FrameEntry[] {
    return manifest.frames
        .filter((frame): frame is FrameEntry & {tick: number} => frame.client === client && frame.tick !== null)
        .sort((a, b) => a.tick - b.tick);
}

export interface FrameAtTick {
    frame: FrameEntry;
    /** `tick - frame.tick`. Always `>= 0`: never a frame from the future. */
    age: Ticks;
}

/**
 * The frame to show at `tick`: the most recent capture at or before it.
 * `null` when no capture exists yet at or before `tick` -- honesty
 * requirement 2, "no frame for this moment", is this function returning
 * `null`, not a fallback image.
 *
 * Never reaches forward to a later frame, and never reaches back across a gap
 * further than the true distance: `age` on the result is always the frame's
 * *actual* distance from `tick`, so a scrubber sitting inside a dropped-frame
 * gap (honesty requirement 4) gets the true, possibly large, age rather than
 * silent continuity.
 *
 * `sortedFrames` must already be ascending by tick, e.g. from
 * {@link framesForClient}.
 */
export function frameAtTick(sortedFrames: readonly FrameEntry[], tick: Ticks): FrameAtTick | null {
    let best: FrameEntry | null = null;
    for (const frame of sortedFrames) {
        if (frame.tick === null || frame.tick > tick) {
            break;
        }
        best = frame;
    }
    return best === null || best.tick === null ? null : {frame: best, age: tick - best.tick};
}
