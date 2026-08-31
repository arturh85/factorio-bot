import {defineStore} from 'pinia'
import {sendRcon} from '@/api/client';

/**
 * Raw RCON commands, over HTTP.
 *
 * `POST /api/v1/rcon` answers `204` on success and, per
 * `crates/server/src/manage/rcon.rs`, `ErrorResponse::not_started()` -- a
 * `400` carrying `code: 2` -- when no Factorio instance is running.
 *
 * There is deliberately no branch on that code here, and that is not an
 * oversight. `instanceStore.stopInstances` treats `code: 2` as success because
 * "no Factorio" is the state its caller asked for; this store has no such
 * reading. The user typed a command, the command did not run, and calling that
 * a success would leave the RCON page claiming it had done something it had
 * not. Every failure is therefore reported the same way.
 *
 * What must not happen is a branch on the *status*: `code: 2` is a `400` from
 * this route (`not_started`) and a `503` from the script-execution route
 * (`not_running`), so status and condition are not in correspondence. The
 * spec drives both numbers through this store for that reason.
 *
 * The action rejects. `RconPage.vue` shows its error toast from its own
 * `catch`, and the Tauri store this replaces swallowed the error while *also*
 * setting `success = true`, so that toast could never fire and a failed
 * command looked exactly like one that worked.
 *
 * There is deliberately no `lastError` string here, unlike `instanceStore`.
 * That store needs one because `GET /api/v1/instance` reports the failure of a
 * start that was accepted minutes earlier -- a failure that reaches the browser
 * through no rejection at all, so the state field *is* the channel. Here the
 * failure is the rejection: because it is rethrown unwrapped, every caller
 * already holds the thrown value, and a copy of its message would be state
 * derived from something the reader has in hand. Worse, it would be unsafe to
 * read: the empty-command guard below rejects *before* the resets, so a
 * `lastError` would still name the previous command's failure at the moment an
 * accidental blank submit was rejected, and a page trusting it would report the
 * wrong error.
 */
export const useRconStore = defineStore('rcon', {
    state: () => ({
        executing: false,
        success: false,
        error: false
    }),
    getters: {
        isExecuting(): boolean {
            return this.executing
        }
    },
    actions: {
        /**
         * Sends one command and settles when the server has answered.
         *
         * The empty-command guard runs before the resets below, so an
         * accidental submit of a blank box leaves the outcome of the previous
         * command on screen rather than silently blanking it.
         */
        async execute(command: string): Promise<void> {
            if (!command) {
                throw new Error('no command to execute?')
            }
            this.error = false
            this.success = false
            this.executing = true
            try {
                await sendRcon(command)
                this.success = true
            } catch (err) {
                this.error = true
                // Rethrown unwrapped: the page renders the message off this
                // value, and anything else that catches it still needs the
                // server's `code`.
                throw err
            } finally {
                this.executing = false
            }
        }
    }
})
