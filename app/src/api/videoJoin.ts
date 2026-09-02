/**
 * Joining a replay to a recording, and reporting what is wrong with the
 * recording itself.
 *
 * **Nothing here converts a tick to a time.** The join is `game.tick` against
 * `game.tick`, exactly as in `frameJoin.ts`; seconds live in `videoClock.ts` and
 * nowhere else.
 *
 * `runIdCheck` and `combineRunMatchChecks` are imported from `frameJoin.ts`
 * unchanged -- they take strings and `RunMatchCheck`s, not frames. Copying them
 * is how two halves of a system come to disagree about what a match is, so
 * whatever is true of frames stays true of video by construction.
 */

import {Replay} from './replay';
import {RunMatchCheck, replayObservedTickRange} from './frameJoin';
import {VideoManifest} from './types';

export {combineRunMatchChecks, runIdCheck} from './frameJoin';
export type {RunMatchCheck, RunMatchResult, RunMatchVerdict} from './frameJoin';

/**
 * The video's twin of `tickOverlapCheck`.
 *
 * Contradicts only when the two ranges provably never overlap -- which, unlike
 * overlap itself, is a claim this function can stand behind. Two different runs
 * can easily share a tick range, so overlap is never proof.
 *
 * A recording whose clock observed nothing has no range, and that is
 * `inconclusive`: it cannot be judged, which is not the same as failing.
 */
export function videoTickRangeCheck(replay: Replay, manifest: VideoManifest): RunMatchCheck {
    const name = 'video tick range overlap';
    const replayRange = replayObservedTickRange(replay);
    const videoRange = manifest.tick_range;

    if (replayRange === null || videoRange === null) {
        return {
            name,
            result: {
                kind: 'inconclusive',
                reason: 'not enough observed or sampled ticks to compare ranges'
            }
        };
    }

    if (replayRange.start > videoRange.to || videoRange.from > replayRange.end) {
        return {
            name,
            result: {
                kind: 'contradicted',
                reason:
                    `the replay's observed ticks (${replayRange.start}-${replayRange.end}) never overlap ` +
                    `the recording's clock (${videoRange.from}-${videoRange.to}) -- this video is from a ` +
                    'different run'
            }
        };
    }

    return {name, result: {kind: 'consistent'}};
}

/** One thing wrong with a recording, in words a reader can act on. */
export interface VideoDefect {
    kind: 'unstopped' | 'died' | 'failed' | 'low-disk' | 'rate-unverified' | 'geometry';
    message: string;
}

/**
 * What is wrong with this recording, as distinct from whether it belongs to this
 * run.
 *
 * Reported rather than folded into the join because they are different
 * questions: a recording can be unimpeachably *this run's* and still be
 * incomplete, and showing it without saying so is the failure the design's three
 * detection mechanisms exist to prevent.
 *
 * Order is severity: what makes the video untrustworthy comes before what merely
 * makes it imperfect.
 */
export function videoDefects(manifest: VideoManifest): VideoDefect[] {
    const record = manifest.video;
    if (record === null) return [];
    const defects: VideoDefect[] = [];

    if (record.status === 'recording') {
        // By itself proof that the recorder was never stopped: the run finished
        // and nobody told the encoder. Its length, its last frames and its rate
        // are all unknown.
        defects.push({
            kind: 'unstopped',
            message:
                'this recording was never stopped -- the run finished with the encoder still ' +
                'running, so how much of the run it covers is unknown'
        });
    }
    if (record.status === 'died') {
        defects.push({
            kind: 'died',
            message:
                'the encoder stopped advancing part-way through' +
                (record.reason === null ? '' : `: ${record.reason}`)
        });
    }
    if (record.status === 'failed') {
        defects.push({
            kind: 'failed',
            message:
                'no video was captured for this run' +
                (record.reason === null ? '' : `: ${record.reason}`)
        });
    }
    if (record.status === 'stopped_low_disk') {
        defects.push({
            kind: 'low-disk',
            message:
                'the recording was cut short to leave disk for the run\'s own records' +
                (record.reason === null ? '' : `: ${record.reason}`)
        });
    }
    if (record.rate_ok !== true && record.status !== 'failed') {
        defects.push({
            kind: 'rate-unverified',
            message:
                record.rate_ok === false
                    ? 'the recording ran at a different rate than the clock, so every position in ' +
                      'it is approximate'
                    : 'the recording\'s rate was never checked -- it needs two calibration points ' +
                      'and this one has ' + record.calibration.length
        });
    }
    if (record.status !== 'failed' && (record.width !== record.requested_width ||
        record.height !== record.requested_height)) {
        defects.push({
            kind: 'geometry',
            message:
                `the window manager gave ${record.width}x${record.height}, not the ` +
                `${record.requested_width}x${record.requested_height} that was asked for`
        });
    }
    return defects;
}
