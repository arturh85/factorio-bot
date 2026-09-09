import {describe, expect, it} from 'vitest';
import {itemColor} from './itemColor';

describe('itemColor', () => {
    it('is fixed by item', () => {
        expect(itemColor('iron-plate')).toBe('var(--color-item-iron)');
        expect(itemColor('automation-science-pack')).toBe('var(--color-item-red)');
    });
    it('falls back to muted ink for an unlisted item', () => {
        expect(itemColor('stone-brick')).toBe('var(--color-ink-muted)');
    });
});
