/**
 * Reading world-state samples at a tick.
 *
 * Pure, and kept out of the components for the same reason `runTimeline.ts`
 * is: this is the part that can be wrong in a way you would not notice by
 * looking at the screen.
 */
import {BotSample, Sample} from '@/api/types';

export interface ProductionPoint {
    tick: number;
    made: number;
}

export interface ProductionSeries {
    item: string;
    points: ProductionPoint[];
}

/** The latest `bots` sample at or before `tick`, or null before the first. */
export function botSampleAt(samples: Sample[], tick: number): Sample | null {
    let best: Sample | null = null;
    for (const sample of samples) {
        if (sample.kind !== 'bots' || sample.tick > tick) continue;
        if (best === null || sample.tick > best.tick) best = sample;
    }
    return best;
}

/** The latest `force` sample at or before `tick`, or null before the first. */
export function forceSampleAt(samples: Sample[], tick: number): Sample | null {
    let best: Sample | null = null;
    for (const sample of samples) {
        if (sample.kind !== 'force' || sample.tick > tick) continue;
        if (best === null || sample.tick > best.tick) best = sample;
    }
    return best;
}

/** What one bot held in a sample, or null when it is not in that sample. */
export function inventoryOf(sample: Sample | null, bot: number): BotSample | null {
    if (sample === null || sample.kind !== 'bots') return null;
    return sample.bots.find((b) => b.id === bot) ?? null;
}

/**
 * Cumulative production curves.
 *
 * An item absent from a sample carries its previous total forward rather than
 * reading as zero. The mod omits an item with no production, and a cumulative
 * curve that drops to zero and back is a graph of the file format, not of the
 * factory.
 */
export function productionSeries(samples: Sample[], items: string[]): ProductionSeries[] {
    const last = new Map<string, number>(items.map((item) => [item, 0]));
    const series: ProductionSeries[] = items.map((item) => ({item, points: []}));
    const force = samples.filter((s) => s.kind === 'force').sort((a, b) => a.tick - b.tick);
    for (const sample of force) {
        if (sample.kind !== 'force') continue;
        for (const [i, item] of items.entries()) {
            const made = sample.production.made[item] ?? last.get(item) ?? 0;
            last.set(item, made);
            series[i].points.push({tick: sample.tick, made});
        }
    }
    return series;
}

/**
 * The item a milestone goal string names, or null for a goal that does not
 * name one.
 *
 * `Split.goal` is **not** a rendering of the planner's `Goal` -- it is
 * whatever string a script passed to `record.milestone_started(index, goal)`,
 * documented there as "a human-readable description of what is being
 * pursued" (`crates/scripting_lua/src/globals/record.rs`). Most callers are
 * expected to pass `tostring(goal)` on a `goal.*` value, and the one function
 * that renders those, `render_goal` in
 * `crates/scripting_lua/src/globals/goal/value.rs`, has a fixed shape for
 * `goal.have`: `"have {count} {item}"` (there is no `produce` form -- the
 * planner's `Goal::Produced` has no Lua constructor to render). But nothing
 * enforces that a script calls `tostring` at all: `"iron"`,
 * `"researched(automation)"` and `"smelt iron plates x20"` all appear as real
 * goal strings elsewhere in this repo, and none of them name a parseable
 * item. So this matches the one shape known to occur and otherwise returns
 * null -- it is a best-effort read of free text, not a parser with a
 * guaranteed input, which is exactly why `trackedItems` below falls back to
 * `producedItems` when nothing here matches.
 */
function itemFromGoal(goal: string): string | null {
    const match = /^have \d+ (\S+)$/.exec(goal);
    return match ? match[1] : null;
}

/**
 * The distinct items named by a run's milestone goals, in the order they were
 * first mentioned.
 */
export function itemsFromGoals(goals: string[]): string[] {
    const items: string[] = [];
    for (const goal of goals) {
        const item = itemFromGoal(goal);
        if (item !== null && !items.includes(item)) items.push(item);
    }
    return items;
}

/** Every item that appears in any `force` sample's `made`, alphabetical. */
export function producedItems(samples: Sample[]): string[] {
    const items = new Set<string>();
    for (const sample of samples) {
        if (sample.kind !== 'force') continue;
        for (const item of Object.keys(sample.production.made)) items.add(item);
    }
    return [...items].sort();
}

/**
 * The items a production panel should track: those named by the run's
 * milestone goals, or -- when none of them parse as an item goal, e.g. a
 * research-only run -- every item any sample actually recorded making.
 */
export function trackedItems(goals: string[], samples: Sample[]): string[] {
    const named = itemsFromGoals(goals);
    return named.length > 0 ? named : producedItems(samples);
}
