// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {applyTheme, resetThemeForTests, THEME_KEY, useTheme} from './useTheme';

describe('useTheme', () => {
    beforeEach(() => {
        localStorage.clear();
        delete document.documentElement.dataset.theme;
        resetThemeForTests();
    });
    afterEach(() => {
        vi.restoreAllMocks();
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
    it('applyTheme survives storage that throws, on a detached root', () => {
        const root = document.createElement('div');
        expect(() => applyTheme('light', root)).not.toThrow();
        expect(root.dataset.theme).toBe('light');
    });
    it('useTheme survives storage that throws on every call', () => {
        vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
            throw new Error('blocked');
        });
        vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
            throw new Error('blocked');
        });
        vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => {
            throw new Error('blocked');
        });
        resetThemeForTests();
        const t = useTheme();
        expect(t.theme.value).toBe('system');
        expect(document.documentElement.dataset.theme).toBeUndefined();
        expect(() => t.set('dark')).not.toThrow();
        expect(document.documentElement.dataset.theme).toBe('dark');
    });
});
