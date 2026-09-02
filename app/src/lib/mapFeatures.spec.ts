/**
 * The tooltip wording is asserted here rather than in the component, because
 * the wording is where this can be wrong in a way a screenshot would not
 * catch: "placed by bot 2 at tick 5911" is a claim about the record, and
 * "no placement recorded" is a different claim about the absence of one.
 */
import {describe, expect, it} from 'vitest';
import {EntitySnapshot, MapRecord} from '@/api/types';
import {colorForEntityType} from './entityColor';
import {BotFeature, EntityFeature, PatchFeature, TrailFeature, buildMapFeatures, legendFor, placementIndex} from './mapFeatures';

function ore(name: string, x: number, y: number): EntitySnapshot {
    return {name, position: {x: x + 0.5, y: y + 0.5}, direction: 0};
}

const furnace: EntitySnapshot = {name: 'stone-furnace', position: {x: 5, y: 35}, direction: 0};

const placedFurnace: MapRecord = {
    tick: 5911,
    kind: 'placed',
    bot: 2,
    intent: furnace,
    actual: furnace,
    drift: null
};

describe('placementIndex', () => {
    it('keys on what the game made, not what was asked for', () => {
        const drifted: MapRecord = {
            tick: 100,
            kind: 'placed',
            bot: 1,
            intent: {name: 'stone-furnace', position: {x: 5, y: 35}, direction: 0},
            actual: {name: 'stone-furnace', position: {x: 6, y: 35}, direction: 0},
            drift: ['position']
        };
        const index = placementIndex([drifted]);
        // The map holds the entity at (6,35): that is the one a shape can be
        // matched against, and the one the tooltip has to find.
        expect(index.get('stone-furnace|6|35')?.bot).toBe(1);
        expect(index.has('stone-furnace|5|35')).toBe(false);
    });

    it('lets the later tick win when a tile is rebuilt', () => {
        const index = placementIndex([
            {tick: 10, kind: 'placed', bot: 1, intent: furnace, actual: furnace, drift: null},
            {tick: 900, kind: 'placed', bot: 3, intent: furnace, actual: furnace, drift: null},
            {tick: 500, kind: 'placed', bot: 4, intent: furnace, actual: furnace, drift: null}
        ]);
        expect(index.get('stone-furnace|5|35')).toMatchObject({bot: 3, tick: 900});
    });

    it('ignores every kind that is not a placement', () => {
        const index = placementIndex([
            {tick: 1, kind: 'keyframe', bounds: {left: 0, top: 0, right: 1, bottom: 1}, game: [furnace], model: [], divergence: []},
            {tick: 2, kind: 'removed', bot: 1, entity: furnace},
            {tick: 3, kind: 'unknown'}
        ]);
        expect(index.size).toBe(0);
    });
});

