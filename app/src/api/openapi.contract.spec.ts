/**
 * What pins `app/src/api/client.ts` and `app/src/api/types.ts` to the server.
 *
 * The client is hand-written, and TypeScript cannot see `crates/server`, so
 * without this file a field renamed in Rust typechecks here and fails in the
 * browser. The seam is `openapi.snapshot.json`, and it takes *three* guards
 * to work:
 *
 * 1. `the_committed_openapi_snapshot_matches_the_published_spec` in
 *    `crates/server/tests/openapi.rs` keeps the snapshot equal to what the
 *    server really publishes. Without it the snapshot would rot exactly the
 *    way `app/src/models/types.ts` has rotted away from its generator.
 * 2. This file's assertions check that everything the client assumes is in
 *    the snapshot. Without them, regenerating the snapshot would bless any
 *    server change.
 * 3. The tables below are *typed against the DTO declarations themselves*
 *    (`objectContract<InstanceStatus>`, `enumContract<JobStatus>`), so `tsc`
 *    -- and therefore `pnpm run lint` -- fails if `app/src/api/types.ts` and
 *    this file disagree about a field name in either direction. Without that,
 *    `types.ts` hangs off the side of the seam verified by nothing: a
 *    developer chasing a rename could fix the snapshot and the table below,
 *    go green, and leave `types.ts` declaring the old field for the browser
 *    to trip over.
 *
 * Note what the trio does and does not promise. Each guard *can* be satisfied
 * on its own by editing another's input -- hand-edit the snapshot and the Rust
 * test goes red; regenerate it and this file does; rename in both and `tsc`
 * does. The guarantee is the conjunction: **all three cannot pass unless the
 * server, the snapshot, the contract tables and the DTO declarations all say
 * the same thing.**
 *
 * The expectations below are written as *data mirroring the client*, never
 * derived from the snapshot: anything derived from the file under test passes
 * for the wrong reason. They are deliberately exact (property sets and
 * required sets are compared by equality, not containment) so that an added
 * field fails too -- an added required request field is a break, and an added
 * response field is at minimum a decision.
 *
 * To regenerate the snapshot after an intended server change:
 *   UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p factorio-bot-server --features lua --test openapi
 *   git diff --no-ext-diff app/src/api/openapi.snapshot.json
 */

import {describe, expect, it} from 'vitest';
import snapshot from './openapi.snapshot.json';
// Type-only imports, and they are what makes `types.ts` an *input* to this
// file rather than a bystander: every table entry below is checked against
// the declaration it claims to describe.
import type {AppSettings, GuiSettings} from '@/models/settings';
import type {FactorioSettings, RestApiSettings, ScriptTreeNode} from '@/api/types';
import type {
    ActionFailure,
    Bounds,
    BotSample,
    Calibration,
    Divergence,
    EntitySnapshot,
    Event,
    EventKind,
    EventsResponse,
    ExecuteAccepted,
    ExecuteRequest,
    ExistsResponse,
    FailureKind,
    InstanceStatus,
    Job,
    JobStatus,
    RunDetail,
    Lane,
    MapKind,
    MapRecord,
    PlannedStep,
    WaitingStep,
    Position,
    MachineSample,
    NetworkPower,
    PowerSample,
    ProductionSample,
    ResearchSample,
    RunLanesResponse,
    RunMapResponse,
    RunSamplesResponse,
    RunSummary,
    RunsResponse,
    Sample,
    SampleKind,
    SatisfiedReason,
    ScriptContent,
    Split,
    StartAccepted,
    TickKind,
    TickRange,
    TickSample,
    VideoManifest,
    VideoRecord,
    VideoStatus,
    VideoTicksResponse,
    WalkFailure,
    WalkFailureKind
} from './types';

interface SchemaObject {
    $ref?: string;
    type?: string | string[];
    enum?: string[];
    items?: SchemaObject;
    properties?: Record<string, SchemaObject>;
    required?: string[];
    /**
     * An internally tagged Rust enum (`#[serde(tag = "kind")]`), published as
     * one member per variant. See `taggedUnionContract`.
     */
    oneOf?: SchemaObject[];
    /**
     * A `#[serde(flatten)]` of a tagged enum into its containing struct,
     * published as `[ {$ref: the enum}, {the struct's own fields} ]`. See
     * `mergeContract`.
     */
    allOf?: SchemaObject[];
}

interface Parameter {
    name: string;
    in: string;
    required?: boolean;
}

interface Operation {
    parameters?: Parameter[];
    requestBody?: {required?: boolean; content?: Record<string, {schema?: SchemaObject}>};
    responses?: Record<string, {content?: Record<string, {schema?: SchemaObject}>}>;
}

interface OpenApiDocument {
    paths: Record<string, Record<string, Operation>>;
    components: {schemas: Record<string, SchemaObject>};
}

// The import is typed as the literal shape of the JSON file, which has no
// index signatures; every lookup below is by a computed key.
const spec = snapshot as unknown as OpenApiDocument;

/** How the client expects one operation's success answer to be shaped. */
interface ResponseContract {
    /** The status the client's happy path parses. */
    status: string;
    /** A `$ref`d schema, for a single-object body. */
    schema?: string;
    /** A JSON array of that schema. */
    arrayOf?: string;
    /** A body the client does not parse as JSON (`text/event-stream`). */
    mediaType?: string;
    /** No body at all -- `request<void>` in the client. */
    empty?: true;
}

interface OperationContract {
    path: string;
    method: string;
    /** The function in `client.ts` that calls it, so a failure names a caller. */
    caller: string;
    /** Query parameters the client sends, as `[name, required]`. Exact set. */
    query?: ReadonlyArray<readonly [string, boolean]>;
    /** Templated path segments. The client builds these by concatenation. */
    pathParams?: readonly string[];
    /** Schema of the JSON body the client sends, if it sends one. */
    requestSchema?: string;
    response: ResponseContract;
}

/**
 * Every operation `app/src/api/client.ts` calls, one entry per exported
 * function. Sixteen management operations; the seventeen `/api/v1/game/*`
 * operations the server also publishes are out of this client's scope and
 * belong to a future `app/src/api/game.ts`.
 */
