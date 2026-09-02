<script setup lang="ts">
/**
 * The run's whole-map view at the cursor: resource patches as outlined
 * polygons, placed entities as markers, bots as dots, and each bot's recent
 * trail as a polyline -- every one of them inspectable.
 *
 * SVG, not canvas. This reverses the earlier decision here, and the reason it
 * reverses is a measurement rather than a taste:
 *
 *   The canvas version drew one filled cell per entity, and a recorded map is
 *   overwhelmingly ore. Across the 18 runs in `workspace/runs` the largest
 *   holds 1141 entities of which 1122 are single resource tiles -- so the old
 *   argument ("thousands of entities, an SVG DOM does not redraw that
 *   cheaply") was true of the shapes it chose to draw, not of the map. Those
 *   1122 tiles form 3 contiguous patches. `@/lib/resourcePatches.ts`
 *   aggregates them before anything is drawn, and the worst run in the
 *   archive comes out at about 30 SVG elements; the run this was developed
 *   against goes from 881 entities to 29 features, its two ore patches
 *   becoming two `<path>`s of 46 points each.
 *
 *   That aggregation is not a rendering trick that happens to help
 *   readability. It IS the readability fix: 856 identical cells were an
 *   undifferentiated blob, and a patch is the thing a bot's trail visibly
 *   travels *to*, the thing a tooltip can name, and the thing a legend row
 *   can count.
 *
 * The second consequence is worth as much as the first: a canvas cannot be
 * asserted on in this test suite at all -- jsdom's `getContext('2d')` returns
 * `null` without the optional native `canvas` package, so the old spec here
 * could only check that a `<canvas>` element existed. `MapPanel.spec.ts` now
 * asserts the actual rendered geometry, the tooltip text and the legend rows.
 *
 * `@/lib/mapProjection.ts` still owns the transform, and still earns its
 * separate file: it is applied here as ONE SVG `transform` on a group, so
 * every coordinate in the markup below is a game-world coordinate, unrounded
 * and unnegated. Factorio's `y` grows downward exactly as SVG's does; nothing
 * here flips it.
 */
import {computed, ref} from 'vue';
import {Bounds, EntitySnapshot, MapRecord, Position} from '@/api/types';
import {BotDot, MapFeature, buildMapFeatures, legendFor} from '@/lib/mapFeatures';
import {project, projectionFor} from '@/lib/mapProjection';
import MapLegend from '@/components/map/MapLegend.vue';

const props = withDefaults(
    defineProps<{
        entities: EntitySnapshot[];
        bots: BotDot[];
        /** Per bot id, its recent positions, oldest first. */
        trail: Record<number, Position[]>;
        bounds: Bounds | null;
        /**
         * The run's `map.jsonl` lines, so an entity's tooltip can say which
         * bot placed it and at what tick.
         *
         * Optional, and the map is complete without it: every entity then
         * reads "no placement recorded", which is exactly what the caller
         * knows. Pass `runsStore.map` to get the attribution.
         */
        records?: MapRecord[];
    }>(),
    {records: () => []}
);

/**
 * Fixed logical viewport. The projection preserves the world's own aspect
 * ratio inside this square regardless of its shape, so this does not need to
 * track the world's dimensions -- see `projectionFor`.
 */
const VIEW_SIZE = 480;

const projection = computed(() =>
    projectionFor(props.bounds ?? {left: 0, top: 0, right: 1, bottom: 1}, VIEW_SIZE, VIEW_SIZE)
);

/**
 * One viewport pixel, in world units.
 *
 * Everything inside the projected group is measured in game tiles, so a
 * marker that should stay a constant size on screen has to be divided by the
 * scale. Strokes use `vector-effect="non-scaling-stroke"` instead and need no
 * such correction.
 */
const worldPixel = computed(() => 1 / projection.value.scale);

/** A placed entity's marker: one tile, or 7 screen pixels, whichever is larger. */
const markerSide = computed(() => Math.max(1, 7 * worldPixel.value));
const botRadius = computed(() => 6 * worldPixel.value);
/**
 * An invisible fat stroke over each trail. A 2px line is a miserable target
 * for a fingertip, and this is the whole reason trails are tappable at all.
 */
const trailHitWidth = computed(() => 16 * worldPixel.value);
/** 11 screen pixels, whatever the world's size, for the on-map patch names. */
const patchLabelSize = computed(() => 11 * worldPixel.value);

