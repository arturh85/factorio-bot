/**
 * Placing an archived run on a single tick axis.
 *
 * Every panel in the run viewer reads the same tick cursor, so all of this is
 * pure: given the run's frames and splits and a cursor, what should be on
 * screen. Kept out of the components because it is the part that can be wrong
 * in a way you would not notice by looking.
 */

import {ArchivedFrame, Lane, RunSummary, Split} from '@/api/types';

/** A frame that can be placed on the axis: one whose filename parsed. */
export interface PlacedFrame extends ArchivedFrame {
    tick: number;
    camera: string;
}

/**
 * The frames that can be positioned in time, sorted by tick.
 *
 * A frame whose name did not parse is archived and listed, but it cannot be
 * placed on a tick axis -- so it is excluded here rather than shown at an
 * invented position. `RunFramesResponse` still reports it; this is the
 * timeline's view, not the archive's.
 */
export function placeable(frames: ArchivedFrame[]): PlacedFrame[] {
    return frames
        .filter((f): f is PlacedFrame => f.tick !== null && f.camera !== null)
        .sort((a, b) => a.tick - b.tick || a.bot - b.bot || a.camera.localeCompare(b.camera));
}

/** The distinct bots that captured frames, ascending. */
export function botsOf(frames: PlacedFrame[]): number[] {
    return [...new Set(frames.map((f) => f.bot))].sort((a, b) => a - b);
}

/** The distinct camera ids, alphabetical. */
export function camerasOf(frames: PlacedFrame[]): string[] {
    return [...new Set(frames.map((f) => f.camera))].sort();
}

/**
 * The frame to show at `tick` for one bot and camera: the latest one at or
 * before the cursor.
 *
 * **At or before, never exact.** Capture runs every 300 ticks and frames drop,
 * so an exact match would leave the panel blank for almost every cursor
 * position -- which would read as "nothing was happening" rather than "no
 * frame was taken at precisely this tick".
 *
 * `null` only when the cursor sits before the first frame, which is a real
 * state: the run had started but capture had not yet produced anything.
 */
export function frameAt(
    frames: PlacedFrame[],
    bot: number,
    camera: string,
    tick: number
): PlacedFrame | null {
    let best: PlacedFrame | null = null;
    for (const frame of frames) {
        if (frame.bot !== bot || frame.camera !== camera) continue;
        if (frame.tick > tick) continue;
        if (best === null || frame.tick > best.tick) best = frame;
    }
    return best;
}

/**
 * The tick range the timeline spans.
 *
 * Drawn from the splits *and* the frames, because either can extend past the
 * other: capture keeps running after the last milestone closes, and a run can
 * record milestones before the first frame lands. Taking only one would clip
 * the axis and hide whatever fell outside it.
 *
 * `null` when there is nothing to place at all -- a planning-only run with no
 * milestones. The caller shows an empty timeline rather than one spanning
 * zero to zero, which would look like a run that took no time.
 */
export function tickBounds(
    splits: Split[],
    frames: PlacedFrame[],
    lanes: Lane[] = []
): {from: number; to: number} | null {
    const ticks: number[] = [];
    for (const split of splits) {
        ticks.push(split.started_tick);
        if (split.ended_tick !== null) ticks.push(split.ended_tick);
    }
    for (const frame of frames) ticks.push(frame.tick);
    for (const lane of lanes) {
        ticks.push(lane.from_tick);
        if (lane.to_tick !== null) ticks.push(lane.to_tick);
    }
    if (ticks.length === 0) return null;
    return {from: Math.min(...ticks), to: Math.max(...ticks)};
}

/** Where `tick` falls across the axis, as a 0..1 fraction. */
export function fractionOf(bounds: {from: number; to: number}, tick: number): number {
    const span = bounds.to - bounds.from;
    // A run whose events all share one tick has no extent; report the start
    // rather than dividing by zero and rendering NaN into a style attribute.
    if (span <= 0) return 0;
    return Math.min(1, Math.max(0, (tick - bounds.from) / span));
}

