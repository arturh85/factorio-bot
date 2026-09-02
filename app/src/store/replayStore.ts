import {defineStore} from 'pinia'
import {frames as fetchFrames, listJobs, video as fetchVideo, videoTicks as fetchVideoTicks} from '@/api/client';
import {subscribeJobEvents} from '@/api/jobEvents';
import {Job} from '@/api/types';
import {parseReplayJson, Replay} from '@/api/replay';
import {FramesManifest, VideoManifest, VideoTicksResponse} from '@/api/types';

/**
 * The live stream's unsubscribe callback, and the job id it watches.
 *
 * Module-scoped for the same reason `scriptStore` keeps its own outside
 * `state`: it is a closure, not serialisable data, and Pinia would wrap it in
 * a reactive proxy for no benefit. `watchedJobId` travels with it because the
 * two must always change together -- there is never a callback watching one
 * job while this name says another.
 */
let unsubscribe: (() => void) | null = null;
let watchedJobId: string | null = null;

/** `Job.id` is a decimal counter serialised as a string; compare it as a number. */
function numericId(id: string): number {
    return Number(id);
}

/** The job with the largest id, i.e. the most recently started one. `null` for an empty list. */
function mostRecent(jobs: Job[]): Job | null {
    return jobs.reduce<Job | null>(
        (best, candidate) => (best === null || numericId(candidate.id) > numericId(best.id) ? candidate : best),
        null
    );
}

/**
 * A run's replay document, read from the job list and kept live.
 *
 * **Which job's replay this shows**, because more than one job can exist and
 * the document is per-job: the most recent job that already carries a
 * `replay` -- the simplest answer that stays honest when several runs have
 * happened. That alone would miss a run still in progress, so this store also
 * watches the single overall most recent job's live event stream regardless
 * of whether it has produced a replay yet, and a `replay` event arriving on
 * that stream supersedes whatever `refresh` last found. A later `refresh`
 * never un-shows a live update by falling back to an older job -- see
 * `applyCandidate`.
 */
export const useReplayStore = defineStore('replay', {
    state: () => ({
        /** The job whose document is currently displayed, or `null` for none yet. */
        jobId: null as string | null,
        replay: null as Replay | null,
        /**
         * The frame manifest, or `null` when it has not been fetched or the
         * request failed. `null` is not "no frames": an empty manifest is a
         * fact (capture has not run) and a failed fetch is not, so the two
         * must not collapse. `ReplayScrubber` treats both as nothing to
         * judge, but only because it is told them separately.
         */
        manifest: null as FramesManifest | null,
        /**
         * The live recording's manifest and clock, on the same terms as
         * `manifest`: `null` means "not fetched or the request failed", never
         * "no video". A run that recorded none answers a manifest describing
         * nothing, which is a fact and reaches here as a value.
         *
         * Fetched beside the replay for the same reason the frame manifest is:
         * they are joined by tick, so a clock read at a later moment than the
         * replay it is judged against is exactly the mismatch the join check
         * exists to catch.
         */
        video: null as VideoManifest | null,
        videoTicks: null as VideoTicksResponse | null,
        /**
         * Set when the most recently *received* replay text failed to parse.
         * The previous good `replay`, if any, is left on screen rather than
         * being wiped by a bad update -- a malformed document is a reason to
         * distrust the update, not the one that came before it.
         */
        parseError: null as string | null,
        loading: false
    }),
    getters: {
        getJobId(): string | null {
            return this.jobId
        },
        getReplay(): Replay | null {
            return this.replay
        },
        getParseError(): string | null {
            return this.parseError
        },
        getManifest(): FramesManifest | null {
            return this.manifest
        },
        getVideo(): VideoManifest | null {
            return this.video
        },
        getVideoTicks(): VideoTicksResponse | null {
            return this.videoTicks
        },
        isLoading(): boolean {
            return this.loading
        }
    },
    actions: {
        /**
         * Re-reads the job list and reconciles both halves of "both paths
         * happen": the on-load path (`Job.replay` of a finished job) and the
         * live path (subscribing to whichever job is now most recent).
         */
        async refresh(): Promise<void> {
            this.loading = true
            try {
                // Fetched alongside the replay rather than on its own timer:
                // the two are joined by tick, so a manifest from a later
                // moment than the replay it is judged against is exactly the
                // mismatch the join check exists to catch.
                //
                // A failed frames fetch must not lose the replay -- the
                // timeline is useful without pictures, and the reverse is not
                // true. So this is caught and left `null` rather than allowed
                // to abort the refresh.
                try {
                    this.manifest = await fetchFrames()
                } catch {
                    this.manifest = null
                }
                // Same rule again, and it matters more here: a server too old
                // to have the video routes 404s both of these, and losing the
                // replay over an artefact that is opt-in in the first place
                // would be the worst possible trade.
                try {
                    this.video = await fetchVideo()
                } catch {
                    this.video = null
                }
                try {
                    this.videoTicks = await fetchVideoTicks()
                } catch {
                    this.videoTicks = null
                }
                const jobs = await listJobs()
                const latest = mostRecent(jobs)
                if (latest !== null && latest.id !== watchedJobId) {
                    this.watchLive(latest.id)
                }

                const withReplay = jobs.filter((j) => j.replay !== null)
                const best = mostRecent(withReplay)
                if (best !== null && best.replay !== null) {
                    this.applyCandidate(best.id, best.replay)
                }
            } finally {
                this.loading = false
            }
        },
        /**
         * Displays `json` for `jobId`, unless what is already on screen came
         * from an equal or more recent job. A `refresh` runs on a snapshot of
         * the job list that can already be stale by the time it resolves --
         * in particular, a `replay` event delivered live in the meantime is
         * always at least as recent as anything a following `refresh` can
         * find, and must not be overwritten by it.
         */
        applyCandidate(jobId: string, json: string): void {
            if (this.jobId !== null && numericId(jobId) < numericId(this.jobId)) {
                return
            }
            this.setReplayJson(jobId, json)
        },
        /**
         * Parses `json` and, on success, makes it the displayed document.
         * Called both from a live `replay` event and from `refresh` finding
         * one in the job list -- either way the text arrives verbatim and
         * unparsed, exactly as `subscribeJobEvents` and `Job.replay` hand it
         * over.
         */
        setReplayJson(jobId: string, json: string): void {
            const result = parseReplayJson(json)
            if ('replay' in result) {
                this.jobId = jobId
                this.replay = result.replay
                this.parseError = null
            } else {
                this.parseError = result.error
            }
        },
        /**
         * Subscribes to `jobId`'s event stream for its `replay` event alone.
         * Replaces any previous subscription -- there is only ever one job
         * worth watching live, the current overall most recent one.
         */
        watchLive(jobId: string): void {
            this.stopWatching()
            watchedJobId = jobId
            unsubscribe = subscribeJobEvents(jobId, {
                onOutput: () => undefined,
                onLagged: () => undefined,
                onReplay: (json: string) => {
                    this.applyCandidate(jobId, json)
                },
                onFinished: () => undefined,
                onError: () => undefined
            })
        },
        /** Stops the live subscription, if one is open. Safe to call at any time. */
        stopWatching(): void {
            if (unsubscribe !== null) {
                unsubscribe()
                unsubscribe = null
            }
            watchedJobId = null
        }
    }
})
