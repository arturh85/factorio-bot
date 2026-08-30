/**
 * What pins `app/src/api/client.ts` and `app/src/api/types.ts` to the server.
 *
 * The client is hand-written, and TypeScript cannot see `crates/server`, so
 * without this file a field renamed in Rust typechecks here and fails in the
 * browser. The seam is `openapi.snapshot.json`, and it takes *two* guards to
 * work:
 *
 * 1. `the_committed_openapi_snapshot_matches_the_published_spec` in
 *    `crates/server/tests/openapi.rs` keeps the snapshot equal to what the
 *    server really publishes. Without it the snapshot would rot exactly the
 *    way `app/src/models/types.ts` has rotted away from its generator.
 * 2. This file checks that everything the client assumes is in the snapshot.
 *    Without it, regenerating the snapshot would bless any server change.
 *
 * So a rename in `crates/server` fails the Rust test; regenerating the
 * snapshot without touching the client then fails this one. Neither test can
 * be satisfied by editing the other's input.
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

interface SchemaObject {
    $ref?: string;
    type?: string | string[];
    enum?: string[];
    items?: SchemaObject;
    properties?: Record<string, SchemaObject>;
    required?: string[];
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
    }
] as const;

/** One property of a mirrored DTO, as `app/src/api/types.ts` declares it. */
interface PropertyContract {
    /** Listed in the schema's `required` array. */
    required: boolean;
    /** The JSON type, for a property the client reads as a primitive. */
    type?: 'string' | 'integer' | 'boolean' | 'array' | 'object';
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
    | {kind: 'scalar'; type: string};

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
    AppSettings: {
        kind: 'object',
        properties: {
            factorio: {required: true, ref: 'FactorioSettings'},
            restapi: {required: true, ref: 'RestApiSettings'},
            gui: {required: true, ref: 'GuiSettings'}
        }
    },
    FactorioSettings: {
        kind: 'object',
        properties: {
            client_count: {required: true, type: 'integer'},
            factorio_archive_path: {required: true, type: 'string'},
            map_exchange_string: {required: true, type: 'string'},
            rcon_pass: {required: true, type: 'string'},
            rcon_port: {required: true, type: 'integer'},
            recreate: {required: true, type: 'boolean'},
            seed: {required: true, type: 'string'},
            workspace_path: {required: true, type: 'string'}
        }
    },
    RestApiSettings: {
        kind: 'object',
        properties: {
            port: {required: true, type: 'integer'},
            // `Option<String>` on the Rust side; `string | null` in
            // models/types.ts, which is why it is not in `required`.
            web_root: {required: false, type: 'string', nullable: true}
        }
    },
    GuiSettings: {
        kind: 'object',
        properties: {
            enable_autostart: {required: true, type: 'boolean'},
            enable_restapi: {required: true, type: 'boolean'}
        }
    },
    ScriptTreeNode: {
        kind: 'object',
        properties: {
            key: {required: true, type: 'string'},
            label: {required: true, type: 'string'},
            leaf: {required: true, type: 'boolean'},
            children: {required: true, arrayOf: 'ScriptTreeNode'}
        }
    },

    // -- crates/server, mirrored in app/src/api/types.ts -------------------
    InstanceStatus: {
        kind: 'object',
        properties: {
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
        }
    },
    StartAccepted: {
        kind: 'object',
        properties: {accepted: {required: true, type: 'boolean'}}
    },
    ScriptContent: {
        kind: 'object',
        properties: {code: {required: true, type: 'string'}}
    },
    ExistsResponse: {
        kind: 'object',
        properties: {exists: {required: true, type: 'boolean'}}
    },
    RconBody: {
        kind: 'object',
        properties: {command: {required: true, type: 'string'}}
    },
    ExecuteRequest: {
        kind: 'object',
        // Nothing is required: the server reads "exactly one of `path` or
        // `code`" off which fields are present, so a required field appearing
        // here is a breaking change to every caller of `executeScript`.
        properties: {
            path: {required: false, type: 'string', nullable: true},
            code: {required: false, type: 'string', nullable: true},
            language: {required: false, type: 'string', nullable: true},
            bot_count: {required: false, type: 'integer', nullable: true}
        }
    },
    ExecuteAccepted: {
        kind: 'object',
        properties: {job_id: {required: true, ref: 'JobId'}}
    },
    Job: {
        kind: 'object',
        properties: {
            id: {required: true, ref: 'JobId'},
            script: {required: false, type: 'string', nullable: true},
            status: {required: true, ref: 'JobStatus'},
            started_at_ms: {required: true, type: 'integer'},
            finished_at_ms: {required: false, type: 'integer', nullable: true},
            stdout: {required: true, type: 'string'},
            stderr: {required: true, type: 'string'},
            error: {required: false, type: 'string', nullable: true}
        }
    },
    // A `u64` counter serialised as a string so a browser cannot lose
    // precision on it. `types.ts` types every job id as `string`; a schema
    // that turned back into an integer would break `getJob`'s URL building.
    JobId: {kind: 'scalar', type: 'string'},
    // Mirrored by `export type JobStatus` in types.ts.
    JobStatus: {kind: 'enum', values: ['running', 'succeeded', 'failed']},

    // -- the error body `http.ts` reads on every failure -------------------
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
    return schema?.$ref?.replace('#/components/schemas/', '');
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
            expect(
                Object.keys(schema.properties ?? {}).sort(),
                name + ' publishes different fields than app/src/api/types.ts declares. ' +
                    'A renamed or added field has to be mirrored there before this passes.'
            ).toEqual(Object.keys(contract.properties).sort());

            const expectedRequired = Object.entries(contract.properties)
                .filter(([, property]) => property.required)
                .map(([field]) => field)
                .sort();
            expect(
                [...(schema.required ?? [])].sort(),
                name + ' publishes a different required set. A newly required field is a ' +
                    'breaking change for every caller that omits it.'
            ).toEqual(expectedRequired);
        }
    );

    it.each(names.filter(name => SCHEMAS[name].kind === 'object'))(
        '%s publishes each field with the type the client reads',
        name => {
            const contract = SCHEMAS[name];
            if (contract.kind !== 'object') {
                throw new Error('filtered above');
            }
            const properties = spec.components.schemas[name].properties ?? {};
            for (const [field, expected] of Object.entries(contract.properties)) {
                const property = properties[field];
                expect(property, name + '.' + field + ' is missing').toBeDefined();
                const where = name + '.' + field;

                if (expected.ref) {
                    expect(schemaName(property), where + ' no longer refs ' + expected.ref)
                        .toBe(expected.ref);
                    continue;
                }
                if (expected.arrayOf) {
                    expect(property.type, where + ' is no longer an array').toBe('array');
                    expect(
                        schemaName(property.items),
                        where + ' is no longer an array of ' + expected.arrayOf
                    ).toBe(expected.arrayOf);
                    continue;
                }

                // utoipa writes a nullable field as `type: [t, "null"]` and a
                // non-nullable one as `type: t`, so both facts come out of the
                // same key.
                const types = Array.isArray(property.type) ? property.type : [property.type];
                expect(
                    types,
                    where + ' is published as ' + JSON.stringify(property.type) +
                        ', but types.ts reads it as a ' + expected.type
                ).toContain(expected.type);
                expect(
                    types.includes('null'),
                    where + (expected.nullable
                        ? ' is no longer nullable, but types.ts declares `| null`'
                        : ' became nullable, and types.ts does not declare `| null`')
                ).toBe(expected.nullable === true);
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
