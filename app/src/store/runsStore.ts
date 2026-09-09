import {defineStore} from 'pinia';
import {
    getRun,
    getRunEvents,
    getRunLanes,
    getRunMap,
    getRunProvenance,
    getRunReplay,
    getRunSamples,
    getRunSavepoints,
    getRunVideo,
    getRunVideoTicks,
    listRuns
} from '@/api/client';
import {ApiError} from '@/api/http';
import {
    BotSample,
    Bounds,
    EntitySnapshot,
    Event,
    Lane,
    MapRecord,
    Position,
    Provenance,
    RunDetail,
    RunSummary,
    Sample,
    Savepoint,
    VideoManifest,
    VideoTicksResponse
} from '@/api/types';
import {laneBots, leadInTicks, tickBounds} from '@/lib/runTimeline';
import {botSampleAt, forceSampleAt, inventoryOf, productionSeries, trackedItems, trailsAt} from '@/lib/runSamples';
import {boundsAt, entitiesAt} from '@/lib/runMap';
import {machineStatusAt} from '@/lib/machineTimeline';
import {lagTicks} from '@/lib/runCoverage';
import {parseReplay, Replay, ReplayStatus} from '@/api/replay';

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
 * Every bot index the run's `bots` samples ever mention, ascending.
 *
 * The samples are the only record of which bots a run had -- the manifest
 * counts events, not participants -- so this is what the inventory panel picks
 * from.
 */
function botsInSamples(samples: Sample[]): number[] {
    const seen = new Set<number>();
    for (const sample of samples) {
        if (sample.kind !== 'bots') continue;
        for (const bot of sample.bots) seen.add(bot.id);
    }
    return [...seen].sort((a, b) => a - b);
}

/**
 * Archived runs, and one cursor over the run being viewed.
 *
 * The cursor is a *game tick*, and every panel reads it. Splits, lanes, the
 * map and the scrubber are all positioned on that one axis, which is why the
 * store holds a tick rather than a sample index or a percentage: an index
 * would break the moment a sample dropped, and a percentage cannot be compared
 * between runs.
 */
