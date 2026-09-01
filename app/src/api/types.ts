/**
 * Hand-written mirrors of the server types that are *not* generated into
 * `models/types.ts`.
 *
 * The generator only sees `crates/core` (the types deriving `TypeScriptify`),
 * and everything below lives in `crates/server` instead: the request and
 * response bodies of the management routes. They are written by hand, but
 * they are **not** unchecked. A chain of three guards ties them to what the
 * server publishes:
 *
 * 1. `crates/server/tests/openapi.rs` keeps `openapi.snapshot.json` equal to
 *    the `/openapi.json` the real router serves, so the snapshot cannot rot.
 * 2. `openapi.contract.spec.ts` asserts the snapshot against a table of what
 *    the client assumes each schema looks like.
 * 3. That table is *typed against the interfaces below*
 *    (`objectContract<InstanceStatus>`, `enumContract<JobStatus>`), so `tsc`
 *    fails under `pnpm run lint` if a name here and a row there disagree in
 *    either direction.
 *
 * So renaming a field in `crates/server` and mirroring it through the
 * snapshot and the contract table, but *not* here, does not compile -- which
 * is the failure this chain exists to force, because the alternative is code
 * that compiles and reads `undefined` in the browser.
 *
 * What the chain does not cover, so that this comment does not overstate it:
 *
 * - **The JSON type of a field.** The contract table states `type: 'integer'`
 *   or `'string'` as its own claim; it is pinned to the spec but not to the
 *   `number`/`string` written here. Nullability *is* pinned both ways.
 * - **`OutputStream`.** The job event stream is `text/event-stream` with no
 *   schema in the document, so there is nothing for the table to check it
 *   against. Nothing imports it yet either.
 * - **Application `code` values.** `code: 2` is a value, not a schema; it is
 *   pinned on the server side by the three `not_started`/`not_running` tests
 *   in `crates/server/tests/`.
 */

/** `GET /api/v1/instance` -- `crates/server/src/manage/instance.rs`. */
export interface InstanceStatus {
    started: boolean;
    /** A start accepted by POST /api/v1/instance/start is still running. */
    starting: boolean;
    client_count: number;
    server_port: number | null;
    rcon_port: number | null;
    /** Why the last start attempt failed, or `null`. */
    last_error: string | null;
}

/**
 * The `202` from `POST /api/v1/instance/start`.
 *
 * Accepted is not started: the start runs detached because a first-run
 * archive extraction takes minutes. Progress is read from `InstanceStatus`.
 */
export interface StartAccepted {
    accepted: boolean;
}

/** Body of `GET`/`PUT`/`POST /api/v1/scripts/file`. */
export interface ScriptContent {
    code: string;
}

/** `GET /api/v1/fs/exists` -- an arbitrary server path, not a script path. */
export interface ExistsResponse {
    exists: boolean;
}

/**
 * Body of `POST /api/v1/scripts/execute`.
 *
 * Exactly one of `path` or `code` must be set; the server answers 400 for
 * both and for neither. `language` is ignored when `path` is given and
 * defaults to `"lua"` otherwise, and `bot_count` defaults to the configured
 * `factorio.client_count`.
 */
export interface ExecuteRequest {
    path?: string;
    code?: string;
    language?: string;
    bot_count?: number;
}

/** The `202` from `POST /api/v1/scripts/execute`. */
export interface ExecuteAccepted {
    job_id: string;
}

export type JobStatus = 'running' | 'succeeded' | 'failed';

/** One script run, live or historical -- `crates/server/src/jobs.rs`. */
export interface Job {
    /** A decimal counter, serialised as a string so it never loses precision. */
    id: string;
    /** `null` for inline code from the editor, which has no path. */
    script: string | null;
    status: JobStatus;
    started_at_ms: number;
    finished_at_ms: number | null;
    stdout: string;
    stderr: string;
    /** Set only when `status` is `failed`. */
    error: string | null;
    /**
     * The run's replay document, already serialised as JSON, or `null` for a
     * run that has not produced one. Opaque here on purpose: the server does
     * not parse it either, so this module never types its shape.
     */
    replay: string | null;
}

/** Which of a job's two output buffers a line came from. */
export type OutputStream = 'stdout' | 'stderr';

/** A node of `GET /api/v1/scripts/tree` -- mirrors `ScriptTreeNode` in `crates/core/src/types.rs`. */
export interface ScriptTreeNode {
    key: string;
    label: string;
    leaf: boolean;
    children: ScriptTreeNode[];
}

