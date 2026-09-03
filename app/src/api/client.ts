/**
 * One named function per management route, over the `http.ts` transport.
 *
 * Stores call these instead of building paths themselves: the path, the
 * method and the place each argument travels (query parameter or body) are
 * the server's contract, and a store that spells one out is a store that can
 * drift from it silently.
 *
 * Errors are not handled here. Every failure arrives as the `ApiError` that
 * `request` throws, with the server's application `code` intact -- callers
 * branch on that code, never on the HTTP status, because "no Factorio
 * instance is running" is `code: 2` on a 400 from stop and rcon *and* on a
 * 503 from execute.
 */

import {buildUrl, request} from './http';
import {AppSettings} from '@/models/settings';
import {ScriptTreeNode} from '@/api/types';
import {
    EventsResponse,
    ExecuteAccepted,
    ExecuteRequest,
    ExistsResponse,
    InstanceStatus,
    Job,
    RunDetail,
    RunLanesResponse,
    RunMapResponse,
    RunSamplesResponse,
    RunsResponse,
    ScriptContent,
    StartAccepted,
    VideoManifest,
    VideoTicksResponse
} from './types';

export function getSettings(): Promise<AppSettings> {
    return request<AppSettings>('/api/v1/settings');
}

/** Replaces the settings wholesale -- the server has no partial update. */
export function putSettings(settings: AppSettings): Promise<AppSettings> {
    return request<AppSettings>('/api/v1/settings', {method: 'PUT', body: settings});
}

export function getInstance(): Promise<InstanceStatus> {
    return request<InstanceStatus>('/api/v1/instance');
}

/** Answers as soon as the start is *accepted*; poll `getInstance` for the rest. */
export function startInstance(): Promise<StartAccepted> {
    return request<StartAccepted>('/api/v1/instance/start', {method: 'POST'});
}

export function stopInstance(): Promise<void> {
    return request<void>('/api/v1/instance/stop', {method: 'POST'});
}

export function sendRcon(command: string): Promise<void> {
    return request<void>('/api/v1/rcon', {method: 'POST', body: {command}});
}

/**
 * Lists one directory under the scripts root. `path` is a `ScriptTreeNode`'s
 * `key`, which the server builds as a slash-separated request path precisely
 * so it can be handed straight back.
 */
export function listScripts(path: string): Promise<ScriptTreeNode[]> {
    return request<ScriptTreeNode[]>('/api/v1/scripts', {query: {path}});
}

export function readScript(path: string): Promise<ScriptContent> {
    return request<ScriptContent>('/api/v1/scripts/file', {query: {path}});
}

/** Overwrites an existing script. The server refuses a missing one with 400. */
export function writeScript(path: string, code: string): Promise<void> {
    return request<void>('/api/v1/scripts/file', {method: 'PUT', query: {path}, body: {code}});
}

/**
 * Creates a new script. Differs from `writeScript` only by method: the server
 * refuses an existing target with 409, where the PUT would overwrite it.
 *
 * Not wired to any UI yet; the route exists and a later plan needs it.
 */
export function createScript(path: string, code: string): Promise<void> {
    return request<void>('/api/v1/scripts/file', {method: 'POST', query: {path}, body: {code}});
}

/** Not wired to any UI yet; the route exists and a later plan needs it. */
export function deleteScript(path: string): Promise<void> {
    return request<void>('/api/v1/scripts/file', {method: 'DELETE', query: {path}});
}

/**
 * Whether a path exists on the server's filesystem. Unlike the scripts
 * routes this takes a real server path, not one relative to the scripts root.
 */
export function pathExists(path: string): Promise<ExistsResponse> {
    return request<ExistsResponse>('/api/v1/fs/exists', {query: {path}});
}

/**
 * Starts a script. `body` is passed through as given so that the server sees
 * exactly the fields the caller set: it reads "exactly one of `path` or
 * `code`" off which of the two are present.
 */
export function executeScript(body: ExecuteRequest): Promise<ExecuteAccepted> {
    return request<ExecuteAccepted>('/api/v1/scripts/execute', {method: 'POST', body});
}

export function listJobs(): Promise<Job[]> {
    return request<Job[]>('/api/v1/jobs');
}

