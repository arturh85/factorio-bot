import {afterEach, describe, expect, it, vi} from 'vitest';
import * as client from './client';
import {ApiError} from './http';
import {AppSettings} from '@/models/settings';

/**
 * A `fetch` double that records its arguments.
 *
 * `init` is consumed with `void` rather than renamed: `no-unused-vars`
 * reports arguments *after* the last used one, so touching the second
 * parameter is what keeps the unused first one from being an error.
 */
function stubFetch(respond: () => Response) {
    const fetchMock = vi.fn((_url: string, init?: RequestInit) => {
        void init;
        return Promise.resolve(respond());
    });
    vi.stubGlobal('fetch', fetchMock);
    return fetchMock;
}

function json(status: number, body: unknown) {
    return stubFetch(() => new Response(JSON.stringify(body), {
        status,
        headers: {'Content-Type': 'application/json'}
    }));
}

/** 204 is the success answer of every mutation route that returns nothing. */
function noContent() {
    return stubFetch(() => new Response(null, {status: 204}));
}

function urlOf(fetchMock: ReturnType<typeof stubFetch>): string {
    return fetchMock.mock.calls[0][0];
}

function initOf(fetchMock: ReturnType<typeof stubFetch>): RequestInit {
    return fetchMock.mock.calls[0][1] as RequestInit;
}

/**
 * A fully populated settings object, typed rather than cast: this fixture is
 * also what proves `AppSettings` still describes what the server sends after
 * `restapi` stopped being `any`.
 */
function settingsFixture(): AppSettings {
    return {
        gui: {enable_autostart: false, enable_restapi: true},
        restapi: {port: 7492, web_root: null},
        factorio: {
            client_count: 2,
            factorio_archive_path: '/archives/factorio.tar.xz',
            map_exchange_string: '>>>abc<<<',
            rcon_pass: 'foobar',
            rcon_port: 4321,
            recreate: false,
            seed: '1234',
            workspace_path: '/ws'
        }
    };
}

afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
});

describe('settings routes', () => {
    it('reads settings from GET /api/v1/settings and returns the parsed body', async () => {
        const fetchMock = json(200, settingsFixture());
        await expect(client.getSettings()).resolves.toEqual(settingsFixture());
        expect(urlOf(fetchMock)).toBe('/api/v1/settings');
        expect(initOf(fetchMock).method).toBe('GET');
    });

    it('sends the whole settings object to PUT /api/v1/settings', async () => {
        const sent = settingsFixture();
        const fetchMock = json(200, sent);
        await client.putSettings(sent);
        const init = initOf(fetchMock);
        expect(init.method).toBe('PUT');
        // Byte-for-byte, so dropping or reshaping any branch of the object
        // fails rather than merely losing a field the server then defaults.
        expect(init.body).toBe(JSON.stringify(sent));
    });

    /**
     * A type-level guard, enforced by `pnpm run lint` rather than by vitest:
     * nothing vitest can observe distinguishes `restapi: RestApiSettings`
     * from the `restapi: any` this model used to carry, because both accept
     * the same runtime value.
     *
     * `@ts-expect-error` inverts that -- tsc reports an *unused* directive
     * when the line below compiles, which is exactly what `any` would make
     * happen.
     */
    it('types restapi instead of leaving it any', () => {
        // @ts-expect-error `web_root` is `string | null`, never a number.
        const rejected: AppSettings['restapi'] = {port: 7492, web_root: 7};
        expect(rejected.port).toBe(7492);
    });

    it('returns the settings the server echoed back, not the ones it was given', async () => {
        const sent = settingsFixture();
        const persisted = settingsFixture();
        persisted.factorio.workspace_path = '/resolved-by-the-server';
        json(200, persisted);
        const result = await client.putSettings(sent);
        expect(result.factorio.workspace_path).toBe('/resolved-by-the-server');
    });
});

