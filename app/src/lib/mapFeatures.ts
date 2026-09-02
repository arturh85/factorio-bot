/**
 * Turning a run's map records into labelled, tooltip-able shapes.
 *
 * The map used to be a canvas of undifferentiated coloured cells: correct,
 * and unreadable, because nothing on it said what any colour meant or what a
 * bot's trail was heading towards. Every shape the map draws now comes out of
 * here already carrying the sentence it should say when you hover, tap or tab
 * to it -- so the wording is asserted in a unit test rather than buried in a
 * template, and the component's only job is geometry.
 *
 * World coordinates throughout. The projection to screen pixels is a single
 * SVG `transform` applied by the component, so nothing here has to know how
 * big the viewport is.
 */
import {EntitySnapshot, MapRecord, Position} from '@/api/types';
import {colorForEntityType} from '@/lib/entityColor';
import {ResourcePatch, isResourceName, resourcePatches, ringsToPath} from '@/lib/resourcePatches';

/** Enough to place a bot dot on top of; a full `BotSample` is not required. */
export interface BotDot {
    id: number;
    position: Position;
}

interface FeatureBase {
    /** Stable across renders: used as the Vue key and as the tooltip's target. */
    id: string;
    color: string;
    /** The tooltip's heading, and the shape's accessible name. */
    title: string;
    /** The tooltip's body, one line each. */
    details: string[];
    /** Where the tooltip points, in world coordinates. */
    anchor: Position;
    /** Which legend row this shape belongs to. */
    legendKey: string;
}

export interface PatchFeature extends FeatureBase {
    kind: 'patch';
    /** An SVG `d` in world coordinates. */
    path: string;
    tileCount: number;
}

export interface EntityFeature extends FeatureBase {
    kind: 'entity';
    position: Position;
}

export interface TrailFeature extends FeatureBase {
    kind: 'trail';
    points: Position[];
    botId: number;
}

export interface BotFeature extends FeatureBase {
    kind: 'bot';
    position: Position;
    botId: number;
}

export type MapFeature = PatchFeature | EntityFeature | TrailFeature | BotFeature;

/** What a `placed` record says about one entity that is on the map now. */
export interface Placement {
    bot: number;
    tick: number;
    /** Field names that differ from what the executor asked for, or null. */
    drift: string[] | null;
    intent: EntitySnapshot;
}

/**
 * Rounds for display only.
 *
 * A tile centre is `-42.5` and must read as `-42.5`, but a bot sampled
 * mid-walk is at `-42.48046875` and reading that back does nobody any good.
 * Two decimals keeps the half-tile and drops the noise; never used for
 * geometry.
 */
function num(value: number): string {
    return String(Math.round(value * 100) / 100);
}

/** `1 tile` / `2 tiles`. A legend that says "1 tiles" reads as a bug in the count. */
function plural(count: number, noun: string): string {
    return `${count} ${noun}${count === 1 ? '' : 's'}`;
}

function positionLabel(position: Position): string {
    return `${num(position.x)}, ${num(position.y)}`;
}

function entityKey(entity: EntitySnapshot): string {
    return `${entity.name}|${entity.position.x}|${entity.position.y}`;
}

/**
 * Which bot placed what, keyed by the entity the game actually created.
 *
 * Keyed on `actual`, not `intent`: `actual` is what is on the map now and
 * therefore what a map shape can be matched against, and the two differ
 * exactly when there is `drift` worth reporting. Later ticks win, so a tile
 * built, mined and rebuilt is attributed to the rebuild.
 *
 * `removed` records are deliberately not consulted: `runMap.entitiesAt` has
 * already applied them, so anything still on the map was not removed.
 */
export function placementIndex(records: MapRecord[]): Map<string, Placement> {
    const index = new Map<string, Placement>();
    for (const record of records) {
        if (record.kind !== 'placed') continue;
        const key = entityKey(record.actual);
        const existing = index.get(key);
        if (existing && existing.tick > record.tick) continue;
        index.set(key, {bot: record.bot, tick: record.tick, drift: record.drift, intent: record.intent});
    }
    return index;
}

function patchFeature(patch: ResourcePatch): PatchFeature {
    const width = patch.bounds.right - patch.bounds.left;
    const height = patch.bounds.bottom - patch.bounds.top;
    return {
        kind: 'patch',
        id: `patch-${patch.resource}-${patch.bounds.left}-${patch.bounds.top}`,
        color: colorForEntityType(patch.resource),
        title: `${patch.resource} patch`,
        details: [
            plural(patch.tileCount, 'tile'),
            `centre ${positionLabel(patch.centroid)}`,
            `spans ${width} x ${height} tiles`
        ],
        anchor: patch.centroid,
        legendKey: `resource:${patch.resource}`,
        path: ringsToPath(patch.rings),
        tileCount: patch.tileCount
    };
}

function entityFeature(entity: EntitySnapshot, index: number, placements: Map<string, Placement>): EntityFeature {
    const details = [`at ${positionLabel(entity.position)}`];
    const placement = placements.get(entityKey(entity));
    if (placement) {
        details.push(`placed by bot ${placement.bot} at tick ${placement.tick}`);
        if (placement.drift && placement.drift.length > 0) {
            // Drift is the interesting half of a `placed` record: the
            // executor asked for one thing and the game made another. Saying
            // WHICH fields drifted and where it was aimed is the difference
            // between a tooltip and a label.
            details.push(`drifted from ${positionLabel(placement.intent.position)} (${placement.drift.join(', ')})`);
        }
    } else {
        // Not an omission worth hiding: an entity with no `placed` record was
        // in a keyframe the game reported, which is how crash-site wreckage
        // and anything built before recording began gets here.
        details.push('no placement recorded — seen in a keyframe');
    }
    return {
        kind: 'entity',
        // Position and name are not unique across a map (two furnaces can be
        // re-placed on one tile across a run), so the index joins the key.
        id: `entity-${index}-${entity.name}`,
        color: colorForEntityType(entity.name),
        title: entity.name,
        details,
        anchor: entity.position,
        legendKey: `entity:${entity.name}`,
        position: entity.position
    };
}

