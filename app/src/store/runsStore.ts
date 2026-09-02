import {defineStore} from 'pinia';
import {
    getRun,
    getRunFrames,
    getRunLanes,
    getRunMap,
    getRunSamples,
    getRunVideo,
    getRunVideoTicks,
    listRuns
} from '@/api/client';
import {ApiError} from '@/api/http';
import {
    ArchivedFrame,
    BotSample,
    Bounds,
    EntitySnapshot,
    Lane,
    MapRecord,
    Position,
    RunDetail,
    RunSummary,
    Sample,
    VideoManifest,
    VideoTicksResponse
} from '@/api/types';
import {
    FrameView,
    PlacedFrame,
    leadInTicks,
    placeable,
    tickBounds,
    viewsOf
} from '@/lib/runTimeline';
import {botSampleAt, forceSampleAt, inventoryOf, productionSeries, trackedItems, trailsAt} from '@/lib/runSamples';
import {boundsAt, entitiesAt} from '@/lib/runMap';

/** The `force`-kind half of `Sample`, narrowed for `forceState`. */
type ForceSample = Extract<Sample, {kind: 'force'}>;

/**
 * Turns a failed enrichment fetch into a message that names *which* stream
 * failed and *why*, rather than the bare "not found" an older server's 404
 * would otherwise surface -- that is exactly what read as "this run does not
 * exist" for every run, when really only `/samples` and `/map` were missing
 * from a server binary that predated them.
 */
