<script setup lang="ts">
/**
 * Blueprint Library — visualisation of reusable module designs.
 *
 * Shows each module design as a card with an SVG plan view, entity list,
 * ports, and operating contract. This static page serves hardcoded designs
 * extracted from the planner; a future `/api/v1/modules/designs` endpoint
 * will supply them dynamically.
 */
import {colorForEntityType} from '@/lib/entityColor';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface Offset {
  half_x: number;
  half_y: number;
}

interface Part {
  role: string;
  entity: string;
  offset: Offset;
  direction: number;
  recipe: string | null;
  half_size: Offset;
}

interface Rate {
  numerator: number;
  ticks: number;
}

interface Port {
  id: string;
  mode: string;
  item: string;
  offset: Offset;
  direction: number;
}

interface OperatingContract {
  inputs: Record<string, Rate>;
  outputs: Record<string, Rate>;
  power_watts: number;
  fuel_per_tick: Record<string, Rate>;
  startup_latency_ticks: number;
  startup_items: Record<string, number>;
  required_research: string[];
  required_surface: string;
}

interface ModuleDesign {
  schema: number;
  id: string;
  family: string;
  parameters: {item: string; with_pole: boolean; labs: number};
  parts: Part[];
  ports: Port[];
  bill: Record<string, number>;
  operation: OperatingContract;
}

// ---------------------------------------------------------------------------
// Design state — fetched from API with hardcoded fallback
// ---------------------------------------------------------------------------

import {ref} from 'vue';

const designs = ref<ModuleDesign[]>([]);

/** Per-design tile count keyed by design id. */
const tileCount = ref<Record<string, number>>({});

// Fetch designs from API — no fallback data.
fetch('/api/v1/modules/designs')
  .then(r => r.ok ? r.json() : Promise.reject(r.status))
  .then(data => { designs.value = data.designs; })
  .catch(() => { /* API unavailable — designs stay empty */ });

// ---------------------------------------------------------------------------
// SVG helpers
// ---------------------------------------------------------------------------

/** Compute bounding box of all parts in half-tile units. */

function entityHalfSize(part: Part): {hw: number, hh: number} {
  return {hw: part.half_size.half_x, hh: part.half_size.half_y};
}



function svgArrowPoints(dir: number, hw: number, hh: number, cx: number, cy: number): string {
  // Arrow triangle inside the entity, pointing in the facing direction.
  // Base is at the entity centre, tip at ~60% toward the facing edge.
  const tip = 0.6;
  const base = 0.3;
  const spread = 0.5;
  const dirs: Record<number, [number, number, number, number, number, number]> = {
    0: [0, -tip * hh * 2, spread * hw, -base * hh * 2, -spread * hw, -base * hh * 2],
    2: [tip * hw * 2, 0, base * hw * 2, -spread * hh, base * hw * 2, spread * hh],
    4: [0, tip * hh * 2, spread * hw, base * hh * 2, -spread * hw, base * hh * 2],
    6: [-tip * hw * 2, 0, -base * hw * 2, -spread * hh, -base * hw * 2, spread * hh]
  };
  const pts = dirs[dir] ?? [0, -hh];
  return `${cx + pts[0]},${cy + pts[1]} ${cx + pts[2]},${cy + pts[3]} ${cx + pts[4]},${cy + pts[5]}`;
}

function tiledViewBox(design: ModuleDesign, tiles: number): string {
  const stepY = tileStepY(design);
  let factMinY = Infinity, factMaxY = -Infinity;
  let minX = Infinity, maxX = -Infinity;
  for (const p of design.parts) {
    const half = entityHalfSize(p);
    minX = Math.min(minX, p.offset.half_x - half.hw);
    const southEdge = p.offset.half_y - half.hh;
    const northEdge = p.offset.half_y + half.hh;
    factMinY = Math.min(factMinY, southEdge);
    factMaxY = Math.max(factMaxY, northEdge);
    maxX = Math.max(maxX, p.offset.half_x + half.hw);
  }
  const lastSouth = factMinY - (tiles - 1) * stepY;
  const svgTop = -(factMaxY);
  const svgBottom = -(lastSouth);
  const pad = 2;
  return `${minX - pad} ${svgTop - pad} ${maxX - minX + pad * 2} ${svgBottom - svgTop + pad * 2}`;
}

