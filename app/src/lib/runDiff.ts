/**
 * Comparing a run's plan against what actually happened.
 *
 * A plan is computed once, before dispatch, and can diverge from the run in
 * either direction: a step can take longer (or shorter) than planned, never
 * get dispatched at all, or the run can do something -- a recovery action --
 * that no plan ever mentioned. Every function here is pure, for the same
 * reason `runTimeline.ts` and `runSamples.ts` are: this is the part of the
 * analysis view that can be wrong in a way you would not notice by looking.
 *
 * **Action ids are not unique across a run.** They restart at zero with
 * every `plan_created` -- a real recorded run shows `[0,1,2,3, 0,1,2,3,
 * 0,1,2,3, 0,1,2,3, 0,1,2,3]` for five planning rounds over four milestones.
 * An id only means anything *within* the plan that assigned it, which is why
 * `joinPlanToOutcome` stays scoped to one plan and one slice of the event
 * log, and a whole-run join goes through `planEpochs` first to find the
 * right slice for each id before joining it.
 */
import {
    ActionFailure,
    BotSample,
    Divergence,
    Event,
    MapRecord,
    PlannedStep,
    Position,
    Sample,
    SatisfiedReason,
    WalkFailure
} from '@/api/types';

type Dispatched = Extract<Event, {kind: 'action_dispatched'}>;
type Settled = Extract<Event, {kind: 'action_settled'}>;

/** One row of the overrun table: a planned step joined to its outcome. */
export interface OutcomeRow {
    id: number;
    /**
     * Which milestone's plan this row's `id` was scoped to, or `null` for a
     * settle recorded before any `plan_created` -- a recovery action, or a
     * run recorded before `plan_created` existed. Carried through so two
     * rows that happen to share an id and an action name (routine, since ids
     * restart every plan) are still distinguishable on the page.
     */
    milestoneIndex: number | null;
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
 * **Only correct within a single plan epoch.** `id` is unique within the
 * `plan` and `events` passed in, never across a whole run -- see the module
 * doc comment. Calling this with the plan and event log of more than one
 * `plan_created` mixed together *will* pair a row with the wrong plan or the
 * wrong settle, silently, because two unrelated actions sharing an id is the
 * common case, not an edge case. Use `joinRunOutcome` for a whole run; this
 * function's job is to be correct on the one slice it is given, not to find
 * that slice itself.
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
 *
 * `milestoneIndex` is stamped onto every row this call produces; pass the
 * epoch's own index (or `null` for the pre-plan epoch) so the caller does
 * not have to re-attach it afterward.
 */
export function joinPlanToOutcome(
    plan: PlannedStep[],
    events: Event[],
    milestoneIndex: number | null = null
): OutcomeRow[] {
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
            milestoneIndex,
            bot: planned !== null ? planned.bot : settle!.bot,
            action: planned?.action ?? dispatch?.action ?? '(unrecorded)',
            plannedDuration,
            actualDuration,
            delta,
            status: settle?.status ?? (dispatch !== null ? 'never settled' : 'never dispatched'),
            failure: settle?.failure ?? null
        };
    });

    return sortByDelta(rows);
}

/**
 * Worst overrun first; a step that never ran has no delta at all and must
 * not sort as if it were exactly on time.
 *
 * Generic over anything carrying a `delta`, because walks are sorted by the
 * same rule and a second copy of it would be a second place for "null sorts
 * last" to stop being true.
 */
function sortByDelta<T extends {delta: number | null}>(rows: T[]): T[] {
    return [...rows].sort((a, b) => {
        if (a.delta === null && b.delta === null) return 0;
        if (a.delta === null) return 1;
        if (b.delta === null) return -1;
        return b.delta - a.delta;
    });
}

/**
 * One stretch of the event log belonging to a single plan -- or, for
 * `milestoneIndex: null`, the stretch before any plan existed.
 */
export interface PlanEpoch {
    /** The `plan_created` that opened this epoch's `milestone_index`, or `null` for the pre-plan epoch. */
    milestoneIndex: number | null;
    /** The plan that `plan_created` carried, or `[]` for the pre-plan epoch. */
    plan: PlannedStep[];
    /** Every event at or after this epoch's `plan_created` and before the next one (or before the first one, for the pre-plan epoch). */
    events: Event[];
}

