// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {tickX} from '@/lib/tickScale';
import TickAxis from './TickAxis.vue';

const scale = {from: 3242, to: 25224};
const clock = {from: 3242, to: 25224};

describe('TickAxis', () => {
    it('draws a minute tick per game minute and emphasises the 5-minute mark', () => {
        const w = mount(TickAxis, {props: {scale, clock, cursor: 3242}});
        const ticks = w.findAll('[data-testid="tick-axis"] line.minute');
        expect(ticks).toHaveLength(6); // 1:00 .. 6:00
        const marks = w.findAll('[data-testid="tick-axis"] line.mark');
        expect(marks).toHaveLength(1);
        expect(w.text()).toContain('5:00');
    });
    it('takes the mark from the analysis clock and only its POSITION from the axis', () => {
        // A trimmed axis must not move the tool's 5:00 to another tick.
        const trimmed = {from: 3242 + 175, to: 25224};
        const w = mount(TickAxis, {props: {scale: trimmed, clock, cursor: 3242}});
        expect(w.text()).toContain('5:00 mark');
        const mark = w.get('[data-testid="tick-axis"] line.mark');
        expect(Number(mark.attributes('x1'))).toBeCloseTo(tickX(trimmed, 3242 + 18000), 5);
    });
    it('places the cursor at tickX', () => {
        const w = mount(TickAxis, {props: {scale, clock, cursor: 3242 + 10991}});
        const x = Number(w.get('[data-testid="cursor"]').attributes('x1'));
        expect(x).toBeCloseTo(500, 0);
    });
});
