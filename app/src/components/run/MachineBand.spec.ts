// app/src/components/run/MachineBand.spec.ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {loadFixtureRun} from '@/lib/fixtureRun';
import MachineBand from './MachineBand.vue';

const run = loadFixtureRun();
const scale = {from: run.lo, to: run.hi};

describe('MachineBand', () => {
    const w = mount(MachineBand, {props: {scale, cursor: run.lo, samples: run.samples, selected: null}});
    it('has one row per sampled machine and cells coloured by status', () => {
        expect(w.findAll('text.row-label')).toHaveLength(17);
        const statuses = new Set(w.findAll('rect.cell').map((c) => c.attributes('data-status')));
        expect(statuses.has('working')).toBe(true);
        expect(statuses.has('no_ingredients')).toBe(true);
        expect(statuses.has('no_fuel')).toBe(true);
        const working = w.findAll('rect.cell').find((c) => c.attributes('data-status') === 'working')!;
        expect(working.attributes('fill')).toBe('var(--color-status-good)');
    });
    it('titles every cell with the machine, time and status', () => {
        // `machineRows` sorts mining-drill (rank 0) ahead of furnace (rank 1), and this
        // fixture's one drill (#43) also cycles through `no_fuel` -- so the first `no_fuel`
        // cell in DOM order is the drill's, not a furnace's. Narrow to a furnace cell.
        const cell = w.findAll('rect.cell').find((c) => c.attributes('data-status') === 'no_fuel' && c.find('title').text().startsWith('furnace'))!;
        expect(cell.find('title').text()).toMatch(/furnace #\d+ · \d+:\d\d · no_fuel/);
    });
    it('emits the row key when a cell is clicked', async () => {
        await w.findAll('rect.cell')[0].trigger('click');
        expect(w.emitted('select')?.[0]?.[0]).toMatch(/^\d+$/);
    });
    it('says so when there are no machine samples', () => {
        const v = mount(MachineBand, {props: {scale, cursor: run.lo, samples: run.samples.filter((s) => s.kind !== 'machines'), selected: null}});
        expect(v.text()).toContain('no machine samples in this run');
    });
});
