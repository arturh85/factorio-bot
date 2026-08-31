<script setup lang="ts">
import {Evidence} from '@/api/replay';

/**
 * Marks a row's observation as belief rather than measurement.
 *
 * Renders nothing at all for a `measured` row -- per `replay.rs`, a measured
 * row is structurally incapable of carrying a caveat, and a badge that shows
 * for every row regardless of value would train a reader to ignore it. The
 * `why` is not composed here: it travels with the document (`Evidence.why`),
 * written once in `crates/executor/src/replay.rs::WALK_BELIEF`, so narrowing
 * or changing the caveat is an edit there and nothing here needs to change.
 *
 * **Deliberately quiet.** `Believed` is the normal state of every walk row,
 * not a fault -- the ticks are real, only the *arrival* is inferred rather
 * than re-measured. It must read as an epistemics footnote a reader notices
 * on inspection, not as an alarm a reader is drawn to at a glance: no danger
 * or warning colour, no triangle, no background box competing with the
 * status bar it sits next to. `Failed` and `Lost` are outcomes and are
 * allowed to draw the eye; this is not one and must not borrow their palette.
 */
defineProps<{evidence: Evidence}>();
</script>

<template>
  <span
    v-if="evidence.kind === 'believed'"
    data-testid="evidence-mark"
    :title="evidence.why"
    class="cursor-help border-b border-dotted border-ink-muted text-[0.65rem] uppercase tracking-wide text-ink-muted">
    believed<sup>?</sup>
  </span>
</template>
