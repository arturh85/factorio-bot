/**
 * Where the record has data. Nine of 24 archived runs end on a `plan_created`
 * with nothing after it, and one healthy run was killed because its record
 * went quiet: a blank stretch in any band means "no record", and this is the
 * band that says so by name.
 */
import {Event, Sample} from '@/api/types';

export interface StreamExtent {
    label: string;
    from: number;
    to: number;
    count: number;
}

function extent(label: string, ticks: number[]): StreamExtent | null {
    if (ticks.length === 0) return null;
    return {label, from: Math.min(...ticks), to: Math.max(...ticks), count: ticks.length};
}

export function coverageOf(events: Event[], samples: Sample[]): StreamExtent[] {
    const rows = [
        extent('events', events.map((e) => e.tick)),
        extent('bot samples', samples.filter((s) => s.kind === 'bots').map((s) => s.tick)),
        extent('force + machine samples', samples.filter((s) => s.kind === 'force' || s.kind === 'machines').map((s) => s.tick))
    ];
    return rows.filter((r): r is StreamExtent => r !== null);
}

/** `runEnd - lastSampleTick`; null when nothing was sampled (not zero). */
export function lagTicks(runEnd: number, samples: Sample[]): number | null {
    if (samples.length === 0) return null;
    return runEnd - Math.max(...samples.map((s) => s.tick));
}
