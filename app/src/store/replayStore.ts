import {defineStore} from 'pinia'
import {listJobs} from '@/api/client';
import {subscribeJobEvents} from '@/api/jobEvents';
import {Job} from '@/api/types';
import {parseReplayJson, Replay} from '@/api/replay';

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
