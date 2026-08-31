/**
 * The consumer of `GET /api/v1/jobs/{id}/events`.
 *
 * The stream is single-shot on purpose. `EventSource` reconnects by itself
 * after any error, and the server replays a job's buffered output to every new
 * subscriber -- so an unattended reconnect would append the whole run a second
 * time. This closes on the first error instead and asks `GET /api/v1/jobs/{id}`
 * once what actually happened.
 *
 * That one question also covers the case the wire format cannot express: a
 * server shutting down mid-run ends the stream cleanly, with a `200` and no
 * `finished` event. **End of stream is not a result.** A dropped connection, a
 * proxy timeout and a killed server are indistinguishable from here, and all
 * three resolve the same way -- by reading the job back.
 *
 * One thing this module cannot repair, and a caller that renders the output
 * should know: a subscriber that attaches after some output has already been
 * produced receives the backlog as all of stdout and then all of stderr, not
 * interleaved in the order the two were written -- the server keeps them in two
 * separate buffers. Live events, delivered one at a time, do keep their real
 * order. A replayed run therefore reads differently from a watched one.
 */

import {getJob, jobEventsUrl} from './client';
import {JobStatus, OutputStream} from './types';

/** The slice of `EventSource` this module uses, so tests can supply a fake. */
export interface EventSourceLike {
    addEventListener(type: string, listener: (event: MessageEvent) => void): void;
    close(): void;
}

export interface JobEventHandlers {
    onOutput(stream: OutputStream, text: string): void;
    /**
     * The server dropped `skipped` messages for this slow subscriber. Not an
     * error: it is the gap being *reported* rather than hidden, and the stream
     * continues. `0` means the server did not say how many.
     */
    onLagged(skipped: number): void;
    /**
     * The run's replay document, verbatim JSON text -- this module does not
     * parse it, the same way the server does not. Optional, so existing
     * callers that do not care about replays do not break.
     */
    onReplay?(json: string): void;
    onFinished(status: JobStatus): void;
    onError(error: Error): void;
}

/** The two statuses a job can end in; `running` is not an outcome. */
const OUTCOMES: readonly string[] = ['succeeded', 'failed'];

function parse(event: MessageEvent): Record<string, unknown> | null {
    if (typeof event.data !== 'string' || event.data.length === 0) {
        return null;
    }
    try {
        const parsed: unknown = JSON.parse(event.data);
        return parsed !== null && typeof parsed === 'object'
            ? parsed as Record<string, unknown>
            : null;
    } catch {
        return null;
    }
}

/** The status of a *finished* job, or `null` when the payload does not carry one. */
function outcome(value: unknown): JobStatus | null {
    return typeof value === 'string' && OUTCOMES.includes(value)
        ? value as JobStatus
        : null;
}

/**
 * Subscribes to a job's output stream. Returns an unsubscribe function that is
 * safe to call at any point, including after the job has already finished and
 * while the outcome lookup below is still in flight -- once it has been called
 * no handler runs again, so a caller that unmounts cannot be woken by a late
 * answer.
 */
export function subscribeJobEvents(
    jobId: string,
    handlers: JobEventHandlers,
    createSource: (url: string) => EventSourceLike = (url) => new EventSource(url)
): () => void {
    // `settled` latches the terminal event; `live` also covers the window in
    // which the outcome is being looked up, which `settled` has already closed.
    let settled = false;
    let live = true;
    const source = createSource(jobEventsUrl(jobId));

    const close = () => {
        try {
            source.close();
        } catch {
            // already closed; nothing to do
        }
    };

    /**
     * Resolves an outcome the stream did not deliver, from the job itself.
     * Reached by a broken connection and by a `finished` event whose payload
     * could not be read -- in both cases the run's result is unknown here, and
     * assuming it succeeded is the failure this whole module exists to avoid.
     */
    const resolveOutcome = () => {
        getJob(jobId).then((job) => {
            if (!live) {
                return;
            }
            if (job.status === 'running') {
                handlers.onError(new Error(
                    'lost the job output stream while the job was still running'
                ));
            } else {
                handlers.onFinished(job.status);
            }
        }).catch(() => {
            if (live) {
                handlers.onError(new Error('lost connection to the server'));
            }
        });
    };

    source.addEventListener('output', (event) => {
        if (settled) {
            return;
        }
        const data = parse(event);
        // A line whose text is not a string is not an empty line -- it is an
        // event this build does not understand, and inventing `''` for it would
        // put a phantom blank line in the transcript.
        if (data === null || typeof data.text !== 'string') {
            return;
        }
        const stream: OutputStream = data.stream === 'stderr' ? 'stderr' : 'stdout';
        handlers.onOutput(stream, data.text);
    });

    source.addEventListener('lagged', (event) => {
        if (settled) {
            return;
        }
        // Reported even when the count is unreadable: dropping the event would
        // hide the hole, which is exactly what the server refused to do.
        const data = parse(event);
        const skipped = data !== null && typeof data.skipped === 'number' ? data.skipped : 0;
        handlers.onLagged(skipped);
    });

    source.addEventListener('replay', (event) => {
        if (settled) {
            return;
        }
        // Passed through as raw text, not `parse()`d: the server embeds the
        // document verbatim in `data:` precisely so a caller here gets the
        // JSON text once, rather than a string it would have to parse again.
        if (typeof event.data === 'string' && event.data.length > 0) {
            handlers.onReplay?.(event.data);
        }
    });

    source.addEventListener('finished', (event) => {
        if (settled) {
            return;
        }
        settled = true;
        close();
        const status = outcome(parse(event)?.status);
        if (status === null) {
            resolveOutcome();
        } else {
            handlers.onFinished(status);
        }
    });

    source.addEventListener('error', () => {
        // Fires on a clean end-of-stream too, which is why `settled` is checked
        // first: after `finished` there is nothing left to report.
        if (settled) {
            return;
        }
        settled = true;
        close();
        resolveOutcome();
    });

    return () => {
        settled = true;
        live = false;
        close();
    };
}
