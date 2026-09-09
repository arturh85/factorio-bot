import {describe, expect, it} from 'vitest';
import {loadFixtureRun} from './fixtureRun';
import {machineRows, machineStatusAt, sampleStepTicks, statusClass, statusMatrix} from './machineTimeline';

describe('statusClass', () => {
    it('maps game statuses onto the reserved status set', () => {
        expect(statusClass('working')).toBe('good');
        expect(statusClass('no_ingredients')).toBe('warn');
        expect(statusClass('no_fuel')).toBe('serious');
        expect(statusClass('no_power')).toBe('critical');
        expect(statusClass('low_power')).toBe('critical');
        expect(statusClass('normal')).toBe('neutral');
        expect(statusClass(null)).toBe('neutral');
    });
});

describe('on the fixture run', () => {
    const run = loadFixtureRun();
    const rows = machineRows(run.samples);
    it('lists every sampled machine once, drills first, containers last, in placement order', () => {
        expect(rows).toHaveLength(17);
        expect(rows[0].name).toBe('burner-mining-drill');
        expect(rows[rows.length - 1].isContainer).toBe(true);
        const furnaces = rows.filter((r) => r.name === 'stone-furnace');
        expect(furnaces.map((r) => r.firstTick)).toEqual([...furnaces.map((r) => r.firstTick)].sort((a, b) => a - b));
    });
    it('gives one cell per sample a machine appears in, carrying its status and counter', () => {
        const m = statusMatrix(run.samples, rows);
        const cells = m.get('13') ?? [];
        expect(cells.length).toBeGreaterThan(50);
        expect(cells[cells.length - 1]).toMatchObject({tick: 25200, status: 'no_ingredients'});
        expect(cells.some((c) => c.status === 'working')).toBe(true);
    });
    it('reads the sample beat off the data', () => {
        expect(sampleStepTicks(run.samples)).toBe(300);
    });
    it('answers status by position at a tick, from the latest sample at or before it', () => {
        const at = machineStatusAt(run.samples, 20399);
        expect(at.get('-10,-21')).toBe('no_ingredients');
        expect(machineStatusAt(run.samples, 100).size).toBe(0);
    });
});