/** The split covering `tick`, or `null` between or outside milestones. */
export function splitAt(splits: Split[], tick: number): Split | null {
    for (const split of splits) {
        const end = split.ended_tick;
        if (split.started_tick <= tick && (end === null || tick <= end)) return split;
    }
    return null;
}

/** One row of a run-versus-run comparison. */
export interface SplitDelta {
    goal: string;
    ticks: number | null;
    referenceTicks: number | null;
    /** `ticks - referenceTicks`, or `null` when either side is unfinished. */
    delta: number | null;
}

/**
 * Compares two runs' splits by goal name.
 *
 * Matched on goal rather than index, because a run that skips or reorders a
 * milestone would otherwise line up against whatever happened to sit at the
 * same position -- producing a table of confident, meaningless deltas.
 *
 * A goal the reference does not have gets a `null` delta rather than being
 * dropped: "this run did something the other did not" is the interesting row,
 * not the one to hide.
 */
export function compareSplits(splits: Split[], reference: Split[]): SplitDelta[] {
    const byGoal = new Map(reference.map((s) => [s.goal, s]));
    return splits.map((split) => {
        const other = byGoal.get(split.goal) ?? null;
        const ticks = split.elapsed_ticks;
        const referenceTicks = other?.elapsed_ticks ?? null;
        return {
            goal: split.goal,
            ticks,
            referenceTicks,
            delta: ticks !== null && referenceTicks !== null ? ticks - referenceTicks : null
        };
    });
}

/** Ticks as a human duration. 60 ticks is one second of game time. */
export function formatTicks(ticks: number | null): string {
    if (ticks === null) return '—';
    const totalSeconds = ticks / 60;
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds - minutes * 60;
    return minutes > 0
        ? `${minutes}m ${seconds.toFixed(1)}s`
        : `${seconds.toFixed(1)}s`;
}

/** The bots that have lane entries, ascending. */
export function laneBots(lanes: Lane[]): number[] {
    return [...new Set(lanes.map((l) => l.bot))].sort((a, b) => a - b);
}

/**
 * What a bot was doing at `tick`, or `null` when it was between actions.
 *
 * An unterminated span (dispatched, never settled) covers everything from its
 * start onward: as far as the record goes, the bot never stopped doing it.
 * Treating it as instantaneous would show the bot idle during exactly the
 * stretch something went wrong.
 */
export function laneAt(lanes: Lane[], bot: number, tick: number): Lane | null {
    for (const lane of lanes) {
        if (lane.bot !== bot) continue;
        if (lane.from_tick > tick) continue;
        if (lane.to_tick !== null && tick > lane.to_tick) continue;
        return lane;
    }
    return null;
}

/**
 * When a run began, in unix seconds, for *display*.
 *
 * Prefers the manifest's `started_unix`. A run that never finished has none --
 * the server reports it as null on purpose, because it genuinely does not know
 * -- so this falls back to the timestamp in the run id, which this project
 * mints as `run-<unix seconds>-<sub-second>`. The same fallback the server uses
 * to order the list.
 *
 * `null` only when neither is available, which means an id from somewhere else
 * entirely.
 */
export function startedUnixOf(run: Pick<RunSummary, 'run_id' | 'started_unix'>): number | null {
    if (run.started_unix !== null) return run.started_unix;
    const match = /^run-(\d+)-/.exec(run.run_id);
    return match ? Number(match[1]) : null;
}

/**
 * A run's start as local date and time.
 *
 * Local rather than UTC: this answers "when did I run this", which is a
 * question about the reader's day.
 */
export function formatWhen(unix: number | null): string {
    if (unix === null) return '—';
    return new Date(unix * 1000).toLocaleString(undefined, {
        day: 'numeric',
        month: 'short',
        hour: '2-digit',
        minute: '2-digit'
    });
}

/** How long ago, coarsely: "just now", "12 min ago", "3 h ago", "2 d ago". */
export function formatAgo(unix: number | null, nowUnix: number): string {
    if (unix === null) return '';
    const seconds = Math.max(0, nowUnix - unix);
    if (seconds < 90) return 'just now';
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `${minutes} min ago`;
    const hours = Math.round(minutes / 60);
    if (hours < 36) return `${hours} h ago`;
    return `${Math.round(hours / 24)} d ago`;
}
