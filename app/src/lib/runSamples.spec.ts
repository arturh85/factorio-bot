import {describe, expect, it} from 'vitest';
import {
    botSampleAt,
    forceSampleAt,
    inventoryOf,
    itemsFromGoals,
    producedItems,
    productionSeries,
    trackedItems
} from './runSamples';
import {BotSample, Sample} from '@/api/types';

const bot = (id: number, over: Partial<BotSample> = {}): BotSample => ({
    id,
    position: {x: 0, y: 0},
    inventory: {},
    crafting_queue: 0,
    mining: null,
    ...over
});

const botSample = (tick: number, bots: BotSample[] = [bot(1)]): Sample => ({
    kind: 'bots',
    bots,
    tick,
    run: 'run-1'
});

const forceSample = (tick: number, made: Record<string, number> = {}): Sample => ({
    kind: 'force',
    research: null,
    techs_unlocked: 0,
    production: {made, consumed: {}},
    power: {generated_kw: 0, consumed_kw: 0, satisfaction: 1},
    tick,
    run: 'run-1'
});

describe('botSampleAt', () => {
    it('takes the latest sample at or before the cursor', () => {
        // At-or-before, never exact: samples land on a 60-tick beat and the
        // cursor does not, so an exact match would blank the panel almost
        // always.
        const s = [botSample(61500), botSample(61560), botSample(61620)];
        expect(botSampleAt(s, 61590)?.tick).toBe(61560);
    });

    it('is null before the first sample', () => {
        expect(botSampleAt([botSample(61500)], 61400)).toBeNull();
    });

    it('ignores force samples when looking for bot state', () => {
        // Both kinds share one file; a lookup that ignored `kind` would
        // return a force line and read its missing `bots` as an empty roster.
        const s = [botSample(61500), forceSample(61560)];
        expect(botSampleAt(s, 61590)?.tick).toBe(61500);
    });
});

describe('forceSampleAt', () => {
    it('takes the latest force sample at or before the cursor', () => {
        const s = [forceSample(61500), forceSample(61800)];
        expect(forceSampleAt(s, 61600)?.tick).toBe(61500);
    });

    it('is null before the first force sample', () => {
        expect(forceSampleAt([forceSample(61500)], 61000)).toBeNull();
    });

    it('ignores bot samples when looking for force state', () => {
        const s = [forceSample(61500), botSample(61560)];
        expect(forceSampleAt(s, 61590)?.tick).toBe(61500);
    });
});

describe('inventoryOf', () => {
    it('finds the named bot in a bots sample', () => {
        const s = botSample(61500, [bot(1, {inventory: {'iron-plate': 3}}), bot(2)]);
        expect(inventoryOf(s, 1)?.inventory).toEqual({'iron-plate': 3});
    });

    it('is null when the bot did not appear in that sample', () => {
        const s = botSample(61500, [bot(2)]);
        expect(inventoryOf(s, 1)).toBeNull();
    });

    it('is null given no sample', () => {
        expect(inventoryOf(null, 1)).toBeNull();
    });

    it('is null given a force sample', () => {
        expect(inventoryOf(forceSample(61500), 1)).toBeNull();
    });
});

describe('productionSeries', () => {
    it('reports cumulative totals as recorded', () => {
        const s = [forceSample(61500, {'iron-plate': 10}), forceSample(61800, {'iron-plate': 45})];
        expect(productionSeries(s, ['iron-plate'])).toEqual([
            {item: 'iron-plate', points: [{tick: 61500, made: 10}, {tick: 61800, made: 45}]}
        ]);
    });

    it('carries the last known total forward for an item a sample omits', () => {
        // The mod omits an item with no production rather than writing a zero.
        // Reading absence as zero would draw a cumulative curve that drops.
        const s = [forceSample(61500, {'iron-plate': 10}), forceSample(61800, {})];
        expect(productionSeries(s, ['iron-plate'])[0].points[1]).toEqual({tick: 61800, made: 10});
    });

    it('starts an item never yet produced at zero', () => {
        const s = [forceSample(61500, {'copper-plate': 5})];
        expect(productionSeries(s, ['iron-plate'])[0].points[0]).toEqual({tick: 61500, made: 0});
    });

    it('ignores bot samples entirely and sorts by tick', () => {
        const s = [forceSample(61800, {'iron-plate': 20}), botSample(61600), forceSample(61500, {'iron-plate': 10})];
        expect(productionSeries(s, ['iron-plate']).map((p) => p.item)).toEqual(['iron-plate']);
        expect(productionSeries(s, ['iron-plate'])[0].points.map((p) => p.tick)).toEqual([61500, 61800]);
    });
});

describe('itemsFromGoals', () => {
    it('reads the item out of a have goal, the shape render_goal actually emits', () => {
        expect(itemsFromGoals(['have 4 iron-plate'])).toEqual(['iron-plate']);
    });

    it('ignores freeform goal text that names no parseable item', () => {
        // `Split.goal` is caller-supplied free text, not a rendering of the
        // planner's `Goal` -- these are all real strings seen elsewhere in
        // this repo, and none of them match the one known shape.
        expect(itemsFromGoals(['iron', 'researched(automation)', 'smelt iron plates x20'])).toEqual([]);
    });

    it('dedupes while keeping first-seen order', () => {
        expect(itemsFromGoals(['have 4 iron-plate', 'have 4 iron-plate', 'have 5 copper-plate']))
            .toEqual(['iron-plate', 'copper-plate']);
    });
});

describe('producedItems', () => {
    it('unions items across every force sample, alphabetically', () => {
        const s = [forceSample(61500, {'iron-plate': 1}), forceSample(61800, {'copper-plate': 1})];
        expect(producedItems(s)).toEqual(['copper-plate', 'iron-plate']);
    });

    it('ignores bot samples', () => {
        expect(producedItems([botSample(61500)])).toEqual([]);
    });
});

describe('trackedItems', () => {
    it('prefers items named by the goals', () => {
        const s = [forceSample(61500, {'copper-plate': 1})];
        expect(trackedItems(['have 50 iron-plate'], s)).toEqual(['iron-plate']);
    });

    it('falls back to produced items when no goal parses -- the common case, since most goal text is freeform', () => {
        const s = [forceSample(61500, {'copper-plate': 1})];
        expect(trackedItems(['researched(automation)'], s)).toEqual(['copper-plate']);
    });
});