export const useRunsStore = defineStore('runs', {
    state: () => ({
        runs: [] as RunSummary[],
        detail: null as RunDetail | null,
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
         * comparing runs does not need the other run's whole log.
         */
        reference: null as RunDetail | null,
        /** The tick every panel renders at. */
        cursor: 0,
        playing: false,
        /** Ticks advanced per play step. 60 ticks is one second of game time. */
        rate: 300,
        /**
         * The bot the inventory panel reports on.
         *
         * Chosen from the run's own `bots` samples when it opens -- the lowest
         * index that ever reported -- because that is the only place the set of
         * bots is recorded. It used to be a side effect of picking a screenshot
         * camera; when the cameras were retired (2026-09-02) that left nothing
         * setting it, and the panel would have sat on its empty state forever.
         */
        bot: null as number | null,
        loading: false,
        error: null as string | null,
        /**
         * Per-enrichment fetch failures for the open run, `null` when that
         * stream loaded (or has not been asked for yet).
         *
         * These are deliberately separate from `error`: `error` means the
         * run itself could not be opened, while these mean the run opened
         * fine but one of its enrichments -- lanes, samples, the entity map or
         * the video -- did not load. A page that folded these into `error`
         * would fail the whole run over one missing route, which is the bug
         * this store exists to not have.
         */
        lanesError: null as string | null,
        sampleError: null as string | null,
        mapError: null as string | null,
        videoError: null as string | null,
        /**
         * What the run was launched with -- seed, mods, git commit, roster.
         *
         * `null` either because the fetch failed (see `provenanceError`) or
         * because the server answered a genuinely older run: `provenance.json`
         * is written at run start, so its absence means "not captured", not
         * "empty".
         */
        provenance: null as Provenance | null,
        provenanceError: null as string | null,
        /**
         * The executor's replay -- planned against observed per step.
         *
         * Parsed through `parseReplay` rather than trusted as the server's
         * `unknown` body: a document that fails to narrow sets `replayError`
         * exactly like a failed fetch would, because a malformed replay is
         * exactly as unusable as a missing one.
         */
        replay: null as Replay | null,
        replayError: null as string | null,
        /** The milestone savepoints this run wrote. Empty is the ordinary case. */
        savepoints: [] as Savepoint[],
        savepointsError: null as string | null,
        /** The run's raw event log. */
        events: [] as Event[],
        /**
         * Event lines `/events` could not parse -- in practice the torn last
         * line of a killed run.
         *
         * Kept rather than dropped: the coverage band's whole job is to say
         * what the record does not have, and a line nobody could read is
         * exactly that. Reading it as 0 when the server reported it would be
         * the same substitution this store's error fields exist to avoid.
         */
        eventsSkipped: 0,
        eventsError: null as string | null,
        /**
         * The machine the Machines band has selected, keyed by the
         * MACHINES-SAMPLE key -- the entity's `unit_number` as a string, e.g.
         * `"13"`, which is what `machineRows` and `statusMatrix` key on.
         *
         * NOT a `"x,y"` position key, which this comment claimed while the
         * value never was one. The map joins on position, so a reader that
         * wants to show this selection there translates through
         * `machineRows`; `RunPage` does exactly that.
         */
        selectedMachine: null as string | null
    }),

    getters: {
        /**
         * The tick range the timeline spans, or `null` when the run has
         * nothing to place -- a planning-only run with no milestones.
         */
        bounds(): {from: number; to: number} | null {
            return tickBounds(
                this.detail?.splits ?? [],
                this.lanes,
                this.video?.tick_range ?? null
            );
        },
        /** Ticks the axis skips at the front, 0 when it starts at the run. */
        leadIn(): number {
            return leadInTicks(
                this.detail?.splits ?? [],
                this.lanes,
                this.video?.tick_range ?? null
            );
        },
        /**
         * The selected bot as of the cursor, or null when the run recorded no
         * bot at all or has no `bots` sample at or before the cursor yet.
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
        /** Every bot this run sampled, ascending -- the inventory picker's list. */
        bots(): number[] {
            return botsInSamples(this.samples);
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
        },
        /**
         * The ANALYSIS window: `run_started` to `run_finished` (or the last
         * event). Distinct from `bounds`, the drawn axis, which trims a
         * lead-in. Rates and verdicts are measured from `run_started`, as
         * `just analyse` measures them; `null` when the run has no events.
         */
        window(): {lo: number; hi: number} | null {
            if (this.events.length === 0) return null;
            const started = this.events.find((e) => e.kind === 'run_started');
            const finished = this.events.find((e) => e.kind === 'run_finished');
            const lo = started?.tick ?? Math.min(...this.events.map((e) => e.tick));
            const hi = finished?.tick ?? Math.max(...this.events.map((e) => e.tick));
            return {lo, hi};
        },
        /** Machine status by "x,y" at the cursor -- the map's fill lookup. */
        machineFills(): Map<string, string | null> {
            return machineStatusAt(this.samples, this.cursor);
        },
        /** Every bot the lanes ever mention, ascending. */
        laneBotIds(): number[] {
            return laneBots(this.lanes);
        },
        /**
         * Per-status counts over the replay's steps, or `null` when the run
         * has no parsed replay -- distinct from a replay whose counts are all
         * zero, which is a real (if odd) document.
         */
        replayCounts(): {steps: number; abandoned: number; lost: number; failed: number; pending: number; believed: number} | null {
            if (this.replay === null) return null;
            const steps = this.replay.steps;
            const count = (status: ReplayStatus) => steps.filter((step) => step.status === status).length;
            return {
                steps: steps.length,
                abandoned: count('Abandoned'),
                lost: count('Lost'),
                failed: count('Failed'),
                pending: count('Pending'),
                believed: steps.filter((step) => step.evidence.kind === 'believed').length
            };
        },
        /**
         * Ticks the archived samples fall short of the run's end.
         *
         * The manifest's own `samples_lag_ticks` outranks a client-side
         * derivation when the server carries one -- it was computed against
         * the run's actual end, where `lagTicks` only has the analysis
         * window's `hi` to work from. Falls back to `lagTicks` whenever the
         * manifest field is null, which is true of two different runs: a
         * summary from a server that predates the field, and an unfinished
         * run on a current one (the field is only ever computed against a
         * finished run's end).
         */
        sampleLag(): number | null {
            const fromManifest = this.detail?.summary.samples_lag_ticks;
            if (fromManifest !== undefined && fromManifest !== null) return fromManifest;
            const win = this.window;
            return win === null ? null : lagTicks(win.hi, this.samples);
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
         * `error`. Lanes, samples, the entity map and the video are
         * enrichments:
         * each is fetched independently (`Promise.allSettled`, not
         * `Promise.all`) so that one of them 404ing on an older server binary
         * degrades that one panel instead of making the run look like it does
         * not exist -- which is exactly what happened when a server predating
         * `/samples` and `/map` made every run in the list read as "not
         * found". A failed enrichment falls back to empty and records why in
         * its own `*Error` field, named for the stream that failed.
         *
         * The cursor still waits on all of it before moving: seeking into a
         * run whose samples have not arrived (or failed) would show an empty
         * panel that looks like a gap in the record rather than a page still
         * loading.
         */
        async openRun(id: string) {
            this.loading = true;
            this.error = null;
            this.playing = false;
            this.lanesError = null;
            this.sampleError = null;
            this.mapError = null;
            this.videoError = null;
            this.eventsError = null;
            this.eventsSkipped = 0;
            this.selectedMachine = null;
            this.provenanceError = null;
            this.replayError = null;
            this.savepointsError = null;
            try {
                this.detail = await getRun(id);

                const [
                    lanesResult,
                    samplesResult,
                    mapResult,
                    videoResult,
                    videoTicksResult,
                    eventsResult,
                    provenanceResult,
                    replayResult,
                    savepointsResult
                ] = await Promise.allSettled([
                    getRunLanes(id),
                    getRunSamples(id),
                    getRunMap(id),
                    getRunVideo(id),
                    getRunVideoTicks(id),
                    getRunEvents(id),
                    getRunProvenance(id),
                    getRunReplay(id),
                    getRunSavepoints(id)
                ]);

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

                if (eventsResult.status === 'fulfilled') {
                    this.events = eventsResult.value.events;
                    this.eventsSkipped = eventsResult.value.skipped;
                } else {
                    this.events = [];
                    this.eventsSkipped = 0;
                    this.eventsError = enrichmentUnavailable('events', '/events', eventsResult.reason);
                }

                if (provenanceResult.status === 'fulfilled') {
                    this.provenance = provenanceResult.value;
                } else {
                    this.provenance = null;
                    this.provenanceError = enrichmentUnavailable(
                        'provenance',
                        '/provenance',
                        provenanceResult.reason
                    );
                }

                if (replayResult.status === 'fulfilled') {
                    try {
                        this.replay = parseReplay(replayResult.value);
                    } catch (err) {
                        this.replay = null;
                        this.replayError = err instanceof Error ? err.message : String(err);
                    }
                } else {
                    this.replay = null;
                    this.replayError = enrichmentUnavailable('replay', '/replay', replayResult.reason);
                }

                if (savepointsResult.status === 'fulfilled') {
                    this.savepoints = savepointsResult.value.savepoints;
                } else {
                    this.savepoints = [];
                    this.savepointsError = enrichmentUnavailable(
                        'savepoints',
                        '/savepoints',
                        savepointsResult.reason
                    );
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
                // The lowest bot the run ever sampled. Nothing else names the
                // set of bots, and leaving this null would park the inventory
                // panel on its empty state for every run.
                this.bot = botsInSamples(this.samples)[0] ?? null;
                this.cursor = this.bounds?.from ?? 0;
            } catch (err) {
                this.error = err instanceof Error ? err.message : String(err);
                this.detail = null;
                this.lanes = [];
                this.samples = [];
                this.map = [];
                this.events = [];
                this.eventsSkipped = 0;
                this.provenance = null;
                this.replay = null;
                this.savepoints = [];
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

        /** Points the inventory panel at another bot. */
        selectBot(bot: number | null) {
            this.bot = bot;
        },

        /** Points the viewer at another machine, keyed by its machines-sample key. */
        selectMachine(key: string | null) {
            this.selectedMachine = key;
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
