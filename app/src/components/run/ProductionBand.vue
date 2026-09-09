<script setup lang="ts">
/**
 * Items per minute with the attribution verdict painted BEHIND the curve.
 *
 * A rising curve is not evidence of a working factory: a hand-loaded stone
 * furnace is a machine. So each minute interval's background is its verdict
 * (roster-fed hatch, factory solid, hand-made dotted, unclear grey) and the
 * word is written in the band, never left to a legend.
 */
import {computed} from 'vue';
import {Event, Sample} from '@/api/types';
import {Attribution, attributionIntervals, Verdict} from '@/lib/runAttribution';
import {markAt, rateSeries} from '@/lib/runRates';
import {AXIS_WIDTH, formatGameTime, markTicks, TickScale, tickX} from '@/lib/tickScale';
import {itemColor} from '@/lib/itemColor';

/**
 * `scale` is the drawn axis (positions); `clock` is the analysis window
 * (labels and marks), which is the same span `lo`/`hi` already carry. See
 * the header of `@/lib/tickScale`.
 */
const props = defineProps<{
    scale: TickScale; clock: TickScale; cursor: number; samples: Sample[]; events: Event[]; items: string[]; lo: number; hi: number;
}>();

const ROW = 50;
const height = computed(() => Math.max(ROW, props.items.length * ROW));
const x = (t: number) => tickX(props.scale, t);

/**
 * Whether the record has any force samples AT ALL -- the question that comes
 * before "did this item move".
 *
 * `/samples` answering an empty list is ordinary (5 of the 21 archived runs
 * have no `samples.jsonl`), and every per-item series derived from it is
 * empty. Reading that as "no output in this run" per item states something
 * about the factory the record never said; the honest answer names the
 * missing stream once, for the band.
 */
const hasForceSamples = computed(() => props.samples.some((s) => s.kind === 'force'));

interface Row {
    item: string;
    color: string;
    // `rateSeries` points can carry `perMinute: null` (a trailing window of
    // zero width, at a force sample exactly at `lo`) -- an absence, not a
    // zero. Null points are filtered out below before they ever reach a
    // `Row`, so this type stays honest: every point here has a real rate.
    points: {tick: number; perMinute: number}[];
    max: number;
    peak: {tick: number; perMinute: number} | null;
    // `source` rides with the verdict because an INFERRED verdict and a
    // MEASURED one are different claims and must not look alike: the
    // inference fallback has no machine counters behind it at all.
    // `delta` rides too, for the same reason a label needs a denominator:
    // `factory` over one item and `factory` over a thousand read identical
    // without it (a real run, `run-1788926478-07032`, is `factory` at 20:00
    // over exactly one pack).
    verdicts: {from: number; to: number; verdict: Verdict; source: Attribution['source']; delta: number}[];
    marks: {tick: number; label: string}[];
    empty: boolean;
}

const rows = computed<Row[]>(() => props.items.map((item, i) => {
    const points = rateSeries(props.samples, item, props.lo, props.hi)
        .filter((p): p is {tick: number; perMinute: number} => p.perMinute !== null);
    const max = Math.max(1, ...points.map((p) => p.perMinute)) * 1.15;
    const peak = points.reduce<Row['peak']>((m, p) => (m === null || p.perMinute > m.perMinute ? p : m), null);
    const verdicts = attributionIntervals(props.samples, props.events, props.lo, props.hi, item)
        .filter((v) => v.verdict !== 'no output')
        .map(({from, to, verdict, source, delta}) => ({from, to, verdict, source, delta}));
    // The tool's marks, off the analysis clock -- 5:00 here must be the same
    // tick the headline calls 5:00. A mark outside the drawn axis is dropped
    // rather than clamped onto its edge.
    const marks = markTicks(props.clock)
        .filter((tick) => tick >= props.scale.from && tick <= props.scale.to)
        .map((tick) => {
            const minute = (tick - props.lo) / 3600;
            const m = markAt(props.samples, props.lo, props.hi, minute, Math.max(0, minute - 5), item);
            const label = m.status === 'ok' && m.rateWindow !== null
                ? `${m.rateWindow.toFixed(0)}/min at ${formatGameTime(props.clock, tick)}`
                : `${m.status.replace('_', ' ')} at ${formatGameTime(props.clock, tick)}`;
            return {tick, label};
        });
    return {item, color: itemColor(item), points, max, peak, verdicts, marks, empty: verdicts.length === 0 && (peak?.perMinute ?? 0) === 0, row: i};
}));

function y0(i: number) { return (i + 1) * ROW - 6; }
function yFor(row: Row, i: number, v: number) { return y0(i) - (v / row.max) * (ROW - 16); }