/** Vertical spacing (in half-tile units) between tiled copies.
 *  Cell extent from lowest bottom edge to highest top edge, plus a 1-tile gap. */
function tileStepY(design: ModuleDesign): number {
  let minY = Infinity, maxY = -Infinity;
  for (const p of design.parts) {
    const half = entityHalfSize(p);
    minY = Math.min(minY, p.offset.half_y - half.hh);
    maxY = Math.max(maxY, p.offset.half_y + half.hh);
  }
  // Cell height, no gap — these are hand-fed starter cells with no
  // belt connections between tiles.
  return (maxY - minY);
}

function scaledBill(bill: Record<string, number>, tiles: number): Record<string, number> {
  const out: Record<string, number> = {};
  for (const [k, v] of Object.entries(bill)) {
    out[k] = v * tiles;
  }
  return out;
}

function scaledRates(rates: Record<string, Rate>, tiles: number): Record<string, Rate> {
  const out: Record<string, Rate> = {};
  for (const [k, v] of Object.entries(rates)) {
    out[k] = { numerator: v.numerator * tiles, ticks: v.ticks };
  }
  return out;
}


function ticksToMinutes(ticks: number): string {
  return (ticks / 3600).toFixed(1) + ' min';
}

function entityLabel(entity: string): string {
  return entity.replace(/-/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}
</script>

<template>
  <div class="mx-auto max-w-5xl p-6">
    <h1 class="mb-2 text-2xl font-bold">Blueprint Library</h1>
    <p class="mb-8 text-gray-500">
      Reusable factory module designs. Each card shows the entity layout, bill of materials, rates, and a tiling slider to preview how the cell expands.
    </p>

    <div class="grid gap-8">
      <div v-for="design in designs" :key="design.id"
           class="rounded-lg border border-gray-200 bg-white p-6 shadow-sm">

        <!-- Header -->
        <div class="mb-4 flex items-start justify-between">
          <div>
            <h2 class="text-xl font-semibold">{{ entityLabel(design.parameters.item) }} Cell</h2>
            <p class="text-xs font-mono text-gray-400">{{ design.id }}</p>
            <span class="mt-1 inline-block rounded bg-blue-100 px-2 py-0.5 text-xs font-medium text-blue-800">
              {{ design.family }}
            </span>
          </div>
          <!-- Tiling slider -->
          <div class="flex items-center gap-3">
            <label for="tile-count" class="text-xs text-gray-500">Tiles</label>
            <input id="tile-count" type="range" min="1" max="12" v-model.number="tileCount[design.id]"
                   class="w-24 h-1.5 rounded bg-gray-200 accent-blue-600 cursor-pointer" />
            <span class="text-sm font-mono tabular-nums text-gray-700">{{ tileCount[design.id] || 1 }}×</span>
          </div>
        </div>

        <div class="grid grid-cols-1 gap-6 lg:grid-cols-3">
          <!-- SVG Plan View -->
          <div class="lg:col-span-2">
            <div class="mb-2 text-sm font-medium text-gray-600">Layout</div>
            <svg :viewBox="tiledViewBox(design, tileCount[design.id] || 1)"
                 class="w-full max-w-sm rounded border bg-gray-50"
                 xmlns="http://www.w3.org/2000/svg">

              <!-- Grid covering the viewBox, 1 tile = 2 half-tile units -->
              <defs>
                <pattern id="grid" width="1" height="1" patternUnits="userSpaceOnUse"
                         x="0" y="0">
                  <path d="M 1 0 L 0 0 0 1" fill="none" stroke="#e5e7eb" stroke-width="0.10"/>
                </pattern>
              </defs>
              <rect x="-999" y="-999" width="1998" height="1998" fill="url(#grid)"/>

              <!-- Tiled cells. SVG y-positive = down, so north (Factorio -y) is SVG -y.
                   Entity rects are drawn at -(factorio_y + half_hh) so their centre
                   aligns with -factorio_y. -->
              <g v-for="i in (tileCount[design.id] || 1)" :key="'tile-' + i"
                 :transform="'translate(0, ' + ((i-1) * tileStepY(design)) + ')'">
                <g v-for="(part, pidx) in design.parts" :key="'t-' + i + '-' + pidx">
                  <rect :x="part.offset.half_x - entityHalfSize(part).hw"
                        :y="-(part.offset.half_y + entityHalfSize(part).hh)"
                        :width="entityHalfSize(part).hw * 2"
                        :height="entityHalfSize(part).hh * 2"
                        rx="0.3"
                        :fill="colorForEntityType(part.entity)"
                        stroke="#374151" stroke-width="0.2"/>
                  <text :x="part.offset.half_x" :y="-(part.offset.half_y) + 0.3"
                        text-anchor="middle" font-size="0.45" fill="white"
                        font-weight="bold">{{ entityLabel(part.role) }}</text>
                  <text v-if="part.role === 'furnace'" :x="part.offset.half_x" :y="-(part.offset.half_y) + 3.5"
                        text-anchor="middle" font-size="0.35" fill="#fbbf24" font-weight="bold">hand</text>
                  <polygon v-if="part.role !== 'furnace'"
                           :points="svgArrowPoints(part.direction, entityHalfSize(part).hw, entityHalfSize(part).hh, part.offset.half_x, part.offset.half_y)"
                           fill="#fbbf24" opacity="0.7"/>
                </g>
              </g>
            </svg>
          </div>

          <!-- Info panel -->
          <div class="space-y-4 text-sm">
            <!-- Parts / Bill (scaled by tile count) -->
            <div>
              <div class="mb-1 font-medium text-gray-600">Bill of Materials</div>
              <ul class="space-y-0.5">
                <li v-for="(count, name) in scaledBill(design.bill, tileCount[design.id] || 1)" :key="name"
                    class="flex justify-between">
                  <span class="text-gray-600">{{ entityLabel(name) }}</span>
                  <span class="font-mono">×{{ count }}</span>
                </li>
              </ul>
            </div>

            <!-- Operating contract (scaled) -->
            <div>
              <div class="mb-1 font-medium text-gray-600">Rates <span class="text-gray-400 font-normal">(×{{ tileCount[design.id] || 1 }} tiles)</span></div>
              <div v-for="(rate, item) in scaledRates(design.operation.outputs, tileCount[design.id] || 1)" :key="'out-'+item"
                   class="flex justify-between">
                <span class="text-gray-600">▶ {{ item }}</span>
                <span class="font-mono">{{ rate.numerator }}/{{ rate.ticks }}t</span>
              </div>
              <div v-for="(rate, item) in scaledRates(design.operation.inputs, tileCount[design.id] || 1)" :key="'in-'+item"
                   class="flex justify-between">
                <span class="text-gray-600">◀ {{ item }}</span>
                <span class="font-mono">{{ rate.numerator }}/{{ rate.ticks }}t</span>
              </div>
            </div>

            <div v-if="design.operation.power_watts > 0">
              <span class="font-medium text-gray-600">Power:</span>
              <span class="ml-1 font-mono">{{ ((design.operation.power_watts * (tileCount[design.id] || 1)) / 1000).toFixed(0) }} kW</span>
            </div>

            <div>
              <span class="font-medium text-gray-600">Hand feed</span>
              <span class="ml-1 text-gray-500 text-xs">— place and fuel each cell by hand; collect plates from the furnace output</span>
            </div>

            <div>
              <div class="mb-1 font-medium text-gray-600">Startup</div>
              <div class="flex justify-between">
                <span class="text-gray-600">Latency</span>
                <span class="font-mono">{{ ticksToMinutes(design.operation.startup_latency_ticks) }}</span>
              </div>
              <div v-for="(qty, item) in design.operation.startup_items" :key="'start-'+item"
                   class="flex justify-between text-xs">
                <span class="text-gray-500">{{ item }}</span>
                <span class="font-mono">×{{ qty * (tileCount[design.id] || 1) }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

