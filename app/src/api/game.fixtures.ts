/**
 * Fixtures for the map view's tests -- the *tracked*, real captures, not
 * hand-written data. See `crates/core/tests/README.md` for how and when each
 * was recorded.
 *
 * Run through `parseFactorioEntities` here, not assigned to `FactorioEntity[]`
 * with a type annotation or an `as` cast, for the same reason
 * `replay.fixtures.ts` runs its documents through `parseReplay`: a JSON
 * import's fields widen to plain `string`/`number`, so a cast would compile
 * without checking anything. This call succeeding IS the check that this
 * module's declared shape has not drifted from a real server capture.
 */

import spawnData from '../../../crates/core/tests/live-2.1.17-entities-spawn.json';
import resourcesData from '../../../crates/core/tests/live-2.1.17-entities-resources.json';
import {parseFactorioEntities} from './game';
import {FactorioEntity} from './types';

/**
 * Seven entities from a fresh spawn area: `crash-site-spaceship` and three
 * wrecks, the starting `character`, a `stone-furnace` (whose inventories are
 * the empty-object capture `parseFactorioEntities` has to tolerate) and an
 * `inserter` (the only entity type that carries `pickup_position`).
 *
 * `crash-site-spaceship`'s `bounding_box` is the orientation fixture: real
 * Factorio data, not invented, with `left_top.y = -9.296875` numerically
 * SMALLER than `right_bottom.y = -1.5`. See `Rect` in `app/src/api/types.ts`.
 */
export const SPAWN_ENTITIES: FactorioEntity[] = parseFactorioEntities(spawnData);

/** Forty resource/rock entities from a separate area: `iron-ore` and `big-rock`. */
export const RESOURCE_ENTITIES: FactorioEntity[] = parseFactorioEntities(resourcesData);