function areaPath(row: Row, i: number): string {
    if (row.points.length === 0) return '';
    let d = `M${x(row.points[0].tick)},${y0(i)}`;
    for (const p of row.points) d += ` L${x(p.tick)},${yFor(row, i, p.perMinute)}`;
    return `${d} L${x(row.points[row.points.length - 1].tick)},${y0(i)} Z`;
}
function linePath(row: Row, i: number): string {
    return row.points.map((p, k) => `${k ? 'L' : 'M'}${x(p.tick)},${yFor(row, i, p.perMinute)}`).join(' ');
}
function verdictFill(v: Verdict): string {
    if (v === 'roster-fed') return 'url(#verdict-roster)';
    if (v === 'factory') return 'var(--color-status-good)';
    if (v === 'hand-made') return 'url(#verdict-hand)';
    return 'var(--color-status-neutral)';
}
function verdictOpacity(v: Verdict): number { return v === 'factory' ? 0.18 : v === 'roster-fed' || v === 'hand-made' ? 1 : 0.25; }
/**
 * The word a reader sees -- an inference says so, in the word itself, and the
 * count it was computed over rides beside it. A verdict with no denominator
 * is misread: `factory` over one item and `factory` over a thousand look
 * identical otherwise -- a real run, `run-1788926478-07032`, is `factory` at
 * 20:00 over exactly one pack.
 */
function verdictWord(v: {verdict: Verdict; source: Attribution['source']; delta: number}): string {
    const counted = `${v.verdict} · ${v.delta} ${v.delta === 1 ? 'item' : 'items'}`;
    return v.source === 'inference' ? `${counted} (inferred)` : counted;
}
</script>

<template>
  <svg :viewBox="`0 0 ${AXIS_WIDTH} ${height}`" class="block h-auto w-full" aria-label="production rates with attribution">
    <defs>
      <pattern id="verdict-roster" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(135)">
        <rect width="6" height="6" fill="var(--color-verdict-roster-soft)"/>
        <line x1="0" y1="0" x2="0" y2="6" stroke="var(--color-verdict-roster)" stroke-width="1.2" opacity="0.55"/>
      </pattern>
      <pattern id="verdict-hand" width="5" height="5" patternUnits="userSpaceOnUse">
        <circle cx="2.5" cy="2.5" r="0.9" fill="var(--color-ink-muted)" opacity="0.7"/>
      </pattern>
    </defs>
    <!-- The record's silence about the samples is said once, for the band:
         "no output in this run" per item would read as a fact about the
         factory when it is a fact about `/samples`. -->
    <text v-if="!hasForceSamples" :x="AXIS_WIDTH / 2" :y="height / 2 + 4" font-size="10" text-anchor="middle" fill="var(--color-ink-muted)">no production samples in this run</text>
    <template v-else>
    <template v-for="(row, i) in rows" :key="row.item">
      <rect v-for="v in row.verdicts" :key="`${row.item}-${v.from}`" class="verdict" :data-verdict="v.verdict" :data-source="v.source" :data-delta="v.delta"
            :x="x(v.from)" :y="i * ROW" :width="x(v.to) - x(v.from)" :height="ROW"
            :fill="verdictFill(v.verdict)" :opacity="verdictOpacity(v.verdict)"
            :stroke="v.source === 'inference' ? 'var(--color-ink-muted)' : undefined"
            :stroke-dasharray="v.source === 'inference' ? '3 2' : undefined"/>
      <line :x1="0" :y1="y0(i)" :x2="AXIS_WIDTH" :y2="y0(i)" stroke="var(--color-plot-grid)" stroke-width="1"/>
      <path :data-testid="`area-${row.item}`" :d="areaPath(row, i)" :fill="row.color" opacity="0.18"/>
      <path :d="linePath(row, i)" fill="none" :stroke="row.color" stroke-width="1.8" stroke-linejoin="round"/>
      <text :x="6" :y="i * ROW + 12" font-size="11" font-weight="500" fill="var(--color-ink)">{{ row.item }}</text>
      <template v-if="row.empty">
        <text :x="AXIS_WIDTH / 2" :y="i * ROW + ROW / 2 + 4" font-size="10" text-anchor="middle" fill="var(--color-ink-muted)">no output in this run</text>
      </template>
      <template v-else>
        <template v-if="row.peak">
          <circle :cx="x(row.peak.tick)" :cy="yFor(row, i, row.peak.perMinute)" r="3" :fill="row.color" stroke="var(--color-plot)" stroke-width="1.5"/>
          <text :x="x(row.peak.tick) + 6" :y="yFor(row, i, row.peak.perMinute) + 3" font-size="10" fill="var(--color-ink-muted)">peak {{ row.peak.perMinute.toFixed(0) }}/min</text>
        </template>
        <template v-for="m in row.marks" :key="`${row.item}-${m.tick}`">
          <line :x1="x(m.tick)" :y1="i * ROW + 4" :x2="x(m.tick)" :y2="y0(i)" stroke="var(--color-verdict-roster)" stroke-width="1" stroke-dasharray="2 3"/>
          <text :x="x(m.tick) + 4" :y="i * ROW + 24" font-size="10" font-weight="500" fill="var(--color-verdict-roster)">{{ m.label }}</text>
        </template>
        <text v-for="v in row.verdicts" :key="`w-${row.item}-${v.from}`" :x="x(v.from) + 4" :y="(i + 1) * ROW - 2" font-size="9"
              :fill="v.verdict === 'roster-fed' ? 'var(--color-verdict-roster)' : 'var(--color-ink-muted)'">{{ verdictWord(v) }}</text>
      </template>
    </template>
    </template>
    <line :x1="x(cursor)" y1="0" :x2="x(cursor)" :y2="height" stroke="var(--color-verdict-roster)" stroke-width="1.5"/>
  </svg>
</template>
