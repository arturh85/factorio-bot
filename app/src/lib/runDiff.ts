/**
 * Comparing a run's plan against what actually happened.
 *
 * A plan is computed once, before dispatch, and can diverge from the run in
 * either direction: a step can take longer (or shorter) than planned, never
 * get dispatched at all, or the run can do something -- a recovery action --
 * that no plan ever mentioned. Every function here is pure, for the same
 * reason `runTimeline.ts` and `runSamples.ts` are: this is the part of the
 * analysis view that can be wrong in a way you would not notice by looking.
 */
import {
    ActionFailure,
    BotSample,
    Divergence,
    Event,
    MapRecord,
    PlannedStep,
    Sample,
    SatisfiedReason
} from '@/api/types';

type Dispatched = Extract<Event, {kind: 'action_dispatched'}>;
type Settled = Extract<Event, {kind: 'action_settled'}>;

/** One row of the overrun table: a planned step joined to its outcome. */
export interface OutcomeRow {
    id: number;
    bot: number;
    action: string;
    plannedDuration: number | null;
    actualDuration: number | null;
    /** `actualDuration - plannedDuration`, or `null` when either is missing. */
    delta: number | null;
    status: string;
    failure: ActionFailure | null;
}

/**
 * A full outer join of a plan against a run's settle events, keyed on `id`.
 *
 * **Every planned step and every settle appears exactly once.** A planned
 * step with no matching settle keeps a `null actualDuration` and status
 * `'never dispatched'` (or `'never settled'` when it *was* dispatched but the
 * game never reported back) -- that row is the most interesting one on the
 * page, and dropping it because it has no settle would hide the failure. A
 * settle with no planned step -- a recovery action, dispatched outside the
 * plan by the executor's recovery tiers -- keeps a `null plannedDuration`
 * rather than being discarded, because a join that only walks the plan loses
 * exactly the actions worth scrutinising.
 *
 * `action_dispatched` events are not joined on directly -- an id dispatched
 * but never settled contributes no row of its own -- but are consulted for
 * the action's label when a settle has no planned step to name it.
 */
export function joinPlanToOutcome(plan: PlannedStep[], events: Event[]): OutcomeRow[] {
    const dispatchedById = new Map<number, Dispatched>();
    const settledById = new Map<number, Settled>();
    for (const event of events) {
        if (event.kind === 'action_dispatched') dispatchedById.set(event.id, event);
        else if (event.kind === 'action_settled') settledById.set(event.id, event);
    }

    const planById = new Map(plan.map((step) => [step.id, step]));
    const ids = new Set<number>([...planById.keys(), ...settledById.keys()]);

    const rows: OutcomeRow[] = [...ids].map((id) => {
        const planned = planById.get(id) ?? null;
        const settle = settledById.get(id) ?? null;
        const dispatch = dispatchedById.get(id) ?? null;
        const plannedDuration = planned?.planned_duration ?? null;
        const actualDuration = settle?.elapsed_ticks ?? null;
        const delta =
            plannedDuration !== null && actualDuration !== null ? actualDuration - plannedDuration : null;
        // `ids` is the union of the plan's ids and the settled ids, so
        // whichever of `planned`/`settle` is null, the other is not -- there
        // is no third source of `bot` to fall back to.
        return {
            id,
            bot: planned !== null ? planned.bot : settle!.bot,
            action: planned?.action ?? dispatch?.action ?? '(unrecorded)',
            plannedDuration,
            actualDuration,
            delta,
            status: settle?.status ?? (dispatch !== null ? 'never settled' : 'never dispatched'),
            failure: settle?.failure ?? null
        };
    });

    // Worst overrun first; a step that never ran has no delta at all and
    // must not sort as if it were exactly on time.
    return rows.sort((a, b) => {
        if (a.delta === null && b.delta === null) return 0;
        if (a.delta === null) return 1;
        if (b.delta === null) return -1;
        return b.delta - a.delta;
    });
}

/** One keyframe's divergence, with the tick it was observed at. */
export interface DivergenceRow {
    tick: number;
    divergence: Divergence;
}