const OPERATIONS: readonly OperationContract[] = [
    {
        path: '/api/v1/runs',
        method: 'get',
        caller: 'listRuns',
        response: {status: '200', schema: 'RunsResponse'}
    },
    {
        path: '/api/v1/runs/{id}',
        method: 'get',
        caller: 'getRun',
        pathParams: ['id'],
        response: {status: '200', schema: 'RunDetail'}
    },
    {
        path: '/api/v1/runs/{id}/events',
        method: 'get',
        caller: 'getRunEvents',
        pathParams: ['id'],
        query: [['kind', false]],
        response: {status: '200', schema: 'EventsResponse'}
    },
    {
        path: '/api/v1/runs/{id}/lanes',
        method: 'get',
        caller: 'getRunLanes',
        pathParams: ['id'],
        response: {status: '200', schema: 'RunLanesResponse'}
    },
    {
        path: '/api/v1/runs/{id}/samples',
        method: 'get',
        caller: 'getRunSamples',
        pathParams: ['id'],
        response: {status: '200', schema: 'RunSamplesResponse'}
    },
    {
        path: '/api/v1/runs/{id}/map',
        method: 'get',
        caller: 'getRunMap',
        pathParams: ['id'],
        response: {status: '200', schema: 'RunMapResponse'}
    },
    {
        path: '/api/v1/settings',
        method: 'get',
        caller: 'getSettings',
        response: {status: '200', schema: 'AppSettings'}
    },
    {
        path: '/api/v1/settings',
        method: 'put',
        caller: 'putSettings',
        requestSchema: 'AppSettings',
        response: {status: '200', schema: 'AppSettings'}
    },
    {
        path: '/api/v1/instance',
        method: 'get',
        caller: 'getInstance',
        response: {status: '200', schema: 'InstanceStatus'}
    },
    {
        path: '/api/v1/instance/start',
        method: 'post',
        caller: 'startInstance',
        // 202, not 200: the start is accepted and runs detached, because a
        // first-run archive extraction takes minutes.
        response: {status: '202', schema: 'StartAccepted'}
    },
    {
        path: '/api/v1/instance/stop',
        method: 'post',
        caller: 'stopInstance',
        response: {status: '204', empty: true}
    },
    {
        path: '/api/v1/rcon',
        method: 'post',
        caller: 'sendRcon',
        requestSchema: 'RconBody',
        response: {status: '204', empty: true}
    },
    {
        path: '/api/v1/scripts',
        method: 'get',
        caller: 'listScripts',
        query: [['path', true]],
        response: {status: '200', arrayOf: 'ScriptTreeNode'}
    },
    {
        path: '/api/v1/scripts/file',
        method: 'get',
        caller: 'readScript',
        query: [['path', true]],
        response: {status: '200', schema: 'ScriptContent'}
    },
    {
        path: '/api/v1/scripts/file',
        method: 'put',
        caller: 'writeScript',
        query: [['path', true]],
        requestSchema: 'ScriptContent',
        response: {status: '204', empty: true}
    },
    {
        path: '/api/v1/scripts/file',
        method: 'post',
        caller: 'createScript',
        query: [['path', true]],
        requestSchema: 'ScriptContent',
        response: {status: '201', empty: true}
    },
    {
        path: '/api/v1/scripts/file',
        method: 'delete',
        caller: 'deleteScript',
        query: [['path', true]],
        response: {status: '204', empty: true}
    },
    {
        path: '/api/v1/fs/exists',
        method: 'get',
        caller: 'pathExists',
        query: [['path', true]],
        response: {status: '200', schema: 'ExistsResponse'}
    },
    {
        path: '/api/v1/scripts/execute',
        method: 'post',
        caller: 'executeScript',
        requestSchema: 'ExecuteRequest',
        response: {status: '202', schema: 'ExecuteAccepted'}
    },
    {
        path: '/api/v1/jobs',
        method: 'get',
        caller: 'listJobs',
        response: {status: '200', arrayOf: 'Job'}
    },
    {
        path: '/api/v1/jobs/{id}',
        method: 'get',
        caller: 'getJob',
        pathParams: ['id'],
        response: {status: '200', schema: 'Job'}
    },
    {
        path: '/api/v1/jobs/{id}/events',
        method: 'get',
        caller: 'jobEventsUrl',
        pathParams: ['id'],
        // Consumed by `EventSource`, not by `request()`, so the client never
        // parses this as JSON -- but the media type is still part of what it
        // assumes.
        response: {status: '200', mediaType: 'text/event-stream'}
    },
    {
        path: '/api/v1/video',
        method: 'get',
        caller: 'video',
        response: {status: '200', schema: 'VideoManifest'}
    },
    {
        path: '/api/v1/video/ticks',
        method: 'get',
        caller: 'videoTicks',
        response: {status: '200', schema: 'VideoTicksResponse'}
    },
    {
        path: '/api/v1/video/file',
        method: 'get',
        caller: 'videoUrl',
        // No `query` entry, deliberately. `videoUrl` appends `?run=<id>` and the
        // server never reads it: it is a cache key, not a parameter. The live
        // recording is one URL that the next run overwrites, so the browser has
        // to be told the bytes changed -- but publishing an argument the handler
        // ignores would put a lie in a generated client.
        response: {status: '200', mediaType: 'video/mp4'}
    },
    {
        path: '/api/v1/runs/{id}/video',
        method: 'get',
        caller: 'getRunVideo',
        pathParams: ['id'],
        response: {status: '200', schema: 'VideoManifest'}
    },
    {
        path: '/api/v1/runs/{id}/video/ticks',
        method: 'get',
        caller: 'getRunVideoTicks',
        pathParams: ['id'],
        response: {status: '200', schema: 'VideoTicksResponse'}
    },
    {
        path: '/api/v1/runs/{id}/video/file',
        method: 'get',
        caller: 'runVideoUrl',
        pathParams: ['id'],
        response: {status: '200', mediaType: 'video/mp4'}
    }
] as const;

/** One property of a mirrored DTO, as `app/src/api/types.ts` declares it. */
interface PropertyContract {
    /** Listed in the schema's `required` array. */
    required: boolean;
    /** The JSON type, for a property the client reads as a primitive. */
    type?: 'string' | 'integer' | 'number' | 'boolean' | 'array' | 'object';
    /** The schema this property `$ref`s, for a nested object. */
    ref?: string;
    /** A JSON array of that schema. */
    arrayOf?: string;
    /** The client's TypeScript declares `| null` (or `?`) for this field. */
    nullable?: true;
}

type SchemaContract =
    | {kind: 'object'; properties: Record<string, PropertyContract>}
    | {kind: 'enum'; values: readonly string[]}
    | {kind: 'scalar'; type: string}
    // An internally tagged Rust enum (`#[serde(tag = "kind")]`), published as
    // `oneOf`, one inline object per variant.
    | {kind: 'taggedUnion'; discriminant: string; variants: Record<string, Record<string, PropertyContract>>}
    // A `#[serde(flatten)]` of a tagged union into its containing struct,
    // published as `allOf: [{$ref: the union}, {the struct's own fields}]`.
    | {kind: 'merge'; base: string; extra: Record<string, PropertyContract>};

/**
 * Whether the DTO declares a field as absent-able: `| null`, or optional with
 * `?`.
 *
 * Both reach the wire the same way -- utoipa publishes them as
 * `type: [t, "null"]` -- so both must carry `nullable: true` below, and a
 * field declared as neither must not.
 */
type Absentable<Value> = undefined extends Value
    ? true
    : null extends Value
        ? true
        : false;

/**
 * The JSON type a declared TypeScript type is allowed to claim.
 *
 * Without this, `type` was a free-standing literal with no link to the
 * declaration: `port: {required: true, type: 'string'}` against `port: number`
 * passed `tsc`, and the runtime assertions only compare the table's `type`
 * string to the snapshot -- never to the DTO -- so a table and a declaration
 * wrong in the SAME direction passed both halves of the guard.
 *
 * Non-primitive fields fall through to the full union deliberately: they carry
 * `ref` or `arrayOf` instead of `type`, and constraining an unused key would
 * buy nothing. `NonNullable` is applied first so `| null` does not defeat the
 * match -- nullability is already pinned separately by `Absentable`.
 *
 * A TS `number` allows *either* `'integer'` or `'number'`: the two JSON types
 * are indistinguishable in TypeScript (`u32` and `f64` are both just
 * `number`), so this cannot force one -- only the runtime assertion against
 * the snapshot, which compares the table's literal to the actual published
 * type, catches a mismatch there.
 */
type ScalarTypeFor<Value> = [NonNullable<Value>] extends [string]
    ? 'string'
    : [NonNullable<Value>] extends [number]
        ? 'integer' | 'number'
        : [NonNullable<Value>] extends [boolean]
            ? 'boolean'
            : PropertyContract['type'];

/**
 * One property's contract, with its nullability AND its JSON type tied to the
 * declaration.
 *
 * Declaring `nullable` where `types.ts` says the field is always present (or
 * omitting it where `types.ts` says `| null`) is a type error, and so is
 * claiming a JSON type the declaration does not have, so the three statements
 * -- spec, table, DTO -- cannot drift apart pairwise.
 */
type PropertyContractFor<Value> = PropertyContract &
    (Absentable<Value> extends true ? {nullable: true} : {nullable?: never}) &
    {type?: ScalarTypeFor<Value>};

/**
 * The contract for one object schema, keyed by the *TypeScript* declaration
 * the client reads it as.
 *
 * This is the whole point of the generic. `{[K in keyof T]-?: …}` demands an
 * entry for every declared field, and the excess-property check on the object
 * literal rejects an entry for a field `T` does not declare -- so renaming a
 * field in `types.ts` alone, or adding one there alone, fails `tsc` under
 * `pnpm run lint`. Before this, the table was a third hand-written mirror
 * (server → snapshot → table) that `types.ts` merely sat beside, and a rename
 * carried through server, snapshot and table left a green suite reassuring
 * the developer that the browser would work.
 */
