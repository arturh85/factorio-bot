<script setup lang="ts">
/**
 * The map's key: one row per thing the map draws, with the swatch it is
 * actually drawn in.
 *
 * Derived from the rendered features rather than declared here (see
 * `legendFor` in `@/lib/mapFeatures.ts`): the palette in `entityColor.ts` is
 * a hash of the entity name, so a hand-written legend would be a second
 * source of truth that goes stale the first time an unlisted entity appears
 * on a map. The swatch's `background` and the shape's `fill` come from the
 * same call for the same string, which is the only reason a legend is worth
 * trusting.
 *
 * The `shape` is drawn, not named: a patch reads as a translucent block with
 * an outline exactly as it does on the map, an entity as a solid marker, a
 * bot as a dot. Matching the map by colour alone would fail anyone who cannot
 * separate two hues.
 */
import {LegendEntry} from '@/lib/mapFeatures';

defineProps<{entries: LegendEntry[]}>();
</script>

<template>
    <!-- A definition list, because that is what this is: a term and what it
         means. Screen readers get the pairing for free. -->
    <dl v-if="entries.length > 0" class="legend" data-testid="map-legend">
        <div v-for="entry in entries" :key="entry.key" class="legend__row" :data-testid="`legend-${entry.key}`">
            <dt class="legend__term">
                <span
                    class="legend__swatch"
                    :class="`legend__swatch--${entry.shape}`"
                    :style="{'--swatch': entry.color}"
                    aria-hidden="true"
                />
                <span class="legend__label">{{ entry.label }}</span>
            </dt>
            <dd class="legend__detail">{{ entry.detail }}</dd>
        </div>
    </dl>
</template>

<style scoped>
.legend {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
    gap: 0.1rem 1rem;
    margin: 0.5rem 0 0;
    font-size: 0.78rem;
}
.legend__row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
    min-width: 0;
}
.legend__term {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    min-width: 0;
}
.legend__label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.legend__detail {
    margin: 0;
    font-family: monospace;
    opacity: 0.6;
    white-space: nowrap;
}
.legend__swatch {
    flex: none;
    width: 0.85rem;
    height: 0.85rem;
    background: var(--swatch);
}
/* Same three appearances the map itself uses, so the key is legible without
   relying on colour discrimination alone. */
.legend__swatch--patch {
    background: color-mix(in srgb, var(--swatch) 40%, transparent);
    border: 1px solid var(--swatch);
}
.legend__swatch--entity {
    border: 1px solid rgba(255, 255, 255, 0.55);
}
.legend__swatch--bot {
    border-radius: 50%;
    border: 2px solid #ffffff;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.5);
}
</style>
