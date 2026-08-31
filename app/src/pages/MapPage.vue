<script setup lang="ts">
/**
 * The map view: a query form over `GET /api/v1/game/find-entities`, rendered
 * as an SVG viewport fit to the queried area. See `MapEntities.vue` for the
 * rendering itself and `mapStore.ts` for why this is a snapshot, not a live
 * view, and why "not queried yet", "Factorio is not running" and "the area
 * is empty" are three different states rather than one "nothing to show".
 */
import {computed, ref} from 'vue';
import {useMapStore} from '@/store/mapStore';
import Card from '@/components/ui/Card.vue';
import Button from '@/components/ui/Button.vue';
import Input from '@/components/ui/Input.vue';
import Label from '@/components/ui/Label.vue';
import MapEntities from '@/components/map/MapEntities.vue';

const mapStore = useMapStore();

// Bound as strings: `Input`'s v-model is `string`-only (see
// `components/ui/Input.vue`), and a `type="number"` input still emits string
// values through `v-model`.
const x = ref('0');
const y = ref('0');
const radius = ref('50');
const entityType = ref('');

function runQuery(): void {
  void mapStore.query({
    x: Number(x.value) || 0,
    y: Number(y.value) || 0,
    radius: Number(radius.value) || 0,
    entityType: entityType.value.trim()
  });
}

function refresh(): void {
  void mapStore.refresh();
}

const result = computed(() => mapStore.current);

const fetchedAtLabel = computed(() => {
  if (result.value.status !== 'ready') {
    return '';
  }
  return new Date(result.value.fetchedAtMs).toLocaleString();
});
</script>

<template>
  <div class="mx-auto max-w-4xl">
    <Card title="Map">
      <form class="mb-4 flex flex-wrap items-end gap-3" @submit.prevent="runQuery">
        <div>
          <Label for="map-x">X</Label>
          <Input id="map-x" v-model="x" type="number" class="w-24" data-testid="x-input"/>
        </div>
        <div>
          <Label for="map-y">Y</Label>
          <Input id="map-y" v-model="y" type="number" class="w-24" data-testid="y-input"/>
        </div>
        <div>
          <Label for="map-radius">Radius</Label>
          <Input id="map-radius" v-model="radius" type="number" class="w-24" data-testid="radius-input"/>
        </div>
        <div>
          <Label for="map-entity-type">Entity type</Label>
          <Input
            id="map-entity-type"
            v-model="entityType"
            placeholder="e.g. resource"
            class="w-40"
            data-testid="entity-type-input"/>
        </div>
        <Button type="button" data-testid="query-button" @click="runQuery">Query</Button>
      </form>

      <!--
        Not yet queried: the neutral starting state. Says so, and the form
        above is the offered action -- there is no separate call to action
        needed here.
      -->
      <div
        v-if="result.status === 'not-queried'"
        data-testid="not-queried-state"
        class="rounded-card border border-divider p-6 text-center text-sm text-ink-muted">
        No query has been run yet. Set a position and radius above, then click Query.
      </div>

      <div v-else-if="result.status === 'loading'" data-testid="loading-state" class="p-6 text-center text-sm text-ink-muted">
        Querying ...
      </div>

      <!--
        `code: 2` -- no Factorio instance is running. Says exactly that and
        nothing about entities: the query never reached a world to search, so
        a caption about "the area" would be false.
      -->
      <div
        v-else-if="result.status === 'not-running'"
        data-testid="not-running-state"
        class="rounded-card border border-warn bg-warn/10 p-6 text-center text-sm text-ink">
        Factorio is not running. Start an instance before querying the map.
      </div>

      <div
        v-else-if="result.status === 'error'"
        data-testid="error-state"
        class="rounded-card border border-danger bg-danger/10 p-6 text-center text-sm text-ink">
        The query failed: {{ result.message }}
      </div>

      <template v-else-if="result.status === 'ready'">
        <!--
          Instance running, query answered, zero entities: distinct from both
          states above. The instance IS running and the query DID answer --
          the area is genuinely empty, which is itself information.
        -->
        <div
          v-if="result.entities.length === 0"
          data-testid="empty-result-state"
          class="rounded-card border border-divider p-6 text-center text-sm text-ink-muted">
          The area is empty: no entities found within radius {{ result.query.radius }} of
          ({{ result.query.x }}, {{ result.query.y }}).
        </div>

        <template v-else>
          <div class="mb-2 flex items-center justify-between text-sm text-ink-muted">
            <span data-testid="entity-count">{{ result.entities.length }} entities</span>
            <span class="flex items-center gap-2">
              <!--
                This is a SNAPSHOT: `find-entities` answered for this instant
                and nothing pushes updates afterwards. Labelling *when* it was
                fetched, plus a manual refresh, is what stops this from
                looking live when it is not.
              -->
              <span data-testid="fetched-at">fetched {{ fetchedAtLabel }}</span>
              <Button variant="ghost" size="sm" data-testid="refresh-button" @click="refresh">Refresh</Button>
            </span>
          </div>
          <MapEntities
            :entities="result.entities"
            :center="{x: result.query.x, y: result.query.y}"
            :radius="result.query.radius"/>
        </template>
      </template>
    </Card>
  </div>
</template>