/**
 * Every divergence any keyframe recorded, tick-ascending.
 *
 * Flattened out of the keyframes rather than grouped by one, because the
 * page lists divergences as rows a reader jumps to individually -- each row
 * seeks the map panel to its own tick, not to "the keyframe" as a unit.
 */
export function divergencesOf(map: MapRecord[]): DivergenceRow[] {
    const rows: DivergenceRow[] = [];
    for (const record of map) {
        if (record.kind !== 'keyframe') continue;
        for (const divergence of record.divergence) {
            rows.push({tick: record.tick, divergence});
        }
    }
    return rows.sort((a, b) => a.tick - b.tick);
}

/** One milestone's plan, and how it closed. */
export interface MilestoneRow {
    index: number;
    goal: string;
    /**
     * The steps the last plan produced for this milestone -- "last" because a
     * milestone that gets stuck can be replanned, and the latest attempt is
     * the one worth drawing beside what ran. Empty when no `plan_created`
     * event named this milestone.
     */
    plan: PlannedStep[];
    /**
     * Why the milestone needed no work, or `null` while it is still open or
     * it closed with `milestone_stuck` instead. Present specifically to
     * distinguish "already true" from "the planner returned nothing", which
     * a supervisor's "empty plan means satisfied" rule would otherwise
     * conflate.
     */
    satisfiedReason: SatisfiedReason | null;
    /** How many planning iterations it took to close, or `null` if still open. */
    iterations: number | null;
}

/**
 * One row per milestone, its plan and its closing reason, index-ascending.
 *
 * A `plan_created` or `milestone_satisfied` event naming a milestone index
 * with no matching `milestone_started` is skipped rather than fabricating a
 * row for it -- that combination has never been observed and inventing a
 * goal string for it would be a guess dressed up as data.
 */
export function milestonesOf(events: Event[]): MilestoneRow[] {
    const rows = new Map<number, MilestoneRow>();
    for (const event of events) {
        if (event.kind === 'milestone_started') {
            rows.set(event.index, {
                index: event.index,
                goal: event.goal,
                plan: [],
                satisfiedReason: null,
                iterations: null
            });
        } else if (event.kind === 'plan_created') {
            const row = rows.get(event.milestone_index);
            if (row) row.plan = event.plan;
        } else if (event.kind === 'milestone_satisfied') {
            const row = rows.get(event.index);
            if (row) {
                row.satisfiedReason = event.reason;
                row.iterations = event.iterations;
            }
        }
    }
    return [...rows.values()].sort((a, b) => a.index - b.index);
}

/** A failed action, and the bot sample nearest its tick. */
export interface FailureInventory {
    event: Settled;
    /**
     * What the bot was carrying at the sample closest to the failure's tick,
     * or `null` when the run recorded no `bots` sample naming that bot at
     * all -- distinct from a sample that names it with an empty inventory.
     */
    inventory: BotSample | null;
}

/**
 * Every failed action, paired with the nearest recorded inventory for the
 * bot that ran it.
 *
 * "Nearest" rather than "at or before": a sample taken shortly *after* the
 * failure can be the closer, more informative one -- unlike a lane, which
 * must not show a bot as idle before an unterminated action's start, an
 * inventory reading has no direction to be wrong in, so the closer sample in
 * either direction is simply the better answer.
 */
export function inventoryAtFailure(events: Event[], samples: Sample[]): FailureInventory[] {
    const failed = events.filter(
        (event): event is Settled => event.kind === 'action_settled' && event.failure !== null
    );
    return failed.map((event) => ({event, inventory: nearestInventory(samples, event.bot, event.tick)}));
}

function nearestInventory(samples: Sample[], bot: number, tick: number): BotSample | null {
    let best: BotSample | null = null;
    let bestDistance = Infinity;
    for (const sample of samples) {
        if (sample.kind !== 'bots') continue;
        const found = sample.bots.find((b) => b.id === bot);
        if (found === undefined) continue;
        const distance = Math.abs(sample.tick - tick);
        if (distance < bestDistance) {
            bestDistance = distance;
            best = found;
        }
    }
    return best;
}