/** The `factorio` field of `GET /api/v1/settings` -- mirrors `FactorioSettings` in `crates/core/src/settings.rs`. */
export interface FactorioSettings {
    client_count: number;
    factorio_archive_path: string;
    map_exchange_string: string;
    rcon_pass: string;
    rcon_port: number;
    recreate: boolean;
    seed: string;
    workspace_path: string;
}

/** The `restapi` field of `GET /api/v1/settings` -- mirrors `RestApiSettings` in `crates/core/src/settings.rs`. */
export interface RestApiSettings {
    port: number;
    web_root: string | null;
}

/**
 * A 2D game-world coordinate. Mirrors `Position` in `crates/core/src/types.rs`.
 *
 * `y` increases DOWNWARD -- the same direction screen and SVG coordinates do
 * -- verified against a live capture (see `Rect` below). A view that flips
 * this axis to look "more like a normal graph" renders every position wrong
 * while looking entirely plausible.
 */
export interface Position {
    x: number;
    y: number;
}

/**
 * An axis-aligned bounding box, `left_top` to `right_bottom`. Mirrors `Rect`
 * in `crates/core/src/types.rs`.
 *
 * "Top" has the numerically SMALLER `y`: `crash-site-spaceship` in
 * `crates/core/tests/live-2.1.17-entities-spawn.json` has
 * `left_top.y = -9.296875` and `right_bottom.y = -1.5`. Do not swap them when
 * computing a rect's height, and do not negate `y` when placing one on
 * screen -- see `app/src/components/map/MapEntities.vue`.
 */
export interface Rect {
    left_top: Position;
    right_bottom: Position;
}

/**
 * One inventory slot, Factorio 2.0's quality-tagged format. Mirrors
 * `InventoryItemWithQuality` in `crates/core/src/types.rs`.
 */
export interface InventoryItemWithQuality {
    name: string;
    quality: string;
    count: number;
}

/**
 * One entity as `GET /api/v1/game/find-entities` reports it. Mirrors
 * `FactorioEntity` in `crates/core/src/types.rs`.
 *
 * `name`, `entity_type`, `position`, `bounding_box` and `direction` are
 * always present. Everything else is `Option<_>` on the Rust side with no
 * `skip_serializing_if`, so the server always sends the field -- present and
 * `null`, never omitted -- which is why these are typed `| null` rather than
 * `?`, matching `InstanceStatus` above.
 *
 * The map view (`app/src/components/map/`) reads only the required fields
 * plus `amount`; the inventory and ghost fields are declared here for
 * completeness with the server's schema but nothing renders them yet.
 */
export interface FactorioEntity {
    name: string;
    entity_type: string;
    position: Position;
    bounding_box: Rect;
    /** Factorio 2.x `defines.direction`: 0..=15, north at 0, clockwise. */
    direction: number;
    drop_position: Position | null;
    pickup_position: Position | null;
    output_inventory: InventoryItemWithQuality[] | null;
    fuel_inventory: InventoryItemWithQuality[] | null;
    /** Only present (non-null) for `entity_type: "resource"`. */
    amount: number | null;
    /** Only present for crafting machines. */
    recipe: string | null;
    ghost_name: string | null;
    ghost_type: string | null;
}

/**
 * One frame file exactly as it exists on disk -- `crates/server/src/manage/frames.rs`.
 *
 * A frame's filename is a measurement, not a claim: the mod names it from
 * `game.tick` inside the game, and `tick`/`camera` here are only ever parsed
 * back out of that name. Both are `| null`, always present as keys, for a
 * name that does not fit the `tick-<digits>-<camera>.jpg` pattern -- such a
 * file is reported, never dropped from the list.
 */
export interface FrameEntry {
    /** Which `client<N>` directory this frame was captured by. */
    client: number;
    tick: number | null;
    camera: string | null;
    /** The filename exactly as it appears on disk; pass to `frameUrl`. */
    name: string;
    bytes: number;
}

/**
 * `GET /api/v1/frames` response.
 *
 * `clients` and `frames` together distinguish three states that must not be
 * conflated: no `clients` at all means the run has never happened; a
 * `clients` entry with no matching `frames` means capture has not produced
 * anything yet; both non-empty means frames are present. Never computed from
 * a start tick and a stride -- always a live directory listing, so a dropped
 * frame (multiplayer clients catching up do not honour `force_render`) shows
 * up as a genuine gap rather than being smoothed over.
 */
