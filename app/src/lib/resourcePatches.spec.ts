/**
 * The outline tracer is the part of the map nobody can check by looking: a
 * wrong ring still renders as a plausible blob, and the tooltip it feeds
 * would then state a wrong tile count as fact. Everything here is asserted on
 * exact coordinates.
 */
import {describe, expect, it} from 'vitest';
import {EntitySnapshot, Position} from '@/api/types';
import {isResourceName, resourcePatches, ringsToPath} from './resourcePatches';

/** An entity at a TILE CENTRE, the way the game actually reports resources. */
function ore(name: string, x: number, y: number): EntitySnapshot {
    return {name, position: {x: x + 0.5, y: y + 0.5}, direction: 0};
}

/** Shoelace area. With y growing downward, clockwise on screen is positive. */
function signedArea(ring: Position[]): number {
    let sum = 0;
    for (let i = 0; i < ring.length; i++) {
        const a = ring[i];
        const b = ring[(i + 1) % ring.length];
        sum += a.x * b.y - b.x * a.y;
    }
    return sum / 2;
}

/** Total length of a ring set, in unit tile edges. */
function unitEdges(rings: Position[][]): number {
    return rings.reduce(
        (total, ring) =>
            total +
            ring.reduce((sum, point, i) => {
                const next = ring[(i + 1) % ring.length];
                return sum + Math.abs(next.x - point.x) + Math.abs(next.y - point.y);
            }, 0),
        0
    );
}

/** How many tile sides a tile set exposes, counted the naive way. */
function boundaryEdges(tiles: [number, number][]): number {
    const set = new Set(tiles.map(([x, y]) => `${x},${y}`));
    let count = 0;
    for (const [x, y] of tiles) {
        for (const [dx, dy] of [
            [1, 0],
            [-1, 0],
            [0, 1],
            [0, -1]
        ]) {
            if (!set.has(`${x + dx},${y + dy}`)) count++;
        }
    }
    return count;
}

describe('isResourceName', () => {
    it('accepts anything spelled as an ore', () => {
        expect(isResourceName('iron-ore')).toBe(true);
        expect(isResourceName('copper-ore')).toBe(true);
        expect(isResourceName('uranium-ore')).toBe(true);
        // The suffix rule is what covers a modded ore with no entry anywhere.
        expect(isResourceName('thorium-ore')).toBe(true);
    });

    it('accepts the three base resources that are not spelled as ores', () => {
        expect(isResourceName('coal')).toBe(true);
        expect(isResourceName('stone')).toBe(true);
        expect(isResourceName('crude-oil')).toBe(true);
    });

    it('rejects things that are built, including ones whose names start with a resource', () => {
        expect(isResourceName('stone-furnace')).toBe(false);
        expect(isResourceName('coal-something')).toBe(false);
        expect(isResourceName('transport-belt')).toBe(false);
        expect(isResourceName('')).toBe(false);
    });
});

