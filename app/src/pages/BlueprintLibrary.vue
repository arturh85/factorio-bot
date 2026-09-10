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

import {ref, onMounted} from 'vue';

const designs = ref<ModuleDesign[]>([]);
const loading = ref(true);
const error = ref<string | null>(null);

const FALLBACK_DESIGNS: ModuleDesign[] = [
  {
    schema: 1, id: 'ore-to-plate-iron', family: 'OreToPlate',
    parameters: {item: 'iron-plate', with_pole: false, labs: 0},
    parts: [
      {role: 'drill', entity: 'burner-mining-drill', offset: {half_x: 0, half_y: 0}, direction: 4, recipe: null},
      {role: 'furnace', entity: 'stone-furnace', offset: {half_x: 0, half_y: 4}, direction: 0, recipe: 'iron-plate'}
    ],
    ports: [
      {id: 'belt-input', mode: 'BeltInput', item: 'iron-ore', offset: {half_x: -2, half_y: 0}, direction: 12},
      {id: 'inventory-output', mode: 'InventoryOutput', item: 'iron-plate', offset: {half_x: 0, half_y: 6}, direction: 0}
    ],
    bill: {'burner-mining-drill': 1, 'stone-furnace': 1},
    operation: {
      inputs: {'iron-ore': {numerator: 1, ticks: 600}},
      outputs: {'iron-plate': {numerator: 1, ticks: 600}},
      power_watts: 0,
      fuel_per_tick: {coal: {numerator: 1, ticks: 4800}},
      startup_latency_ticks: 4800,
      startup_items: {coal: 10, 'iron-ore': 5},
      required_research: [], required_surface: 'nauvis'
    }
  },
  {
    schema: 1, id: 'ore-to-plate-copper', family: 'OreToPlate',
    parameters: {item: 'copper-plate', with_pole: false, labs: 0},
    parts: [
      {role: 'drill', entity: 'burner-mining-drill', offset: {half_x: 0, half_y: 0}, direction: 4, recipe: null},
      {role: 'furnace', entity: 'stone-furnace', offset: {half_x: 0, half_y: 4}, direction: 0, recipe: 'copper-plate'}
    ],
    ports: [
      {id: 'belt-input', mode: 'BeltInput', item: 'copper-ore', offset: {half_x: -2, half_y: 0}, direction: 12},
      {id: 'inventory-output', mode: 'InventoryOutput', item: 'copper-plate', offset: {half_x: 0, half_y: 6}, direction: 0}
    ],
    bill: {'burner-mining-drill': 1, 'stone-furnace': 1},
    operation: {
      inputs: {'copper-ore': {numerator: 1, ticks: 600}},
      outputs: {'copper-plate': {numerator: 1, ticks: 600}},
      power_watts: 0,
      fuel_per_tick: {coal: {numerator: 1, ticks: 4800}},
      startup_latency_ticks: 4800,
      startup_items: {coal: 10, 'copper-ore': 5},
      required_research: [], required_surface: 'nauvis'
    }
  }
];

onMounted(async () => {
  try {
    const res = await fetch('/api/v1/modules/designs');
    if (res.ok) {
      const data = await res.json();
      designs.value = data.designs;
    } else {
      throw new Error(`HTTP ${res.status}`);
    }
  } catch {
    error.value = 'Could not load from API, using fallback designs';
    designs.value = FALLBACK_DESIGNS;
  } finally {
    loading.value = false;
  }
});

// ---------------------------------------------------------------------------
// SVG helpers
// ---------------------------------------------------------------------------

/** Compute bounding box of all parts in half-tile units. */
function designBounds(parts: Part[], ports: Port[]): {minX: number; minY: number; maxX: number; maxY: number} {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const p of parts) {
    const x = p.offset.half_x;
    const y = p.offset.half_y;
    minX = Math.min(minX, x - 1);
    minY = Math.min(minY, y - 1);
    maxX = Math.max(maxX, x + 2);
    maxY = Math.max(maxY, y + 2);
  }
  for (const p of ports) {
    minX = Math.min(minX, p.offset.half_x - 1);
    minY = Math.min(minY, p.offset.half_y - 1);
    maxX = Math.max(maxX, p.offset.half_x + 1);
    maxY = Math.max(maxY, p.offset.half_y + 1);
  }
  return {minX, minY, maxX, maxY};
}

/** SVG viewBox string for a design's parts and ports. */
function designViewBox(parts: Part[], ports: Port[]): string {
  const b = designBounds(parts, ports);
  const pad = 1;
  return `${b.minX - pad} ${b.minY - pad} ${b.maxX - b.minX + pad * 2} ${b.maxY - b.minY + pad * 2}`;
}