/**
 * Splits a run's event log into plan epochs, in log order.
 *
 * `events.jsonl` is append-only and the route serves it in file order, so a
 * settle belongs to the most recent `plan_created` that precedes it in the
 * log -- there is no other signal linking a settle to a plan, since ids
 * restart at zero with every `plan_created`. Each `plan_created` closes the
 * previous epoch and opens a new one; every other event kind is carried
 * along inside whichever epoch it falls in, unfiltered, because it is
 * `joinPlanToOutcome` that decides which kinds it cares about, not this
 * function.
 *
 * The first epoch is always emitted, even when it is empty: a settle
 * recorded before any `plan_created` -- a recovery action, or *every* run on
 * disk before `plan_created` existed -- must still surface somewhere, with
 * `milestoneIndex: null` rather than being silently dropped for lack of a
 * plan to belong to.
 */
export function planEpochs(events: Event[]): PlanEpoch[] {
    const epochs: PlanEpoch[] = [];
    let current: PlanEpoch = {milestoneIndex: null, plan: [], events: []};
    for (const event of events) {
        if (event.kind === 'plan_created') {
            epochs.push(current);
            current = {milestoneIndex: event.milestone_index, plan: event.plan, events: []};
        } else {
            current.events.push(event);
        }
    }
    epochs.push(current);
    return epochs;
}

/**
 * The overrun table for a whole run: every plan epoch joined on its own
 * terms, concatenated, and re-sorted by overrun.
 *
 * This is the function a page should call for "the run's outcome" --
 * `joinPlanToOutcome` on its own is only correct for one epoch, and a run
 * routinely has several. Re-sorting after concatenating is required, not
 * cosmetic: each epoch's rows arrive already sorted internally, but
 * concatenating several individually-sorted lists does not produce one
 * sorted list.
 */
export function joinRunOutcome(events: Event[]): OutcomeRow[] {
    const rows = planEpochs(events).flatMap((epoch) =>
        joinPlanToOutcome(epoch.plan, epoch.events, epoch.milestoneIndex)
    );
    return sortByDelta(rows);
}


/**
 * One walk of a run, joined from its dispatch and its settle.
 *
 * A separate row type from [`OutcomeRow`] rather than a variant of it,
 * because a walk is keyed by something else entirely: it has no action id,
 * and `(bot, stepIndex)` -- the walk's index within *that bot's* slice of the
 * schedule -- is the only thing that names it. Sharing the row would put two
 * different id spaces in one `id` column, which is how a table comes to
 * suggest a join that does not exist.
 */
export interface WalkRow {
    /** Which plan's epoch this walk belongs to, or `null` before any plan. */
    milestoneIndex: number | null;
    bot: number;
    /** The walk's index in this bot's own slice of the schedule. Not an action id. */
    stepIndex: number;
    /**
     * Where the *schedule* sent the bot -- an intent, never an arrival, and
     * routinely a position the bot cannot stand on. Compare it against
     * `failure.destination`, which is where the walk was really steering.
     */
    to: Position;
    /** `null` for a walk the game never acknowledged, which has no dispatch line. */
    dispatchedTick: number | null;
    /** `null` for a walk the run never finished. */
    settledTick: number | null;
    /** What the scheduler predicted, or `null` when there was no dispatch line to read it from. */
    plannedDuration: number | null;
    /** What the game measured, or `null` when it was not timed at both ends. */
    observedDuration: number | null;
    /** `observedDuration - plannedDuration`, or `null` when either is missing. */
    delta: number | null;
    /** The verdict, or `'never settled'` -- which is a state, not a verdict. */
    status: string;
    error: string | null;
    failure: WalkFailure | null;
}

/** `(bot, stepIndex)` as a map key -- the pair is the walk's whole identity. */
function walkKey(bot: number, stepIndex: number): string {
    return `${bot}:${stepIndex}`;
}

