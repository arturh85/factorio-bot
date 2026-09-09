/**
 * Placing an archived run on a single tick axis.
 *
 * Every panel in the run viewer reads the same tick cursor, so all of this is
 * pure: given the run's splits, lanes and video range and a cursor, what should
 * be on screen. Kept out of the components because it is the part that can be
 * wrong in a way you would not notice by looking.
 */

import {Event, Lane, RunSummary, Split, TickRange} from '@/api/types';

/**
 * The tick range the timeline spans.
 *
 * Drawn from the splits, the lanes *and* the recording, because any of them can
 * extend past the others: a recording keeps running after the last milestone
 * closes, and a run can record milestones before the recording starts. Taking
 * only one would clip the axis and hide whatever fell outside it.
 *
 * `null` when there is nothing to place at all -- a planning-only run with no
 * milestones. The caller shows an empty timeline rather than one spanning
 * zero to zero, which would look like a run that took no time.
 */
function tickSources(
    splits: Split[],
    lanes: Lane[],
    videoRange: TickRange | null = null
): {all: number[]; drawn: number[]} {
    const all: number[] = [];
    // Ticks at which something is actually *drawn*. A split contributes a span
    // whose bar can be clipped; a lane bar or the recording cannot appear before
    // its own first tick, so these are what decide when the axis has content.
    const drawn: number[] = [];
    for (const split of splits) {
        all.push(split.started_tick);
        if (split.ended_tick !== null) all.push(split.ended_tick);
    }
    for (const lane of lanes) {
        all.push(lane.from_tick);
        drawn.push(lane.from_tick);
        if (lane.to_tick !== null) all.push(lane.to_tick);
    }
    // **The video carries the axis.** Screenshot frame ticks decided where this
    // axis started for as long as the cameras existed; since they were retired
    // (2026-09-02) the recording is the only capture left, so it contributes
    // both ends of its span and the tick at which it starts drawing.
    if (videoRange !== null) {
        all.push(videoRange.from, videoRange.to);
        drawn.push(videoRange.from);
    }
    return {all, drawn};
}

export function tickBounds(
    splits: Split[],
    lanes: Lane[] = [],
    videoRange: TickRange | null = null
): {from: number; to: number} | null {
    const {all, drawn} = tickSources(splits, lanes, videoRange);
    if (all.length === 0) return null;
    // Start where there is something to see.
    //
    // A run's first milestone opens before capture or any bot has produced
    // anything -- 231 ticks before, in the run that prompted this, while the
    // planner was still thinking. Spanning that gap spends axis width on a
    // stretch with no lane bar and no recording, so scrubbing into it answers
    // "nothing happened" when what happened simply is not drawn.
    //
    // `drawn` is a subset of `all`, so this can only move the start forward,
    // and never past `to`. The clipped milestone keeps its true `started_tick`
    // in the splits table; only its bar is cut.
    return {from: axisFrom(all, drawn), to: Math.max(...all)};
}

/**
 * Where the axis begins: the first drawn tick, unless that cuts more than it
 * keeps.
 *
 * **Never cut more than you keep.** Trimming is meant to shave a small dead
 * margin off the front, not to reframe the run. A run whose only capture
 * landed at the very end would otherwise collapse to a zero-width axis --
 * splits carry the whole extent, and dropping everything before that one
 * capture throws that extent away. So the trim applies only while the remaining
 * span is the larger half.
 */
function axisFrom(all: number[], drawn: number[]): number {
    const start = Math.min(...all);
    if (drawn.length === 0) return start;
    const first = Math.min(...drawn);
    const cut = first - start;
    return cut * 2 < Math.max(...all) - start ? first : start;
}

/**
 * Ticks cut from the front of the axis by `tickBounds` -- the run had started
 * but nothing was being drawn yet.
 *
 * Reported rather than swallowed: the axis no longer begins where the run
 * does, and a viewer comparing it against the splits table deserves to be
 * told that instead of discovering it.
 */
export function leadInTicks(
    splits: Split[],
    lanes: Lane[] = [],
    videoRange: TickRange | null = null
): number {
    const {all, drawn} = tickSources(splits, lanes, videoRange);
    if (all.length === 0) return 0;
    return axisFrom(all, drawn) - Math.min(...all);
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

/**
 * Every stuck milestone's planner refusal, by split index.
 *
 * A `milestone_stuck` event can close with no refusal recorded at all --
 * `last_error` is `null` rather than absent -- and that silence is not a
 * message worth showing, so only a non-null error contributes an entry. The
 * last matching event wins when a milestone gets stuck more than once: it is
 * the refusal that was still standing when the record ended.
 */
export function stuckReasons(events: Event[]): Map<number, string> {
    const reasons = new Map<number, string>();
    for (const event of events) {
        if (event.kind !== 'milestone_stuck' || event.last_error === null) continue;
        reasons.set(event.index, event.last_error);
    }
    return reasons;
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
