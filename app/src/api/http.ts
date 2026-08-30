/**
 * The HTTP transport every store calls instead of Tauri's `invoke()`.
 *
 * One `fetch` wrapper, no retry: the routes it serves include non-idempotent
 * POSTs (`/api/v1/rcon`, `/api/v1/scripts/execute`), where a silent second
 * attempt would be worse than the failure it hides.
 */

export type QueryValue = string | number | boolean | undefined;

/**
 * Prefix for every request path.
 *
 * Empty by default: in production the axum server serves both the SPA and the
 * API from one origin. `VITE_API_BASE` exists for a Vite dev server that is
 * not proxying to the backend -- as of this commit `vite.config.mts` declares
 * no `server.proxy`, so a dev session needs either that proxy or this variable.
 */
export function apiBase(): string {
    return (import.meta.env.VITE_API_BASE as string | undefined) ?? '';
}

export function buildUrl(path: string, query?: Record<string, QueryValue>): string {
    let url = apiBase() + path;
    if (query) {
        const params = new URLSearchParams();
        for (const [key, value] of Object.entries(query)) {
            // An omitted option must vanish from the URL. Letting it through
            // would send the literal string "undefined", which the server
            // would either reject or, worse, accept as a path.
            if (value !== undefined) {
                params.append(key, String(value));
            }
        }
        const search = params.toString();
        if (search.length > 0) {
            url += '?' + search;
        }
    }
    return url;
}

/** A non-2xx answer from the API, with the server's own message if it sent one. */
export class ApiError extends Error {
    readonly status: number;
    /**
     * The application code from the error body, *not* the HTTP status: the
     * 409 from `POST /api/v1/scripts/execute` carries `code: 5`. `null` when
     * the body was not the server's JSON error shape.
     */
    readonly code: number | null;
    /** The parsed error body, or the raw text when it was not JSON. */
    readonly body: unknown;

    constructor(status: number, message: string, code: number | null, body: unknown) {
        super(message);
        this.name = 'ApiError';
        this.status = status;
        this.code = code;
        this.body = body;
    }
}

export interface RequestOptions {
    method?: string;
    query?: Record<string, QueryValue>;
    body?: unknown;
    signal?: AbortSignal;
}

/**
 * Turns a failed response into an `ApiError`.
 *
 * `crates/server/src/error.rs` serialises `{ message, code }` for every
 * handler failure, plus `running_job_id` on the execute route's 409. Anything
 * else reaching here -- a proxy's HTML page, axum's plain-text router
 * fallback -- has no message worth showing, so the status line is used and the
 * raw payload is kept on `body` for a caller that wants to look.
 */
async function errorFromResponse(response: Response): Promise<ApiError> {
    let body: unknown = null;
    let code: number | null = null;
    // statusText is empty over HTTP/2, hence the trim: "502", not "502 ".
    let message = (response.status + ' ' + response.statusText).trim();

    // A body that fails mid-read must still produce a usable error rather
    // than replacing the status with a stream failure the caller cannot act on.
    const text = await response.text().catch(() => '');
    if (text.length > 0) {
        try {
            body = JSON.parse(text);
        } catch {
            body = text;
        }
    }

    if (body !== null && typeof body === 'object') {
        const record = body as Record<string, unknown>;
        const detail = record.message;
        if (typeof detail === 'string' && detail.length > 0) {
            message = detail;
        }
        if (typeof record.code === 'number') {
            code = record.code;
        }
    }

    return new ApiError(response.status, message, code, body);
}

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const {method = 'GET', query, body, signal} = options;
    const headers: Record<string, string> = {Accept: 'application/json'};
    const init: RequestInit = {method, signal, headers};
    if (body !== undefined) {
        headers['Content-Type'] = 'application/json';
        init.body = JSON.stringify(body);
    }

    const response = await fetch(buildUrl(path, query), init);
    if (!response.ok) {
        throw await errorFromResponse(response);
    }
    // 204 is the success answer of several mutation routes in this API, and
    // `JSON.parse('')` throws. Keyed on the body rather than the status so an
    // empty 200 is handled too.
    const text = await response.text();
    if (text.length === 0) {
        return undefined as T;
    }
    return JSON.parse(text) as T;
}
