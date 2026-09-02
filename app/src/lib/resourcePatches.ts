/**
 * Aggregating a run's raw resource tiles into patches.
 *
 * A recorded map is overwhelmingly ore. Across the 18 runs in `workspace/runs`
 * the worst case holds 1141 entities of which 1122 are single resource tiles;
 * drawing one shape per tile is what made the map an undifferentiated blob and
 * what made an SVG rendering look expensive. Those 1122 tiles form **3**
 * connected patches. Aggregating first is therefore not a rendering
 * optimisation bolted onto a readability change -- it is the same change: a
 * patch is the thing a bot's movement trail actually goes *to*, and it is the
 * thing a tooltip can name ("iron-ore patch, 510 tiles").
 *
 * Pure and deterministic, and kept out of the component for the usual reason:
 * a wrong outline looks like a plausible blob on screen, so this is the part
 * that has to be asserted on numbers rather than eyeballed.
 */
import {Bounds, EntitySnapshot, Position} from '@/api/types';

/**
 * Resource names that are not spelled `<something>-ore`.
 *
 * The `-ore` suffix covers `iron-ore`, `copper-ore`, `uranium-ore` and any
 * modded ore that follows the convention; these three do not follow it. Base
 * Factorio has exactly these six resources, and `crates/core/src/draw.rs`
 * lists the same six -- the NAMES are a fact about the game and are shared,
 * the COLOURS are not (see `entityColor.ts`, which stays independent of that
 * file on purpose).
 *
 * `EntitySnapshot` carries only a name -- no `entity_type` -- so a name test
 * is the only signal available here. Getting it wrong is not silent: a
 * resource this misses renders as an individual entity marker with its own
 * legend row, which is visible, not invisible.
 */
const NON_ORE_RESOURCES = new Set(['coal', 'stone', 'crude-oil']);

/** Whether an entity name is a mineable resource rather than something built. */
export function isResourceName(name: string): boolean {
    return name.endsWith('-ore') || NON_ORE_RESOURCES.has(name);
}

/** One contiguous run of same-resource tiles. */
export interface ResourcePatch {
    /** The resource entity name, e.g. `iron-ore`. */
    resource: string;
    /**
     * Distinct TILES covered, not entities seen. Two records for the same
     * tile are one tile: a count that double-counted them would be reported
     * in a tooltip as fact.
     */
    tileCount: number;
    /** The patch's extent in world coordinates, at tile EDGES not centres. */
    bounds: Bounds;
    /** Centre of mass of the tile centres. The tooltip's anchor. */
    centroid: Position;
    /**
     * Closed rings tracing the patch's outline in world coordinates, outer
     * rings clockwise and holes counter-clockwise so `fill-rule="nonzero"`
     * leaves the holes empty. First point is not repeated at the end; a `Z`
     * closes each ring.
     */
    rings: Position[][];
}

/** Integer tile coordinates, as a map key. */
function tileKey(x: number, y: number): string {
    return `${x},${y}`;
}

/**
 * Row-major order, by NUMBER.
 *
 * `Array.sort()` on these keys sorts them as strings, which puts `"10,4"`
 * before `"4,4"` -- deterministic, but it makes a ring start at whichever
 * vertex happens to have the fewest digits, so a patch's `d` changes shape in
 * a diff when the map moves across a power of ten. Sorting numerically starts
 * every ring at its top-left corner instead.
 */
function compareKeys(a: string, b: string): number {
    const left = parseKey(a);
    const right = parseKey(b);
    return left.y - right.y || left.x - right.x;
}

function parseKey(key: string): {x: number; y: number} {
    const comma = key.indexOf(',');
    return {x: Number(key.slice(0, comma)), y: Number(key.slice(comma + 1))};
}

/**
 * The tile a world position sits on.
 *
 * Resource positions are tile CENTRES -- every real ore entity is at
 * `-40.5`, never `-41` -- so flooring is what recovers the tile, and the
 * tile's own extent is then `[x, x+1] x [y, y+1]`. See CLAUDE.md's note on
 * this: reading a position back out of a floored map without restoring the
 * half-tile offset is a bug this project has already paid for once.
 */
function tileOf(position: Position): {x: number; y: number} {
    return {x: Math.floor(position.x), y: Math.floor(position.y)};
}

/** Rotates a unit direction 90 degrees clockwise ON SCREEN, where y grows downward. */
function turnRight(d: Position): Position {
    return {x: -d.y, y: d.x};
}

function turnLeft(d: Position): Position {
    return {x: d.y, y: -d.x};
}

interface BoundaryEdge {
    dir: Position;
    to: string;
}

/**
 * Traces the boundary of a tile set as closed rings.
 *
 * Each tile contributes one directed edge per side that has no neighbour,
 * oriented so the filled area is always to the RIGHT of travel. That makes
 * outer rings clockwise and hole rings counter-clockwise without a separate
 * winding pass.
 *
 * The only interesting case is a "pinch": two tiles meeting at a single
 * corner, where one vertex has two outgoing edges and picking the wrong one
 * chains two blobs into a crossing ring. Preferring a right turn keeps the
 * walk hugging the blob it arrived on, which yields two clean rings instead
 * of one self-intersecting one.
 */