function labelForMode(mode: string): string {
  switch (mode) {
    case 'BeltInput': return '⬅ Belt in';
    case 'InventoryOutput': return '➡ Output';
    case 'DirectLabOutput': return '🔬 Lab';
    default: return mode;
  }
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
      Reusable factory module designs. Each card shows the entity layout, ports, and operating contract.
    </p>

    <div v-if="loading" class="text-gray-500">Loading designs...</div>
    <div v-else-if="error" class="mb-4 rounded border border-yellow-200 bg-yellow-50 p-3 text-sm text-yellow-800">{{ error }}</div>
    <div v-else class="grid gap-8">
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
        </div>

        <div class="grid grid-cols-1 gap-6 lg:grid-cols-3">
          <!-- SVG Plan View -->
          <div class="lg:col-span-2">
            <div class="mb-2 text-sm font-medium text-gray-600">Layout</div>
            <svg :viewBox="designViewBox(design.parts, design.ports)"
                 class="w-full max-w-sm rounded border bg-gray-50"
                 xmlns="http://www.w3.org/2000/svg">

              <!-- Grid background -->
              <defs>
                <pattern id="grid" width="2" height="2" patternUnits="userSpaceOnUse">
                  <path d="M 2 0 L 0 0 0 2" fill="none" stroke="#e5e7eb" stroke-width="0.1"/>
                </pattern>
              </defs>
              <rect width="100%" height="100%" fill="url(#grid)"/>

              <!-- Port indicators -->
              <g v-for="port in design.ports" :key="port.id">
                <circle :cx="port.offset.half_x" :cy="port.offset.half_y" r="0.6" fill="#93c5fd" stroke="#3b82f6" stroke-width="0.15"/>
                <text :x="port.offset.half_x" :y="port.offset.half_y + 0.2"
                      text-anchor="middle" font-size="0.6" fill="#1e40af">{{ labelForMode(port.mode).charAt(0) }}</text>
              </g>

              <!-- Entity rectangles -->
              <g v-for="part in design.parts" :key="part.role">
                <rect :x="part.offset.half_x - 1" :y="part.offset.half_y - 1"
                      width="2" height="2" rx="0.3"
                      :fill="colorForEntityType(part.entity)"
                      stroke="#374151" stroke-width="0.15"/>
                <text :x="part.offset.half_x" :y="part.offset.half_y + 0.15"
                      text-anchor="middle" font-size="0.5" fill="white"
                      font-weight="bold">{{ entityLabel(part.role).substring(0, 4) }}</text>
              </g>
            </svg>
          </div>

          <!-- Info panel -->
          <div class="space-y-4 text-sm">
            <!-- Parts / Bill -->
            <div>
              <div class="mb-1 font-medium text-gray-600">Bill of Materials</div>
              <ul class="space-y-0.5">
                <li v-for="(count, name) in design.bill" :key="name"
                    class="flex justify-between">
                  <span class="text-gray-600">{{ entityLabel(name) }}</span>
                  <span class="font-mono">×{{ count }}</span>
                </li>
              </ul>
            </div>

            <!-- Operating contract -->
            <div>
              <div class="mb-1 font-medium text-gray-600">Rates</div>
              <div v-for="(rate, item) in design.operation.outputs" :key="'out-'+item"
                   class="flex justify-between">
                <span class="text-gray-600">▶ {{ item }}</span>
                <span class="font-mono">{{ rate.numerator }}/{{ rate.ticks }}t</span>
              </div>
              <div v-for="(rate, item) in design.operation.inputs" :key="'in-'+item"
                   class="flex justify-between">
                <span class="text-gray-600">◀ {{ item }}</span>
                <span class="font-mono">{{ rate.numerator }}/{{ rate.ticks }}t</span>
              </div>
            </div>

            <div v-if="design.operation.power_watts > 0">
              <span class="font-medium text-gray-600">Power:</span>
              <span class="ml-1 font-mono">{{ (design.operation.power_watts / 1000).toFixed(0) }} kW</span>
            </div>

            <div>
              <span class="font-medium text-gray-600">Startup:</span>
              <span class="ml-1 font-mono">{{ ticksToMinutes(design.operation.startup_latency_ticks) }}</span>
            </div>

            <!-- Ports -->
            <div>
              <div class="mb-1 font-medium text-gray-600">Ports</div>
              <div v-for="port in design.ports" :key="port.id"
                   class="flex justify-between text-xs">
                <span>{{ labelForMode(port.mode) }} <span class="text-gray-400">{{ port.item }}</span></span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

