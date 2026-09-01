import {defineStore} from 'pinia';
import {getRun, getRunFrames, getRunLanes, listRuns} from '@/api/client';
import {ArchivedFrame, Lane, RunDetail, RunSummary} from '@/api/types';
import {
    FrameView,
    PlacedFrame,
    leadInTicks,
    placeable,
    tickBounds,
    viewsOf
} from '@/lib/runTimeline';

/**
 * Archived runs, and one cursor over the run being viewed.
 *
 * The cursor is a *game tick*, and every panel reads it. Frames, splits and
 * the scrubber are all positioned on that one axis, which is why the store
 * holds a tick rather than a frame index or a percentage: an index would
 * break the moment a frame dropped, and a percentage cannot be compared
 * between runs.
 */
export const useRunsStore = defineStore('runs', {
    state: () => ({
        runs: [] as RunSummary[],
        detail: null as RunDetail | null,
        frames: [] as ArchivedFrame[],
        lanes: [] as Lane[],
        /**
         * Another run's splits, to diff against. Only the splits are fetched:
         * comparing runs does not need the other run's whole log or frames.
         */
        reference: null as RunDetail | null,
        /** The tick every panel renders at. */
        cursor: 0,
        playing: false,
        /** Ticks advanced per play step. 60 ticks is one second of game time. */
        rate: 300,
        bot: null as number | null,
        camera: null as string | null,
        loading: false,
        error: null as string | null
    }),

    getters: {
        /** Frames that can be positioned in time, sorted by tick. */
        placedFrames(state): PlacedFrame[] {
            return placeable(state.frames);
        },
        /**
         * The tick range the timeline spans, or `null` when the run has
         * nothing to place -- a planning-only run with no milestones.
         */
        bounds(): {from: number; to: number} | null {
            return tickBounds(this.detail?.splits ?? [], this.placedFrames, this.lanes);
        },
        /** Ticks the axis skips at the front, 0 when it starts at the run. */
        leadIn(): number {
            return leadInTicks(this.detail?.splits ?? [], this.placedFrames, this.lanes);
        },
        /** The (bot, camera) pairs this run actually captured. */
        views(): FrameView[] {
            return viewsOf(this.placedFrames);
        }
    },

    actions: {
        async loadRuns() {
            this.loading = true;
            this.error = null;
            try {
                this.runs = (await listRuns()).runs;
            } catch (err) {
                this.error = err instanceof Error ? err.message : String(err);
            } finally {
                this.loading = false;
            }
        },

        /**
         * Loads one run and parks the cursor at the start of the axis.
         *
         * Both requests are awaited before the cursor moves: seeking into a
         * run whose frames have not arrived would show an empty panel that
         * looks like a gap in capture rather than a page still loading.
         */
        async openRun(id: string) {
            this.loading = true;
            this.error = null;
            this.playing = false;
            try {
                const [detail, frames, lanes] = await Promise.all([
                    getRun(id),
                    getRunFrames(id),
                    getRunLanes(id)
                ]);
                this.detail = detail;
                this.frames = frames.frames;
                this.lanes = lanes.lanes;
                // A comparison against the previously open run is almost never
                // what is wanted, and would be read as belonging to this one.
                this.reference = null;
                const placed = this.placedFrames;
                this.bot = placed.length > 0 ? placed[0].bot : null;
                this.camera = placed.length > 0 ? placed[0].camera : null;
                // Open on the first frame, not the start of the axis.
                //
                // The axis spans splits *and* frames, and a run's first
                // milestone usually starts before capture produces anything --
                // 231 ticks before, in the run that prompted this. Opening at
                // the axis start is correct and shows an empty panel, which
                // reads as "this run has no frames" rather than "not yet".
                this.cursor = placed.length > 0 ? placed[0].tick : (this.bounds?.from ?? 0);
            } catch (err) {
                this.error = err instanceof Error ? err.message : String(err);
                this.detail = null;
                this.frames = [];
                this.lanes = [];
            } finally {
                this.loading = false;
            }
        },

        /**
         * Loads another run to compare against, or clears the comparison.
         *
         * A failure clears the reference rather than leaving the previous one
         * in place, where its deltas would silently describe the wrong run.
         */
        async setReference(id: string | null) {
            if (id === null) {
                this.reference = null;
                return;
            }
            try {
                this.reference = await getRun(id);
            } catch {
                this.reference = null;
            }
        },

        /**
         * Selects a capture view by its (bot, camera) pair.
         *
         * Both together, never one at a time: only specific pairs exist, so
         * changing one and leaving the other produces a combination the run
         * never captured, and the panel goes blank for a reason that looks
         * like a bug rather than a choice.
         */
        selectView(view: FrameView) {
            this.bot = view.bot;
            this.camera = view.camera;
            // Nothing for this view before its first frame, so do not sit
            // somewhere it cannot show anything.
            if (this.cursor < view.from) this.seek(view.from);
        },

        /** Moves the cursor, clamped to the axis. */
        seek(tick: number) {
            const bounds = this.bounds;
            if (bounds === null) {
                this.cursor = 0;
                return;
            }
            this.cursor = Math.min(bounds.to, Math.max(bounds.from, Math.round(tick)));
        },

        /**
         * Advances one step, stopping at the end.
         *
         * Stops rather than wrapping: a timeline that silently restarts looks
         * like a run that happened twice.
         */
        advance() {
            const bounds = this.bounds;
            if (bounds === null) return;
            if (this.cursor >= bounds.to) {
                this.playing = false;
                return;
            }
            this.seek(this.cursor + this.rate);
        },

        togglePlay() {
            const bounds = this.bounds;
            if (bounds === null) return;
            // Restart from the beginning when play is pressed at the end,
            // rather than toggling into a run that immediately stops.
            if (!this.playing && this.cursor >= bounds.to) this.cursor = bounds.from;
            this.playing = !this.playing;
        }
    }
});
