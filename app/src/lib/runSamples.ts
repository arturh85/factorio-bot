/**
 * Reading world-state samples at a tick.
 *
 * Pure, and kept out of the components for the same reason `runTimeline.ts`
 * is: this is the part that can be wrong in a way you would not notice by
 * looking at the screen.
 */
import {BotSample, Position, Sample} from '@/api/types';

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

/** 30 seconds at 60 UPS -- the map panel's fixed trail lookback window. */
export const TRAIL_WINDOW_TICKS = 1800;

/**
 * Per bot, its positions from every `bots` sample in the last
 * `TRAIL_WINDOW_TICKS` up to and including `tick`, oldest first.
 *
 * A bot absent from every sample in the window is simply absent from the
 * result, not present with an empty array -- a map panel drawing a polyline
 * per key would otherwise iterate zero-length arrays for every bot that has
 * ever appeared in the run.
 */
export function trailsAt(samples: Sample[], tick: number): Record<number, Position[]> {
    const from = tick - TRAIL_WINDOW_TICKS;
    const inWindow = samples
        .filter((s): s is Extract<Sample, {kind: 'bots'}> => s.kind === 'bots' && s.tick > from && s.tick <= tick)
        .sort((a, b) => a.tick - b.tick);
    const trails: Record<number, Position[]> = {};
    for (const sample of inWindow) {
        for (const bot of sample.bots) {
            (trails[bot.id] ??= []).push(bot.position);
        }
    }
    return trails;
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
 * The items a milestone goal string names, in the order it names them --
 * possibly more than one, for a composite goal.
 *
 * `Split.goal` is **not** a rendering of the planner's `Goal` -- it is
 * whatever string a script passed to `record.milestone_started(index, goal)`,
 * documented there as "a human-readable description of what is being
 * pursued" (`crates/scripting_lua/src/globals/record.rs`). Most callers are
 * expected to pass `tostring(goal)` on a `goal.*` value, and the one function
 * that renders those, `render_goal` in
 * `crates/scripting_lua/src/globals/goal/value.rs`, has three fixed shapes:
 * `"have {count} {item}"`, `"researched {tech}"` (no item), and
 * `"all { <part>, <part>, ... }"` where each part is again one of these three
 * -- a milestone can genuinely be `goal.all { ... }` over several items, and
 * that `all` can nest. This function recurses into `all` the same way
 * `render_goal` and `goal_from_lua` do (`crates/scripting_lua/src/globals/goal/value.rs`),
 * for the same reason they do: an `all` that decomposes into its parts and
 * hands each part back to this same function cannot disagree with the shape
 * those two produce and consume, and a nested `all` is handled for free
 * rather than by a second special case.
 *
 * But nothing enforces that a script calls `tostring` at all: `"iron"`,
 * `"researched(automation)"` and `"smelt iron plates x20"` all appear as real
 * goal strings elsewhere in this repo, and none of them name a parseable
 * item. So this matches the shapes known to occur and otherwise names
 * nothing -- it is a best-effort read of free text, not a parser with a
 * guaranteed input, which is exactly why `trackedItems` below falls back to
 * `producedItems` when nothing here matches.
 */
function itemsFromGoal(goal: string): string[] {
    const have = /^have \d+ (\S+)$/.exec(goal);
    if (have) return [have[1]];

    const all = /^all \{ (.*) \}$/.exec(goal);
    if (all) return splitTopLevel(all[1]).flatMap(itemsFromGoal);

    // `researched <tech>` names a technology, not an item; anything else is
    // free text this format does not cover.
    return [];
}

/**
 * Splits `all { ... }`'s inner text on its own top-level ", " separators,
 * skipping any that sit inside a nested `all { ... }`.
 *
 * A plain `.split(', ')` is not safe here: `render_goal` reuses the same
 * `", "` separator at every nesting depth, so it would cut a nested group's
 * items apart at their own separator, mistaking them for siblings of the
 * outer list. That could only ever *drop* items, never rename one --
 * `itemsFromGoal`'s patterns are anchored at both ends, so a fragment that
 * gets the wrong boundary fails to match anything rather than matching the
 * wrong item -- but dropping items a run genuinely tracked defeats the point
 * of curating by goal, so this tracks brace depth instead of taking that risk.
 */
function splitTopLevel(text: string): string[] {
    const parts: string[] = [];
    let depth = 0;
    let start = 0;
    for (let i = 0; i < text.length; i++) {
        const c = text[i];
        if (c === '{') depth++;
        else if (c === '}') depth--;
        else if (c === ',' && depth === 0) {
            parts.push(text.slice(start, i));
            start = i + 1;
        }
    }
    parts.push(text.slice(start));
    return parts.map((p) => p.trim());
}

/**
 * The distinct items named by a run's milestone goals, in the order they were
 * first mentioned.
 *
 * Deduplicated: a goal naming the same item twice -- directly, or once each
 * in two different milestones -- contributes it once. This feeds a panel
 * listing which items to show, not a count of how many goals mention one, so
 * collapsing the duplicate loses nothing this caller needs.
 */
export function itemsFromGoals(goals: string[]): string[] {
    const items: string[] = [];
    for (const goal of goals) {
        for (const item of itemsFromGoal(goal)) {
            if (!items.includes(item)) items.push(item);
        }
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
