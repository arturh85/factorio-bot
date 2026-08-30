import {afterEach, describe, expect, it, vi} from 'vitest';
import {ApiError, apiBase, buildUrl, request} from './http';

function jsonResponse(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
        status,
        headers: {'Content-Type': 'application/json'}
    });
}

/**
 * A `fetch` double that records its arguments. Typed with the two parameters
 * so the assertions on `init` do not have to reach past the end of a tuple.
 */
function stubFetch(respond: () => Promise<Response>) {
    const fetchMock = vi.fn((_url: string, init?: RequestInit) => {
        void init;
        return respond();
    });
    vi.stubGlobal('fetch', fetchMock);
    return fetchMock;
}

function initOf(fetchMock: ReturnType<typeof stubFetch>): RequestInit {
    return fetchMock.mock.calls[0][1] as RequestInit;
}

afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
});

describe('buildUrl', () => {
    it('drops undefined query values instead of sending the string "undefined"', () => {
        expect(buildUrl('/api/v1/scripts', {path: '/a.lua', bot_count: undefined}))
            .toBe('/api/v1/scripts?path=%2Fa.lua');
    });

    it('encodes reserved characters in query values', () => {
        expect(buildUrl('/api/v1/fs/exists', {path: '/tmp/a b&c'}))
            .toBe('/api/v1/fs/exists?path=%2Ftmp%2Fa+b%26c');
    });

    it('serialises numeric and boolean query values', () => {
        expect(buildUrl('/api/v1/scripts/execute', {bot_count: 2, headless: false}))
            .toBe('/api/v1/scripts/execute?bot_count=2&headless=false');
    });

    it('omits the question mark when there is no query', () => {
        expect(buildUrl('/api/v1/settings')).toBe('/api/v1/settings');
    });

    it('omits the question mark when every query value was undefined', () => {
        expect(buildUrl('/api/v1/settings', {path: undefined})).toBe('/api/v1/settings');
    });

    it('prefixes VITE_API_BASE so a non-proxying dev server can reach the backend', () => {
        vi.stubEnv('VITE_API_BASE', 'http://localhost:7123');
        expect(apiBase()).toBe('http://localhost:7123');
        expect(buildUrl('/api/v1/settings')).toBe('http://localhost:7123/api/v1/settings');
    });

    it('prefixes nothing when VITE_API_BASE is unset', () => {
        expect(apiBase()).toBe('');
    });
});

