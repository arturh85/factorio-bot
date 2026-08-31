import type {Component} from 'vue';

/**
 * One sidebar entry.
 *
 * The type this replaces carried twelve optional fields inherited from the
 * admin template it came from (`items`, `command`, `url`, `badge`,
 * `separator`, `target`, `class`, `style`, `disabled`, …). No menu entry has
 * ever set any of them, and the recursive submenu component that read them is
 * deleted in the same commit as this change.
 */
export type MenuEntry = {
    label: string;
    /** A @lucide/vue icon component, rendered with <component :is>. */
    icon: Component;
    to: string;
};
