<script setup lang="ts">
/**
 * The first line, and the chips that say whether this run may be compared
 * with another. A chip is present when the record answers, "not captured"
 * when the field is null, and every chip is absent -- with the reason shown --
 * when there is no provenance at all. Absence is drawn, never defaulted.
 */
import {computed} from 'vue';
import {Provenance, RunSummary, Savepoint} from '@/api/types';

const props = defineProps<{
    summary: RunSummary; provenance: Provenance | null; provenanceError: string | null;
    lagTicks: number | null; headline: string; roster: number[];
    replayCounts: {steps: number; abandoned: number; lost: number; failed: number; pending: number; believed: number} | null;
    savepoints: Savepoint[];
}>();

interface Chip { key: string; value: string | null; title?: string }

const chips = computed<Chip[]>(() => {
    const p = props.provenance;
    if (p === null) return ['seed', 'mode', 'speed', 'commit', 'profile', 'mods', 'map'].map((key) => ({key, value: null}));
    const mods = p.mods === null ? null : Object.entries(p.mods);
    return [
        {key: 'seed', value: p.seed},
        {key: 'mode', value: p.bot_mode},
        {key: 'speed', value: p.game_speed === null ? null : `${p.game_speed}×`},
        {key: 'commit', value: p.git === null ? null : `${p.git.commit.slice(0, 8)}${p.git.dirty ? ' dirty' : ''}`},
        {key: 'profile', value: p.profile},
        {key: 'mods', value: mods === null ? null : `${mods.length} mods`, title: mods?.map(([n, v]) => `${n} ${v}`).join(', ')},
        {key: 'map', value: p.map?.digest ?? null}
    ];
});

/**
 * The counts for ONE plan: the last `goal.run` batch of this run.
 *
 * `replay.json` is overwritten by each batch (`LiveRecord::write_replay`), and
 * the supervisor plans once per milestone, so a run with three milestones wrote
 * three replays and kept the third. The chip therefore says `last plan` rather
 * than `plan` -- `176 steps` under a bare `plan` reads as the whole run, which
 * would understate every multi-milestone run by however many batches came
 * before.
 */
const plan = computed(() => {
    const c = props.replayCounts;
    if (c === null) return null;
    const parts = [`${c.steps} steps`];
    for (const [k, n] of [['abandoned', c.abandoned], ['lost', c.lost], ['failed', c.failed], ['pending', c.pending], ['believed', c.believed]] as const) {
        if (n > 0) parts.push(`${n} ${k}`);
    }
    return {text: parts.join(' · '), truncated: c.abandoned > 0};
});
</script>

<template>
  <header class="flex flex-wrap items-baseline gap-x-6 gap-y-3 border-b border-divider px-5 pb-3 pt-4">
    <h2 class="text-xl font-semibold">{{ summary.run_id }}</h2>
    <div class="flex flex-wrap">
      <span data-chip="roster" data-state="present" class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted">
        roster <b class="font-medium text-ink">{{ roster.join(' ') }}</b>
      </span>
      <span v-if="provenance === null" data-chip="provenance" data-state="absent"
            class="mb-1.5 mr-1.5 rounded border border-dashed border-warn/60 px-2 py-1 font-mono text-xs text-warn-dark">
        provenance <i>{{ provenanceError ?? 'not captured' }}</i>
      </span>
      <span v-for="c in chips" :key="c.key" :data-chip="c.key" :data-state="c.value === null ? 'absent' : 'present'" :title="c.title"
            class="mb-1.5 mr-1.5 rounded border px-2 py-1 font-mono text-xs text-ink-muted"
            :class="c.value === null ? 'border-dashed border-divider' : 'border-divider bg-surface'">
        {{ c.key }} <b v-if="c.value !== null" class="font-medium text-ink">{{ c.value }}</b><i v-else>not captured</i>
      </span>
      <span data-chip="samples" :data-state="lagTicks === null ? 'absent' : 'present'" class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted">
        <!-- lag <= 0: the last sample is at or past the run's end -- coverage, not a gap. -->
        samples <b class="font-medium text-ink">{{ lagTicks === null ? 'not sampled' : lagTicks <= 0 ? 'cover to end' : `lag ${lagTicks.toLocaleString()} ticks` }}</b>
      </span>
      <!-- "last plan", not "plan": see the `plan` computed above. -->
      <span v-if="plan" data-chip="plan" :data-state="plan.truncated ? 'truncated' : 'present'"
            title="the last goal.run batch of this run; earlier batches are not in replay.json"
            class="mb-1.5 mr-1.5 rounded border px-2 py-1 font-mono text-xs"
            :class="plan.truncated ? 'border-warn/60 bg-warn/10 text-warn-dark' : 'border-divider bg-surface text-ink-muted'">
        last plan <b class="font-medium" :class="plan.truncated ? 'text-warn-dark' : 'text-ink'">{{ plan.text }}</b>
      </span>
      <span v-for="s in savepoints" :key="s.milestone_index" data-chip="resume" data-state="present"
            class="mb-1.5 mr-1.5 rounded border border-divider bg-surface px-2 py-1 font-mono text-xs text-ink-muted"
            :title="`milestone ${s.milestone_index} at tick ${s.tick}, ${(s.bytes / 1048576).toFixed(1)} MB`">
        resume <code class="select-all text-ink">--resume-from {{ s.run_id }}:{{ s.milestone_index }}</code>
      </span>
    </div>
    <p class="basis-full text-sm text-ink-muted"><b class="font-medium text-ink">{{ headline }}</b></p>
  </header>
</template>