const features = computed<MapFeature[]>(() =>
    buildMapFeatures({
        entities: props.entities,
        bots: props.bots,
        trail: props.trail,
        records: props.records
    })
);

const legend = computed(() => legendFor(features.value));

const patches = computed(() => features.value.filter((f) => f.kind === 'patch'));
const entityMarkers = computed(() => features.value.filter((f) => f.kind === 'entity'));
const trails = computed(() => features.value.filter((f) => f.kind === 'trail'));
const botDots = computed(() => features.value.filter((f) => f.kind === 'bot'));

/**
 * The inspected feature.
 *
 * Hover is only one of three ways in, and the other two are not a courtesy:
 * a hover-only tooltip is unreachable from a keyboard and unreachable on a
 * touch screen, which between them is most of the ways this map gets looked
 * at. Every shape is a `tabindex="0"` `role="button"`, so Tab reaches it and
 * a tap activates it, and the panel below is `aria-live` so the text is
 * announced rather than merely displayed.
 */
const activeId = ref<string | null>(null);
/**
 * Tracked separately from `activeId` so a pointer leaving the map does not
 * silently discard what the keyboard is pointing at.
 */
const focusedId = ref<string | null>(null);

const active = computed(() => features.value.find((feature) => feature.id === activeId.value) ?? null);

function show(id: string): void {
    activeId.value = id;
}

function onFocus(id: string): void {
    focusedId.value = id;
    activeId.value = id;
}

function onBlur(id: string): void {
    if (focusedId.value !== id) return;
    focusedId.value = null;
    activeId.value = null;
}

function onPointerLeave(): void {
    // Keyboard focus outranks the pointer: moving the mouse off the map while
    // a shape is focused must leave that shape's tooltip up.
    if (focusedId.value === null) activeId.value = null;
}

function dismiss(): void {
    activeId.value = null;
    focusedId.value = null;
}

/**
 * Where the tooltip sits, as percentages of the viewport, so it tracks the
 * SVG through whatever width the page gives it.
 *
 * Clamped away from the edges rather than measured and flipped: the box is
 * centred on its anchor, and an anchor at x=0 would otherwise hang half of
 * it outside the panel. It moves below the anchor for features in the top
 * third, for the same reason.
 */
const tooltipStyle = computed(() => {
    if (!active.value) return {};
    const point = project(projection.value, active.value.anchor);
    const x = Math.min(85, Math.max(15, (point.x / VIEW_SIZE) * 100));
    const y = Math.min(100, Math.max(0, (point.y / VIEW_SIZE) * 100));
    return {
        left: `${x}%`,
        top: `${y}%`,
        transform: y < 33 ? 'translate(-50%, 1.25rem)' : 'translate(-50%, calc(-100% - 1.25rem))'
    };
});
</script>