describe('request', () => {
    it('parses the JSON body of a successful response', async () => {
        stubFetch(async () => jsonResponse(200, {job_id: 7, status: 'running'}));
        await expect(request<{job_id: number}>('/api/v1/jobs/7'))
            .resolves.toEqual({job_id: 7, status: 'running'});
    });

    it('requests the built url with the query string attached', async () => {
        const fetchMock = stubFetch(async () => jsonResponse(200, []));
        await request('/api/v1/scripts', {query: {path: '/a b.lua'}});
        expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/scripts?path=%2Fa+b.lua');
    });

    it('defaults to GET with only an Accept header', async () => {
        const fetchMock = stubFetch(async () => jsonResponse(200, []));
        await request('/api/v1/scripts');
        const init = initOf(fetchMock);
        expect(init.method).toBe('GET');
        // toEqual, not a `not.toHaveProperty`: an absence assertion passes
        // just as happily when the fixture is broken.
        expect(init.headers).toEqual({Accept: 'application/json'});
    });

    it('returns undefined for a 204 rather than choking on an empty body', async () => {
        stubFetch(async () => new Response(null, {status: 204}));
        await expect(request<void>('/api/v1/rcon', {method: 'POST', body: {command: '/x'}}))
            .resolves.toBeUndefined();
    });

    it('returns undefined for a 200 with an empty body', async () => {
        stubFetch(async () => new Response('', {status: 200}));
        await expect(request<void>('/api/v1/instance/stop', {method: 'POST'}))
            .resolves.toBeUndefined();
    });

    it('sends the body as JSON with a content-type', async () => {
        const fetchMock = stubFetch(async () => new Response(null, {status: 204}));
        await request<void>('/api/v1/rcon', {method: 'POST', body: {command: '/x'}});
        const init = initOf(fetchMock);
        expect(init.method).toBe('POST');
        expect(init.body).toBe('{"command":"/x"}');
        expect((init.headers as Record<string, string>)['Content-Type']).toBe('application/json');
    });

    it('hands the abort signal to fetch', async () => {
        const fetchMock = stubFetch(async () => jsonResponse(200, []));
        const controller = new AbortController();
        await request('/api/v1/scripts', {signal: controller.signal});
        expect(initOf(fetchMock).signal).toBe(controller.signal);
    });

    it('throws an ApiError carrying the servers message field', async () => {
        stubFetch(async () => jsonResponse(400, {message: 'not a file: /x', code: 1}));
        const error = await request('/api/v1/scripts/file', {query: {path: '/x'}}).catch(e => e);
        expect(error).toBeInstanceOf(ApiError);
        expect((error as ApiError).status).toBe(400);
        expect((error as ApiError).code).toBe(1);
        expect((error as ApiError).message).toBe('not a file: /x');
    });

    it('reads the 409 from the job registry, code and running_job_id included', async () => {
        // The exact body ErrorResponse::already_running serialises: see
        // crates/server/src/error.rs. `code` is the application code 5, not
        // the HTTP status, and the job id is its own field so the UI can
        // follow it to GET /api/v1/jobs/{id} without parsing prose.
        stubFetch(async () => jsonResponse(409, {
            message: 'a script is already running as job 2',
            code: 5,
            running_job_id: '2'
        }));
        const error = await request('/api/v1/scripts/execute', {method: 'POST', body: {}}).catch(e => e);
        expect((error as ApiError).status).toBe(409);
        expect((error as ApiError).code).toBe(5);
        expect((error as ApiError).message).toBe('a script is already running as job 2');
        expect(((error as ApiError).body as Record<string, unknown>).running_job_id).toBe('2');
    });

    it('reports the 503 when no Factorio instance is running', async () => {
        stubFetch(async () => jsonResponse(503, {message: 'not started', code: 2}));
        const error = await request('/api/v1/scripts/execute', {method: 'POST', body: {}}).catch(e => e);
        expect((error as ApiError).status).toBe(503);
        expect((error as ApiError).code).toBe(2);
        expect((error as ApiError).message).toBe('not started');
    });

    it('falls back to the status line when the error body is not JSON', async () => {
        // The body deliberately contains neither "502" nor "Bad Gateway", so
        // a message taken from the payload cannot satisfy this assertion.
        stubFetch(async () => new Response('<html><body>proxy is unhappy</body></html>', {
            status: 502,
            statusText: 'Bad Gateway'
        }));
        const error = await request('/api/v1/settings').catch(e => e);
        expect((error as ApiError).status).toBe(502);
        expect((error as ApiError).message).toBe('502 Bad Gateway');
        expect((error as ApiError).code).toBeNull();
        // The payload is not thrown away, only kept out of the message.
        expect((error as ApiError).body).toBe('<html><body>proxy is unhappy</body></html>');
    });

    it('leaves no trailing space when the response carries no status text', async () => {
        stubFetch(async () => new Response(null, {status: 503, statusText: ''}));
        const error = await request('/api/v1/settings').catch(e => e);
        expect((error as ApiError).message).toBe('503');
    });

    it('falls back to the status line when a JSON body carries no message', async () => {
        stubFetch(async () => new Response(JSON.stringify({unexpected: true}), {
            status: 500,
            statusText: 'Internal Server Error',
            headers: {'Content-Type': 'application/json'}
        }));
        const error = await request('/api/v1/settings').catch(e => e);
        expect((error as ApiError).message).toBe('500 Internal Server Error');
        expect((error as ApiError).code).toBeNull();
    });

    it('still reports the status when the error body fails to read', async () => {
        stubFetch(async () => new Response(
            new ReadableStream({
                start(controller) {
                    controller.error(new Error('connection reset mid-body'));
                }
            }),
            {status: 500, statusText: 'Internal Server Error'}
        ));
        const error = await request('/api/v1/settings').catch(e => e);
        expect(error).toBeInstanceOf(ApiError);
        expect((error as ApiError).message).toBe('500 Internal Server Error');
    });

    it('propagates a network failure rather than swallowing it', async () => {
        stubFetch(async () => {
            throw new TypeError('Failed to fetch');
        });
        await expect(request('/api/v1/settings')).rejects.toThrow('Failed to fetch');
    });
});