export interface FramesManifest {
    /** `client<N>` directories discovered under the workspace. */
    clients: number[];
    frames: FrameEntry[];
    /**
     * The opaque run identifier from `frames/run.json`, or `null` when capture
     * ran without one, the file is absent, or it could not be read.
     *
     * Opaque: compare it for equality and nothing else. `null` means
     * **unknown**, never *no match* — see `runIdCheck` in `@/api/frameJoin`.
     */
    run: string | null;
    /**
     * What each client's own sidecar says, in `clients` order.
     *
     * `run` is which run the manifest is about; this is which run each
     * client's directory belongs to. They diverge when a client sat out the
     * current run and still holds an older one's frames — the case worth
     * marking in the UI rather than mixing in silently.
     */
    client_runs: ClientRun[];
}

/** One client's answer to "which run do your frames belong to". */
export interface ClientRun {
    /** The `N` of the `client<N>` directory. */
    client: number;
    /** That directory's own run id; `null` is **unknown**, never *no match*. */
    run: string | null;
}

/**
 * One archived run, as it appears in a listing.
 *
 * Everything except `run_id` and `finished` is nullable because an unfinished
 * run genuinely lacks it. Present-and-null, never omitted: a caller must be
 * able to tell "this run never finished" from "this build does not report
 * outcomes".
 */
export interface RunSummary {
    run_id: string;
    /**
     * Whether the run reached `finish`. `false` means crashed *or* still
     * going -- the server cannot tell those apart and does not pretend to.
     */
    finished: boolean;
    /** Unix seconds. For "when was this", never for comparing two runs. */
    started_unix: number | null;
    finished_unix: number | null;
    outcome: string | null;
    /** A duration in game ticks, not the tick the run ended at. */
    elapsed_ticks: number | null;
    events: number | null;
    frames: number | null;
    splits: number | null;
}

/** `GET /api/v1/runs` response. */
export interface RunsResponse {
    runs: RunSummary[];
}

/**
 * One milestone's timing -- the speedrun split.
 *
 * Measured in ticks, because two runs are compared on the game's clock and
 * never on wall time: a headless server and a graphical client with three
 * cameras do not run at the same speed, so wall time compares hardware.
 */
export interface Split {
    index: number;
    goal: string;
    started_tick: number;
    /** `null` when the run ended without closing this milestone. */
    ended_tick: number | null;
    /** `satisfied`, `stuck`, `stuck_silent`, `exhausted`, or `unfinished`. */
    outcome: string;
    /**
     * `ended_tick - started_tick`, or `null` while unfinished.
     *
     * Never subtract these yourself: a null minus a number is a plausible
     * zero, and zero looks exactly like a fast milestone.
     */
    elapsed_ticks: number | null;
}

/** `GET /api/v1/runs/{id}` response. */
export interface RunDetail {
    summary: RunSummary;
    /**
     * From `splits.json` when the run finished, otherwise derived from its
     * events -- so a crashed run still shows the milestones it got through.
     */
    splits: Split[];
}

/** One frame copied into a run archive. */
export interface ArchivedFrame {
    /** The bot that captured it. Bots and clients are 1:1. */
    bot: number;
    /** `game.tick` at capture, or `null` when the filename does not parse. */
    tick: number | null;
    camera: string | null;
    /** Path relative to the run directory. */
    file: string;
}

/** `GET /api/v1/runs/{id}/frames` response. */
export interface RunFramesResponse {
    frames: ArchivedFrame[];
}

/** One thing a bot did, placed on the tick axis. */
export interface Lane {
    bot: number;
    /**
     * The action id. **Not unique across a run** -- ids restart with every
     * plan, so this identifies an entry only together with `bot` and
     * `from_tick`.
     */
    id: number;
    /** What the plan called it, e.g. `mine 4 iron-ore`. */
    action: string;
    from_tick: number;
    /**
     * `null` for an action dispatched and never settled.
     *
     * Drawn unterminated rather than dropped: the bot really did start it and
     * nothing came back. Dropping it would make a lost action look like one
     * that never happened.
     */
    to_tick: number | null;
    /** `null` while unterminated, otherwise the verdict the game gave. */
    status: string | null;
    error: string | null;
}

/** `GET /api/v1/runs/{id}/lanes` response. */
export interface RunLanesResponse {
    lanes: Lane[];
}