function traceRings(tiles: Set<string>): Position[][] {
    const outgoing = new Map<string, BoundaryEdge[]>();
    const push = (from: Position, dir: Position): void => {
        const key = tileKey(from.x, from.y);
        const edge: BoundaryEdge = {dir, to: tileKey(from.x + dir.x, from.y + dir.y)};
        const existing = outgoing.get(key);
        if (existing) existing.push(edge);
        else outgoing.set(key, [edge]);
    };

    // Sorted so the edge lists -- and therefore the rings -- come out the same
    // on every run. A tooltip that renamed its patch between renders would be
    // worse than no tooltip.
    for (const key of [...tiles].sort(compareKeys)) {
        const {x, y} = parseKey(key);
        if (!tiles.has(tileKey(x, y - 1))) push({x, y}, {x: 1, y: 0});
        if (!tiles.has(tileKey(x + 1, y))) push({x: x + 1, y}, {x: 0, y: 1});
        if (!tiles.has(tileKey(x, y + 1))) push({x: x + 1, y: y + 1}, {x: -1, y: 0});
        if (!tiles.has(tileKey(x - 1, y))) push({x, y: y + 1}, {x: 0, y: -1});
    }

    const starts = [...outgoing.keys()].sort(compareKeys);
    const rings: Position[][] = [];
    for (const start of starts) {
        while ((outgoing.get(start)?.length ?? 0) > 0) {
            const ring: Position[] = [];
            let vertex = start;
            let heading: Position | null = null;
            do {
                const edges = outgoing.get(vertex);
                // Every vertex has as many outgoing edges as incoming ones,
                // so a walk can only run dry back at its own start.
                if (!edges || edges.length === 0) break;
                let index = 0;
                if (heading) {
                    const preferred = [turnRight(heading), heading, turnLeft(heading)];
                    for (const want of preferred) {
                        const found = edges.findIndex((e) => e.dir.x === want.x && e.dir.y === want.y);
                        if (found !== -1) {
                            index = found;
                            break;
                        }
                    }
                }
                const [edge] = edges.splice(index, 1);
                ring.push(parseKey(vertex));
                heading = edge.dir;
                vertex = edge.to;
            } while (vertex !== start);
            rings.push(dropCollinear(ring));
        }
    }
    return rings;
}

/**
 * Drops the vertices where the outline does not actually turn.
 *
 * A 510-tile patch traces 110 unit edges but only turns at a few dozen of
 * them; keeping the rest would make the `d` attribute five times longer for
 * an identical shape.
 */
function dropCollinear(ring: Position[]): Position[] {
    if (ring.length < 3) return ring;
    const kept: Position[] = [];
    for (let i = 0; i < ring.length; i++) {
        const previous = ring[(i - 1 + ring.length) % ring.length];
        const current = ring[i];
        const next = ring[(i + 1) % ring.length];
        const inX = current.x - previous.x;
        const inY = current.y - previous.y;
        const outX = next.x - current.x;
        const outY = next.y - current.y;
        if (inX !== outX || inY !== outY) kept.push(current);
    }
    return kept;
}

/**
 * Every contiguous same-resource patch in `entities`.
 *
 * Tiles are 4-connected: two patches touching only at a corner stay two
 * patches, which is what the game's own resource generation produces and what
 * a player would call them.
 *
 * Ordered largest-first within a resource, resources alphabetically, so the
 * legend and the render order do not depend on record order.
 */
export function resourcePatches(entities: EntitySnapshot[]): ResourcePatch[] {
    const byResource = new Map<string, Set<string>>();
    for (const entity of entities) {
        if (!isResourceName(entity.name)) continue;
        const tile = tileOf(entity.position);
        const tiles = byResource.get(entity.name);
        if (tiles) tiles.add(tileKey(tile.x, tile.y));
        else byResource.set(entity.name, new Set([tileKey(tile.x, tile.y)]));
    }

    const patches: ResourcePatch[] = [];
    for (const resource of [...byResource.keys()].sort()) {
        const all = byResource.get(resource) as Set<string>;
        const seen = new Set<string>();
        const found: ResourcePatch[] = [];
        for (const key of [...all].sort(compareKeys)) {
            if (seen.has(key)) continue;
            const component = new Set<string>([key]);
            seen.add(key);
            const stack = [key];
            while (stack.length > 0) {
                const {x, y} = parseKey(stack.pop() as string);
                for (const [dx, dy] of [
                    [1, 0],
                    [-1, 0],
                    [0, 1],
                    [0, -1]
                ]) {
                    const neighbour = tileKey(x + dx, y + dy);
                    if (all.has(neighbour) && !seen.has(neighbour)) {
                        seen.add(neighbour);
                        component.add(neighbour);
                        stack.push(neighbour);
                    }
                }
            }
            found.push(describe(resource, component));
        }
        found.sort((a, b) => b.tileCount - a.tileCount || a.bounds.left - b.bounds.left || a.bounds.top - b.bounds.top);
        patches.push(...found);
    }
    return patches;
}

function describe(resource: string, component: Set<string>): ResourcePatch {
    let left = Infinity;
    let top = Infinity;
    let right = -Infinity;
    let bottom = -Infinity;
    let sumX = 0;
    let sumY = 0;
    for (const key of component) {
        const {x, y} = parseKey(key);
        left = Math.min(left, x);
        top = Math.min(top, y);
        right = Math.max(right, x + 1);
        bottom = Math.max(bottom, y + 1);
        sumX += x + 0.5;
        sumY += y + 0.5;
    }
    const tileCount = component.size;
    return {
        resource,
        tileCount,
        bounds: {left, top, right, bottom},
        centroid: {x: sumX / tileCount, y: sumY / tileCount},
        rings: traceRings(component)
    };
}

/**
 * An SVG `d` for a patch's rings, in world coordinates.
 *
 * The caller is expected to place this under a single `transform` rather than
 * pre-multiplying every point -- that keeps the numbers here identical to the
 * game's own coordinates, which is what makes a test on this string legible.
 */
export function ringsToPath(rings: Position[][]): string {
    return rings
        .filter((ring) => ring.length > 0)
        .map((ring) => {
            const [head, ...rest] = ring;
            const moves = rest.map((point) => `L${point.x} ${point.y}`).join('');
            return `M${head.x} ${head.y}${moves}Z`;
        })
        .join('');
}
