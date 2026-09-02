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

/**
 * Saturation and lightness are varied too, not just hue.
 *
 * Hue alone gives 360 buckets drawn at random by a hash, and a real run's map
 * legend holds around thirty rows -- `crash-site-spaceship` and `iron-ore`
 * landed on hue 114 and hue 117, two greens nobody could tell apart in a
 * legend. Three saturations and three lightnesses multiply the space by nine
 * and, more usefully, separate a near-collision along an axis the eye reads
 * differently from hue.
 */
const SATURATIONS = [52, 68, 84];
const LIGHTNESSES = [44, 56, 68];

function hashString(value: string): number {
    let hash = 0;
    for (let i = 0; i < value.length; i++) {
        hash = (hash * 31 + value.charCodeAt(i)) | 0;
    }
    return Math.abs(hash);
}

/**
 * Deterministic: the same `entityType` always returns the same colour.
 *
 * Salted rather than sliced: the low bits of a `* 31 +` hash track its high
 * bits closely enough that two names one character apart would land on the
 * same saturation AND the same lightness, which is the case this exists to
 * break up.
 */
export function colorForEntityType(entityType: string): string {
    const hue = (hashString(entityType) * HUE_STEP) % 360;
    const saturation = SATURATIONS[hashString(`${entityType}#s`) % SATURATIONS.length];
    const lightness = LIGHTNESSES[hashString(`${entityType}#l`) % LIGHTNESSES.length];
    return `hsl(${hue}, ${saturation}%, ${lightness}%)`;
}
