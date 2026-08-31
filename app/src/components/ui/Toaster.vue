<script setup lang="ts">
import {X} from '@lucide/vue';
import {toastMessages, useToast, type ToastSeverity} from '@/composables/useToast';

const {remove} = useToast();

const severityClasses: Record<ToastSeverity, string> = {
    success: 'border-success',
    info: 'border-brand',
    warn: 'border-warn',
    error: 'border-danger'
};
</script>

<template>
  <div
    class="pointer-events-none fixed bottom-4 right-4 z-[1000] flex w-80 flex-col gap-2"
    role="status"
    aria-live="polite"
    data-testid="toaster">
    <div
      v-for="message in toastMessages"
      :key="message.id"
      class="pointer-events-auto flex items-start gap-2 rounded-card border-l-4 bg-card p-3 text-ink shadow-lg"
      :class="severityClasses[message.severity]"
      data-testid="toast">
      <div class="grow">
        <p class="font-semibold">{{ message.summary }}</p>
        <p v-if="message.detail" class="mt-1 text-ink-muted">{{ message.detail }}</p>
      </div>
      <button
        type="button"
        class="text-ink-muted transition-colors hover:text-ink"
        aria-label="Dismiss"
        @click="remove(message.id)">
        <X class="size-4"/>
      </button>
    </div>
  </div>
</template>
