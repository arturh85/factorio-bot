// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import TickAxis from './TickAxis.vue';

const scale = {from: 3242, to: 25224};

describe('TickAxis', () => {
    it('draws a minute tick per game minute and emphasises the 5-minute mark', () => {
        const w = mount(TickAxis, {props: {scale, cursor: 3242}});
        const ticks = w.findAll('[data-testid="tick-axis"] line.minute');
        expect(ticks).toHaveLength(6); // 1:00 .. 6:00
        const marks = w.findAll('[data-testid="tick-axis"] line.mark');
        expect(marks).toHaveLength(1);
        expect(w.text()).toContain('5:00');
    });
    it('places the cursor at tickX', () => {
        const w = mount(TickAxis, {props: {scale, cursor: 3242 + 10991}});
        const x = Number(w.get('[data-testid="cursor"]').attributes('x1'));
        expect(x).toBeCloseTo(500, 0);
    });
});
