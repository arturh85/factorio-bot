import {describe, expect, it} from 'vitest';
import {Event, Sample} from '@/api/types';
import {loadFixtureRun} from './fixtureRun';
import {attributeInterval, attributionIntervals, feedingDispatches, machineProduction, verbOf} from './runAttribution';

const force = (tick: number, made: Record<string, number>): Sample => ({
    kind: 'force', research: null, techs_unlocked: 0, production: {made, consumed: {}}, pollution: null,
    power: {generated_kw: 0, consumed_kw: 0, satisfaction: 1, networks: {}}, schema: 3, tick, run: 'r'
});
const machines = (tick: number, rows: Record<string, {name: string; recipe: string | null; produced: number | null; source: string | null}>): Sample => ({
    kind: 'machines', truncated: 0, schema: 3, tick, run: 'r',
    machines: Object.fromEntries(Object.entries(rows).map(([k, r]) => [k, {
        name: r.name, type: 'furnace', position: {x: 0, y: 0}, status: 'working', network: null,
        recipe: r.recipe, crafting: null, progress: null, products_finished: null,
        produced: r.produced, produced_source: r.source, produced_shared: false, mining: null,
        input: {}, output: {}, fuel: {}
    }]))
});
const dispatch = (tick: number, action: string): Event => ({
    kind: 'action_dispatched', tick, id: 1, bot: 1, action, target: null, delivery: null
} as Event);

describe('verbOf / feedingDispatches', () => {
    it('counts feeding verbs in (lo, hi], by count not ticks', () => {
        const ev = [dispatch(100, 'insert 2 coal'), dispatch(200, 'craft 1 pipe'), dispatch(300, 'mine 4 iron-ore'), dispatch(400, 'take 1 plate')];
        expect(verbOf('walk to [1, 2]')).toBe('walk');
        expect(feedingDispatches(ev, 100, 300)).toBe(1); // 100 excluded (lo <), 300 included
        expect(feedingDispatches(ev, 0, 400)).toBe(3);
    });
});

describe('machineProduction', () => {
    it('is unavailable, not zero, for rows without counters', () => {
        const s = [machines(300, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: null, source: null}})];
        expect(machineProduction(s, 0, 300).available).toBe(false);
    });
    it('names an idle furnace\'s output by the last recipe it was ever seen with', () => {
        const s = [
            machines(300, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 2, source: 'game'}}),
            machines(600, {a: {name: 'stone-furnace', recipe: null, produced: 5, source: 'game'}})
        ];
        expect(machineProduction(s, 300, 600).byItem).toEqual({'iron-plate': 3});
    });
});

describe('attributeInterval', () => {
    const base = [force(0, {}), force(600, {'iron-plate': 10})];
    it('hand-made when no machine produced any', () => {
        const s = [...base, machines(0, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 0, source: 'game'}}),
            machines(600, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 0, source: 'game'}})];
        expect(attributeInterval(s, [], 0, 600, 'iron-plate').verdict).toBe('hand-made');
    });
    it('unclear-by-inference when the machine set is empty: no counter evidence at all', () => {
        const s = [...base, machines(0, {}), machines(600, {})];
        const a = attributeInterval(s, [], 0, 600, 'iron-plate');
        expect(a.source).toBe('inference');
        expect(a.verdict).toBe('unclear');
    });
    it('roster-fed when machines made ≥95% and the roster fed', () => {
        const s = [...base, machines(0, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 0, source: 'game'}}),
            machines(600, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 10, source: 'game'}})];
        const a = attributeInterval(s, [dispatch(300, 'insert 5 iron-ore')], 0, 600, 'iron-plate');
        expect(a.verdict).toBe('roster-fed');
        expect(a.machineMade).toBe(10);
        expect(a.rosterMade).toBe(0);
    });
    it('factory when machines made ≥95% and nobody fed', () => {
        const s = [...base, machines(0, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 0, source: 'game'}}),
            machines(600, {a: {name: 'stone-furnace', recipe: 'iron-plate', produced: 10, source: 'game'}})];
        expect(attributeInterval(s, [], 0, 600, 'iron-plate').verdict).toBe('factory');
    });
    it('unclear when the machine counters exceed the force statistics', () => {
        const s = [...base, machines(0, {a: {name: 'burner-mining-drill', recipe: 'iron-plate', produced: 0, source: 'accumulated'}}),
            machines(600, {a: {name: 'burner-mining-drill', recipe: 'iron-plate', produced: 20, source: 'accumulated'}})];
        expect(attributeInterval(s, [], 0, 600, 'iron-plate').verdict).toBe('unclear');
    });
    it('no output when nothing was made', () => {
        expect(attributeInterval([force(0, {}), force(600, {})], [], 0, 600, 'iron-plate').verdict).toBe('no output');
    });
});

describe('golden: every mark verdict equals just analyse --json', () => {
    const run = loadFixtureRun();
    it('agrees on verdict, machine_made and roster_made for every item at every reached mark', () => {
        let prevMinute = 0;
        for (const golden of run.rates.marks.filter((m) => !m.is_end && m.status === 'ok')) {
            const lo = run.lo + Math.round(prevMinute * 3600);
            const hi = golden.tick;
            for (const item of run.rates.items) {
                const g = golden.items[item];
                const a = attributeInterval(run.samples, run.events, lo, hi, item);
                expect(a.verdict, `${golden.label} ${item}`).toBe(g.verdict);
                if (g.verdict === 'no output') continue;
                expect(a.machineMade).toBe(g.machine_made);
                expect(a.rosterMade).toBe(g.roster_made);
                expect(a.feeds).toBe(golden.attribution?.roster.feed_actions);
            }
            prevMinute = golden.minute;
        }
    });
    it('paints one-minute intervals across the whole run', () => {
        const rows = attributionIntervals(run.samples, run.events, run.lo, run.hi, 'iron-plate');
        expect(rows).toHaveLength(7); // 21,982 ticks = 6.1 min -> 7 intervals, last one short
        expect(rows[0].from).toBe(run.lo);
        expect(rows[6].to).toBe(run.hi);
        expect(rows.every((r) => ['roster-fed', 'factory', 'hand-made', 'mixed', 'unclear', 'no output'].includes(r.verdict))).toBe(true);
    });
});
