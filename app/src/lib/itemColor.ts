/** A tracked item's series colour. Fixed by item, never by rank in the list. */
const ITEM_TOKENS: Record<string, string> = {
    'iron-plate': 'iron',
    'copper-plate': 'copper',
    'electronic-circuit': 'circuit',
    'iron-gear-wheel': 'gear',
    'automation-science-pack': 'red',
    'logistic-science-pack': 'green'
};

export function itemColor(item: string): string {
    const token = ITEM_TOKENS[item];
    return token === undefined ? 'var(--color-ink-muted)' : `var(--color-item-${token})`;
}
