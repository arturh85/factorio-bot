import {describe, expect, it} from 'vitest';
import {entitiesAt} from './runMap';
import {EntitySnapshot, MapRecord} from '@/api/types';

function snap(name: string, x: number, y: number, direction = 0): EntitySnapshot {
    return {name, position: {x, y}, direction};
}

function placed(tick: number, name: string, x: number, y: number): MapRecord {
    const snapshot = snap(name, x, y);
    return {kind: 'placed', tick, bot: 1, intent: snapshot, actual: snapshot, drift: null};
}

function removed(tick: number, name: string, x: number, y: number): MapRecord {
    return {kind: 'removed', tick, bot: 1, entity: snap(name, x, y)};
}

function keyframe(tick: number, game: EntitySnapshot[]): MapRecord {
    return {
        kind: 'keyframe',
        tick,
        bounds: {left: 0, top: 0, right: 0, bottom: 0},
        game,
        model: [],
        divergence: []
    };
}

describe('entitiesAt', () => {
    it('applies placements up to the cursor', () => {
        const r = [placed(100, 'stone-furnace', -12, 8), placed(200, 'stone-furnace', -10, 8)];
        expect(entitiesAt(r, 150)).toHaveLength(1);
        expect(entitiesAt(r, 250)).toHaveLength(2);
    });

    it('drops an entity removed before the cursor', () => {
        const r = [placed(100, 'stone-furnace', -12, 8), removed(200, 'stone-furnace', -12, 8)];
        expect(entitiesAt(r, 250)).toHaveLength(0);
    });

    it('restarts from the latest keyframe at or before the cursor', () => {
        // The keyframe is the truth; replaying deltas from tick zero would
        // carry forward anything the keyframe corrected.
        const r = [
            placed(100, 'stone-furnace', -12, 8),
            keyframe(300, [{name: 'steel-furnace', position: {x: -12, y: 8}, direction: 0}]),
            placed(400, 'stone-furnace', -8, 8)
        ];
        const at = entitiesAt(r, 450);
        expect(at.map((e) => e.name).sort()).toEqual(['steel-furnace', 'stone-furnace']);
    });

    it('is empty before anything was placed', () => {
        expect(entitiesAt([placed(100, 'stone-furnace', -12, 8)], 50)).toEqual([]);
    });

    it('does not remove an entity of a different type standing on the same tile', () => {
        // Position alone would remove a replaced entity of a different type.
        const r = [
            placed(100, 'stone-furnace', -12, 8),
            removed(200, 'steel-furnace', -12, 8)
        ];
        expect(entitiesAt(r, 250)).toHaveLength(1);
    });

    it('skips a kind this build does not know rather than throwing', () => {
        const r: MapRecord[] = [placed(100, 'stone-furnace', -12, 8), {kind: 'unknown', tick: 150}];
        expect(() => entitiesAt(r, 200)).not.toThrow();
        expect(entitiesAt(r, 200)).toHaveLength(1);
    });
});
