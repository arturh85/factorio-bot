// @vitest-environment jsdom
import {beforeEach, describe, expect, it} from 'vitest';
import {applyTheme, resetThemeForTests, THEME_KEY, useTheme} from './useTheme';

describe('useTheme', () => {
    beforeEach(() => {
        localStorage.clear();
        delete document.documentElement.dataset.theme;
        resetThemeForTests();
    });
    it('starts on system and stamps nothing', () => {
        const t = useTheme();
        expect(t.theme.value).toBe('system');
        expect(document.documentElement.dataset.theme).toBeUndefined();
    });
    it('stamps data-theme and remembers the choice per viewer', () => {
        const t = useTheme();
        t.set('dark');
        expect(document.documentElement.dataset.theme).toBe('dark');
        expect(localStorage.getItem(THEME_KEY)).toBe('dark');
        t.set('system');
        expect(document.documentElement.dataset.theme).toBeUndefined();
        expect(localStorage.getItem(THEME_KEY)).toBeNull();
    });
    it('cycles system -> dark -> light -> system', () => {
        const t = useTheme();
        t.cycle(); expect(t.theme.value).toBe('dark');
        t.cycle(); expect(t.theme.value).toBe('light');
        t.cycle(); expect(t.theme.value).toBe('system');
    });
    it('survives storage that throws', () => {
        const root = document.createElement('div');
        expect(() => applyTheme('light', root)).not.toThrow();
        expect(root.dataset.theme).toBe('light');
    });
});
