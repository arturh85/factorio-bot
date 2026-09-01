import {describe, expect, it} from 'vitest';
import {project, projectionFor} from './mapProjection';

describe('projectionFor', () => {
    it('scales a square world to fill a square canvas', () => {
        const bounds = {left: -100, top: -100, right: 100, bottom: 100};
        const projection = projectionFor(bounds, 200, 200);
        expect(projection.scale).toBe(1);
        expect(projection.offsetX).toBe(100);
        expect(projection.offsetY).toBe(100);
    });

    it('does not stretch a wide world -- picks the smaller of the two scales', () => {
        // World is 200 wide, 20 tall (10:1). Fit into a 200x200 canvas: the
        // width scale (1) and height scale (10) disagree, and the smaller
        // one must win, or the world is stretched vertically to fill the
        // square.
        const bounds = {left: -100, top: -10, right: 100, bottom: 10};
        const projection = projectionFor(bounds, 200, 200);
        expect(projection.scale).toBe(1);
    });

    it('centres the world on the axis with leftover space', () => {
        const bounds = {left: 0, top: 0, right: 200, bottom: 20};
        const projection = projectionFor(bounds, 200, 200);
        // World height (20) at scale 1 leaves 180px of canvas height spare;
        // half of that, 90px, is the top margin.
        expect(projection.offsetY).toBeCloseTo(90);
        expect(projection.offsetX).toBeCloseTo(0);
    });

    it('does not divide by zero on a degenerate (zero-area) world', () => {
        const bounds = {left: 5, top: 5, right: 5, bottom: 5};
        expect(() => projectionFor(bounds, 200, 200)).not.toThrow();
        const projection = projectionFor(bounds, 200, 200);
        expect(Number.isFinite(projection.scale)).toBe(true);
        expect(Number.isFinite(projection.offsetX)).toBe(true);
        expect(Number.isFinite(projection.offsetY)).toBe(true);
    });

    it('does not divide by zero on a zero-sized canvas', () => {
        const bounds = {left: -10, top: -10, right: 10, bottom: 10};
        expect(() => projectionFor(bounds, 0, 0)).not.toThrow();
    });
});

describe('project', () => {
    it('maps the bounds corners to the canvas corners when the aspect ratios match', () => {
        const bounds = {left: -100, top: -100, right: 100, bottom: 100};
        const projection = projectionFor(bounds, 200, 200);
        expect(project(projection, {x: -100, y: -100})).toEqual({x: 0, y: 0});
        expect(project(projection, {x: 100, y: 100})).toEqual({x: 200, y: 200});
    });

    it('never rounds a fractional position, such as a tile centre', () => {
        const bounds = {left: -100, top: -100, right: 100, bottom: 100};
        const projection = projectionFor(bounds, 200, 200);
        const p = project(projection, {x: -40.5, y: -48.5});
        expect(p.x).toBe(59.5);
        expect(p.y).toBe(51.5);
    });
});