function enrichmentUnavailable(label: string, route: string, err: unknown): string {
    if (err instanceof ApiError && err.status === 404) {
        return `${label} unavailable — this server does not provide ${route}`;
    }
    const reason = err instanceof Error ? err.message : String(err);
    return `${label} unavailable — ${reason}`;
}

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
        /** The run's archived world-state samples, `bots` and `force` lines mixed. */
        samples: [] as Sample[],
        /** The run's archived entity map, `placed`/`removed`/`keyframe` lines mixed. */
        map: [] as MapRecord[],
        /**
         * The run's archived recording and its clock.
         *
         * `null` means the enrichment did not load; a run that recorded no
         * video answers a *manifest describing nothing*, which is a value, not
         * a null. Keeping the two apart is what lets the viewer say "this run
         * had no video" rather than "we could not tell".
         */
        video: null as VideoManifest | null,
        videoTicks: null as VideoTicksResponse | null,
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
        error: null as string | null,
        /**
         * Per-enrichment fetch failures for the open run, `null` when that
         * stream loaded (or has not been asked for yet).
         *
         * These are deliberately separate from `error`: `error` means the
         * run itself could not be opened, while these mean the run opened
         * fine but one of its enrichments -- frames, lanes, samples or the
         * entity map -- did not load. A page that folded these into `error`
         * would fail the whole run over one missing route, which is the bug
         * this store exists to not have.
         */
        frameError: null as string | null,
        lanesError: null as string | null,
        sampleError: null as string | null,
        mapError: null as string | null,
        videoError: null as string | null
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
            return tickBounds(
                this.detail?.splits ?? [],
                this.placedFrames,
                this.lanes,
                this.video?.tick_range ?? null
            );
        },
        /** Ticks the axis skips at the front, 0 when it starts at the run. */
        leadIn(): number {
            return leadInTicks(
                this.detail?.splits ?? [],
                this.placedFrames,
                this.lanes,
                this.video?.tick_range ?? null
            );
        },
        /** The (bot, camera) pairs this run actually captured. */
        views(): FrameView[] {
            return viewsOf(this.placedFrames);
        },
        /**
         * The selected view's bot as of the cursor, or null when there is no
         * view selected or no `bots` sample at or before it yet.
         */
        botState(): BotSample | null {
            if (this.bot === null) return null;
            return inventoryOf(botSampleAt(this.samples, this.cursor), this.bot);
        },
        /**
         * The force's state as of the cursor, or null before the first
         * `force` sample -- distinct from a present sample whose `research`
         * is itself null because nothing is queued.
         */
        forceState(): ForceSample | null {
            return forceSampleAt(this.samples, this.cursor) as ForceSample | null;
        },
        /**
         * Cumulative totals at the cursor for the run's tracked items --
         * those named by its milestone goals, or every item any sample
         * recorded making when none of the goals name one.
         */
        production(): {item: string; made: number}[] {
            const goals = (this.detail?.splits ?? []).map((s) => s.goal);
            const items = trackedItems(goals, this.samples);
            return productionSeries(this.samples, items).map((series) => {
                const upToCursor = series.points.filter((p) => p.tick <= this.cursor);
                const made = upToCursor.length > 0 ? upToCursor[upToCursor.length - 1].made : 0;
                return {item: series.item, made};
            });
        },
        /** The entities on the map at the cursor, reconstructed from `map.jsonl`. */
        entities(): EntitySnapshot[] {
            return entitiesAt(this.map, this.cursor);
        },
        /**
         * The bounds of the latest keyframe at or before the cursor, or null
         * before the first one -- distinct from "no entities in bounds",
         * which is what an empty `entities` array means instead.
         */
        mapBounds(): Bounds | null {
            return boundsAt(this.map, this.cursor);
        },
        /** Every bot's position as of the cursor's latest `bots` sample. */
        mapBots(): BotSample[] {
            const sample = botSampleAt(this.samples, this.cursor);
            return sample !== null && sample.kind === 'bots' ? sample.bots : [];
        },
        /**
         * Per bot, its positions over the last 30 seconds (1,800 ticks) of
         * game time up to the cursor -- the map panel's trail.
         */
        trail(): Record<number, Position[]> {
            return trailsAt(this.samples, this.cursor);
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
         * Only the run's detail is essential -- a run that does not exist, or
         * a server that cannot be reached, fails the whole call and lands in
         * `error`. Frames, lanes, samples and the entity map are enrichments:
         * each is fetched independently (`Promise.allSettled`, not
         * `Promise.all`) so that one of them 404ing on an older server binary
         * degrades that one panel instead of making the run look like it does
         * not exist -- which is exactly what happened when a server predating
         * `/samples` and `/map` made every run in the list read as "not
         * found". A failed enrichment falls back to empty and records why in
         * its own `*Error` field, named for the stream that failed.
         *
         * The cursor still waits on all of it before moving: seeking into a
         * run whose frames have not arrived (or failed) would show an empty
         * panel that looks like a gap in capture rather than a page still
         * loading.
         */
        async openRun(id: string) {
            this.loading = true;
            this.error = null;
            this.playing = false;
            this.frameError = null;
            this.lanesError = null;
            this.sampleError = null;
            this.mapError = null;
            this.videoError = null;
            try {
                this.detail = await getRun(id);

                const [
                    framesResult,
                    lanesResult,
                    samplesResult,
                    mapResult,
                    videoResult,
                    videoTicksResult
                ] = await Promise.allSettled([
                    getRunFrames(id),
                    getRunLanes(id),
                    getRunSamples(id),
                    getRunMap(id),
                    getRunVideo(id),
                    getRunVideoTicks(id)
                ]);

                if (framesResult.status === 'fulfilled') {
                    this.frames = framesResult.value.frames;
                } else {
                    this.frames = [];
                    this.frameError = enrichmentUnavailable('frames', '/frames', framesResult.reason);
                }

                if (lanesResult.status === 'fulfilled') {
                    this.lanes = lanesResult.value.lanes;
                } else {
                    this.lanes = [];
                    this.lanesError = enrichmentUnavailable('lanes', '/lanes', lanesResult.reason);
                }

                if (samplesResult.status === 'fulfilled') {
                    this.samples = samplesResult.value.samples;
                } else {
                    this.samples = [];
                    this.sampleError = enrichmentUnavailable(
                        'world-state samples',
                        '/samples',
                        samplesResult.reason
                    );
                }

                if (mapResult.status === 'fulfilled') {
                    this.map = mapResult.value.map;
                } else {
                    this.map = [];
                    this.mapError = enrichmentUnavailable('entity map', '/map', mapResult.reason);
                }

                // Both halves of the recording, or neither: a manifest without
                // its clock can place nothing, and a clock without its manifest
                // has no calibration to place it against. Reporting one error
                // for the pair keeps the viewer from showing half a recording.
                if (videoResult.status === 'fulfilled' && videoTicksResult.status === 'fulfilled') {
                    this.video = videoResult.value;
                    this.videoTicks = videoTicksResult.value;
                } else {
                    this.video = null;
                    this.videoTicks = null;
                    this.videoError = enrichmentUnavailable(
                        'video',
                        '/video',
                        videoResult.status === 'rejected' ? videoResult.reason :
                            (videoTicksResult as PromiseRejectedResult).reason
                    );
                }

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
                this.samples = [];
                this.map = [];
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
