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
    ExecuteAccepted,
    ExecuteRequest,
    ExistsResponse,
    FramesManifest,
    InstanceStatus,
    Job,
    ScriptContent,
    StartAccepted
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

/**
 * The manifest of captured frames, derived fresh from a directory listing on
 * every call -- see `FramesManifest` for why nothing here is computed from a
 * start tick and a stride.
 */
export function frames(): Promise<FramesManifest> {
    return request<FramesManifest>('/api/v1/frames');
}

/**
 * The URL for one frame's JPEG bytes, for an `<img>` tag rather than a
 * `request()` call -- the response is immutable image bytes, not JSON.
 * `name` is a `FrameEntry.name` as reported by `frames()`.
 */
export function frameUrl(name: string): string {
    return buildUrl('/api/v1/frames/' + encodeURIComponent(name));
}