function objectContract<T>(properties: {[K in keyof T]-?: PropertyContractFor<T[K]>}): SchemaContract {
    return {kind: 'object', properties: properties as Record<string, PropertyContract>};
}

/**
 * The contract for a string enum, keyed by the union `types.ts` declares.
 *
 * A `readonly string[]` would have accepted any strings at all. `Record<T,
 * true>` demands one key per union member and rejects a member the union does
 * not have, so the union is pinned to the table and the table to the spec.
 */
function enumContract<T extends string>(values: Record<T, true>): SchemaContract {
    return {kind: 'enum', values: Object.keys(values)};
}

/**
 * The contract for an internally tagged Rust enum (`#[serde(tag = "kind")]`),
 * keyed by the *TypeScript* discriminated union the client reads it as.
 *
 * utoipa publishes this as `oneOf`, one inline object per variant, each
 * carrying its own literal `kind`. `{[K in T['kind']]-?: …}` demands an entry
 * for every variant the union declares, and `Extract<T, {kind: K}>` narrows
 * each entry to exactly that variant's own fields (via `objectContract`'s
 * mechanism, minus the discriminant itself) -- so a variant added, removed or
 * renamed only in `types.ts` fails `tsc` the same way a plain field does.
 */
function taggedUnionContract<T extends {kind: string}>(
    variants: {
        [K in T['kind']]: {
            [P in keyof Omit<Extract<T, {kind: K}>, 'kind'>]-?: PropertyContractFor<
                Extract<T, {kind: K}>[P]
            >;
        };
    }
): SchemaContract {
    return {
        kind: 'taggedUnion',
        discriminant: 'kind',
        variants: variants as unknown as Record<string, Record<string, PropertyContract>>
    };
}

/**
 * The contract for a `#[serde(flatten)]` of a tagged-union enum into its
 * containing struct.
 *
 * utoipa cannot fold the flatten back into one flat object -- it publishes
 * `allOf: [{$ref: the enum}, {the struct's own fields}]` instead. `base`
 * names the enum's own entry in `SCHEMAS`; `extra` is keyed like
 * `objectContract`, but exhaustive only over the fields `T` adds beyond the
 * enum (`Exclude<keyof T, 'kind'>`, since `keyof` on a union already
 * collapses to the fields common to every variant).
 */
function mergeContract<T extends {kind: string}>(
    base: string,
    extra: {[K in Exclude<keyof T, 'kind'>]-?: PropertyContractFor<T[K]>}
): SchemaContract {
    return {kind: 'merge', base, extra: extra as unknown as Record<string, PropertyContract>};
}

/**
 * Every schema reachable from the operations above, and the shape the client
 * reads it as.
 *
 * `every_schema_the_client_can_receive_is_pinned_here` proves this list is
 * complete rather than merely non-empty: a new `$ref` appearing anywhere in a
 * request or response the client touches fails until it is described here.
 */