function botColor(botId: number): string {
    return colorForEntityType(`bot-${botId}`);
}

/**
 * Every shape the map should draw, in paint order: patches first so the
 * things built on top of them are legible, then entities, then trails, then
 * the bots themselves.
 *
 * `records` is optional. Without it every entity reads "no placement
 * recorded", which is honest -- it is exactly what the caller knows.
 */
export function buildMapFeatures(input: {
    entities: EntitySnapshot[];
    bots: BotDot[];
    trail: Record<number, Position[]>;
    records?: MapRecord[];
}): MapFeature[] {
    const placements = placementIndex(input.records ?? []);
    const features: MapFeature[] = resourcePatches(input.entities).map(patchFeature);

    input.entities
        .filter((entity) => !isResourceName(entity.name))
        .forEach((entity, index) => features.push(entityFeature(entity, index, placements)));

    // Numeric bot order, not the object-key order `Object.entries` gives --
    // that is insertion order for integer-like keys in practice, but relying
    // on it would make the legend's order an accident.
    const trailBots = Object.keys(input.trail)
        .map(Number)
        .sort((a, b) => a - b);
    for (const botId of trailBots) {
        const points = input.trail[botId];
        // One point is a dot, not a path: a polyline through it draws nothing
        // and would still take a tab stop.
        if (!points || points.length < 2) continue;
        features.push({
            kind: 'trail',
            id: `trail-${botId}`,
            color: botColor(botId),
            title: `bot ${botId} trail`,
            details: [
                `${points.length} sampled positions`,
                `from ${positionLabel(points[0])}`,
                `to ${positionLabel(points[points.length - 1])}`
            ],
            anchor: points[points.length - 1],
            legendKey: `bot:${botId}`,
            points,
            botId
        });
    }

    for (const bot of [...input.bots].sort((a, b) => a.id - b.id)) {
        features.push({
            kind: 'bot',
            id: `bot-${bot.id}`,
            color: botColor(bot.id),
            title: `bot ${bot.id}`,
            details: [`at ${positionLabel(bot.position)}`],
            anchor: bot.position,
            legendKey: `bot:${bot.id}`,
            position: bot.position,
            botId: bot.id
        });
    }

    return features;
}

/** One row of the map's legend. */
export interface LegendEntry {
    key: string;
    /** The swatch's colour, identical to the shapes it stands for. */
    color: string;
    label: string;
    /**
     * What the count MEANS depends on the row, so it is spelled out rather
     * than left as a bare number: resource rows count tiles, entity rows
     * count entities, bot rows count nothing.
     */
    detail: string;
    shape: 'patch' | 'entity' | 'bot';
}

/**
 * The legend for a set of features.
 *
 * Derived from what is actually on the map, never from a fixed table: the
 * palette in `entityColor.ts` is a hash, so a hard-coded legend would be a
 * second source of truth that drifts the first time a new entity appears.
 *
 * Rows are ordered patches, then entities, then bots -- the same order the
 * shapes paint in -- and alphabetically within each group.
 */
export function legendFor(features: MapFeature[]): LegendEntry[] {
    const patches = new Map<string, {color: string; label: string; tiles: number; count: number}>();
    const entities = new Map<string, {color: string; label: string; count: number}>();
    const bots = new Map<string, {color: string; label: string}>();

    for (const feature of features) {
        if (feature.kind === 'patch') {
            const row = patches.get(feature.legendKey);
            if (row) {
                row.tiles += feature.tileCount;
                row.count += 1;
            } else {
                patches.set(feature.legendKey, {
                    color: feature.color,
                    label: feature.title.replace(/ patch$/, ''),
                    tiles: feature.tileCount,
                    count: 1
                });
            }
        } else if (feature.kind === 'entity') {
            const row = entities.get(feature.legendKey);
            if (row) row.count += 1;
            else entities.set(feature.legendKey, {color: feature.color, label: feature.title, count: 1});
        } else {
            // A bot contributes the same legend row whether it appears as a
            // dot, a trail, or both -- one bot, one colour, one row.
            if (!bots.has(feature.legendKey)) {
                bots.set(feature.legendKey, {color: feature.color, label: `bot ${feature.botId}`});
            }
        }
    }

    const rows: LegendEntry[] = [];
    for (const key of [...patches.keys()].sort()) {
        const row = patches.get(key) as {color: string; label: string; tiles: number; count: number};
        const patchWord = row.count === 1 ? 'patch' : 'patches';
        rows.push({
            key,
            color: row.color,
            label: row.label,
            detail: `${plural(row.tiles, 'tile')} in ${row.count} ${patchWord}`,
            shape: 'patch'
        });
    }
    for (const key of [...entities.keys()].sort()) {
        const row = entities.get(key) as {color: string; label: string; count: number};
        rows.push({
            key,
            color: row.color,
            label: row.label,
            // "on map", not "placed": crash-site wreckage and anything built
            // before recording started is on the map without any bot having
            // placed it, and the legend must not claim otherwise.
            detail: row.count === 1 ? '1 on map' : `${row.count} on map`,
            shape: 'entity'
        });
    }
    for (const key of [...bots.keys()].sort((a, b) => Number(a.slice(4)) - Number(b.slice(4)))) {
        const row = bots.get(key) as {color: string; label: string};
        rows.push({key, color: row.color, label: row.label, detail: 'position and trail', shape: 'bot'});
    }
    return rows;
}
