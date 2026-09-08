/**
 * The one tick axis every run-page band draws on.
 *
 * Bands are SVGs with `viewBox="0 0 1000 H"`, so a tick maps to an x in
 * 0..1000 and every band agrees on where a tick is without sharing a DOM
 * width. Clamped: a tick outside the axis lands on its edge, never off the
 * drawing.
 */
export interface TickScale {
    from: number;
    to: number;
}

export const AXIS_WIDTH = 1000;
export const TICKS_PER_MINUTE = 3600;

export function tickX(scale: TickScale, tick: number): number {
    const span = scale.to - scale.from;
    if (span <= 0) return 0;
    const clamped = Math.min(scale.to, Math.max(scale.from, tick));
    return ((clamped - scale.from) / span) * AXIS_WIDTH;
}

/** "m:ss" of game time since the axis start; ticks before it read 0:00. */
export function formatGameTime(scale: TickScale, tick: number): string {
    const seconds = Math.max(0, Math.round((tick - scale.from) / 60));
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}:${String(s).padStart(2, '0')}`;
}

/** The fixed game-time marks (5, 10, 15 … minutes from the start) inside the axis. */
export function markTicks(scale: TickScale, everyMinutes = 5): number[] {
    const step = everyMinutes * TICKS_PER_MINUTE;
    const out: number[] = [];
    for (let t = scale.from + step; t <= scale.to; t += step) out.push(t);
    return out;
}