<template>
    <div class="map-panel">
        <!-- A blank canvas and "nothing was built yet" look identical on
             screen, and one of them is a bug -- say which this is. -->
        <p v-if="!bounds" class="map-panel__empty">Nothing placed yet</p>
        <template v-else>
            <div class="map-panel__viewport" @keydown.esc="dismiss">
                <svg
                    :viewBox="`0 0 ${VIEW_SIZE} ${VIEW_SIZE}`"
                    class="map-panel__svg"
                    role="group"
                    aria-label="Map of the run at the current tick"
                    data-testid="map-svg"
                    @pointerleave="onPointerLeave">
                    <rect :width="VIEW_SIZE" :height="VIEW_SIZE" class="map-panel__ground" />
                    <g :transform="`translate(${projection.offsetX} ${projection.offsetY}) scale(${projection.scale})`">
                        <!-- The origin. Trails and patches are only readable
                             as "north-west of spawn" if spawn is on the map. -->
                        <g class="map-panel__origin" aria-hidden="true">
                            <line
                                :x1="bounds.left"
                                :x2="bounds.right"
                                y1="0"
                                y2="0"
                                vector-effect="non-scaling-stroke" />
                            <line
                                x1="0"
                                x2="0"
                                :y1="bounds.top"
                                :y2="bounds.bottom"
                                vector-effect="non-scaling-stroke" />
                        </g>

                        <path
                            v-for="patch in patches"
                            :key="patch.id"
                            :d="patch.path"
                            :fill="patch.color"
                            :stroke="patch.color"
                            :class="['map-panel__patch', {'is-active': patch.id === activeId}]"
                            :data-testid="`map-feature-${patch.id}`"
                            fill-rule="nonzero"
                            vector-effect="non-scaling-stroke"
                            role="button"
                            tabindex="0"
                            :aria-label="`${patch.title}, ${patch.details.join(', ')}`"
                            @pointerenter="show(patch.id)"
                            @click="show(patch.id)"
                            @focus="onFocus(patch.id)"
                            @blur="onBlur(patch.id)">
                            <title>{{ patch.title }} — {{ patch.details.join(' · ') }}</title>
                        </path>

                        <!-- Two polylines per bot, not one. A single line
                             cannot be both a hairline you can read and a
                             target a fingertip can hit: drawing one fat
                             translucent line instead turns every trail into a
                             smear across the map, which is the thing this
                             panel exists to stop doing. The wide one is
                             invisible and carries `pointer-events="stroke"`,
                             so it is hit-tested regardless of paint. -->
                        <g
                            v-for="trail_ in trails"
                            :key="trail_.id"
                            :class="['map-panel__trail', {'is-active': trail_.id === activeId}]"
                            :data-testid="`map-feature-${trail_.id}`"
                            role="button"
                            tabindex="0"
                            :aria-label="`${trail_.title}, ${trail_.details.join(', ')}`"
                            @pointerenter="show(trail_.id)"
                            @click="show(trail_.id)"
                            @focus="onFocus(trail_.id)"
                            @blur="onBlur(trail_.id)">
                            <title>{{ trail_.title }} — {{ trail_.details.join(' · ') }}</title>
                            <polyline
                                class="map-panel__trail-hit"
                                :points="trail_.points.map((p) => `${p.x},${p.y}`).join(' ')"
                                :stroke-width="trailHitWidth"
                                fill="none"
                                stroke="none"
                                pointer-events="stroke" />
                            <polyline
                                class="map-panel__trail-line"
                                :points="trail_.points.map((p) => `${p.x},${p.y}`).join(' ')"
                                :stroke="trail_.color"
                                fill="none"
                                vector-effect="non-scaling-stroke"
                                pointer-events="none" />
                        </g>

                        <rect
                            v-for="marker in entityMarkers"
                            :key="marker.id"
                            :x="marker.position.x - markerSide / 2"
                            :y="marker.position.y - markerSide / 2"
                            :width="markerSide"
                            :height="markerSide"
                            :fill="marker.color"
                            :class="['map-panel__entity', {'is-active': marker.id === activeId}]"
                            :data-testid="`map-feature-${marker.id}`"
                            vector-effect="non-scaling-stroke"
                            role="button"
                            tabindex="0"
                            :aria-label="`${marker.title}, ${marker.details.join(', ')}`"
                            @pointerenter="show(marker.id)"
                            @click="show(marker.id)"
                            @focus="onFocus(marker.id)"
                            @blur="onBlur(marker.id)">
                            <!-- A marker at the position the record reports,
                                 NOT the entity's footprint: `EntitySnapshot`
                                 carries no bounding box, so a 2x2 furnace and
                                 a 1x1 inserter are the same square here. -->
                            <title>{{ marker.title }} — {{ marker.details.join(' · ') }}</title>
                        </rect>

                        <circle
                            v-for="bot in botDots"
                            :key="bot.id"
                            :cx="bot.position.x"
                            :cy="bot.position.y"
                            :r="botRadius"
                            :fill="bot.color"
                            :class="['map-panel__bot', {'is-active': bot.id === activeId}]"
                            :data-testid="`map-feature-${bot.id}`"
                            vector-effect="non-scaling-stroke"
                            role="button"
                            tabindex="0"
                            :aria-label="`${bot.title}, ${bot.details.join(', ')}`"
                            @pointerenter="show(bot.id)"
                            @click="show(bot.id)"
                            @focus="onFocus(bot.id)"
                            @blur="onBlur(bot.id)">
                            <title>{{ bot.title }} — {{ bot.details.join(' · ') }}</title>
                        </circle>

                        <!-- The patch names, on the map itself.
                             "without even a legend it's not comprehensible"
                             was the complaint, and a legend alone still makes
                             you look away from the map and match a colour.
                             There are only ever a handful of patches, so the
                             map can just say which is which. Not interactive
                             and not announced: the patch underneath already
                             carries the same words as its accessible name. -->
                        <text
                            v-for="patch in patches"
                            :key="`${patch.id}-label`"
                            :x="patch.anchor.x"
                            :y="patch.anchor.y"
                            :font-size="patchLabelSize"
                            class="map-panel__patch-label"
                            text-anchor="middle"
                            dominant-baseline="middle"
                            pointer-events="none"
                            aria-hidden="true">
                            {{ patch.title.replace(' patch', '') }}
                        </text>
                    </g>
                </svg>

                <!-- Announced, not just shown: this is the only place the
                     detail exists, so a screen reader has to get it too. -->
                <div
                    v-if="active"
                    class="map-panel__tooltip"
                    :style="tooltipStyle"
                    role="status"
                    aria-live="polite"
                    data-testid="map-tooltip">
                    <span class="map-panel__tooltip-title">{{ active.title }}</span>
                    <span v-for="line in active.details" :key="line" class="map-panel__tooltip-line">{{ line }}</span>
                </div>
            </div>

            <MapLegend :entries="legend" />
            <!-- The affordance is invisible otherwise: nothing about an SVG
                 polygon says it can be focused. -->
            <p class="map-panel__hint">
                Hover, tap or Tab to a shape for detail; Esc dismisses it.
            </p>
        </template>
    </div>