const SCHEMAS: Record<string, SchemaContract> = {
    // -- crates/core, mirrored in app/src/models (settings.ts, types.ts) ----
    AppSettings: objectContract<AppSettings>({
        factorio: {required: true, ref: 'FactorioSettings'},
        restapi: {required: true, ref: 'RestApiSettings'},
        gui: {required: true, ref: 'GuiSettings'}
    }),
    FactorioSettings: objectContract<FactorioSettings>({
        client_count: {required: true, type: 'integer'},
        factorio_archive_path: {required: true, type: 'string'},
        // `Option<u16>` with `#[serde(default)]`: the game port, null for
        // Factorio's default 34197.
        factorio_port: {required: false, type: 'integer', nullable: true},
        map_exchange_string: {required: true, type: 'string'},
        rcon_pass: {required: true, type: 'string'},
        rcon_port: {required: true, type: 'integer'},
        recreate: {required: true, type: 'boolean'},
        seed: {required: true, type: 'string'},
        workspace_path: {required: true, type: 'string'}
    }),
    RestApiSettings: objectContract<RestApiSettings>({
        port: {required: true, type: 'integer'},
        // `Option<String>` on the Rust side; `string | null` in
        // models/types.ts, which is why it is not in `required`.
        web_root: {required: false, type: 'string', nullable: true}
    }),
    GuiSettings: objectContract<GuiSettings>({
        enable_autostart: {required: true, type: 'boolean'},
        enable_restapi: {required: true, type: 'boolean'}
    }),
    ScriptTreeNode: objectContract<ScriptTreeNode>({
        key: {required: true, type: 'string'},
        label: {required: true, type: 'string'},
        leaf: {required: true, type: 'boolean'},
        children: {required: true, arrayOf: 'ScriptTreeNode'}
    }),

    // -- crates/server, mirrored in app/src/api/types.ts -------------------
    InstanceStatus: objectContract<InstanceStatus>({
        started: {required: true, type: 'boolean'},
        starting: {required: true, type: 'boolean'},
        client_count: {required: true, type: 'integer'},
        // The three `Option` fields are always serialised (no
        // `skip_serializing_if`), so `types.ts` declares them present and
        // nullable rather than optional. utoipa still leaves them out of
        // `required`, hence `required: false` here.
        server_port: {required: false, type: 'integer', nullable: true},
        rcon_port: {required: false, type: 'integer', nullable: true},
        last_error: {required: false, type: 'string', nullable: true}
    }),
    StartAccepted: objectContract<StartAccepted>({
        accepted: {required: true, type: 'boolean'}
    }),
    ScriptContent: objectContract<ScriptContent>({
        code: {required: true, type: 'string'}
    }),
    ExistsResponse: objectContract<ExistsResponse>({
        exists: {required: true, type: 'boolean'}
    }),
    // -- run archives (crates/server/src/runs.rs) -------------------------
    RunsResponse: objectContract<RunsResponse>({
        runs: {required: true, arrayOf: 'RunSummary'}
    }),
    RunSummary: objectContract<RunSummary>({
        run_id: {required: true, type: 'string'},
        finished: {required: true, type: 'boolean'},
        // Everything below is present-and-null for a run that never finished,
        // so a caller can tell "never finished" from "not reported by this
        // build". Same rule the video manifest follows for `run`.
        started_unix: {required: false, type: 'integer', nullable: true},
        finished_unix: {required: false, type: 'integer', nullable: true},
        outcome: {required: false, type: 'string', nullable: true},
        elapsed_ticks: {required: false, type: 'integer', nullable: true},
        events: {required: false, type: 'integer', nullable: true},
        splits: {required: false, type: 'integer', nullable: true}
    }),
    RunDetail: objectContract<RunDetail>({
        summary: {required: true, ref: 'RunSummary'},
        splits: {required: true, arrayOf: 'Split'}
    }),
    Split: objectContract<Split>({
        index: {required: true, type: 'integer'},
        goal: {required: true, type: 'string'},
        started_tick: {required: true, type: 'integer'},
        // Null while the milestone is still open. `elapsed_ticks` is
        // materialised rather than left to the client to subtract: a null
        // minus a number is a zero, and zero looks like a fast milestone.
        ended_tick: {required: false, type: 'integer', nullable: true},
        outcome: {required: true, type: 'string'},
        elapsed_ticks: {required: false, type: 'integer', nullable: true}
    }),
    RunLanesResponse: objectContract<RunLanesResponse>({
        lanes: {required: true, arrayOf: 'Lane'}
    }),
    Lane: objectContract<Lane>({
        bot: {required: true, type: 'integer'},
        // Null for a lane that is not an action -- today, a walk, which the
        // scheduler emits with no action id at all.
        id: {required: false, type: 'integer', nullable: true},
        action: {required: true, type: 'string'},
        from_tick: {required: true, type: 'integer'},
        // Null for an action dispatched and never settled -- an unterminated
        // span, which is a state rather than a gap in the data.
        to_tick: {required: false, type: 'integer', nullable: true},
        status: {required: false, type: 'string', nullable: true},
        error: {required: false, type: 'string', nullable: true}
    }),

    // -- entity map (crates/core/src/record/map.rs) -----------------------
    RunMapResponse: objectContract<RunMapResponse>({
        map: {required: true, arrayOf: 'MapRecord'},
        skipped: {required: true, type: 'integer'}
    }),
    // `MapKind` flattened into `MapRecord`, the same shape `Sample` takes over
    // `SampleKind`: utoipa cannot fold a `#[serde(flatten)]` back into one
    // flat object, so it publishes `allOf: [{$ref: MapKind}, {tick}]`.
    MapRecord: mergeContract<MapRecord>('MapKind', {
        tick: {required: true, type: 'integer'}
    }),
    // An internally tagged enum (`#[serde(tag = "kind")]`): one inline object
    // per variant, so `taggedUnionContract`, not `objectContract`.
    MapKind: taggedUnionContract<MapKind>({
        placed: {
            bot: {required: true, type: 'integer'},
            intent: {required: true, ref: 'EntitySnapshot'},
            actual: {required: true, ref: 'EntitySnapshot'},
            // Null when intent and actual agree -- null and empty would mean
            // the same thing, so only one of them is ever written.
            drift: {required: false, type: 'array', nullable: true}
        },
        removed: {
            bot: {required: true, type: 'integer'},
            entity: {required: true, ref: 'EntitySnapshot'}
        },
        keyframe: {
            bounds: {required: true, ref: 'Bounds'},
            game: {required: true, arrayOf: 'EntitySnapshot'},
            model: {required: true, arrayOf: 'EntitySnapshot'},
            divergence: {required: true, arrayOf: 'Divergence'}
        },
        // A kind this build does not know. The server never emits it, but a
        // future variant must still decode rather than fail to parse.
        unknown: {}
    }),
    EntitySnapshot: objectContract<EntitySnapshot>({
        name: {required: true, type: 'string'},
        position: {required: true, ref: 'Position'},
        direction: {required: true, type: 'integer'}
    }),
    Bounds: objectContract<Bounds>({
        left: {required: true, type: 'number'},
        top: {required: true, type: 'number'},
        right: {required: true, type: 'number'},
        bottom: {required: true, type: 'number'}
    }),
    Divergence: objectContract<Divergence>({
        entity: {required: true, ref: 'EntitySnapshot'},
        only_in: {required: true, type: 'string'}
    }),

    // -- run event log (crates/core/src/record/mod.rs) ---------------------
    EventsResponse: objectContract<EventsResponse>({
        events: {required: true, arrayOf: 'Event'},
        skipped: {required: true, type: 'integer'}
    }),
    // `EventKind` flattened into `Event`, the same shape `Sample` takes over
    // `SampleKind`: utoipa cannot fold a `#[serde(flatten)]` back into one
    // flat object, so it publishes `allOf: [{$ref: EventKind}, {tick}]`.
    // There is no `wall_ms` any more -- see `Event`'s doc comment in
    // `app/src/api/types.ts` for why it was removed rather than fixed.
    Event: mergeContract<Event>('EventKind', {
        tick: {required: true, type: 'integer'}
    }),
    // An internally tagged enum (`#[serde(tag = "kind")]`): one inline object
    // per variant, so `taggedUnionContract`, not `objectContract`.
    //
    // `plan`, `reason` and `failure` publish `required: false` purely because
    // `#[serde(default)]` lets an *old* record on disk deserialise without
    // them -- a live server always serialises all three, so `types.ts`
    // declares them present rather than optional. `PropertyContractFor` ties
    // `nullable` to the TypeScript type, not to this `required` flag, which
    // is why these three carry no `nullable` key despite being `required:
    // false`: the field is never actually absent from a real response, only
    // from a historical file this API never reads for you.
    EventKind: taggedUnionContract<EventKind>({
        run_started: {
            run_id: {required: true, type: 'string'},
            bots: {required: true, type: 'array'},
            // Present-and-null when unknown, never absent.
            seed: {required: false, type: 'integer', nullable: true},
            factorio: {required: false, type: 'string', nullable: true},
            git: {required: false, type: 'string', nullable: true}
        },
        milestone_started: {
            index: {required: true, type: 'integer'},
            goal: {required: true, type: 'string'}
        },
        milestone_satisfied: {
            index: {required: true, type: 'integer'},
            iterations: {required: true, type: 'integer'},
            reason: {required: false, ref: 'SatisfiedReason'}
        },
        milestone_stuck: {
            index: {required: true, type: 'integer'},
            outcome: {required: true, type: 'string'},
            best_steps: {required: false, type: 'integer', nullable: true},
            last_error: {required: false, type: 'string', nullable: true}
        },
        // Written at every milestone a run satisfies, once the engine has
        // actually finished the file. `wrote_ms` is wall clock rather than
        // ticks on purpose: the save happens outside the tick it was asked in.
        savepoint_written: {
            milestone_index: {required: true, type: 'integer'},
            file: {required: true, type: 'string'},
            bytes: {required: true, type: 'integer'},
            wrote_ms: {required: true, type: 'integer'}
        },
        // A milestone with no savepoint and a run that never asked for one are
        // otherwise the same silence.
        savepoint_failed: {
            milestone_index: {required: true, type: 'integer'},
            error: {required: true, type: 'string'}
        },
        plan_created: {
            milestone_index: {required: true, type: 'integer'},
            steps: {required: true, type: 'integer'},
            makespan: {required: true, type: 'integer'},
            // Null when the caller did not state the roster the plan was
            // expanded against. Nullable rather than always-present because
            // the two substitutes the field used to carry -- the bots in the
            // steps, and the roster the process was started with -- were both
            // wrong, in opposite directions.
            bots: {required: false, type: 'array', nullable: true},
            plan: {required: false, arrayOf: 'PlannedStep'}
        },
        // The heartbeat a batch in flight writes, and the only event the
        // record produces while a plan is executing. Every counter is
        // required: it is written live, so there is no "we did not observe
        // this" case for any of them -- a counter that could not be read would
        // mean the log itself could not be read.
        //
        // `waiting`/`waiting_total` are the exception, and only because they
        // are `#[serde(default)]` so a run recorded before they existed still
        // opens. A live writer always fills them.
        batch_progress: {
            elapsed_ms: {required: true, type: 'integer'},
            total: {required: true, type: 'integer'},
            dispatched: {required: true, type: 'integer'},
            in_flight: {required: true, type: 'integer'},
            settled: {required: true, type: 'integer'},
            failed: {required: true, type: 'integer'},
            lost: {required: true, type: 'integer'},
            walks_dispatched: {required: true, type: 'integer'},
            walks_settled: {required: true, type: 'integer'},
            since_last_dispatch_ms: {required: true, type: 'integer'},
            bots_in_flight: {required: true, type: 'array'},
            waiting: {required: false, arrayOf: 'WaitingStep'},
            waiting_total: {required: false, type: 'integer'}
        },
        action_dispatched: {
            id: {required: true, type: 'integer'},
            bot: {required: true, type: 'integer'},
            action: {required: true, type: 'string'},
            target: {required: false, ref: 'Position', nullable: true}
        },
        action_settled: {
            id: {required: true, type: 'integer'},
            bot: {required: true, type: 'integer'},
            status: {required: true, type: 'string'},
            elapsed_ticks: {required: false, type: 'integer', nullable: true},
            error: {required: false, type: 'string', nullable: true},
            failure: {required: false, ref: 'ActionFailure', nullable: true}
        },
        // A walk. Two events, on the same terms as the action pair: the
        // dispatch needs a tick from the game, the settle needs only a
        // verdict -- a lost walk never has a reply tick, so gating the settle
        // on one would make `status: 'lost'` unrecordable.
        walk_dispatched: {
            bot: {required: true, type: 'integer'},
            step_index: {required: true, type: 'integer'},
            to: {required: true, ref: 'Position'},
            planned_start: {required: true, type: 'integer'},
            planned_duration: {required: true, type: 'integer'}
        },
        walk_settled: {
            bot: {required: true, type: 'integer'},
            step_index: {required: true, type: 'integer'},
            // Repeated from the dispatch: a walk the game never acknowledged
            // has no dispatch line, and a walk has no label anywhere.
            to: {required: true, ref: 'Position'},
            status: {required: true, type: 'string'},
            elapsed_ticks: {required: false, type: 'integer', nullable: true},
            error: {required: false, type: 'string', nullable: true},
            failure: {required: false, ref: 'WalkFailure', nullable: true}
        },
        teleport: {
            bot: {required: true, type: 'integer'},
            reason: {required: true, type: 'string'},
            from: {required: true, ref: 'Position'},
            to: {required: true, ref: 'Position'},
            distance: {required: true, type: 'number'},
            action_id: {required: false, type: 'integer', nullable: true}
        },
        placement_refused: {
            entity: {required: true, type: 'string'},
            position: {required: true, ref: 'Position'},
            direction: {required: false, type: 'integer', nullable: true},
            source: {required: true, type: 'string'},
            blockers: {required: true, type: 'array'},
            tile: {required: false, type: 'string', nullable: true}
        },
        bot_enclosed: {
            bot: {required: true, type: 'integer'},
            position: {required: true, ref: 'Position'},
            pocket_tiles: {required: true, type: 'number'},
            searched_tiles: {required: true, type: 'number'}
        },
        bot_benched: {
            bot: {required: true, type: 'integer'},
            position: {required: true, ref: 'Position'},
            refused_hops: {required: true, type: 'integer'},
            hop_tiles: {required: true, type: 'number'}
        },
        bot_released: {
            bot: {required: true, type: 'integer'},
            position: {required: true, ref: 'Position'},
            why: {required: true, type: 'string'}
        },
        bot_stepped_aside: {
            bot: {required: true, type: 'integer'},
            from: {required: true, ref: 'Position'},
            to: {required: true, ref: 'Position'},
            placing: {required: true, type: 'string'},
            site: {required: true, ref: 'Position'},
            reason: {required: true, type: 'string'},
            pocket_tiles: {required: true, type: 'number'}
        },
        bot_died: {
            bot: {required: true, type: 'integer'},
            position: {required: false, ref: 'Position', nullable: true},
            cause: {required: false, type: 'string', nullable: true},
            cause_type: {required: false, type: 'string', nullable: true},
            respawn_in: {required: false, type: 'integer', nullable: true}
        },
        bot_respawned: {
            bot: {required: true, type: 'integer'},
            position: {required: false, ref: 'Position', nullable: true}
        },
        roster_changed: {
            bots: {required: true, type: 'array'},
            left: {required: true, type: 'array'},
            returned: {required: true, type: 'array'},
            reason: {required: true, type: 'string'}
        },
        // The free-vision disclosure. Every "unknown" here is nullable and
        // has a counter beside it (`bot_samples`, `model_resource_tiles`)
        // that says which kind of unknown it is -- a null distance with a
        // zero count is "nothing looked", and a null distance with a non-zero
        // count would be a broken writer.
        vision_measured: {
            travelled_tiles: {required: false, type: 'number', nullable: true},
            travelled_bot: {required: false, type: 'integer', nullable: true},
            travelled_at_tick: {required: false, type: 'integer', nullable: true},
            bot_samples: {required: true, type: 'integer'},
            model_tiles: {required: false, type: 'number', nullable: true},
            model_furthest: {required: false, type: 'string', nullable: true},
            model_furthest_position: {required: false, ref: 'Position', nullable: true},
            model_resource_tiles: {required: true, type: 'integer'},
            model_enemy_structures: {required: true, type: 'integer'},
            unearned_ratio: {required: false, type: 'number', nullable: true}
        },
        run_finished: {
            outcome: {required: true, type: 'string'},
            elapsed_ticks: {required: true, type: 'integer'}
        },
        // A kind this build does not know. The server never emits it, but a
        // future variant must still decode rather than fail to parse.
        unknown: {}
    }),
    PlannedStep: objectContract<PlannedStep>({
        id: {required: true, type: 'integer'},
        bot: {required: true, type: 'integer'},
        action: {required: true, type: 'string'},
        deps: {required: true, type: 'array'},
        planned_start: {required: true, type: 'integer'},
        planned_duration: {required: true, type: 'integer'}
    }),
    // What the counters cannot say: which step is waiting, for what, and for
    // how long. `id` and `step_index` are nullable in opposite directions --
    // an action has the first, a walk the second -- so neither may be read as
    // "we failed to look it up".
    WaitingStep: objectContract<WaitingStep>({
        id: {required: false, type: 'integer', nullable: true},
        step_index: {required: false, type: 'integer', nullable: true},
        bot: {required: true, type: 'integer'},
        action: {required: true, type: 'string'},
        target: {required: false, ref: 'Position', nullable: true},
        waiting_on: {required: true, type: 'string'},
        blocked_by: {required: false, type: 'integer', nullable: true},
        deadline_tick: {required: false, type: 'integer', nullable: true},
        waiting_ms: {required: true, type: 'integer'}
    }),
    SatisfiedReason: enumContract<SatisfiedReason>({
        already_satisfied: true,
        plan_empty: true,
        unknown: true
    }),
    FailureKind: enumContract<FailureKind>({
        missing_item: true,
        unreachable: true,
        blocked: true,
        partial_transfer: true,
        rejected: true,
        timeout: true,
        no_character: true,
        other: true
    }),
    ActionFailure: objectContract<ActionFailure>({
        kind: {required: true, ref: 'FailureKind'},
        detail: {required: false, type: 'string', nullable: true}
    }),
    // A walk's own failure vocabulary, deliberately not more members on
    // `FailureKind`: every distinction here is about the pathfinder.
    // `no_path` (it searched, and there is no way there) and
    // `pathfinder_busy` (it never searched, so nothing was learned) are the
    // pair the whole type exists to keep apart.
    WalkFailureKind: enumContract<WalkFailureKind>({
        no_path: true,
        pathfinder_busy: true,
        repath_limit: true,
        stalled: true,
        timeout: true,
        no_character: true,
        destination_blocked: true,
        boxed_in: true,
        other: true
    }),
    WalkFailure: objectContract<WalkFailure>({
        kind: {required: true, ref: 'WalkFailureKind'},
        // Both observed, and both null when the mod's wording named no
        // position. `destination` is NOT `walk_settled.to`: it is the last
        // waypoint of the path the game returned, which is what the walk was
        // really steering at.
        from: {required: false, ref: 'Position', nullable: true},
        destination: {required: false, ref: 'Position', nullable: true}
    }),

    // -- world-state samples (crates/core/src/record/samples.rs) ----------
    RunSamplesResponse: objectContract<RunSamplesResponse>({
        samples: {required: true, arrayOf: 'Sample'},
        skipped: {required: true, type: 'integer'}
    }),
    // `SampleKind` flattened into `Sample`: utoipa cannot fold a
    // `#[serde(flatten)]` back into one flat object, so it publishes
    // `allOf: [{$ref: SampleKind}, {schema, tick, run}]` instead of a single
    // object schema. `mergeContract` is exhaustive only over what `Sample`
    // adds beyond `SampleKind` -- `schema`, `tick` and `run`.
    Sample: mergeContract<Sample>('SampleKind', {
        // A real field, not just probed-and-discarded on read: an archived
        // line must still carry its schema stamp.
        schema: {required: true, type: 'integer'},
        tick: {required: true, type: 'integer'},
        // `None` for a line written before this field existed. Always
        // serialised (never omitted), so present-and-null, not absent --
        // like every other such field in this API.
        run: {required: false, type: 'string', nullable: true}
    }),
    // An internally tagged enum (`#[serde(tag = "kind")]`): utoipa publishes
    // one inline object per variant rather than a single schema, so this is
    // `taggedUnionContract`, not `objectContract`.
    SampleKind: taggedUnionContract<SampleKind>({
        bots: {
            bots: {required: true, arrayOf: 'BotSample'}
        },
        force: {
            // Present-and-null: "nothing queued" is a fact, not an absence.
            research: {required: false, ref: 'ResearchSample', nullable: true},
            techs_unlocked: {required: true, type: 'integer'},
            production: {required: true, ref: 'ProductionSample'},
            power: {required: true, ref: 'PowerSample'}
        },
        // `machines` is a map keyed by `unit_number`, so `type: 'object'` and
        // not `arrayOf: 'MachineSample'` -- the map shape is deliberate (the
        // mod's `table_to_json` writes an empty Lua table as `{}`, which is
        // what makes an empty `bots` array unparseable) and the element type
        // is pinned by `MachineSample`'s own row below.
        machines: {
            machines: {required: true, type: 'object'},
            truncated: {required: true, type: 'integer'}
        },
        // A kind this build does not know. The server never emits it, but a
        // future variant must still decode rather than fail to parse.
        unknown: {}
    }),
    BotSample: objectContract<BotSample>({
        id: {required: true, type: 'integer'},
        position: {required: true, ref: 'Position'},
        inventory: {required: true, type: 'object'},
        crafting_queue: {required: true, type: 'integer'},
        mining: {required: false, type: 'string', nullable: true}
    }),
    ResearchSample: objectContract<ResearchSample>({
        name: {required: true, type: 'string'},
        progress: {required: true, type: 'number'},
        eta_ticks: {required: false, type: 'integer', nullable: true}
    }),
    ProductionSample: objectContract<ProductionSample>({
        made: {required: true, type: 'object'},
        consumed: {required: true, type: 'object'}
    }),
    PowerSample: objectContract<PowerSample>({
        generated_kw: {required: true, type: 'number'},
        consumed_kw: {required: true, type: 'number'},
        satisfaction: {required: true, type: 'number'},
        // `required: false` purely because `#[serde(default)]` lets a
        // pre-schema-2 archive decode without it; a live server always
        // serialises it, so `types.ts` declares it present and this row
        // carries no `nullable` -- the same reading as `EventKind`'s `plan`.
        networks: {required: false, type: 'object'}
    }),
    NetworkPower: objectContract<NetworkPower>({
        sub_ids: {required: true, type: 'array'},
        generated_kw: {required: true, type: 'number'},
        consumed_kw: {required: true, type: 'number'},
        demanded_kw: {required: true, type: 'number'},
        satisfaction: {required: true, type: 'number'}
    }),
    MachineSample: objectContract<MachineSample>({
        name: {required: true, type: 'string'},
        type: {required: true, type: 'string'},
        position: {required: true, ref: 'Position'},
        // Present-and-null, every one of them: the six sampled entity types do
        // not all answer the same questions, and "this machine is not a
        // crafting machine" is a fact rather than a gap.
        status: {required: false, type: 'string', nullable: true},
        network: {required: false, type: 'integer', nullable: true},
        recipe: {required: false, type: 'string', nullable: true},
        crafting: {required: false, type: 'boolean', nullable: true},
        progress: {required: false, type: 'number', nullable: true},
        products_finished: {required: false, type: 'integer', nullable: true},
        mining: {required: false, type: 'string', nullable: true},
        // The three inventories are `required: false` for the same reason
        // `PowerSample.networks` is: the *mod* omits an empty one to save
        // forty bytes a row, and the server restores it, so a response never
        // actually lacks them and `types.ts` declares them present.
        input: {required: false, type: 'object'},
        output: {required: false, type: 'object'},
        fuel: {required: false, type: 'object'}
    }),
    Position: objectContract<Position>({
        x: {required: true, type: 'number'},
        y: {required: true, type: 'number'}
    }),

    // Not `objectContract<…>`: `sendRcon(command)` builds this body as an
    // inline literal, so there is no declaration in `types.ts` to bind it to
    // and this row is the client's only statement of the shape -- a second
    // mirror of the server, not a third. Give it one and this becomes
    // `objectContract<RconBody>` like the rest.
    RconBody: {
        kind: 'object',
        properties: {command: {required: true, type: 'string'}}
    },
    // Nothing is required: the server reads "exactly one of `path` or `code`"
    // off which fields are present, so a required field appearing here is a
    // breaking change to every caller of `executeScript`. The four
    // `nullable: true`s are not optional decoration either -- `types.ts`
    // declares all four with `?`, and `PropertyContractFor` makes that agree.
    ExecuteRequest: objectContract<ExecuteRequest>({
        path: {required: false, type: 'string', nullable: true},
        code: {required: false, type: 'string', nullable: true},
        language: {required: false, type: 'string', nullable: true},
        bot_count: {required: false, type: 'integer', nullable: true}
    }),
    ExecuteAccepted: objectContract<ExecuteAccepted>({
        job_id: {required: true, ref: 'JobId'}
    }),
    Job: objectContract<Job>({
        id: {required: true, ref: 'JobId'},
        script: {required: false, type: 'string', nullable: true},
        status: {required: true, ref: 'JobStatus'},
        started_at_ms: {required: true, type: 'integer'},
        finished_at_ms: {required: false, type: 'integer', nullable: true},
        stdout: {required: true, type: 'string'},
        stderr: {required: true, type: 'string'},
        error: {required: false, type: 'string', nullable: true},
        replay: {required: false, type: 'string', nullable: true}
    }),
    // A `u64` counter serialised as a string so a browser cannot lose
    // precision on it. `types.ts` types every job id as `string` inline (in
    // `Job.id` and `ExecuteAccepted.job_id`) rather than as a named alias, so
    // there is nothing to bind this row to; the two `ref: 'JobId'` entries
    // above are what tie it to those two fields. A schema that turned back
    // into an integer would break `getJob`'s URL building.
    JobId: {kind: 'scalar', type: 'string'},
    JobStatus: enumContract<JobStatus>({running: true, succeeded: true, failed: true}),

    // -- video: the host-side recording ------------------------------------
    VideoManifest: objectContract<VideoManifest>({
        run: {required: true, type: 'string', nullable: true},
        video: {required: true, ref: 'VideoRecord', nullable: true},
        bytes: {required: true, type: 'integer', nullable: true},
        samples: {required: true, type: 'integer'},
        skipped: {required: true, type: 'integer'},
        tick_range: {required: true, ref: 'TickRange', nullable: true}
    }),
    VideoRecord: objectContract<VideoRecord>({
        run: {required: true, type: 'string'},
        file: {required: true, type: 'string'},
        width: {required: true, type: 'integer'},
        height: {required: true, type: 'integer'},
        requested_width: {required: true, type: 'integer'},
        requested_height: {required: true, type: 'integer'},
        fps: {required: true, type: 'integer'},
        status: {required: true, ref: 'VideoStatus'},
        reason: {required: true, type: 'string', nullable: true},
        ffmpeg_exit: {required: true, type: 'integer', nullable: true},
        calibration: {required: true, arrayOf: 'Calibration'},
        // `null` is *unknown*, never "fine": a recording with one calibration
        // pair had its rate checked by nothing.
        rate_ok: {required: true, type: 'boolean', nullable: true},
        window_id: {required: true, type: 'string', nullable: true}
    }),
    VideoStatus: enumContract<VideoStatus>({
        recording: true,
        stopped: true,
        killed: true,
        died: true,
        failed: true,
        stopped_low_disk: true
    }),
    Calibration: objectContract<Calibration>({
        host_wall_ms: {required: true, type: 'integer'},
        out_time_ms: {required: true, type: 'integer'}
    }),
    TickRange: objectContract<TickRange>({
        from: {required: true, type: 'integer'},
        to: {required: true, type: 'integer'}
    }),
    // Short keys because the file this mirrors has thousands of lines. `t` and
    // `reason` are absent-able because a gap line has neither; `k` is always
    // written, which is why it is required here and `types.ts` declares it
    // without a `?`.
    TickSample: objectContract<TickSample>({
        t: {required: false, type: 'integer', nullable: true},
        w: {required: true, type: 'integer'},
        k: {required: true, ref: 'TickKind'},
        reason: {required: false, type: 'string', nullable: true}
    }),
    TickKind: enumContract<TickKind>({sample: true, start: true, gap: true, stop: true}),
    VideoTicksResponse: objectContract<VideoTicksResponse>({
        samples: {required: true, arrayOf: 'TickSample'},
        skipped: {required: true, type: 'integer'}
    }),

    // -- the error body `http.ts` reads on every failure -------------------
    // Also unbound, for the same reason as `RconBody`: `errorFromResponse`
    // narrows the parsed body field by field (`typeof record.code ===
    // 'number'`) instead of declaring a DTO, so this row is the only
    // client-side statement of the shape.
    //
    // What it cannot see is the *value* of `code`: `code: 2` means "no
    // Factorio instance is running", every store identifies the condition by
    // that number rather than by the status, and a number is not part of a
    // schema. That contract is pinned on the server instead, by
    // `stopping_when_nothing_runs_is_an_error_not_a_panic`,
    // `rcon_without_a_running_instance_reports_not_started` and
    // `executing_without_a_running_instance_is_service_unavailable`.
    ErrorResponse: {
        kind: 'object',
        properties: {
            message: {required: true, type: 'string'},
            code: {required: true, type: 'integer'},
            running_job_id: {required: false, type: 'string', nullable: true}
        }
    }
};