describe('resourcePatches', () => {
    it('ignores entities that are not resources', () => {
        expect(
            resourcePatches([
                {name: 'stone-furnace', position: {x: 5, y: 35}, direction: 0},
                {name: 'transport-belt', position: {x: 6, y: 35}, direction: 4}
            ])
        ).toEqual([]);
    });

    it('traces a single tile as its four corners, at tile EDGES not centres', () => {
        const [patch] = resourcePatches([ore('coal', 0, 0)]);
        // The entity is at (0.5, 0.5); its tile spans [0,1] x [0,1].
        expect(patch.rings).toEqual([
            [
                {x: 0, y: 0},
                {x: 1, y: 0},
                {x: 1, y: 1},
                {x: 0, y: 1}
            ]
        ]);
        expect(patch.bounds).toEqual({left: 0, top: 0, right: 1, bottom: 1});
        expect(patch.centroid).toEqual({x: 0.5, y: 0.5});
        expect(patch.tileCount).toBe(1);
    });

    it('recovers the tile from a negative half-tile centre without drifting', () => {
        // -40.5 floors to -41, whose tile spans [-41,-40]. Getting this wrong
        // is the half-tile bug CLAUDE.md warns about, one tile off.
        const [patch] = resourcePatches([{name: 'iron-ore', position: {x: -40.5, y: -48.5}, direction: 0}]);
        expect(patch.bounds).toEqual({left: -41, top: -49, right: -40, bottom: -48});
        expect(patch.centroid).toEqual({x: -40.5, y: -48.5});
    });

    it('collapses a rectangle into four corners, not one square per tile', () => {
        const entities: EntitySnapshot[] = [];
        for (let x = 0; x < 4; x++) for (let y = 0; y < 3; y++) entities.push(ore('iron-ore', x, y));
        const [patch] = resourcePatches(entities);
        expect(patch.tileCount).toBe(12);
        expect(patch.rings).toEqual([
            [
                {x: 0, y: 0},
                {x: 4, y: 0},
                {x: 4, y: 3},
                {x: 0, y: 3}
            ]
        ]);
        // The whole point: 12 entities in, one 4-point ring out.
        expect(ringsToPath(patch.rings)).toBe('M0 0L4 0L4 3L0 3Z');
    });

    it('counts a tile once however many entities report it', () => {
        const [patch] = resourcePatches([ore('stone', 2, 2), ore('stone', 2, 2)]);
        expect(patch.tileCount).toBe(1);
    });

    it('keeps corner-touching tiles as separate patches', () => {
        // 4-connected on purpose: two ore fields that graze at a corner are
        // two fields, and a bot walks to one of them.
        const patches = resourcePatches([ore('coal', 0, 0), ore('coal', 1, 1)]);
        expect(patches).toHaveLength(2);
        expect(patches.map((p) => p.tileCount)).toEqual([1, 1]);
    });

    it('separates patches by resource even when their tiles are adjacent', () => {
        const patches = resourcePatches([ore('iron-ore', 0, 0), ore('copper-ore', 1, 0)]);
        expect(patches.map((p) => p.resource)).toEqual(['copper-ore', 'iron-ore']);
    });

    it('winds a hole opposite to its outer ring so nonzero fill leaves it empty', () => {
        const entities: EntitySnapshot[] = [];
        for (let x = 0; x < 3; x++)
            for (let y = 0; y < 3; y++) if (!(x === 1 && y === 1)) entities.push(ore('stone', x, y));
        const [patch] = resourcePatches(entities);
        expect(patch.tileCount).toBe(8);
        expect(patch.rings).toHaveLength(2);
        const areas = patch.rings.map(signedArea);
        // Outer clockwise (positive with y down), hole counter-clockwise.
        expect(areas.filter((a) => a > 0)).toHaveLength(1);
        expect(areas.filter((a) => a < 0)).toHaveLength(1);
        expect(Math.abs(areas[0]) + Math.abs(areas[1])).toBe(10); // 9 outer + 1 hole
        expect(ringsToPath(patch.rings).match(/Z/g)).toHaveLength(2);
    });

    it('walks a pinch without chaining two blobs into one crossing ring', () => {
        // Two tiles meeting at a single corner, joined into ONE patch by an
        // arc around the outside -- so vertex (1,1) has two outgoing boundary
        // edges and the tracer has to pick the one that hugs its own blob.
        const tiles: [number, number][] = [
            [0, 0],
            [1, 1],
            [-1, 0],
            [-1, 1],
            [-1, 2],
            [0, 2],
            [1, 2]
        ];
        const patches = resourcePatches(tiles.map(([x, y]) => ore('coal', x, y)));
        expect(patches).toHaveLength(1);
        expect(patches[0].tileCount).toBe(7);
        // Every boundary edge is accounted for exactly once, which is the
        // invariant a mis-chained ring breaks.
        expect(unitEdges(patches[0].rings)).toBe(boundaryEdges(tiles));
        for (const ring of patches[0].rings) expect(ring.length).toBeGreaterThanOrEqual(4);
    });

    it('orders patches largest first within a resource, resources alphabetically', () => {
        const patches = resourcePatches([
            ore('iron-ore', 10, 10),
            ore('iron-ore', 0, 0),
            ore('iron-ore', 1, 0),
            ore('copper-ore', 20, 20)
        ]);
        expect(patches.map((p) => `${p.resource}:${p.tileCount}`)).toEqual([
            'copper-ore:1',
            'iron-ore:2',
            'iron-ore:1'
        ]);
    });

    it('starts a ring at its top-left corner even when the digit counts differ', () => {
        // Sorting the tile keys as STRINGS puts "10,4" before "4,4", which
        // would start this ring at the top-RIGHT corner and silently reshape
        // every `d` in a diff the moment a patch crosses a power of ten.
        const entities: EntitySnapshot[] = [];
        for (let x = 4; x < 11; x++) entities.push(ore('iron-ore', x, 4));
        const [patch] = resourcePatches(entities);
        expect(patch.rings[0][0]).toEqual({x: 4, y: 4});
        expect(ringsToPath(patch.rings)).toBe('M4 4L11 4L11 5L4 5Z');
    });

    it('breaks a tie between equal-sized patches by position, not by record order', () => {
        const patches = resourcePatches([ore('coal', 30, 0), ore('coal', 0, 9), ore('coal', 0, 3)]);
        expect(patches.map((p) => [p.bounds.left, p.bounds.top])).toEqual([
            [0, 3],
            [0, 9],
            [30, 0]
        ]);
    });

    it('is order-independent: shuffling the records changes nothing', () => {
        const entities = [ore('iron-ore', 0, 0), ore('iron-ore', 1, 0), ore('iron-ore', 1, 1), ore('iron-ore', 0, 1)];
        const forwards = resourcePatches(entities);
        const backwards = resourcePatches([...entities].reverse());
        expect(backwards).toEqual(forwards);
    });

    it('handles an empty map', () => {
        expect(resourcePatches([])).toEqual([]);
        expect(ringsToPath([])).toBe('');
    });
});

describe('ringsToPath', () => {
    it('emits one closed subpath per ring', () => {
        expect(
            ringsToPath([
                [
                    {x: 0, y: 0},
                    {x: 1, y: 0},
                    {x: 1, y: 1}
                ],
                [
                    {x: 5, y: 5},
                    {x: 6, y: 5},
                    {x: 6, y: 6}
                ]
            ])
        ).toBe('M0 0L1 0L1 1ZM5 5L6 5L6 6Z');
    });

    it('drops an empty ring rather than emitting a bare M', () => {
        expect(ringsToPath([[], [{x: 1, y: 2}]])).toBe('M1 2Z');
    });
});
