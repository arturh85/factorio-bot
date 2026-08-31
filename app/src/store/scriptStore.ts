import {defineStore} from 'pinia'
import {languageFromPath} from '@/utils';
import {executeScript as postExecute, listScripts, readScript, writeScript} from '@/api/client';
import {subscribeJobEvents} from '@/api/jobEvents';
import {ApiError} from '@/api/http';
import {ExecuteRequest, JobStatus, OutputStream} from '@/api/types';

/**
 * One node of the scripts tree, as `GET /api/v1/scripts` returns it.
 *
 * `@/api/types` does now export a nominal `ScriptTreeNode`, so this could be a
 * plain import. It stays derived from the client's return type on purpose:
 * that way the store follows whatever the route actually returns, and a change
 * to `listScripts`'s signature is a type error here rather than a silent
 * disagreement between two names that happen to match today.
 */
type ScriptTreeNode = Awaited<ReturnType<typeof listScripts>>[number];

/**
 * The unsubscribe callback of the live stream. Kept outside the Pinia state on
 * purpose: it is a closure, not serialisable data, and putting a function in
 * `state` makes it reactive for no reason.
 */
let unsubscribe: (() => void) | null = null;

/**
 * A line the *page* wrote into the transcript, not the script.
 *
 * One shape for all three (a reported gap, a gap of unknown size, a dead
 * stream) so a reader can tell at a glance which lines came from the run and
 * which came from the connection carrying it.
 */
function notice(text: string): string {
    return '... ' + text + ' ...\n';
}

/**
 * The job holding the execution slot, from a rejected execute request.
 *
 * Keyed on the 409 specifically. `code` cannot be used instead -- `code: 5` is
 * also the "script already exists" conflict from `POST /api/v1/scripts/file`
 * -- and neither can the presence of the field alone: attaching the output
 * pane to a job named by some *other* failure would show the user a run that
 * has nothing to do with what they just asked for.
 */
function runningJobIdOf(err: unknown): string | null {
    if (!(err instanceof ApiError) || err.status !== 409) {
        return null;
    }
    const body = err.body;
    if (body === null || typeof body !== 'object') {
        return null;
    }
    const id = (body as Record<string, unknown>).running_job_id;
    return typeof id === 'string' ? id : null;
}

/**
 * Scripts: the tree, the editor buffer, and the run.
 *
 * Running a script is a **job**, not a request that returns output.
 * `POST /api/v1/scripts/execute` answers `202` with a job id and the run
 * proceeds detached; the output arrives over SSE from
 * `GET /api/v1/jobs/{id}/events`, and the outcome with it. So `executeScript`
 * resolving means the run *started* -- `success` is set later, by the stream,
 * and only ever from a `finished` event.
 *
 * **The two output buffers are kept separate, and the page renders them as two
 * blocks, deliberately.** A subscriber that attaches to a run already in
 * progress receives the backlog as all of stdout and then all of stderr: the
 * server keeps two buffers and their relative interleaving is not recoverable
 * (`crates/server/src/manage/execute.rs::backlog`). Merging them into one pane
 * would therefore present an order that, for every replayed run, is invented.
 * Live events do keep their real order, which is why each buffer on its own is
 * honest.
 */
