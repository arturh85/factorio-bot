/**
 * `/api/v1/game/*` query routes.
 *
 * Kept separate from `client.ts`: that file's `OPERATIONS` comment says the
 * seventeen `/api/v1/game/*` operations the server publishes are out of its
 * scope and belong to a future `app/src/api/game.ts`. This is that file, for
 * the one route the map view needs so far (`find_entities`,
 * `crates/server/src/game/query.rs`). It is not yet pinned by
 * `openapi.contract.spec.ts` -- a later task that adds more of these routes
 * should fold them into that guard rather than letting a second, unpinned
 * client grow beside the first.
 */

import {request} from './http';
import {FactorioEntity, InventoryItemWithQuality, Position, Rect, UndergroundHalf} from './types';

export interface FindEntitiesQuery {
    position: Position;
    /** Metres. `undefined` asks the server for entities at the exact point only. */
    radius?: number;
    name?: string;
    entity_type?: string;
}

/**
 * `GET /api/v1/game/find-entities`. Answers `503 {code: 2}` (via `http.ts`'s
 * `ApiError`) when no Factorio instance is running -- see
 * `crates/server/src/error.rs::ErrorResponse::not_started`.
 *
 * This is a snapshot: the array describes entities as they were at the
 * moment the server answered, and nothing pushes updates after that. Callers
 * must attach their own fetch time and offer a re-query rather than implying
 * the result stays current -- see `mapStore.ts`.
 */
export function findEntities(query: FindEntitiesQuery): Promise<FactorioEntity[]> {
    return request<FactorioEntity[]>('/api/v1/game/find-entities', {
        query: {
            position: query.position.x + ',' + query.position.y,
            radius: query.radius,
            name: query.name,
            entity_type: query.entity_type
        }
    });
}

/**
 * Thrown by {@link parseFactorioEntities} naming the exact field that did not
 * fit -- the same reasoning as `replay.ts`'s `ReplayShapeError`: a JSON
 * import's fields widen to plain `string`/`number` in TypeScript, so an `as
 * FactorioEntity[]` cast on a fixture would compile whether or not the file
 * actually matches this shape. This is the runtime check that a cast skips.
 */
export class EntityShapeError extends Error {
    constructor(path: string, detail: string) {
        super(`entity document invalid at ${path}: ${detail}`);
        this.name = 'EntityShapeError';
    }
}

function fail(path: string, detail: string): never {
    throw new EntityShapeError(path, detail);
}

function asObject(value: unknown, path: string): Record<string, unknown> {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        fail(path, `expected an object, got ${JSON.stringify(value)}`);
    }
    return value as Record<string, unknown>;
}

function asArray(value: unknown, path: string): unknown[] {
    if (!Array.isArray(value)) {
        fail(path, `expected an array, got ${JSON.stringify(value)}`);
    }
    return value;
}

function asNumber(value: unknown, path: string): number {
    if (typeof value !== 'number') {
        fail(path, `expected a number, got ${JSON.stringify(value)}`);
    }
    return value;
}

function asNumberOrNull(value: unknown, path: string): number | null {
    return value === null || value === undefined ? null : asNumber(value, path);
}

function asString(value: unknown, path: string): string {
    if (typeof value !== 'string') {
        fail(path, `expected a string, got ${JSON.stringify(value)}`);
    }
    return value;
}

function asStringOrNull(value: unknown, path: string): string | null {
    return value === null || value === undefined ? null : asString(value, path);
}

/**
 * `underground_half`: `"input"` / `"output"` / `null`. A plain
 * `asStringOrNull` cast would accept any string, which defeats the point of
 * this module -- checking the shape a cast would only assume.
 */
function asUndergroundHalfOrNull(value: unknown, path: string): UndergroundHalf | null {
    if (value === null || value === undefined) {
        return null;
    }
    const s = asString(value, path);
    if (s !== 'input' && s !== 'output') {
        fail(path, `expected "input" or "output", got ${JSON.stringify(value)}`);
    }
    return s;
}

