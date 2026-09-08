/**
 * Per-machine status over time, the band the record has always carried and
 * never drawn. It turned "the far end starves of ore" into "the far end
 * starves of everything" in one afternoon (CLAUDE.md, entity.status note).
 */
import {MachineSample, Position, Sample} from '@/api/types';

export type StatusClass = 'good' | 'warn' | 'serious' | 'critical' | 'neutral';

const WARN = ['no_ingredients', 'item_ingredient_shortage', 'no_minable_resources', 'missing_science_packs'];

export function statusClass(status: string | null): StatusClass {
    if (status === 'working') return 'good';
    if (status !== null && WARN.includes(status)) return 'warn';
    if (status === 'no_fuel') return 'serious';
    if (status === 'no_power' || status === 'low_power') return 'critical';
    return 'neutral';
}

export interface MachineRow {
    key: string;
    name: string;
    type: string;
    position: Position;
    firstTick: number;
    isContainer: boolean;
}

type MachinesSample = Extract<Sample, {kind: 'machines'}>;

function machinesSamples(samples: Sample[]): MachinesSample[] {
    return samples.filter((s): s is MachinesSample => s.kind === 'machines').sort((a, b) => a.tick - b.tick);
}

const CONTAINER_TYPES = ['container', 'logistic-container', 'infinity-container'];

function kindRank(m: MachineSample): number {
    if (m.type === 'mining-drill') return 0;
    if (m.type === 'furnace') return 1;
    if (m.type === 'assembling-machine') return 2;
    if (m.type === 'lab') return 3;
    if (CONTAINER_TYPES.includes(m.type)) return 5;
    return 4;
}

/** Every machine the run sampled, grouped by kind and ordered by first appearance. */
export function machineRows(samples: Sample[]): MachineRow[] {
    const first = new Map<string, {m: MachineSample; tick: number}>();
    for (const s of machinesSamples(samples)) {
        for (const [key, m] of Object.entries(s.machines)) {
            if (!first.has(key)) first.set(key, {m, tick: s.tick});
        }
    }
    return [...first.entries()]
        .map(([key, {m, tick}]) => ({
            row: {
                key, name: m.name, type: m.type, position: m.position, firstTick: tick,
                isContainer: CONTAINER_TYPES.includes(m.type)
            },
            rank: kindRank(m)
        }))
        .sort((a, b) => a.rank - b.rank || a.row.firstTick - b.row.firstTick || Number(a.row.key) - Number(b.row.key))
        .map(({row}) => row);
}

export interface StatusCell {
    tick: number;
    status: string | null;
    produced: number | null;
    /** Items in a container's output inventory; null for a non-container. */
    fill: number | null;
}

export function statusMatrix(samples: Sample[], rows: MachineRow[]): Map<string, StatusCell[]> {
    const out = new Map<string, StatusCell[]>(rows.map((r) => [r.key, []]));
    for (const s of machinesSamples(samples)) {
        for (const row of rows) {
            const m = s.machines[row.key];
            if (!m) continue;
            const fill = row.isContainer ? Object.values(m.output).reduce((n, c) => n + c, 0) : null;
            out.get(row.key)!.push({tick: s.tick, status: m.status, produced: m.produced, fill});
        }
    }
    return out;
}

/** The most common gap between consecutive `machines` samples; null below two samples. */
export function sampleStepTicks(samples: Sample[]): number | null {
    const ticks = machinesSamples(samples).map((s) => s.tick);
    if (ticks.length < 2) return null;
    const counts = new Map<number, number>();
    for (let i = 1; i < ticks.length; i++) {
        const d = ticks[i] - ticks[i - 1];
        counts.set(d, (counts.get(d) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0] - b[0])[0][0];
}

export function positionKey(p: Position): string {
    return `${p.x},${p.y}`;
}

/** Status by position from the latest `machines` sample at or before `tick`. */
export function machineStatusAt(samples: Sample[], tick: number): Map<string, string | null> {
    let latest: MachinesSample | null = null;
    for (const s of machinesSamples(samples)) {
        if (s.tick <= tick) latest = s;
        else break;
    }
    const out = new Map<string, string | null>();
    if (latest === null) return out;
    for (const m of Object.values(latest.machines)) out.set(positionKey(m.position), m.status);
    return out;
}
