/**
 * Light, dark, or follow the OS. Three states, not two: the un-stamped
 * document is the default and only `prefers-color-scheme` separates light
 * from dark there; an explicit choice stamps `data-theme` so it wins over the
 * OS in both directions. Stored per viewer; storage may be unavailable, so
 * every read and write is guarded.
 */
import {ref, Ref} from 'vue';

export type Theme = 'light' | 'dark' | 'system';
export const THEME_KEY = 'factorio-bot.theme';

function readStored(): Theme {
    try {
        const v = localStorage.getItem(THEME_KEY);
        return v === 'light' || v === 'dark' ? v : 'system';
    } catch {
        return 'system';
    }
}

export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void {
    if (theme === 'system') delete root.dataset.theme;
    else root.dataset.theme = theme;
}

const theme: Ref<Theme> = ref('system');
let initialised = false;

export function useTheme() {
    if (!initialised) {
        initialised = true;
        theme.value = readStored();
        if (typeof document !== 'undefined') applyTheme(theme.value);
    }
    function set(next: Theme) {
        theme.value = next;
        applyTheme(next);
        try {
            if (next === 'system') localStorage.removeItem(THEME_KEY);
            else localStorage.setItem(THEME_KEY, next);
        } catch {
            // storage unavailable: the choice still applies for this page
        }
    }
    function cycle() {
        set(theme.value === 'system' ? 'dark' : theme.value === 'dark' ? 'light' : 'system');
    }
    return {theme, set, cycle};
}

/** Test seam: forget the module-level state between specs. */
export function resetThemeForTests() {
    initialised = false;
    theme.value = 'system';
}
