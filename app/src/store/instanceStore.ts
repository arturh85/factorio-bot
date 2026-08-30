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
 * `GET /api/v1/instance` may, and that poll's timer lives in `App.vue`. A
 * store that owned the timer would leak one into every test that touched it.
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
         * Asks the server to start Factorio and returns as soon as it accepts.
         *
         * `starting` stays set until a poll says otherwise: this call learns
         * nothing about whether Factorio came up, and the failure it would
         * report arrives minutes later in `last_error`.
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