function schemaName(schema: SchemaObject | undefined): string | undefined {
    if (schema?.$ref) {
        return schema.$ref.replace('#/components/schemas/', '');
    }
    // A nullable $ref (an `Option<T>` naming another schema, e.g.
    // `SampleKind::Force::research`) is published as
    // `oneOf: [{type: "null"}, {$ref: ...}]` rather than a bare `$ref`.
    const refMember = schema?.oneOf?.find(member => member.$ref !== undefined);
    return refMember?.$ref?.replace('#/components/schemas/', '');
}

/** Whether a property schema allows `null`, in either form utoipa emits it. */
function isNullable(schema: SchemaObject | undefined): boolean {
    if (Array.isArray(schema?.type)) {
        return schema.type.includes('null');
    }
    return schema?.oneOf?.some(member => member.type === 'null') ?? false;
}

/**
 * Field-set and required-set assertions shared by a plain object schema and
 * by one variant of a tagged union -- the same comparison, just against a
 * narrower slice of the spec and the contract in the union case.
 */
function expectFieldsMatch(
    where: string,
    schemaProperties: Record<string, SchemaObject> | undefined,
    schemaRequired: string[] | undefined,
    contractProperties: Record<string, PropertyContract>
): void {
    expect(
        Object.keys(schemaProperties ?? {}).sort(),
        where + ' publishes different fields than app/src/api/types.ts declares. ' +
            'A renamed or added field has to be mirrored there before this passes.'
    ).toEqual(Object.keys(contractProperties).sort());

    const expectedRequired = Object.entries(contractProperties)
        .filter(([, property]) => property.required)
        .map(([field]) => field)
        .sort();
    expect(
        [...(schemaRequired ?? [])].sort(),
        where + ' publishes a different required set. A newly required field is a ' +
            'breaking change for every caller that omits it.'
    ).toEqual(expectedRequired);
}

