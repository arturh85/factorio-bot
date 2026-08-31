import {defineStore} from 'pinia'
import {getInstance, startInstance, stopInstance} from '@/api/client';
import {ApiError} from '@/api/http';
import {useAppStore} from '@/store/appStore';

/**
 * "No Factorio instance is running", as `crates/server/src/error.rs` codes it.
 *
 * Branching on this rather than on an HTTP status is not a stylistic
 * preference: `ErrorResponse::not_started()` answers `400` (which is what
 * `POST /api/v1/instance/stop` sends) and `ErrorResponse::not_running()`
 * answers `503` for the same condition, both with `code: 2`. Code keeps the
 * two readings together; a status check would handle one and miss the other.
 */
const CODE_NOT_RUNNING = 2;

/**
 * How often a start in flight is re-read from `GET /api/v1/instance`.
 *
 * Two seconds against an 8-10 minute first-run extraction is roughly 250
 * requests, each of them an atomic load and a lock -- cheap enough that the
 * cost worth avoiding is not the rate but the *duration*, which is why the
 * poll stops itself (see `pollWhileStarting`).
 */
const POLL_INTERVAL_MS = 2000;

/**
 * The poll's timer handle.
 *
 * Module-scoped rather than in `state` for two reasons. Pinia's state is
 * deeply reactive, and wrapping a platform timer handle in a proxy is asking
 * for a `clearInterval` that silently fails to match. And the handle is not
 * data any view should read: nothing renders it, and devtools showing it would
 * be noise. The scope is the same either way -- one store instance per page.
 */
let pollTimer: ReturnType<typeof setInterval> | null = null;

/**
 * Set while a poll's request is outstanding.
 *
 * The interval fires on a clock, not on the previous answer. Without this a
 * server slower than two seconds would accumulate one in-flight request per
 * tick and the queue would never drain.
 */
let pollInFlight = false;

/** The message of any thrown value, `ApiError` included -- its `message` is the server's. */
function messageOf(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
}

/**
 * The Factorio instance, over HTTP.
 *
 * Starting Factorio is a *background job on the server*: `POST
 * /api/v1/instance/start` answers `202 {accepted: true}` and returns
 * immediately, because a first-run archive extraction takes 8-10 minutes and
 * no browser or proxy would hold a request open that long. Accepted is not
 * started, so nothing here sets `started` off that answer -- only a poll of
 * `GET /api/v1/instance` may, and this store owns that poll.
 *
 * The timer lived in `App.vue` until the poll was actually built, on the
 * argument that a store owning one would leak it into every test. It does not:
 * no timer exists until `pollWhileStarting` is called with a start in flight.
 * What the `App.vue` version cost was the ability to test any of this, since
 * the project has no component-test harness -- and the result was a suite that
 * drove a `setInterval` of its own over `checkInstanceState` while production
 * ran no poll at all.
 */
