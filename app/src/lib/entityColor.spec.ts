import {describe, expect, it} from 'vitest';
import {colorForEntityType} from './entityColor';

describe('colorForEntityType', () => {
    it('is deterministic: the same type always gets the same colour', () => {
        expect(colorForEntityType('resource')).toBe(colorForEntityType('resource'));
    });

    it('gives different types different colours (not guaranteed universally, but true for these)', () => {
        expect(colorForEntityType('resource')).not.toBe(colorForEntityType('furnace'));
        expect(colorForEntityType('container')).not.toBe(colorForEntityType('inserter'));
    });

    it('returns a valid hsl() colour string', () => {
        expect(colorForEntityType('tree')).toMatch(/^hsl\(\d+(\.\d+)?, \d+%, \d+%\)$/);
    });

    it('handles the empty string without throwing', () => {
        expect(() => colorForEntityType('')).not.toThrow();
    });
});
