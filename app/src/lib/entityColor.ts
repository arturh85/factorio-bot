/**
 * A colour for one `FactorioEntity.entity_type`, for the map view.
 *
 * Deliberately unrelated to `crates/core/src/draw.rs`'s resource palette for
 * its PNG dump: copying those colours into TypeScript would create a mirror
 * nothing keeps in sync, and this file must not claim the two renderings
 * match (see `.superpowers/sdd/map-view/brief.md` rule 5). Instead, every
 * `entity_type` string gets a colour deterministically, so the same type is
 * always the same colour across renders and across sessions without a table
 * to maintain or a new type ever falling through to an "unknown" grey.
 */

/** A small, evenly-spaced set of hues, so adjacent types are visually distinct. */
const HUE_STEP = 47;

function hashString(value: string): number {
    let hash = 0;
    for (let i = 0; i < value.length; i++) {
        hash = (hash * 31 + value.charCodeAt(i)) | 0;
    }
    return Math.abs(hash);
}

/** Deterministic: the same `entityType` always returns the same colour. */
export function colorForEntityType(entityType: string): string {
    const hue = (hashString(entityType) * HUE_STEP) % 360;
    return `hsl(${hue}, 65%, 50%)`;
}