export const useInstanceStore = defineStore('instance', {
    state: () => ({
        starting: false,
        stopping: false,
        started: false,
        failed: false,
        lastError: null as string | null,
        clientCount: 0
    }),
    getters: {
        isStarting(): boolean {
            return this.starting
        },
        isStopping(): boolean {
            return this.stopping
        },
        isFailed(): boolean {
            return this.failed
        },
        isStarted(): boolean {
            return this.started
        },
        getLastError(): string | null {
            return this.lastError
        }
    },
    actions: {
        /**
         * One poll of `GET /api/v1/instance`, answering whether Factorio is up.
         *
         * The server is authoritative for all four fields, including
         * `last_error`: a start that failed minutes after its `202` has no
         * other way to reach the browser, and a start that has since succeeded
         * must be able to clear the flag again.
         *
         * Every assignment happens after the await, so a failed poll -- a
         * proxy hiccup, a server restart -- rejects with the last known status
         * intact rather than blanking a UI that was correct a moment ago.
         */
        async checkInstanceState(): Promise<boolean> {
            const status = await getInstance()
            this.started = status.started
            this.starting = status.starting
            this.clientCount = status.client_count
            this.lastError = status.last_error
            this.failed = status.last_error !== null
            return this.started
        },
        /**
         * Watches an in-flight start through to its outcome, and no longer.
         *
         * A no-op unless `starting` is set, which is the whole cost model: a
         * tab that opens onto an idle server makes its one mount-time request
         * and then nothing, and a tab watching a start stops the moment the
         * server reports an instance or a `last_error`. The alternative --
         * polling forever -- spends a request every two seconds per open tab
         * for as long as the tab is open, to learn nothing new.
         *
         * The price of stopping is that a tab which was idle when *another*
         * tab started Factorio does not notice. `startInstances` restarting
         * the poll covers the case that matters (this tab's own start); the
         * stale tab's Start button answers 409 with the server's own words,
         * which is a legible outcome rather than a silent one.
         */
        pollWhileStarting(): void {
            if (!this.starting) {
                // Idempotent for the caller's sake: `App.vue` calls this on
                // every mount without first asking what the server said.
                this.stopPolling()
                return
            }
            if (pollTimer !== null) {
                return
            }
            pollTimer = setInterval(() => {
                void this.pollOnce()
            }, POLL_INTERVAL_MS)
        },
        /** One tick of the poll: re-read the status, then decide whether to keep going. */
        async pollOnce(): Promise<void> {
            if (pollInFlight) {
                return
            }
            pollInFlight = true
            try {
                await this.checkInstanceState()
            } catch (err) {
                // A failed poll is not an outcome. `checkInstanceState` leaves
                // the last known status intact, the start is still running,
                // and the next tick may well answer -- so this neither stops
                // the poll nor writes a failure the server never reported.
                // Swallowing it also keeps an unhandled rejection out of a
                // `setInterval` callback, where nothing could catch it.
                void err
            } finally {
                pollInFlight = false
            }
            // Stop, never (re)start: an unmount that landed while this
            // request was outstanding has already cleared the timer, and
            // routing back through `pollWhileStarting` here would resurrect it.
            if (!this.starting) {
                this.stopPolling()
            }
        },
        /**
         * Clears the poll's timer.
         *
         * `App.vue` calls this from `onUnmounted`. A timer that outlives its
         * component leaks one per teardown, and this one holds a reference to
         * the store it polls.
         */
        stopPolling(): void {
            if (pollTimer !== null) {
                clearInterval(pollTimer)
                pollTimer = null
            }
        },
        /**
         * Asks the server to start Factorio and returns as soon as it accepts.
         *
         * `starting` stays set until a poll says otherwise: this call learns
         * nothing about whether Factorio came up, and the failure it would
         * report arrives minutes later in `last_error`. So an accepted start
         * begins the poll here rather than in the component that clicked it --
         * every caller of this action wants the outcome, and putting it here
         * spares `ProcessControl.vue` from knowing a timer exists.
         *
         * A *rejected* start starts no poll, which is what keeps the
         * `lastError` set below from being cleared two seconds later: the
         * message the user needs ("please set the factorio archive path")
         * survives precisely because nothing is watching the server, which
         * knows nothing about a request it refused or never received.
         */
        async startInstances(): Promise<void> {
            if (this.started) {
                throw new Error('already started')
            }
            const appStore = useAppStore()
            if (!appStore.settings?.factorio.factorio_archive_path) {
                throw new Error('please set the factorio archive path under settings first')
            }
            // The server clears its own `last_start_error` when it accepts a
            // start. Clearing here too keeps this store from showing a message
            // the very next poll would contradict.
            this.failed = false
            this.lastError = null
            this.starting = true
            try {
                await startInstance()
            } catch (err) {
                this.starting = false
                this.failed = true
                this.lastError = messageOf(err)
                throw err
            }
            this.pollWhileStarting()
        },
        /**
         * Stops the running instance.
         *
         * `code: 2` back from the route is not treated as a failure. It means
         * the server has no instance, which is exactly the state the caller
         * asked for -- reachable whenever this store's `started` is stale
         * (the server was restarted, or the game died on its own). Reconciling
         * is the useful response; a toast saying "failed to stop" over a
         * Factorio that is already gone is not.
         */
        async stopInstances(): Promise<void> {
            if (!this.started) {
                throw new Error('not started')
            }
            this.failed = false
            this.lastError = null
            this.stopping = true
            try {
                await stopInstance()
                this.started = false
                this.clientCount = 0
            } catch (err) {
                if (err instanceof ApiError && err.code === CODE_NOT_RUNNING) {
                    this.started = false
                    this.clientCount = 0
                    return
                }
                // Anything else -- a 500 from a kill that failed, a transport
                // error with no code at all -- leaves `started` set: the
                // processes may still be alive, and reporting them gone would
                // hide a Factorio nothing in this UI can then reach.
                this.failed = true
                this.lastError = messageOf(err)
                throw err
            } finally {
                this.stopping = false
            }
        }
    }
})
