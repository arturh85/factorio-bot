/**
 * Idleness drawn, not implied.
 *
 * A verb histogram cannot see waiting: five feeding verbs settle in the tick
 * they dispatch and idle time appears nowhere. So the lane's background is
 * the idle state and only dispatched work paints over it; this file computes
 * the gaps (overlap-aware — a bot doing two things at once is not idle twice)
 * and numbers every segment by the plan it belongs to, because action ids
 * restart with every `plan_created` and must never be joined across one.
 */
import {Event, Lane} from '@/api/types';
import {verbOf} from './runAttribution';

export type VerbClass = 'walk' | 'mine' | 'craft' | 'place' | 'feed' | 'research' | 'other';

export function verbClass(action: string): VerbClass {
    switch (verbOf(action)) {
        case 'walk': return 'walk';
        case 'mine': case 'chop': return 'mine';
        case 'craft': return 'craft';
        case 'place': return 'place';
        case 'insert': case 'stock': case 'take': case 'fuel': case 'charge': return 'feed';
        case 'research': return 'research';
        default: return 'other';
    }
}

export interface LaneSegment {
    bot: number;
    /** 0 before the first `plan_created`, then 1, 2, … — never join across it. */
    planIndex: number;
    id: number | null;
    action: string;
    verb: VerbClass;
    from: number;
    to: number | null;
    status: string | null;
    error: string | null;
    /** Settled in its dispatch tick; drawn as a fixed-width tick, not a span. */
    instant: boolean;
}

/** Every `plan_created` tick, ascending. */
export function replanBoundaries(events: Event[]): number[] {
    return events.filter((e) => e.kind === 'plan_created').map((e) => e.tick).sort((a, b) => a - b);
}

/**
 * The tick of every `planning_timed` that produced no plan, ascending.
 *
 * `planning_timed` is written by the plan itself, before `plan_created` --
 * "written by the plan itself rather than folded into `plan_created` ... a
 * plan that raised has no `plan_created` and still cost time"
 * (`EventKind::PlanningTimed`'s own doc). So a `planning_timed` with no
 * `plan_created` before the next `planning_timed`, or before the end of the
 * log, is a replan that was attempted and refused -- real Space Age record:
 * `run-1788923927-04849` has two `planning_timed` (452, 48017) and one
 * `plan_created` (452), the second attempt refusing right after 48 abandoned
 * steps.
 */
export function refusedReplans(events: Event[]): number[] {
    const timed = events.filter((e) => e.kind === 'planning_timed').map((e) => e.tick).sort((a, b) => a - b);
    const created = new Set(replanBoundaries(events));
    const refused: number[] = [];
    for (let i = 0; i < timed.length; i++) {
        const from = timed[i];
        const to = i + 1 < timed.length ? timed[i + 1] : Infinity;
        const answered = [...created].some((t) => t >= from && t < to);
        if (!answered) refused.push(from);
    }
    return refused;
}

export function laneSegments(lanes: Lane[], boundaries: number[]): LaneSegment[] {
    return lanes.map((l) => ({
        bot: l.bot,
        planIndex: boundaries.filter((b) => b <= l.from_tick).length,
        id: l.id,
        action: l.action,
        verb: verbClass(l.action),
        from: l.from_tick,
        to: l.to_tick,
        status: l.status,
        error: l.error,
        instant: l.to_tick !== null && l.to_tick === l.from_tick
    }));
}

export interface Interval {
    from: number;
    to: number;
}

/**
 * The axis minus the union of this bot's lanes. An unterminated lane covers to the end.
 *
 * Zero-length spans are KEPT, not dropped. `place`/`insert`/`take`/`fuel`
 * settle in the tick they dispatch, so as intervals they are points; dropping
 * a point for having no width lets the gap run past the very action the bot
 * was waiting for and onto whatever it did next -- matching `idle_gaps` in
 * `tools/run_analysis.py`, whose docstring names the exact misattribution
 * this caused (`run-1788459085-32452`: "waited 12,246 for `craft 3 pipe`" in
 * place of "waited 12,244 for `take 50 iron-plate from the cell`").
 */
export function idleIntervals(lanes: Lane[], bot: number, scale: Interval): Interval[] {
    const busy = lanes
        // An abandoned step is not work: the bot never touched it, so its
        // zero-length lane must not carve a gap out of the bot's idle time.
        .filter((l) => l.bot === bot && l.status !== 'abandoned')
        .map((l) => ({from: Math.max(scale.from, l.from_tick), to: Math.min(scale.to, l.to_tick ?? scale.to)}))
        .filter((i) => i.to >= i.from)
        .sort((a, b) => a.from - b.from);
    const gaps: Interval[] = [];
    let cursor = scale.from;
    for (const b of busy) {
        if (b.from > cursor) gaps.push({from: cursor, to: b.from});
        cursor = Math.max(cursor, b.to);
    }
    if (cursor < scale.to) gaps.push({from: cursor, to: scale.to});
    return gaps;
}

export function idleTicks(intervals: Interval[]): number {
    return intervals.reduce((n, i) => n + (i.to - i.from), 0);
}