export function getJob(id: string): Promise<Job> {
    return request<Job>('/api/v1/jobs/' + encodeURIComponent(id));
}

/**
 * `EventSource` takes a URL rather than a fetch call, so this is a URL
 * builder -- through `buildUrl`, so the stream lands on the same origin the
 * rest of the client uses under `VITE_API_BASE`.
 */
export function jobEventsUrl(id: string): string {
    return buildUrl('/api/v1/jobs/' + encodeURIComponent(id) + '/events');
}

/** Archived runs, newest first. */
export function listRuns(): Promise<RunsResponse> {
    return request<RunsResponse>('/api/v1/runs');
}

/** One run's summary and splits. */
export function getRun(id: string): Promise<RunDetail> {
    return request<RunDetail>(`/api/v1/runs/${encodeURIComponent(id)}`);
}

/**
 * A run's raw event log, optionally narrowed to one `kind` (e.g.
 * `"milestone_satisfied"`) -- the same filter `derive_lanes`/`derive_splits`
 * apply server-side, exposed here for a caller that wants the events
 * themselves rather than a derived view.
 */
export function getRunEvents(id: string, kind?: string): Promise<EventsResponse> {
    return request<EventsResponse>(`/api/v1/runs/${encodeURIComponent(id)}/events`, {
        query: {kind}
    });
}

/** What each bot did, derived from the run's event log. */
export function getRunLanes(id: string): Promise<RunLanesResponse> {
    return request<RunLanesResponse>(`/api/v1/runs/${encodeURIComponent(id)}/lanes`);
}

/**
 * A run's archived world-state samples.
 *
 * Empty for a run recorded before sampling existed, or one the mod never
 * captured samples for -- not an error.
 */
export function getRunSamples(id: string): Promise<RunSamplesResponse> {
    return request<RunSamplesResponse>(`/api/v1/runs/${encodeURIComponent(id)}/samples`);
}

/**
 * A run's entity map: what got built, and whether the game agreed.
 *
 * Empty for a run recorded before this feature existed, or one that placed
 * nothing -- not an error.
 */
export function getRunMap(id: string): Promise<RunMapResponse> {
    return request<RunMapResponse>(`/api/v1/runs/${encodeURIComponent(id)}/map`);
}

/**
 * The current run's video manifest.
 *
 * A workspace that recorded no video answers a manifest describing nothing, not
 * a 404 — video is opt-in, so "there is none" is the ordinary case.
 */
export function video(): Promise<VideoManifest> {
    return request<VideoManifest>('/api/v1/video');
}

/** The current run's video clock, gap lines included. */
export function videoTicks(): Promise<VideoTicksResponse> {
    return request<VideoTicksResponse>('/api/v1/video/ticks');
}

/**
 * The URL for the live recording's bytes, for a `<video>` element rather than a
 * `request()` call.
 *
 * **Cache-busted by run id.** The live recording is one file at one URL that
 * the *next run overwrites*, so without the parameter a browser would happily
 * replay the previous run's video beside this run's timeline. An archived run's
 * copy needs no such thing -- it is immutable, because a finished run never
 * runs again. `null` when the run is unknown: the URL is still usable, it just
 * cannot be busted.
 */
export function videoUrl(runId: string | null): string {
    const base = buildUrl('/api/v1/video/file');
    return runId === null ? base : base + '?run=' + encodeURIComponent(runId);
}

/** One archived run's video manifest. */
export function getRunVideo(id: string): Promise<VideoManifest> {
    return request<VideoManifest>(`/api/v1/runs/${encodeURIComponent(id)}/video`);
}

/** One archived run's video clock. */
export function getRunVideoTicks(id: string): Promise<VideoTicksResponse> {
    return request<VideoTicksResponse>(`/api/v1/runs/${encodeURIComponent(id)}/video/ticks`);
}

/**
 * The URL of one archived recording's bytes. No cache-buster: an archived run
 * is over, so the bytes at this URL never change.
 */
export function runVideoUrl(id: string): string {
    return buildUrl(`/api/v1/runs/${encodeURIComponent(id)}/video/file`);
}