describe('buildMapFeatures', () => {
    it('turns a field of ore tiles into one patch feature that names its size', () => {
        const entities: EntitySnapshot[] = [];
        for (let x = 0; x < 4; x++) for (let y = 0; y < 3; y++) entities.push(ore('iron-ore', x, y));
        const features = buildMapFeatures({entities, bots: [], trail: {}});
        expect(features).toHaveLength(1);
        const patch = features[0] as PatchFeature;
        expect(patch.kind).toBe('patch');
        expect(patch.title).toBe('iron-ore patch');
        expect(patch.details).toEqual(['12 tiles', 'centre 2, 1.5', 'spans 4 x 3 tiles']);
        expect(patch.path).toBe('M0 0L4 0L4 3L0 3Z');
        expect(patch.anchor).toEqual({x: 2, y: 1.5});
        expect(patch.color).toBe(colorForEntityType('iron-ore'));
    });

    it('attributes a placed entity to the bot and tick that placed it', () => {
        const features = buildMapFeatures({
            entities: [furnace],
            bots: [],
            trail: {},
            records: [placedFurnace]
        });
        const marker = features[0] as EntityFeature;
        expect(marker.kind).toBe('entity');
        expect(marker.title).toBe('stone-furnace');
        expect(marker.details).toEqual(['at 5, 35', 'placed by bot 2 at tick 5911']);
    });

    it('says where a drifted placement was aimed and which fields moved', () => {
        const actual = {name: 'stone-furnace', position: {x: 6.5, y: 35}, direction: 4};
        const features = buildMapFeatures({
            entities: [actual],
            bots: [],
            trail: {},
            records: [{tick: 77, kind: 'placed', bot: 1, intent: furnace, actual, drift: ['position', 'direction']}]
        });
        expect((features[0] as EntityFeature).details).toEqual([
            'at 6.5, 35',
            'placed by bot 1 at tick 77',
            'drifted from 5, 35 (position, direction)'
        ]);
    });

    it('says an entity came from a keyframe rather than pretending a bot placed it', () => {
        // Crash-site wreckage is on every map and was placed by nobody.
        const features = buildMapFeatures({
            entities: [{name: 'crash-site-spaceship', position: {x: -5, y: -6}, direction: 0}],
            bots: [],
            trail: {},
            records: [placedFurnace]
        });
        expect((features[0] as EntityFeature).details).toEqual([
            'at -5, -6',
            'no placement recorded — seen in a keyframe'
        ]);
    });

    it('reads the same without records at all', () => {
        const features = buildMapFeatures({entities: [furnace], bots: [], trail: {}});
        expect((features[0] as EntityFeature).details[1]).toBe('no placement recorded — seen in a keyframe');
    });

    it('rounds a display position to two decimals and leaves the geometry alone', () => {
        const wreck = {name: 'crash-site-spaceship-wreck-big-2', position: {x: -36.40625, y: -0.90234375}, direction: 0};
        const features = buildMapFeatures({entities: [wreck], bots: [], trail: {}});
        const marker = features[0] as EntityFeature;
        expect(marker.details[0]).toBe('at -36.41, -0.9');
        expect(marker.position).toEqual({x: -36.40625, y: -0.90234375});
    });

    it('describes a trail by its endpoints, which is what makes it point at something', () => {
        const features = buildMapFeatures({
            entities: [],
            bots: [],
            trail: {2: [{x: 0, y: 0}, {x: 4, y: 8}, {x: -40.5, y: 12.5}]}
        });
        const trail = features[0] as TrailFeature;
        expect(trail.kind).toBe('trail');
        expect(trail.title).toBe('bot 2 trail');
        expect(trail.details).toEqual(['3 sampled positions', 'from 0, 0', 'to -40.5, 12.5']);
        // Anchored at the far end: that is where the bot is going.
        expect(trail.anchor).toEqual({x: -40.5, y: 12.5});
    });

    it('skips a trail that is a single point, which draws nothing but would take a tab stop', () => {
        const features = buildMapFeatures({entities: [], bots: [], trail: {1: [{x: 0, y: 0}], 2: []}});
        expect(features).toEqual([]);
    });

    it('gives a bot the same colour as its own trail', () => {
        const features = buildMapFeatures({
            entities: [],
            bots: [{id: 3, position: {x: 1, y: 2}}],
            trail: {3: [{x: 0, y: 0}, {x: 1, y: 2}]}
        });
        const trail = features.find((f) => f.kind === 'trail') as TrailFeature;
        const bot = features.find((f) => f.kind === 'bot') as BotFeature;
        expect(bot.color).toBe(trail.color);
        expect(bot.title).toBe('bot 3');
        expect(bot.details).toEqual(['at 1, 2']);
    });

    it('paints patches under entities under trails under bots', () => {
        const features = buildMapFeatures({
            entities: [ore('coal', 0, 0), furnace],
            bots: [{id: 1, position: {x: 0, y: 0}}],
            trail: {1: [{x: 0, y: 0}, {x: 1, y: 1}]}
        });
        expect(features.map((f) => f.kind)).toEqual(['patch', 'entity', 'trail', 'bot']);
    });

    it('orders bots numerically rather than by whatever key order the trail record has', () => {
        const features = buildMapFeatures({
            entities: [],
            bots: [{id: 10, position: {x: 0, y: 0}}, {id: 2, position: {x: 0, y: 0}}],
            trail: {}
        });
        expect(features.map((f) => (f as BotFeature).botId)).toEqual([2, 10]);
    });

    it('gives every feature a distinct id, even for two of the same entity', () => {
        const features = buildMapFeatures({
            entities: [furnace, {...furnace}],
            bots: [],
            trail: {}
        });
        expect(new Set(features.map((f) => f.id)).size).toBe(features.length);
    });
});

describe('legendFor', () => {
    it('counts resource rows in tiles and entity rows in entities', () => {
        const entities: EntitySnapshot[] = [ore('iron-ore', 0, 0), ore('iron-ore', 1, 0), ore('iron-ore', 40, 40), furnace, {...furnace, position: {x: 7, y: 35}}];
        const legend = legendFor(buildMapFeatures({entities, bots: [], trail: {}}));
        expect(legend).toEqual([
            {
                key: 'resource:iron-ore',
                color: colorForEntityType('iron-ore'),
                label: 'iron-ore',
                detail: '3 tiles in 2 patches',
                shape: 'patch'
            },
            {
                key: 'entity:stone-furnace',
                color: colorForEntityType('stone-furnace'),
                label: 'stone-furnace',
                detail: '2 on map',
                shape: 'entity'
            }
        ]);
    });

    it('does not say "1 tiles"', () => {
        const legend = legendFor(buildMapFeatures({entities: [ore('coal', 0, 0), furnace], bots: [], trail: {}}));
        expect(legend[0].detail).toBe('1 tile in 1 patch');
        expect(legend[1].detail).toBe('1 on map');
    });

    it('gives a bot one row whether it appears as a dot, a trail, or both', () => {
        const legend = legendFor(
            buildMapFeatures({
                entities: [],
                bots: [{id: 1, position: {x: 0, y: 0}}],
                trail: {1: [{x: 0, y: 0}, {x: 1, y: 1}], 2: [{x: 0, y: 0}, {x: 2, y: 2}]}
            })
        );
        expect(legend.map((row) => row.key)).toEqual(['bot:1', 'bot:2']);
        expect(legend[0]).toMatchObject({label: 'bot 1', detail: 'position and trail', shape: 'bot'});
    });

    it('orders bots numerically, not as strings', () => {
        const legend = legendFor(
            buildMapFeatures({
                entities: [],
                bots: [{id: 2, position: {x: 0, y: 0}}, {id: 10, position: {x: 0, y: 0}}],
                trail: {}
            })
        );
        expect(legend.map((row) => row.key)).toEqual(['bot:2', 'bot:10']);
    });

    it('is empty for an empty map rather than showing a key to nothing', () => {
        expect(legendFor([])).toEqual([]);
    });
});
