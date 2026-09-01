<script setup lang="ts">
/**
 * The run's whole-map view at the cursor: entities as filled cells, bots as
 * dots, and each bot's recent trail as a polyline.
 *
 * Canvas, not SVG (contrast `components/map/MapEntities.vue`, which draws a
 * bounded radius query and stays SVG for free hit-testing): a run's map can
 * hold thousands of entities across a run's lifetime, and this view is
 * redrawn on every cursor move, which a large SVG DOM does not do cheaply.
 *
 * The coordinate transform itself lives in `@/lib/mapProjection.ts`, not
 * here, and that split is deliberate rather than tidiness: jsdom's
 * `HTMLCanvasElement.getContext('2d')` returns `null` without the optional
 * native `canvas` package, so nothing this component draws can be asserted
 * on in a test -- only the transform's numbers can. `watchEffect` below
 * tolerates that null context rather than throwing.
 */
import {ref, watchEffect} from 'vue';
import {Bounds, EntitySnapshot, Position} from '@/api/types';
import {colorForEntityType} from '@/lib/entityColor';
import {project, projectionFor} from '@/lib/mapProjection';

/** Enough to place a bot dot on top of; a full `BotSample` is not required. */
interface BotDot {
    id: number;
    position: Position;
}

const props = defineProps<{
    entities: EntitySnapshot[];
    bots: BotDot[];
    /** Per bot id, its recent positions, oldest first. */
    trail: Record<number, Position[]>;
    bounds: Bounds | null;
}>();

// Fixed logical pixel size. The projection preserves the world's own aspect
// ratio inside this square regardless of its shape, so this does not need to
// track the world's dimensions -- see `projectionFor`.
const CANVAS_SIZE = 480;

const canvasRef = ref<HTMLCanvasElement | null>(null);

watchEffect(() => {
    const canvas = canvasRef.value;
    const bounds = props.bounds;
    if (!canvas || !bounds) return;

    const ctx = canvas.getContext('2d');
    // jsdom has no real 2D canvas context without the optional `canvas`
    // package -- draw only when one is actually available.
    if (!ctx) return;

    ctx.clearRect(0, 0, canvas.width, canvas.height);
    const projection = projectionFor(bounds, canvas.width, canvas.height);

    // Entities first, as filled cells, so bots and trails draw on top of them.
    const cell = Math.max(2, projection.scale);
    for (const entity of props.entities) {
        const p = project(projection, entity.position);
        ctx.fillStyle = colorForEntityType(entity.name);
        ctx.fillRect(p.x - cell / 2, p.y - cell / 2, cell, cell);
    }

    // Each bot's trail as a polyline.
    for (const [id, positions] of Object.entries(props.trail)) {
        if (positions.length < 2) continue;
        ctx.strokeStyle = colorForEntityType(`bot-${id}`);
        ctx.lineWidth = 1;
        ctx.beginPath();
        positions.forEach((position, index) => {
            const p = project(projection, position);
            if (index === 0) ctx.moveTo(p.x, p.y);
            else ctx.lineTo(p.x, p.y);
        });
        ctx.stroke();
    }

    // Bot dots on top of everything.
    for (const bot of props.bots) {
        const p = project(projection, bot.position);
        ctx.beginPath();
        ctx.arc(p.x, p.y, 4, 0, Math.PI * 2);
        ctx.fillStyle = '#ffffff';
        ctx.fill();
        ctx.strokeStyle = '#111111';
        ctx.stroke();
    }
});
</script>

<template>
    <div class="map-panel">
        <!-- A blank canvas and "nothing was built yet" look identical on
             screen, and one of them is a bug -- say which this is. -->
        <p v-if="!bounds" class="map-panel__empty">Nothing placed yet</p>
        <canvas
            v-else
            ref="canvasRef"
            :width="CANVAS_SIZE"
            :height="CANVAS_SIZE"
            class="map-panel__canvas"
        />
    </div>
</template>

<style scoped>
.map-panel__canvas {
    display: block;
    width: 100%;
    max-width: 480px;
    aspect-ratio: 1 / 1;
    border: 1px solid var(--surface-border, #ccc);
    background: #1a1a1a;
}
.map-panel__empty {
    font-size: 0.85rem;
    opacity: 0.6;
    margin: 0;
}
</style>
