import {clsx, type ClassValue} from 'clsx';
import {twMerge} from 'tailwind-merge';

/**
 * Join class names, letting the last conflicting Tailwind utility win.
 *
 * Components take a `class` prop and merge it over their own defaults with
 * this, so a caller writing `class="bg-success"` on a primary Button gets a
 * green button rather than two competing background rules whose winner
 * depends on stylesheet order.
 */
export function cn(...inputs: ClassValue[]): string {
    return twMerge(clsx(inputs));
}
