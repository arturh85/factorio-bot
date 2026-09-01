import {defineStore} from 'pinia';
import {getRun, getRunFrames, getRunLanes, listRuns} from '@/api/client';
import {ArchivedFrame, Lane, RunDetail, RunSummary} from '@/api/types';
import {PlacedFrame, placeable, tickBounds} from '@/lib/runTimeline';

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
                const placed = this.placedFrames;
                this.bot = placed.length > 0 ? placed[0].bot : null;
                this.camera = placed.length > 0 ? placed[0].camera : null;
                this.cursor = this.bounds?.from ?? 0;
            } catch (err) {
                this.error = err instanceof Error ? err.message : String(err);
                this.detail = null;
                this.frames = [];
                this.lanes = [];
            } finally {
                this.loading = false;
            }
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
