import {describe, expect, it} from 'vitest';
import {cn} from './utils';

describe('cn', () => {
    it('lets the later padding utility win instead of emitting both', () => {
        // Plain concatenation yields 'p-2 p-4', where the winner depends on
        // the order Tailwind happens to emit the two rules in. This is the
        // whole reason tailwind-merge is a dependency.
        expect(cn('p-2', 'p-4')).toBe('p-4');
    });

    it('keeps utilities that do not conflict', () => {
        expect(cn('rounded-card', 'bg-card')).toBe('rounded-card bg-card');
    });

    it('drops falsy entries from a conditional class list', () => {
        expect(cn('bg-card', false, undefined, null, 'p-4')).toBe('bg-card p-4');
    });
});