describe('instance routes', () => {
    it('reads the instance status from GET /api/v1/instance', async () => {
        const status = {
            started: true,
            starting: false,
            client_count: 2,
            server_port: 34197,
            rcon_port: 4321,
            last_error: null
        };
        const fetchMock = json(200, status);
        await expect(client.getInstance()).resolves.toEqual(status);
        expect(urlOf(fetchMock)).toBe('/api/v1/instance');
        expect(initOf(fetchMock).method).toBe('GET');
    });

    it('starts the instance with POST /api/v1/instance/start', async () => {
        const fetchMock = json(202, {accepted: true});
        await expect(client.startInstance()).resolves.toEqual({accepted: true});
        expect(urlOf(fetchMock)).toBe('/api/v1/instance/start');
        expect(initOf(fetchMock).method).toBe('POST');
    });

    it('stops the instance with POST /api/v1/instance/stop and tolerates the empty 204', async () => {
        const fetchMock = noContent();
        await expect(client.stopInstance()).resolves.toBeUndefined();
        expect(urlOf(fetchMock)).toBe('/api/v1/instance/stop');
        expect(initOf(fetchMock).method).toBe('POST');
    });
});

describe('rcon route', () => {
    it('wraps an rcon command in the servers body shape', async () => {
        const fetchMock = noContent();
        await client.sendRcon('/server-save');
        // The command travels in the body; putting it in the query would
        // reach a route that answers 400 for a missing body.
        expect(urlOf(fetchMock)).toBe('/api/v1/rcon');
        expect(initOf(fetchMock).body).toBe('{"command":"/server-save"}');
        expect(initOf(fetchMock).method).toBe('POST');
    });
});

describe('script routes', () => {
    it('passes the tree key straight through as the path query parameter', async () => {
        const nodes = [{key: '/sub/a.lua', label: 'a.lua', leaf: true, children: []}];
        const fetchMock = json(200, nodes);
        await expect(client.listScripts('/sub')).resolves.toEqual(nodes);
        expect(urlOf(fetchMock)).toBe('/api/v1/scripts?path=%2Fsub');
    });

    it('reads a script with the path in the query', async () => {
        const fetchMock = json(200, {code: 'print(1)'});
        await expect(client.readScript('/a.lua')).resolves.toEqual({code: 'print(1)'});
        expect(urlOf(fetchMock)).toBe('/api/v1/scripts/file?path=%2Fa.lua');
        expect(initOf(fetchMock).method).toBe('GET');
    });

    it('writes a script with the path in the query and the code in the body', async () => {
        const fetchMock = noContent();
        await client.writeScript('/a.lua', 'print(1)');
        expect(urlOf(fetchMock)).toBe('/api/v1/scripts/file?path=%2Fa.lua');
        const init = initOf(fetchMock);
        expect(init.method).toBe('PUT');
        expect(init.body).toBe('{"code":"print(1)"}');
    });

    /**
     * Create and overwrite differ *only* by method on this route -- the
     * server refuses a POST over an existing file with 409 and a PUT to a
     * missing one with 400 -- so the method is the whole contract here.
     */
    it('creates a script with POST on the same path as the overwrite', async () => {
        const fetchMock = json(201, {});
        await client.createScript('/new.lua', 'print(2)');
        expect(urlOf(fetchMock)).toBe('/api/v1/scripts/file?path=%2Fnew.lua');
        const init = initOf(fetchMock);
        expect(init.method).toBe('POST');
        expect(init.body).toBe('{"code":"print(2)"}');
    });

    it('deletes a script with DELETE and no body', async () => {
        const fetchMock = noContent();
        await client.deleteScript('/a.lua');
        expect(urlOf(fetchMock)).toBe('/api/v1/scripts/file?path=%2Fa.lua');
        const init = initOf(fetchMock);
        expect(init.method).toBe('DELETE');
        expect(init.body).toBeUndefined();
    });
});

describe('filesystem route', () => {
    it('asks GET /api/v1/fs/exists about an absolute server path', async () => {
        const fetchMock = json(200, {exists: true});
        await expect(client.pathExists('/tmp/a b')).resolves.toEqual({exists: true});
        expect(urlOf(fetchMock)).toBe('/api/v1/fs/exists?path=%2Ftmp%2Fa+b');
    });
});

