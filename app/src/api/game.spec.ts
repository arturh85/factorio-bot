import {afterEach, describe, expect, it} from 'vitest';
import {vi} from 'vitest';
import {EntityShapeError, findEntities, parseFactorioEntities, parseFactorioEntity} from './game';
import {RESOURCE_ENTITIES, SPAWN_ENTITIES} from './game.fixtures';

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

function urlOf(fetchMock: ReturnType<typeof stubFetch>): string {
    return fetchMock.mock.calls[0][0];
}

afterEach(() => {
    vi.unstubAllGlobals();
});

describe('findEntities', () => {
    it('sends position as "x,y" and drops radius/name/entity_type when omitted', async () => {
        const fetchMock = json(200, []);
        await findEntities({position: {x: 3, y: -7}});
        expect(urlOf(fetchMock)).toBe('/api/v1/game/find-entities?position=3%2C-7');
    });

    it('sends radius and entity_type when given', async () => {
        const fetchMock = json(200, []);
        await findEntities({position: {x: 0, y: 0}, radius: 25, entity_type: 'resource'});
        const url = urlOf(fetchMock);
        expect(url).toContain('position=0%2C0');
        expect(url).toContain('radius=25');
        expect(url).toContain('entity_type=resource');
    });

    it('sends name when given', async () => {
        const fetchMock = json(200, []);
        await findEntities({position: {x: 0, y: 0}, name: 'iron-ore'});
        expect(urlOf(fetchMock)).toContain('name=iron-ore');
    });

    it('resolves with the parsed body on 200', async () => {
        json(200, SPAWN_ENTITIES);
        await expect(findEntities({position: {x: 0, y: 0}})).resolves.toEqual(SPAWN_ENTITIES);
    });
});

describe('parseFactorioEntity / parseFactorioEntities -- real captures', () => {
    it('parses the seven-entity spawn capture without throwing', () => {
        expect(SPAWN_ENTITIES).toHaveLength(7);
    });

    it('parses the forty-entity resources capture without throwing', () => {
        expect(RESOURCE_ENTITIES).toHaveLength(40);
    });

    it('carries every required field for the crash-site-spaceship entity', () => {
        const ship = SPAWN_ENTITIES.find(e => e.name === 'crash-site-spaceship');
        expect(ship).toBeDefined();
        expect(ship?.entity_type).toBe('container');
        expect(ship?.bounding_box.left_top.y).toBeCloseTo(-9.296875);
        expect(ship?.bounding_box.right_bottom.y).toBeCloseTo(-1.5);
    });

    /**
     * The orientation pin, at the data layer: `left_top.y` must stay
     * numerically SMALLER than `right_bottom.y` after parsing -- a parser
     * that swapped or negated `y` would still produce a well-typed `Rect`
     * that renders every entity upside down with no type error anywhere.
     */
    it('does not flip or swap y while parsing a bounding box', () => {
        const ship = SPAWN_ENTITIES.find(e => e.name === 'crash-site-spaceship');
        expect(ship?.bounding_box.left_top.y).toBeLessThan(ship!.bounding_box.right_bottom.y);
    });

    it('tolerates the empty-object form of an inventory ("{}" for an empty Lua table)', () => {
        const furnace = SPAWN_ENTITIES.find(e => e.name === 'stone-furnace');
        expect(furnace).toBeDefined();
        expect(furnace?.output_inventory).toEqual([]);
        expect(furnace?.fuel_inventory).toEqual([]);
    });

    it('parses pickup_position/drop_position for the inserter, null for everything else', () => {
        const inserter = SPAWN_ENTITIES.find(e => e.name === 'inserter');
        expect(inserter?.pickup_position).toEqual({x: 5.5, y: 2.5});
        expect(inserter?.drop_position).toEqual({x: 3.30078125, y: 2.5});

        const ship = SPAWN_ENTITIES.find(e => e.name === 'crash-site-spaceship');
        expect(ship?.pickup_position).toBeNull();
        expect(ship?.drop_position).toBeNull();
    });

    it('leaves amount null for non-resource entities and set for resources', () => {
        const ship = SPAWN_ENTITIES.find(e => e.name === 'crash-site-spaceship');
        expect(ship?.amount).toBeNull();
    });

    /**
     * These captures predate `underground_half` (task 5), so the key is
     * entirely absent -- the same shape as `ghost_name`/`ghost_type` on
     * every entity that is not a ghost. `parseFactorioEntity` must default
     * a missing key to `null`, not throw.
     */
    it('leaves underground_half null for a capture that predates the field', () => {
        const ship = SPAWN_ENTITIES.find(e => e.name === 'crash-site-spaceship');
        expect(ship?.underground_half).toBeNull();
    });
});

describe('parseFactorioEntity -- underground_half', () => {
    const base = {
        name: 'underground-belt',
        entity_type: 'underground-belt',
        position: {x: 0, y: 0},
        bounding_box: {left_top: {x: -0.4, y: -0.4}, right_bottom: {x: 0.4, y: 0.4}},
        direction: 0
    };

    it('parses "input" and "output"', () => {
        expect(parseFactorioEntity({...base, underground_half: 'input'}).underground_half).toBe('input');
        expect(parseFactorioEntity({...base, underground_half: 'output'}).underground_half).toBe('output');
    });

    it('throws on a value that is neither "input" nor "output"', () => {
        expect(() => parseFactorioEntity({...base, underground_half: 'sideways'}))
            .toThrow(/underground_half/);
    });
});

describe('parseFactorioEntity / parseFactorioEntities -- rejects malformed input', () => {
    it('throws EntityShapeError, naming the field, when a required field is missing', () => {
        expect(() => parseFactorioEntity({entity_type: 'resource'})).toThrow(EntityShapeError);
        expect(() => parseFactorioEntity({entity_type: 'resource'})).toThrow(/\$\.name/);
    });

    it('throws when bounding_box is missing entirely', () => {
        expect(() => parseFactorioEntity({
            name: 'x', entity_type: 'y', position: {x: 0, y: 0}, direction: 0
        })).toThrow(/bounding_box/);
    });

    it('throws when a nested position field is the wrong type', () => {
        expect(() => parseFactorioEntity({
            name: 'x',
            entity_type: 'y',
            position: {x: 'not-a-number', y: 0},
            bounding_box: {left_top: {x: 0, y: 0}, right_bottom: {x: 1, y: 1}},
            direction: 0
        })).toThrow(/\$\.position\.x/);
    });

    it('throws when an inventory field is neither an array, null, nor an empty object', () => {
        expect(() => parseFactorioEntity({
            name: 'x',
            entity_type: 'y',
            position: {x: 0, y: 0},
            bounding_box: {left_top: {x: 0, y: 0}, right_bottom: {x: 1, y: 1}},
            direction: 0,
            output_inventory: 'nope'
        })).toThrow(/output_inventory/);
    });

    it('parseFactorioEntities rejects a non-array top level', () => {
        expect(() => parseFactorioEntities({not: 'an array'})).toThrow(EntityShapeError);
    });
});