/** Per-field type and nullability assertions, shared the same way. */
function expectFieldTypesMatch(
    where: string,
    schemaProperties: Record<string, SchemaObject> | undefined,
    contractProperties: Record<string, PropertyContract>
): void {
    const properties = schemaProperties ?? {};
    for (const [field, expected] of Object.entries(contractProperties)) {
        const property = properties[field];
        expect(property, where + '.' + field + ' is missing').toBeDefined();
        const fieldWhere = where + '.' + field;

        if (expected.ref) {
            expect(schemaName(property), fieldWhere + ' no longer refs ' + expected.ref)
                .toBe(expected.ref);
            expect(
                isNullable(property),
                fieldWhere + (expected.nullable
                    ? ' is no longer nullable, but types.ts declares `| null`'
                    : ' became nullable, and types.ts does not declare `| null`')
            ).toBe(expected.nullable === true);
            continue;
        }
        if (expected.arrayOf) {
            expect(property.type, fieldWhere + ' is no longer an array').toBe('array');
            expect(
                schemaName(property.items),
                fieldWhere + ' is no longer an array of ' + expected.arrayOf
            ).toBe(expected.arrayOf);
            continue;
        }

        // utoipa writes a nullable field as `type: [t, "null"]` and a
        // non-nullable one as `type: t`, so both facts come out of the same
        // key.
        const types = Array.isArray(property.type) ? property.type : [property.type];
        expect(
            types,
            fieldWhere + ' is published as ' + JSON.stringify(property.type) +
                ', but types.ts reads it as a ' + expected.type
        ).toContain(expected.type);
        expect(
            types.includes('null'),
            fieldWhere + (expected.nullable
                ? ' is no longer nullable, but types.ts declares `| null`'
                : ' became nullable, and types.ts does not declare `| null`')
        ).toBe(expected.nullable === true);
    }
}