describe('execution routes', () => {
    it('posts an execution request and returns the job id', async () => {
        const fetchMock = json(202, {job_id: '7'});
        await expect(client.executeScript({path: '/a.lua'})).resolves.toEqual({job_id: '7'});
        expect(urlOf(fetchMock)).toBe('/api/v1/scripts/execute');
        expect(initOf(fetchMock).method).toBe('POST');
    });

    /**
     * The server reads "exactly one of `path` or `code`" off the two fields
     * being present, so a body that spells an absent field out as `null`
     * still works but a body that spells it out as an empty string would be
     * accepted as inline code. Sending only what the caller set keeps the
     * discriminant the caller's.
     */
    it('sends only the fields the caller set', async () => {
        const fetchMock = json(202, {job_id: '8'});
        await client.executeScript({code: 'print(1)', bot_count: 2});
        expect(initOf(fetchMock).body).toBe('{"code":"print(1)","bot_count":2}');
    });

    it('lists jobs from GET /api/v1/jobs', async () => {
        const jobs = [{
            id: '1',
            script: '/a.lua',
            status: 'succeeded',
            started_at_ms: 1,
            finished_at_ms: 2,
            stdout: 'out',
            stderr: '',
            error: null
        }];
        const fetchMock = json(200, jobs);
        await expect(client.listJobs()).resolves.toEqual(jobs);
        expect(urlOf(fetchMock)).toBe('/api/v1/jobs');
    });

    it('reads one job from GET /api/v1/jobs/{id}', async () => {
        const fetchMock = json(200, {id: '7'});
        await client.getJob('7');
        expect(urlOf(fetchMock)).toBe('/api/v1/jobs/7');
    });

    /**
     * The id is a path *segment*, so an id that is not the decimal counter
     * the server currently issues -- a stale link, a hand-typed value --
     * must not be able to redirect the request to another route.
     */
    it('escapes a job id that would otherwise change the request path', async () => {
        const fetchMock = json(404, {message: 'no such job', code: 4});
        await expect(client.getJob('../settings')).rejects.toBeInstanceOf(ApiError);
        expect(urlOf(fetchMock)).toBe('/api/v1/jobs/..%2Fsettings');
    });
});

describe('job event stream url', () => {
    it('names the events route for a job', () => {
        expect(client.jobEventsUrl('7')).toBe('/api/v1/jobs/7/events');
    });

    it('escapes the job id', () => {
        expect(client.jobEventsUrl('../settings')).toBe('/api/v1/jobs/..%2Fsettings/events');
    });

    /**
     * `EventSource` does not go through `request`, so this is the one place
     * where forgetting `apiBase()` would leave the stream pointed at the Vite
     * dev server while every other call reached the backend.
     */
    it('honours VITE_API_BASE so the stream reaches the same origin as fetch', () => {
        vi.stubEnv('VITE_API_BASE', 'http://localhost:7492');
        expect(client.jobEventsUrl('7')).toBe('http://localhost:7492/api/v1/jobs/7/events');
    });
});

/**
 * "No Factorio instance is running" is `code: 2` on *two different statuses*:
 * 400 from stop and rcon, 503 from execute. A caller that keys the condition
 * off the status handles one of them and silently misses the others, so the
 * client must hand the code through untouched.
 */
describe('error propagation', () => {
    it('surfaces the 400 not-started as ApiError code 2', async () => {
        json(400, {message: 'not started', code: 2});
        const error = await client.stopInstance().catch((err: unknown) => err);
        expect(error).toBeInstanceOf(ApiError);
        expect((error as ApiError).status).toBe(400);
        expect((error as ApiError).code).toBe(2);
    });

    it('surfaces the 503 not-started as the same ApiError code 2', async () => {
        json(503, {message: 'not started', code: 2});
        const error = await client.executeScript({path: '/a.lua'}).catch((err: unknown) => err);
        expect(error).toBeInstanceOf(ApiError);
        expect((error as ApiError).status).toBe(503);
        expect((error as ApiError).code).toBe(2);
    });

    it('keeps running_job_id reachable on the 409 from execute', async () => {
        json(409, {message: 'a script is already running as job 3', code: 5, running_job_id: '3'});
        const error = await client.executeScript({path: '/a.lua'}).catch((err: unknown) => err);
        expect((error as ApiError).code).toBe(5);
        expect((error as ApiError).body).toEqual({
            message: 'a script is already running as job 3',
            code: 5,
            running_job_id: '3'
        });
    });
});