function parsePosition(value: unknown, path: string): Position {
    const obj = asObject(value, path);
    return {x: asNumber(obj.x, path + '.x'), y: asNumber(obj.y, path + '.y')};
}

function parsePositionOrNull(value: unknown, path: string): Position | null {
    return value === null || value === undefined ? null : parsePosition(value, path);
}

function parseRect(value: unknown, path: string): Rect {
    const obj = asObject(value, path);
    return {
        left_top: parsePosition(obj.left_top, path + '.left_top'),
        right_bottom: parsePosition(obj.right_bottom, path + '.right_bottom')
    };
}

function parseInventoryItem(value: unknown, path: string): InventoryItemWithQuality {
    const obj = asObject(value, path);
    return {
        name: asString(obj.name, path + '.name'),
        quality: asString(obj.quality, path + '.quality'),
        count: asNumber(obj.count, path + '.count')
    };
}

/**
 * `output_inventory` / `fuel_inventory`, tolerant of the one quirk noted in
 * `CLAUDE.md`: BotBridge's `helpers.table_to_json({})` serialises an empty
 * Lua table as `"{}"`, not `"[]"`, so a real capture (see
 * `crates/core/tests/live-2.1.17-entities-spawn.json`'s `stone-furnace`) can
 * carry an empty *object* here even though the field is documented as an
 * array. `crates/core/src/types.rs` has a custom deserializer
 * (`option_vec_or_empty_map`) for exactly this on the Rust side; a browser
 * talking to the real HTTP route never sees it (the route re-serialises the
 * already-normalised Rust struct, which always emits an array or `null`), but
 * the raw fixture captures this module imports for tests predate that
 * normalisation, so the parser has to tolerate both.
 */
function parseInventoryOrNull(value: unknown, path: string): InventoryItemWithQuality[] | null {
    if (value === null || value === undefined) {
        return null;
    }
    if (Array.isArray(value)) {
        return value.map((item, i) => parseInventoryItem(item, `${path}[${i}]`));
    }
    const obj = asObject(value, path);
    if (Object.keys(obj).length === 0) {
        return [];
    }
    fail(path, `expected an array (or an empty object standing in for one), got ${JSON.stringify(value)}`);
}

/** Validates and narrows one already-`JSON.parse`d value into a `FactorioEntity`. */
export function parseFactorioEntity(value: unknown, path = '$'): FactorioEntity {
    const obj = asObject(value, path);
    return {
        name: asString(obj.name, path + '.name'),
        entity_type: asString(obj.entity_type, path + '.entity_type'),
        position: parsePosition(obj.position, path + '.position'),
        bounding_box: parseRect(obj.bounding_box, path + '.bounding_box'),
        direction: asNumber(obj.direction, path + '.direction'),
        drop_position: parsePositionOrNull(obj.drop_position, path + '.drop_position'),
        pickup_position: parsePositionOrNull(obj.pickup_position, path + '.pickup_position'),
        output_inventory: parseInventoryOrNull(obj.output_inventory, path + '.output_inventory'),
        fuel_inventory: parseInventoryOrNull(obj.fuel_inventory, path + '.fuel_inventory'),
        amount: asNumberOrNull(obj.amount, path + '.amount'),
        recipe: asStringOrNull(obj.recipe, path + '.recipe'),
        ghost_name: asStringOrNull(obj.ghost_name, path + '.ghost_name'),
        ghost_type: asStringOrNull(obj.ghost_type, path + '.ghost_type'),
        underground_half: asUndergroundHalfOrNull(obj.underground_half, path + '.underground_half')
    };
}

/**
 * Validates and narrows an already-`JSON.parse`d value into a
 * `FactorioEntity[]` -- the array-level counterpart of
 * {@link parseFactorioEntity}, used both by `game.fixtures.ts` on the tracked
 * live captures and available to production code for the same reason
 * `parseReplay` is: a cast trusts the shape, this checks it.
 */
export function parseFactorioEntities(value: unknown): FactorioEntity[] {
    return asArray(value, '$').map((entity, i) => parseFactorioEntity(entity, `$[${i}]`));
}