function findOperation(contract: OperationContract): Operation | undefined {
    return spec.paths[contract.path]?.[contract.method];
}

function label(contract: OperationContract): string {
    return contract.method.toUpperCase() + ' ' + contract.path;
}

describe('the operations app/src/api/client.ts calls', () => {
    it.each(OPERATIONS.map(contract => [label(contract), contract] as const))(
        '%s is published by the server',
        (_name, contract) => {
            expect(
                spec.paths[contract.path],
                'the snapshot has no path ' + contract.path +
                    '; client.ts calls it from ' + contract.caller + '()'
            ).toBeDefined();
            expect(
                findOperation(contract),
                'the snapshot has no ' + label(contract) +
                    '; client.ts calls it from ' + contract.caller + '()'
            ).toBeDefined();
        }
    );

    it.each(OPERATIONS.map(contract => [label(contract), contract] as const))(
        '%s takes exactly the parameters the client sends',
        (_name, contract) => {
            const parameters = findOperation(contract)?.parameters ?? [];
            const expected = [
                ...(contract.query ?? []).map(([name, required]) => ({name, in: 'query', required})),
                // Path parameters are always required in OpenAPI.
                ...(contract.pathParams ?? []).map(name => ({name, in: 'path', required: true}))
            ];
            // Sets, not sequences: the order utoipa emits parameters in is not
            // part of the contract, and pinning it would make this fail for a
            // reason the client does not care about.
            const actual = parameters.map(parameter => ({
                name: parameter.name,
                in: parameter.in,
                required: parameter.required ?? false
            }));
            expect(
                [...actual].sort((a, b) => a.name.localeCompare(b.name)),
                label(contract) + ' publishes different parameters than ' +
                    contract.caller + '() sends'
            ).toEqual([...expected].sort((a, b) => a.name.localeCompare(b.name)));
        }
    );

    it.each(
        OPERATIONS.filter(contract => contract.requestSchema !== undefined).map(
            contract => [label(contract), contract] as const
        )
    )('%s accepts the request body the client sends', (_name, contract) => {
        const body = findOperation(contract)?.requestBody;
        expect(body, label(contract) + ' publishes no request body').toBeDefined();
        expect(
            schemaName(body?.content?.['application/json']?.schema),
            label(contract) + ' no longer takes a ' + contract.requestSchema + ' body'
        ).toBe(contract.requestSchema);
    });

    it.each(
        OPERATIONS.filter(contract => contract.requestSchema === undefined).map(
            contract => [label(contract), contract] as const
        )
    )('%s takes no request body, as the client assumes', (_name, contract) => {
        // The three bodyless POSTs matter: `request()` only sets a
        // Content-Type when a body is given, so a body that became required
        // would be answered 415/400 with nothing in the client to notice.
        expect(
            findOperation(contract)?.requestBody,
            label(contract) + ' now takes a request body that ' +
                contract.caller + '() does not send'
        ).toBeUndefined();
    });

    it.each(OPERATIONS.map(contract => [label(contract), contract] as const))(
        '%s answers the success body the client parses',
        (_name, contract) => {
            const {status, schema, arrayOf, mediaType, empty} = contract.response;
            const response = findOperation(contract)?.responses?.[status];
            expect(
                response,
                label(contract) + ' no longer documents a ' + status +
                    '; ' + contract.caller + '() parses that status'
            ).toBeDefined();

            const content = response?.content;
            if (empty) {
                expect(
                    content,
                    label(contract) + ' now answers ' + status + ' with a body; ' +
                        contract.caller + '() returns void'
                ).toBeUndefined();
                return;
            }
            if (mediaType) {
                expect(
                    Object.keys(content ?? {}),
                    label(contract) + ' no longer answers ' + mediaType
                ).toContain(mediaType);
                return;
            }
            const json = content?.['application/json']?.schema;
            if (arrayOf) {
                expect(json?.type, label(contract) + ' no longer answers a JSON array').toBe('array');
                expect(
                    schemaName(json?.items),
                    label(contract) + ' no longer answers an array of ' + arrayOf
                ).toBe(arrayOf);
                return;
            }
            expect(
                schemaName(json),
                label(contract) + ' no longer answers a ' + schema +
                    '; ' + contract.caller + '() types its result as one'
            ).toBe(schema);
        }
    );
});

