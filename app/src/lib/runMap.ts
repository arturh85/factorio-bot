/**
 * Reconstructing what the map looked like at a tick.
 *
 * Pure, and kept out of the components for the same reason `runSamples.ts`
 * is: this is the part that can be wrong in a way you would not notice by
 * looking at the screen.
 */
import {Bounds, EntitySnapshot, MapRecord} from '@/api/types';

/**
 * Whether two snapshots name the same entity for removal purposes.
 *
 * Position alone is not enough: a stone furnace mined and replaced by a
 * steel furnace on the same tile is two different entities that happen to
 * share a position, and matching on position alone would let a `removed`
 * record for the old one delete the new one instead.
 */
function samePlace(a: EntitySnapshot, b: EntitySnapshot): boolean {
    return a.name === b.name && a.position.x === b.position.x && a.position.y === b.position.y;
}

/**
 * The entities on the map at `tick`, reconstructed from `map.jsonl`.
 *
 * Starts from the latest `keyframe` at or before `tick` -- its `game` array,
 * which is what the game actually reported, not what our model believed --
 * or from nothing when there is none. Replaying every delta from tick zero
 * instead would carry forward exactly the entities the keyframe exists to
 * correct, so the restart is load-bearing, not an optimisation.
 *
 * Every `placed` and `removed` record strictly after that keyframe and at or
 * before `tick` is then applied **in tick order**, not array order: the
 * producer (`record.actions` in `crates/scripting_lua/src/globals/record.rs`)
 * writes each plan step's line at whatever tick that step's own observation
 * carries, in plan/topological order, and `RunRecorder::record` deliberately
 * never reorders or clamps a tick to enforce monotonicity -- concurrent
 * multi-bot execution is this project's flagship case, and its records land
 * out of order routinely. A `MapKind` this build does not know
 * (`kind: 'unknown'`) is skipped rather than throwing, matching the record
 * format's forward-compatibility rule.
 */
export function entitiesAt(records: MapRecord[], tick: number): EntitySnapshot[] {
    let keyframeTick = -Infinity;
    let entities: EntitySnapshot[] = [];
    for (const record of records) {
        if (record.kind === 'keyframe' && record.tick <= tick && record.tick > keyframeTick) {
            keyframeTick = record.tick;
            entities = [...record.game];
        }
    }

    // A copy, sorted stably by tick: mutating the caller's array would be a
    // surprise, and the store holds these records.
    const ordered = [...records].sort((a, b) => a.tick - b.tick);
    for (const record of ordered) {
        if (record.tick <= keyframeTick || record.tick > tick) continue;
        if (record.kind === 'placed') {
            entities = [...entities, record.actual];
        } else if (record.kind === 'removed') {
            const index = entities.findIndex((entity) => samePlace(entity, record.entity));
            if (index !== -1) {
                entities = [...entities.slice(0, index), ...entities.slice(index + 1)];
            }
        }
        // 'keyframe' here is a later keyframe than the one already applied as
        // the start, at or before `tick` -- but only the LATEST one was
        // chosen as the start above, so any other keyframe in this range is
        // superseded and contributes nothing further. 'unknown' is skipped.
    }
    return entities;
}

/**
 * The bounds of the latest `keyframe` at or before `tick`, or `null` before
 * the first one.
 *
 * `entitiesAt` reports "nothing built yet" the same way it reports "nothing
 * built in an area we never looked at" -- an empty array either way. A map
 * panel sizing a canvas to the world needs to tell those apart, because a
 * canvas drawn to bounds that do not exist yet is indistinguishable on
 * screen from an empty one that does.
 */
export function boundsAt(records: MapRecord[], tick: number): Bounds | null {
    let keyframeTick = -Infinity;
    let bounds: Bounds | null = null;
    for (const record of records) {
        if (record.kind === 'keyframe' && record.tick <= tick && record.tick > keyframeTick) {
            keyframeTick = record.tick;
            bounds = record.bounds;
        }
    }
    return bounds;
}
