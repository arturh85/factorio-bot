import {defineStore} from 'pinia';
import {findEntities} from '@/api/game';
import {FactorioEntity} from '@/api/types';
import {ApiError} from '@/api/http';

/**
 * "No Factorio instance is running", as `crates/server/src/error.rs` codes
 * it. See `instanceStore.ts`'s `CODE_NOT_RUNNING` for the fuller rationale:
 * this is a `503` from `find-entities`, but the same code is a `400` from
 * other routes, so the branch below is on `code`, never on HTTP status.
 */
const CODE_NOT_RUNNING = 2;

export interface MapQuery {
    x: number;
    y: number;
    radius: number;
    /** Empty string means "no filter", not "match the empty string". */
    entityType: string;
}

/**
 * What the map page has to show, as one discriminated value rather than a
 * bag of independent booleans -- so a view can `switch` on `status` and the
 * compiler proves every case is handled, instead of the page having to get
 * `loading && !error && entities !== null` right by hand.
 *
 * `not-queried`, `not-running` and `ready` with an empty `entities` array are
 * three different states on purpose (see `.superpowers/sdd/map-view/brief.md`
 * rule 4): "nothing to show" means something different in each, and the
 * user's next action differs too -- start the instance, run a query, or
 * accept that the area really is empty.
 */
export type MapResult =
    | {status: 'not-queried'}
    | {status: 'loading'; query: MapQuery}
    | {status: 'not-running'; query: MapQuery}
    | {status: 'error'; query: MapQuery; message: string}
    | {status: 'ready'; query: MapQuery; entities: FactorioEntity[]; fetchedAtMs: number};

function messageOf(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
}

/**
 * Entities found by `GET /api/v1/game/find-entities`, over HTTP.
 *
 * This is a SNAPSHOT store, not a live one: `find-entities` answers for the
 * instant it was called and nothing pushes updates afterwards, so every
 * `ready` result carries the `fetchedAtMs` it was current as of, and getting
 * a fresh one means calling `query`/`refresh` again -- there is deliberately
 * no polling here. A map that looked live while quietly going stale would be
 * the same kind of lie this codebase has spent a long time removing from its
 * status displays.
 */
export const useMapStore = defineStore('map', {
    state: () => ({
        result: {status: 'not-queried'} as MapResult
    }),
    getters: {
        current(): MapResult {
            return this.result;
        }
    },
    actions: {
        async query(query: MapQuery): Promise<void> {
            this.result = {status: 'loading', query};
            try {
                const entities = await findEntities({
                    position: {x: query.x, y: query.y},
                    radius: query.radius,
                    entity_type: query.entityType === '' ? undefined : query.entityType
                });
                this.result = {status: 'ready', query, entities, fetchedAtMs: Date.now()};
            } catch (err) {
                if (err instanceof ApiError && err.code === CODE_NOT_RUNNING) {
                    this.result = {status: 'not-running', query};
                    return;
                }
                this.result = {status: 'error', query, message: messageOf(err)};
            }
        },

        /** Re-runs the last query, unchanged. A no-op before the first `query()`. */
        async refresh(): Promise<void> {
            if (this.result.status === 'not-queried') {
                return;
            }
            await this.query(this.result.query);
        }
    }
});
