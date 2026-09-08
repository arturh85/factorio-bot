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

/** The axis minus the union of this bot's lanes. An unterminated lane covers to the end. */
export function idleIntervals(lanes: Lane[], bot: number, scale: Interval): Interval[] {
    const busy = lanes
        .filter((l) => l.bot === bot)
        .map((l) => ({from: Math.max(scale.from, l.from_tick), to: Math.min(scale.to, l.to_tick ?? scale.to)}))
        .filter((i) => i.to > i.from)
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