/**
 * Every walk a run recorded, worst overrun first.
 *
 * Walking is most of a run's wall clock, and none of it was in the record at
 * all until 2026-09-02 -- a run could fail three walks and leave a single
 * error string behind. This is the table that answers "where did the time
 * go" for the half of the schedule the overrun table has never covered.
 *
 * Scoped per plan epoch for the same reason `joinRunOutcome` is: `stepIndex`
 * restarts with every plan, exactly as action ids do, so a whole-run join on
 * it would pair a late walk with an early one. Within one epoch a bot walks
 * one leg at a time, so `(bot, stepIndex)` is unique there.
 *
 * Both half-pairs are kept rather than dropped, and they mean different
 * things. A **settle with no dispatch** is a walk the game never
 * acknowledged: its planned duration is unknown, not zero. A **dispatch with
 * no settle** is a bot last seen walking -- `status: 'never settled'`, which
 * is deliberately not a verdict, because nobody gave one.
 */
export function walksOf(events: Event[]): WalkRow[] {
    const rows: WalkRow[] = [];
    for (const epoch of planEpochs(events)) {
        const open = new Map<string, WalkRow>();
        for (const event of epoch.events) {
            if (event.kind === 'walk_dispatched') {
                const row: WalkRow = {
                    milestoneIndex: epoch.milestoneIndex,
                    bot: event.bot,
                    stepIndex: event.step_index,
                    to: event.to,
                    dispatchedTick: event.tick,
                    settledTick: null,
                    plannedDuration: event.planned_duration,
                    observedDuration: null,
                    delta: null,
                    status: 'never settled',
                    error: null,
                    failure: null
                };
                rows.push(row);
                open.set(walkKey(event.bot, event.step_index), row);
            } else if (event.kind === 'walk_settled') {
                const key = walkKey(event.bot, event.step_index);
                const row = open.get(key);
                if (row === undefined) {
                    rows.push({
                        milestoneIndex: epoch.milestoneIndex,
                        bot: event.bot,
                        stepIndex: event.step_index,
                        to: event.to,
                        dispatchedTick: null,
                        settledTick: event.tick,
                        plannedDuration: null,
                        observedDuration: event.elapsed_ticks,
                        delta: null,
                        status: event.status,
                        error: event.error,
                        failure: event.failure
                    });
                    continue;
                }
                open.delete(key);
                row.settledTick = event.tick;
                row.observedDuration = event.elapsed_ticks;
                row.delta =
                    row.plannedDuration !== null && event.elapsed_ticks !== null
                        ? event.elapsed_ticks - row.plannedDuration
                        : null;
                row.status = event.status;
                row.error = event.error;
                row.failure = event.failure;
            }
        }
    }
    return sortByDelta(rows);
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
     * What actually ran under `plan` -- joined within that plan's own epoch,
     * never against the whole run's events, because an id from a different
     * plan (this milestone's own earlier attempt, or any other milestone's)
     * routinely repeats this one's and must not be mistaken for it.
     */
    ran: OutcomeRow[];
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
 * One row per milestone, its plan, what ran under it, and its closing
 * reason, index-ascending.
 *
 * A `plan_created` or `milestone_satisfied` event naming a milestone index
 * with no matching `milestone_started` is skipped rather than fabricating a
 * row for it -- that combination has never been observed and inventing a
 * goal string for it would be a guess dressed up as data.
 *
 * A milestone replanned after getting stuck produces more than one
 * `plan_created` for the same index; `planEpochs` yields one epoch per
 * attempt, in order, and the last one -- the one that actually closed the
 * milestone -- wins here, the same "last plan replaces the previous one"
 * rule `plan` alone has always followed.
 */
export function milestonesOf(events: Event[]): MilestoneRow[] {
    const rows = new Map<number, MilestoneRow>();
    for (const event of events) {
        if (event.kind === 'milestone_started') {
            rows.set(event.index, {
                index: event.index,
                goal: event.goal,
                plan: [],
                ran: [],
                satisfiedReason: null,
                iterations: null
            });
        } else if (event.kind === 'milestone_satisfied') {
            const row = rows.get(event.index);
            if (row) {
                row.satisfiedReason = event.reason;
                row.iterations = event.iterations;
            }
        }
    }
    for (const epoch of planEpochs(events)) {
        if (epoch.milestoneIndex === null) continue;
        const row = rows.get(epoch.milestoneIndex);
        if (row) {
            row.plan = epoch.plan;
            row.ran = joinPlanToOutcome(epoch.plan, epoch.events, epoch.milestoneIndex);
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