describe('the schemas app/src/api/types.ts mirrors', () => {
    const names = Object.keys(SCHEMAS);

    it.each(names)('%s is still published', name => {
        expect(
            spec.components.schemas[name],
            'the snapshot has no schema ' + name
        ).toBeDefined();
    });

    it.each(names.filter(name => SCHEMAS[name].kind === 'object'))(
        '%s has exactly the fields the client declares',
        name => {
            const contract = SCHEMAS[name];
            if (contract.kind !== 'object') {
                throw new Error('filtered above');
            }
            const schema = spec.components.schemas[name];
            expectFieldsMatch(name, schema.properties, schema.required, contract.properties);
        }
    );

    it.each(names.filter(name => SCHEMAS[name].kind === 'object'))(
        '%s publishes each field with the type the client reads',
        name => {
            const contract = SCHEMAS[name];
            if (contract.kind !== 'object') {
                throw new Error('filtered above');
            }
            expectFieldTypesMatch(name, spec.components.schemas[name].properties, contract.properties);
        }
    );

    it.each(names.filter(name => SCHEMAS[name].kind === 'merge'))(
        '%s (a flattened union) has exactly the extra fields the client declares',
        name => {
            const contract = SCHEMAS[name];
            if (contract.kind !== 'merge') {
                throw new Error('filtered above');
            }
            const schema = spec.components.schemas[name];
            const [baseMember, extraMember] = schema.allOf ?? [];
            expect(
                schemaName(baseMember),
                name + ' no longer flattens ' + contract.base
            ).toBe(contract.base);
            expect(
                extraMember,
                name + ' is no longer published as allOf[base, extra fields]'
            ).toBeDefined();
            expectFieldsMatch(
                name,
                extraMember?.properties,
                extraMember?.required,
                contract.extra
            );
        }
    );

    it.each(names.filter(name => SCHEMAS[name].kind === 'merge'))(
        '%s (a flattened union) publishes each extra field with the type the client reads',
        name => {
            const contract = SCHEMAS[name];
            if (contract.kind !== 'merge') {
                throw new Error('filtered above');
            }
            const schema = spec.components.schemas[name];
            const extraMember = schema.allOf?.[1];
            expectFieldTypesMatch(name, extraMember?.properties, contract.extra);
        }
    );

    it.each(names.filter(name => SCHEMAS[name].kind === 'taggedUnion'))(
        '%s publishes exactly the variants the client union lists',
        name => {
            const contract = SCHEMAS[name];
            if (contract.kind !== 'taggedUnion') {
                throw new Error('filtered above');
            }
            const schema = spec.components.schemas[name];
            const members = schema.oneOf ?? [];
            const tagsPublished = members
                .map(member => member.properties?.[contract.discriminant]?.enum?.[0])
                .filter((tag): tag is string => tag !== undefined)
                .sort();
            expect(
                tagsPublished,
                name + ' publishes a different set of ' + contract.discriminant +
                    ' variants than the union in types.ts. A variant added or removed has to be ' +
                    'mirrored there before this passes.'
            ).toEqual(Object.keys(contract.variants).sort());
        }
    );

    it.each(names.filter(name => SCHEMAS[name].kind === 'taggedUnion'))(
        '%s publishes each variant with exactly the fields the client declares',
        name => {
            const contract = SCHEMAS[name];
            if (contract.kind !== 'taggedUnion') {
                throw new Error('filtered above');
            }
            const schema = spec.components.schemas[name];
            for (const member of schema.oneOf ?? []) {
                const tag = member.properties?.[contract.discriminant]?.enum?.[0];
                if (tag === undefined || !(tag in contract.variants)) {
                    // Reported as a set mismatch by the test above already.
                    continue;
                }
                const variantProperties = contract.variants[tag];
                // The discriminant itself is not one of the variant's own
                // fields in the contract -- every variant carries it, and
                // spelling it out per variant would buy nothing.
                const propertiesMinusTag = {...member.properties};
                delete propertiesMinusTag[contract.discriminant];
                const requiredMinusTag = (member.required ?? []).filter(
                    field => field !== contract.discriminant
                );
                const where = name + '[' + tag + ']';
                expectFieldsMatch(where, propertiesMinusTag, requiredMinusTag, variantProperties);
                expectFieldTypesMatch(where, propertiesMinusTag, variantProperties);
            }
        }
    );

    it.each(names.filter(name => SCHEMAS[name].kind === 'enum'))(
        '%s publishes exactly the variants the client union lists',
        name => {
            const contract = SCHEMAS[name];
            if (contract.kind !== 'enum') {
                throw new Error('filtered above');
            }
            const schema = spec.components.schemas[name];
            expect(schema.type, name + ' is no longer a string enum').toBe('string');
            expect(
                [...(schema.enum ?? [])].sort(),
                name + ' publishes different variants than the union in types.ts'
            ).toEqual([...contract.values].sort());
        }
    );

    it.each(names.filter(name => SCHEMAS[name].kind === 'scalar'))(
        '%s is still the primitive the client reads',
        name => {
            const contract = SCHEMAS[name];
            if (contract.kind !== 'scalar') {
                throw new Error('filtered above');
            }
            expect(
                spec.components.schemas[name].type,
                name + ' changed primitive type'
            ).toBe(contract.type);
        }
    );

    /**
     * The completeness half of the guard above.
     *
     * Without it, `SCHEMAS` would only have to be non-empty: a server change
     * that introduced a *new* schema into a response the client parses would
     * be pinned by nothing, and every test here would still pass. This walks
     * the `$ref` graph out of the sixteen operations instead and demands that
     * what it finds is exactly what `SCHEMAS` describes.
     */
    it('pins every schema the client can send or receive', () => {
        const reachable = new Set<string>();
        const visit = (node: unknown): void => {
            if (Array.isArray(node)) {
                node.forEach(visit);
                return;
            }
            if (node === null || typeof node !== 'object') {
                return;
            }
            const record = node as Record<string, unknown>;
            const ref = record.$ref;
            if (typeof ref === 'string') {
                const name = ref.replace('#/components/schemas/', '');
                if (!reachable.has(name)) {
                    reachable.add(name);
                    visit(spec.components.schemas[name]);
                }
            }
            for (const [key, value] of Object.entries(record)) {
                if (key !== '$ref') {
                    visit(value);
                }
            }
        };

        for (const contract of OPERATIONS) {
            const operation = findOperation(contract);
            visit(operation?.requestBody);
            // Every response, not just the success one: `http.ts` parses the
            // error body too, and `ApiError.code` comes out of it.
            visit(operation?.responses);
        }

        expect(
            [...reachable].sort(),
            'the client can now see a schema this file does not describe (or describes ' +
                'one it can no longer see). Add it to SCHEMAS, and mirror it in ' +
                'app/src/api/types.ts, rather than deleting the row.'
        ).toEqual(Object.keys(SCHEMAS).sort());
    });
});
