import {describe, expect, it} from 'vitest';
import {AXIS_WIDTH, formatGameTime, markTicks, tickX} from './tickScale';

const scale = {from: 3242, to: 25224};

describe('tickX', () => {
    it('maps the axis start to 0 and the end to AXIS_WIDTH', () => {
        expect(tickX(scale, 3242)).toBe(0);
        expect(tickX(scale, 25224)).toBe(AXIS_WIDTH);
    });
    it('clamps outside the axis rather than drawing off the SVG', () => {
        expect(tickX(scale, 0)).toBe(0);
        expect(tickX(scale, 99999)).toBe(AXIS_WIDTH);
    });
    it('returns 0 for a zero-span scale instead of NaN', () => {
        expect(tickX({from: 5, to: 5}, 5)).toBe(0);
    });
});

describe('formatGameTime', () => {
    it('counts minutes and seconds from the axis start', () => {
        expect(formatGameTime(scale, 3242)).toBe('0:00');
        expect(formatGameTime(scale, 3242 + 18000)).toBe('5:00');
        expect(formatGameTime(scale, 25216)).toBe('6:06');
    });
});

describe('markTicks', () => {
    it('lists the 5-minute marks inside the axis, excluding the origin', () => {
        expect(markTicks(scale)).toEqual([3242 + 18000]);
        expect(markTicks({from: 0, to: 40000}, 5)).toEqual([18000, 36000]);
    });
});