</template>

<style scoped>
.map-panel__viewport {
    position: relative;
    width: 100%;
    max-width: 480px;
}
.map-panel__svg {
    display: block;
    width: 100%;
    aspect-ratio: 1 / 1;
    border: 1px solid var(--surface-border, #ccc);
}
.map-panel__ground {
    fill: #12161c;
}
.map-panel__origin line {
    stroke: rgba(255, 255, 255, 0.14);
    stroke-width: 1;
    stroke-dasharray: 4 4;
}

.map-panel__patch {
    fill-opacity: 0.32;
    stroke-width: 1.5;
    cursor: pointer;
}
.map-panel__patch.is-active,
.map-panel__patch:focus-visible {
    fill-opacity: 0.55;
    stroke-width: 3;
}

.map-panel__trail {
    cursor: pointer;
}
.map-panel__trail-line {
    stroke-width: 2;
    stroke-opacity: 0.85;
    stroke-linejoin: round;
    stroke-linecap: round;
}
.map-panel__trail.is-active .map-panel__trail-line,
.map-panel__trail:focus-visible .map-panel__trail-line {
    stroke-width: 4;
    stroke-opacity: 1;
}

.map-panel__patch-label {
    fill: #f2f5f8;
    /* A halo, so a name stays readable over its own patch AND over the dark
       ground where a patch's centre of mass falls in one of its holes. */
    paint-order: stroke;
    stroke: rgba(8, 11, 15, 0.85);
    stroke-width: 3;
    vector-effect: non-scaling-stroke;
    font-family: inherit;
    letter-spacing: 0.02em;
}

.map-panel__entity {
    stroke: rgba(255, 255, 255, 0.5);
    stroke-width: 1;
    cursor: pointer;
}
.map-panel__entity.is-active,
.map-panel__entity:focus-visible {
    stroke: #ffffff;
    stroke-width: 2.5;
}

.map-panel__bot {
    stroke: #ffffff;
    stroke-width: 2;
    cursor: pointer;
}
.map-panel__bot.is-active,
.map-panel__bot:focus-visible {
    stroke-width: 4;
}

/* `outline` on an SVG child is unreliable across engines; the `is-active` and
   `:focus-visible` rules above are the focus indicator, and they change
   stroke weight as well as opacity so the cue is not colour-only. */
.map-panel__svg :focus {
    outline: none;
}

.map-panel__tooltip {
    position: absolute;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    max-width: 22rem;
    padding: 0.4rem 0.6rem;
    border-radius: 3px;
    background: rgba(12, 16, 22, 0.94);
    color: #f2f5f8;
    font-size: 0.75rem;
    line-height: 1.35;
    pointer-events: none;
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.45);
}
.map-panel__tooltip-title {
    font-weight: 600;
}
.map-panel__tooltip-line {
    font-family: monospace;
    opacity: 0.85;
}

.map-panel__hint {
    margin: 0.35rem 0 0;
    font-size: 0.72rem;
    opacity: 0.55;
}
.map-panel__empty {
    font-size: 0.85rem;
    opacity: 0.6;
    margin: 0;
}
</style>
