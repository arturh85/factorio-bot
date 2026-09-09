<!-- app/src/components/run/FlowPanel.vue -->
<script setup lang="ts">
/**
 * The flow graph at the cursor: nodes at their real map position, edges as
 * lines whose width is the modelled rate. Each node's `<title>` carries the
 * claim in words -- model rate, measured rate, and the gap between them,
 * since a node with no measured reading must say so rather than draw a
 * silent zero.
 */
import {computed} from 'vue';
import {FlowExport, Sample} from '@/api/types';
import {flowView, FlowViewNode} from '@/lib/flowJoin';
import {projectionFor} from '@/lib/mapProjection';

const props = defineProps<{
    flow: FlowExport | null;
    flowError: string | null;
    samples: Sample[];
    cursor: number;
}>();

const VIEW_SIZE = 480;
const MARGIN = 4;

const view = computed(() => (props.flow === null ? null : flowView(props.flow, props.samples, props.cursor)));

const bounds = computed(() => {
    const nodes = props.flow?.nodes ?? [];
    if (nodes.length === 0) return {left: 0, top: 0, right: 1, bottom: 1};
    const xs = nodes.map((n) => n.position.x);
    const ys = nodes.map((n) => n.position.y);
    return {
        left: Math.min(...xs) - MARGIN, right: Math.max(...xs) + MARGIN,
        top: Math.min(...ys) - MARGIN, bottom: Math.max(...ys) + MARGIN
    };
});

const projection = computed(() => projectionFor(bounds.value, VIEW_SIZE, VIEW_SIZE));

function screenX(x: number): number { return x * projection.value.scale + projection.value.offsetX; }
function screenY(y: number): number { return y * projection.value.scale + projection.value.offsetY; }

const nodeById = computed(() => new Map((view.value?.nodes ?? []).map((n) => [n.id, n])));

function formatRate(perMinute: number | null): string {
    return perMinute === null ? 'not measured' : `${perMinute.toFixed(1)}/min`;
}

function nodeTitle(n: FlowViewNode): string {
    const model = n.primaryItem === null ? 'no outgoing flow' : `model ${n.primaryItem} ${formatRate(n.modelPerMinute)}`;
    const measured = `measured ${formatRate(n.measuredPerMinute)}`;
    const gap = n.gap === null ? '' : ` · gap ${n.gap.toFixed(2)}x`;
    return `${n.name} (${n.kind})\n${model}\n${measured}${gap}`;
}

/** 1 to 6 screen px, scaled by the edge's own modelled rate against the
 *  busiest edge in this keyframe -- so one keyframe's edges are comparable
 *  to each other, never to another keyframe's absolute numbers. */
const strokeWidth = computed(() => {
    const rates = (view.value?.edges ?? []).map((e) => e.totalPerMinute);
    const max = Math.max(1, ...rates);
    return (rate: number) => 1 + 5 * (rate / max);
});
</script>

<template>
  <div class="p-3">
    <p v-if="flowError !== null" class="text-sm text-ink-muted">{{ flowError }}</p>
    <p v-else-if="flow === null" class="text-sm text-ink-muted">no flow keyframe recorded yet</p>
    <svg v-else :viewBox="`0 0 ${VIEW_SIZE} ${VIEW_SIZE}`" class="w-full" role="img" aria-label="flow graph at cursor">
      <line v-for="e in view!.edges" :key="`${e.from}-${e.to}`" class="flow-edge"
            :x1="screenX(nodeById.get(e.from)?.position.x ?? 0)" :y1="screenY(nodeById.get(e.from)?.position.y ?? 0)"
            :x2="screenX(nodeById.get(e.to)?.position.x ?? 0)" :y2="screenY(nodeById.get(e.to)?.position.y ?? 0)"
            :stroke-width="strokeWidth(e.totalPerMinute)" stroke="var(--color-ink-muted)" stroke-linecap="round" />
      <g v-for="n in view!.nodes" :key="n.id">
        <circle class="flow-node" :cx="screenX(n.position.x)" :cy="screenY(n.position.y)" r="5"
                :fill="`var(--color-status-${n.status})`" stroke="var(--color-surface)" stroke-width="1">
          <title>{{ nodeTitle(n) }}</title>
        </circle>
      </g>
    </svg>
  </div>
</template>