export const useScriptStore = defineStore('script', {
    state: () => ({
        code: '',
        language: 'lua',
        executing: false,
        success: false,
        error: false,
        stdout: '',
        stderr: '',
        /** The job being watched, or `null` when no run has started. */
        jobId: null as string | null,

        activeScriptPath: '',
        loadingScriptsInDirectory: false
    }),
    getters: {
        isExecuting(): boolean {
            return this.executing
        },
        getCode(): string {
            return this.code
        },
        getLanguage(): string {
            return this.language
        },
        getStdout(): string {
            return this.stdout
        },
        getStderr(): string {
            return this.stderr
        },
        getLoadingScriptsInDirectory(): boolean {
            return this.loadingScriptsInDirectory
        },
        getActiveScriptPath(): string {
            return this.activeScriptPath
        }
    },
    actions: {
        async loadScriptsInDirectory(path: string): Promise<ScriptTreeNode[]> {
            this.loadingScriptsInDirectory = true
            try {
                return await listScripts(path)
            } finally {
                // In `finally`, not after the await: the tree expands one
                // directory at a time, and a listing that failed with the flag
                // left set would leave the node spinning forever.
                this.loadingScriptsInDirectory = false
            }
        },
        async loadScriptFile(path: string): Promise<string> {
            this.activeScriptPath = path
            const content = await readScript(path)
            this.code = content.code
            this.language = languageFromPath(path)
            return content.code
        },
        /**
         * Updates the buffer and saves it back to the file it came from.
         *
         * Inline code typed with no file selected has nowhere to go --
         * `PUT /api/v1/scripts/file` requires a path and answers 400 without
         * one -- so it stays in the buffer, from where `executeCode` can run it.
         */
        async setCode(code: string): Promise<void> {
            this.code = code
            if (!this.activeScriptPath) {
                return
            }
            await writeScript(this.activeScriptPath, this.code)
        },
        /**
         * Watches a job: subscribes to its event stream and mirrors it into
         * state.
         *
         * The buffers are cleared first because the server replays the job's
         * whole backlog to every new subscriber; keeping what was on screen
         * would interleave two runs in one pane.
         */
        attachToJob(jobId: string): void {
            this.stopWatching()
            this.stdout = ''
            this.stderr = ''
            this.success = false
            this.error = false
            this.jobId = jobId
            this.executing = true
            unsubscribe = subscribeJobEvents(jobId, {
                onOutput: (stream: OutputStream, text: string) => {
                    if (stream === 'stderr') {
                        this.stderr += text + '\n'
                    } else {
                        this.stdout += text + '\n'
                    }
                },
                onLagged: (skipped: number) => {
                    // A visible gap beats silently missing lines. `0` is the
                    // count being *unknown*, not zero -- the server sent the
                    // event precisely because something was dropped, so
                    // printing "0 lines skipped" would deny the one fact it
                    // came to report.
                    this.stdout += skipped > 0
                        ? notice(skipped + ' lines skipped (the page could not keep up with the output)')
                        : notice('some lines skipped (the server did not say how many)')
                },
                onFinished: (status: JobStatus) => {
                    this.executing = false
                    this.success = status === 'succeeded'
                    this.error = status === 'failed'
                },
                onError: (err: Error) => {
                    this.executing = false
                    this.success = false
                    this.error = true
                    this.stderr += notice(err.message)
                }
            })
        },
        /**
         * Stops watching the current job.
         *
         * `executing` is cleared with it: the store is no longer being told
         * anything about the run, so it cannot claim one is in progress -- and
         * a flag stuck at `true` would leave the Run button disabled for the
         * rest of the session, since nothing else ever clears it. The job may
         * well still be running on the server, which is what the 409 path in
         * `_execute` is for.
         */
        stopWatching(): void {
            if (unsubscribe !== null) {
                unsubscribe()
                unsubscribe = null
            }
            this.executing = false
        },
        async executeCode(): Promise<void> {
            if (!this.code) {
                throw new Error('no code to execute?')
            }
            await this._execute({code: this.code, language: this.language})
        },
        async executeScript(): Promise<void> {
            if (!this.activeScriptPath) {
                throw new Error('no script to execute?')
            }
            await this._execute({path: this.activeScriptPath})
        },
        /**
         * Starts a run and attaches to it.
         *
         * The guards above run before anything here, so an accidental Run on
         * an empty buffer leaves the previous run's transcript on screen
         * instead of blanking it.
         */
        async _execute(body: ExecuteRequest): Promise<void> {
            this.executing = true
            this.jobId = null
            try {
                const accepted = await postExecute(body)
                this.attachToJob(accepted.job_id)
            } catch (err) {
                // One script runs at a time. When the slot is taken the server
                // names the occupant, so follow it: the user sees the run that
                // is actually going rather than an error with no way forward.
                // The rejection still stands -- *this* run did not start, and
                // the page reports that from its own catch.
                const runningJobId = runningJobIdOf(err)
                if (runningJobId !== null) {
                    this.attachToJob(runningJobId)
                } else {
                    this.executing = false
                    this.success = false
                    this.error = true
                }
                throw err
            }
        }
    }
})
