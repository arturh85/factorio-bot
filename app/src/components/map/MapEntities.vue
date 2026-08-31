<script setup lang="ts">
/**
 * Renders entities from `GET /api/v1/game/find-entities` as one `<rect>` per
 * entity, fit to a queried position + radius.
 *
 * SVG, not canvas: free hit-testing (the `<title>` below), CSS-able, and
 * assertable from a DOM test the way a canvas map could only be pixel-diffed.
 * This is right for the hundreds of entities a radius query returns and WRONG
 * for tens of thousands -- a whole-map view at that scale needs canvas, and
 * that is a different task. See `.superpowers/sdd/map-view/brief.md` rule 2.
 *
 * No pan or zoom: the viewBox is fixed to the queried circle, exactly once,
 * on every render. Rule 1 (Factorio's `y` increases downward, same as SVG's)
 * means this component never negates or swaps a `y` coordinate anywhere --
 * every value below is used exactly as `find-entities` reported it.
 */
import {computed} from 'vue';
import {FactorioEntity} from '@/api/types';
import {colorForEntityType} from '@/lib/entityColor';

const props = defineProps<{
  entities: FactorioEntity[];
  /** The centre of the queried area, in game-world coordinates. */
  center: {x: number; y: number};
  /** The queried radius, in game-world tiles. Fit to the viewport exactly, no padding logic to get wrong. */
  radius: number;
}>();

// Do NOT negate `center.y` here: Factorio's y already runs the same
// direction as SVG's, so `center.y - radius` is the correct top edge, not a
// bug waiting to be "fixed".
const viewBox = computed(() => {
  const minX = props.center.x - props.radius;
  const minY = props.center.y - props.radius;
  const size = props.radius * 2;
  return `${minX} ${minY} ${size} ${size}`;
});

interface EntityRect {
  key: string;
  name: string;
  fill: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

const rects = computed<EntityRect[]>(() => props.entities.map((entity, index) => {
  const {left_top, right_bottom} = entity.bounding_box;
  return {
    // Position/name are not guaranteed unique across a real query (two
    // different resource patches can share a name), so the render key
    // includes the index rather than risking a Vue key collision.
    key: `${entity.name}-${index}`,
    name: entity.name,
    fill: colorForEntityType(entity.entity_type),
    x: left_top.x,
    y: left_top.y,
    width: right_bottom.x - left_top.x,
    height: right_bottom.y - left_top.y
  };
}));

/**
 * A stroke width that stays visible regardless of how far zoomed the fixed
 * viewBox is -- `vector-effect="non-scaling-stroke"` on the rect achieves the
 * same thing without this, but a hairline-thin fill-only shape for a
 * sub-tile entity (an inserter's 0.3x0.3 box against a 100-tile radius) would
 * otherwise be invisible, so a small stroke is kept regardless.
 */
const STROKE_WIDTH = 0.05;
</script>

<template>
  <svg :viewBox="viewBox" preserveAspectRatio="xMidYMid meet" class="aspect-square w-full">
    <rect
      v-for="r in rects"
      :key="r.key"
      :data-testid="`entity-${r.name}`"
      :x="r.x"
      :y="r.y"
      :width="r.width"
      :height="r.height"
      :fill="r.fill"
      :stroke="r.fill"
      :stroke-width="STROKE_WIDTH"
      fill-opacity="0.75">
      <title>{{ r.name }}</title>
    </rect>
  </svg>
</template>
