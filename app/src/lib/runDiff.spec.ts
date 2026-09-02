import {describe, expect, it} from 'vitest';
import {Event, PlannedStep, Sample} from '@/api/types';
import {divergencesOf, inventoryAtFailure, joinPlanToOutcome, joinRunOutcome, milestonesOf, planEpochs} from './runDiff';

function step(id: number, bot: number, action: string, plannedDuration: number, deps: number[] = []): PlannedStep {
    return {id, bot, action, deps, planned_start: 0, planned_duration: plannedDuration};
}

function planCreated(milestoneIndex: number, plan: PlannedStep[], tick = 0): Event {
    return {
        kind: 'plan_created',
        milestone_index: milestoneIndex,
        steps: plan.length,
        makespan: 0,
        bots: [],
        plan,
        tick
    };
}

function dispatched(id: number, bot: number, tick: number): Event {
    return {kind: 'action_dispatched', id, bot, action: 'x', target: null, tick};
}

function settled(id: number, bot: number, status: string, elapsedTicks: number | null): Event {
    return {
        kind: 'action_settled',
        id,
        bot,
        status,
        elapsed_ticks: elapsedTicks,
        error: null,
        failure: null,
        tick: elapsedTicks ?? 0
    };
}

function failedSettle(id: number, bot: number, tick: number, kind: 'missing_item' | 'timeout' = 'timeout'): Event {
    return {
        kind: 'action_settled',
        id,
        bot,
        status: 'failed',
        elapsed_ticks: 100,
        error: 'boom',
        failure: {kind, detail: null},
        tick
    };
}

describe('joinPlanToOutcome', () => {
    it('pairs a planned step with its settle by id', () => {
        const rows = joinPlanToOutcome(
            [step(0, 1, 'mine 10 iron-ore', 300)],
            [dispatched(0, 1, 61637), settled(0, 1, 'success', 362)]
        );
        expect(rows[0]).toMatchObject({id: 0, plannedDuration: 300, actualDuration: 362, delta: 62});
    });

    it('sorts by overrun, worst first', () => {
        const rows = joinPlanToOutcome(
            [step(0, 1, 'a', 300), step(1, 2, 'b', 100)],
            [settled(0, 1, 'success', 310), settled(1, 2, 'success', 400)]
        );
        expect(rows.map((r) => r.id)).toEqual([1, 0]);
    });

    it('keeps a planned step that never ran, with a null actual', () => {
        // A step that was never dispatched is the most interesting row on the
        // page. Dropping it because it has no settle hides the failure.
        const rows = joinPlanToOutcome([step(0, 1, 'a', 300)], []);
        expect(rows[0].actualDuration).toBeNull();
        expect(rows[0].status).toBe('never dispatched');
    });

    it('keeps a settle with no planned step, with a null planned', () => {
        // Recovery actions are dispatched outside the plan. They belong on the
        // page too, and a join that only walks the plan loses them.
        const rows = joinPlanToOutcome([], [settled(9, 1, 'success', 50)]);
        expect(rows[0].plannedDuration).toBeNull();
    });

    it('reports a dispatched-but-unsettled step as "never settled", not "never dispatched"', () => {
        const rows = joinPlanToOutcome([step(0, 1, 'a', 300)], [dispatched(0, 1, 100)]);
        expect(rows[0].status).toBe('never settled');
        expect(rows[0].delta).toBeNull();
    });

    it('names a recovery action from its dispatch event when the plan does not have it', () => {
        const rows = joinPlanToOutcome([], [dispatched(9, 3, 100), settled(9, 3, 'success', 50)]);
        expect(rows[0]).toMatchObject({bot: 3, action: 'x'});
    });

    it('falls back to a placeholder action when neither the plan nor a dispatch names it', () => {
        const rows = joinPlanToOutcome([], [settled(9, 1, 'success', 50)]);
        expect(rows[0].action).toBe('(unrecorded)');
    });

    it('carries the classified failure through from the settle', () => {
        const rows = joinPlanToOutcome([step(0, 1, 'a', 300)], [failedSettle(0, 1, 500, 'missing_item')]);
        expect(rows[0].failure).toEqual({kind: 'missing_item', detail: null});
    });

    it('treats a settle with no elapsed_ticks as having no actual duration', () => {
        const rows = joinPlanToOutcome([step(0, 1, 'a', 300)], [settled(0, 1, 'lost', null)]);
        expect(rows[0].actualDuration).toBeNull();
        expect(rows[0].delta).toBeNull();
    });

    it('sorts every null-delta row after every row that has a delta', () => {
        const rows = joinPlanToOutcome(
            [step(0, 1, 'never ran a', 300), step(1, 2, 'never ran b', 100), step(2, 3, 'on time', 50)],
            [settled(2, 3, 'success', 50)]
        );
        expect(rows[0].id).toBe(2);
        expect(rows.slice(1).map((r) => r.delta)).toEqual([null, null]);
    });

    it('takes each step at most once even when both rows have a null delta', () => {
        const rows = joinPlanToOutcome([step(0, 1, 'a', 300), step(1, 2, 'b', 300)], []);
        expect(rows).toHaveLength(2);
        expect(rows.every((r) => r.delta === null)).toBe(true);
    });

    it('ignores event kinds that are neither a dispatch nor a settle', () => {
        const rows = joinPlanToOutcome(
            [step(0, 1, 'a', 300)],
            [{kind: 'run_finished', outcome: 'success', elapsed_ticks: 1000, tick: 1000}]
        );
        expect(rows[0].status).toBe('never dispatched');
    });
});

describe('joinRunOutcome / planEpochs', () => {
    it('does not cross-join two plan epochs that both number their steps from zero', () => {
        const events: Event[] = [
            planCreated(0, [step(0, 1, 'mine ore', 100)]),
            settled(0, 1, 'success', 150),
            planCreated(1, [step(0, 1, 'smelt plates', 300)]),
            settled(0, 1, 'success', 362)
        ];
        const rows = joinRunOutcome(events);
        expect(rows).toHaveLength(2);
        const byMilestone = new Map(rows.map((r) => [r.milestoneIndex, r]));
        expect(byMilestone.get(0)).toMatchObject({plannedDuration: 100, actualDuration: 150, delta: 50});
        expect(byMilestone.get(1)).toMatchObject({plannedDuration: 300, actualDuration: 362, delta: 62});
    });

    it('keeps five planning rounds that each restart ids at zero fully separate (real run shape)', () => {
        // A real recorded run's action_dispatched ids: [0,1,2,3, 0,1,2,3,
        // 0,1,2,3, 0,1,2,3, 0,1,2,3] -- five planning rounds, each numbering
        // from zero. Built here with a distinct planned/actual duration per
        // round so a cross-epoch join would show up as a wrong delta.
        const events: Event[] = [];
        for (let round = 0; round < 5; round++) {
            const plan = [0, 1, 2, 3].map((id) => step(id, 1, `round ${round} step ${id}`, 100 + round));
            events.push(planCreated(round, plan, round * 1000));
            for (const id of [0, 1, 2, 3]) {
                events.push(settled(id, 1, 'success', 100 + round + id));
            }
        }
        const rows = joinRunOutcome(events);
        expect(rows).toHaveLength(20);
        // actualDuration - plannedDuration = (100 + round + id) - (100 + round) = id,
        // regardless of round -- true only if every row joined within its own epoch.
        for (const row of rows) {
            expect(row.delta).toBe(row.id);
        }
    });

    it('keeps a settle recorded before any plan_created, with a null planned duration', () => {
        const rows = joinRunOutcome([settled(9, 1, 'success', 50)]);
        expect(rows).toHaveLength(1);
        expect(rows[0]).toMatchObject({plannedDuration: null, actualDuration: 50, milestoneIndex: null});
    });

    it('keeps both planning rounds of a replanned milestone as separate epochs', () => {
        const events: Event[] = [
            planCreated(2, [step(0, 1, 'attempt 1', 10)]),
            settled(0, 1, 'stuck', 10),
            planCreated(2, [step(0, 1, 'attempt 2', 20)], 100),
            settled(0, 1, 'success', 25)
        ];
        const epochs = planEpochs(events).filter((e) => e.milestoneIndex === 2);
        expect(epochs).toHaveLength(2);

        const rows = joinRunOutcome(events);
        expect(rows.filter((r) => r.milestoneIndex === 2)).toHaveLength(2);
    });

    it('re-sorts after concatenating epochs, worst overrun first across the whole run', () => {
        const events: Event[] = [
            planCreated(0, [step(0, 1, 'a', 100)]),
            settled(0, 1, 'success', 110),
            planCreated(1, [step(0, 2, 'b', 100)]),
            settled(0, 2, 'success', 400)
        ];
        const rows = joinRunOutcome(events);
        expect(rows.map((r) => r.milestoneIndex)).toEqual([1, 0]);
    });

    it('always emits the pre-plan epoch, even when it is empty', () => {
        const epochs = planEpochs([planCreated(0, [step(0, 1, 'a', 10)])]);
        expect(epochs[0]).toMatchObject({milestoneIndex: null, plan: [], events: []});
        expect(epochs[1].milestoneIndex).toBe(0);
    });
});

describe('divergencesOf', () => {
    it('is empty when the map has no keyframes', () => {
        expect(divergencesOf([])).toEqual([]);
        expect(
            divergencesOf([{kind: 'removed', tick: 10, bot: 1, entity: {name: 'x', position: {x: 0, y: 0}, direction: 0}}])
        ).toEqual([]);
    });

    it('flattens every keyframe divergence and sorts by tick', () => {
        const entity = {name: 'chest', position: {x: 1, y: 2}, direction: 0};
        const map = divergencesOf([
            {
                kind: 'keyframe',
                tick: 200,
                bounds: {left: 0, top: 0, right: 1, bottom: 1},
                game: [],
                model: [],
                divergence: [{entity, only_in: 'game'}]
            },
            {
                kind: 'keyframe',
                tick: 100,
                bounds: {left: 0, top: 0, right: 1, bottom: 1},
                game: [],
                model: [],
                divergence: [
                    {entity, only_in: 'model'},
                    {entity, only_in: 'game'}
                ]
            }
        ]);
        expect(map.map((r) => r.tick)).toEqual([100, 100, 200]);
    });
});

describe('milestonesOf', () => {
    it('is empty with no milestone_started events', () => {
        expect(milestonesOf([])).toEqual([]);
    });

    it('takes the last plan_created for a milestone, not the first', () => {
        const first = step(0, 1, 'first plan', 10);
        const second = step(0, 1, 'replanned', 20);
        const events: Event[] = [
            {kind: 'milestone_started', index: 0, goal: 'have 10 iron-plate', tick: 0},
            {
                kind: 'plan_created',
                milestone_index: 0,
                steps: 1,
                makespan: 10,
                bots: [1],
                plan: [first],
                tick: 1
            },
            {
                kind: 'plan_created',
                milestone_index: 0,
                steps: 1,
                makespan: 20,
                bots: [1],
                plan: [second],
                tick: 2
            }
        ];
        const rows = milestonesOf(events);
        expect(rows[0].plan).toEqual([second]);
    });

    it('records the satisfied reason and iteration count', () => {
        const events: Event[] = [
            {kind: 'milestone_started', index: 0, goal: 'have 10 iron-plate', tick: 0},
            {kind: 'milestone_satisfied', index: 0, iterations: 0, reason: 'plan_empty', tick: 5}
        ];
        const rows = milestonesOf(events);
        expect(rows[0]).toMatchObject({satisfiedReason: 'plan_empty', iterations: 0});
    });

    it('ignores a plan_created or milestone_satisfied naming a milestone that never started', () => {
        const events: Event[] = [
            {
                kind: 'plan_created',
                milestone_index: 7,
                steps: 0,
                makespan: 0,
                bots: [],
                plan: [],
                tick: 0
            },
            {kind: 'milestone_satisfied', index: 7, iterations: 1, reason: 'unknown', tick: 1}
        ];
        expect(milestonesOf(events)).toEqual([]);
    });

    it('sorts rows by milestone index', () => {
        const events: Event[] = [
            {kind: 'milestone_started', index: 2, goal: 'b', tick: 0},
            {kind: 'milestone_started', index: 1, goal: 'a', tick: 0}
        ];
        expect(milestonesOf(events).map((r) => r.index)).toEqual([1, 2]);
    });
});

describe('inventoryAtFailure', () => {
    const botsSample = (tick: number, id: number, inventory: Record<string, number>): Sample => ({
        kind: 'bots',
        schema: 1,
        tick,
        run: null,
        bots: [{id, position: {x: 0, y: 0}, inventory, crafting_queue: 0, mining: null}]
    });

    it('is empty when no action failed', () => {
        expect(inventoryAtFailure([settled(0, 1, 'success', 10)], [])).toEqual([]);
    });

    it('pairs a failure with the nearest sample naming that bot, even if it is later', () => {
        const samples = [botsSample(50, 1, {'iron-ore': 1}), botsSample(600, 1, {'iron-ore': 99})];
        const rows = inventoryAtFailure([failedSettle(0, 1, 550)], samples);
        expect(rows[0].inventory?.inventory).toEqual({'iron-ore': 99});
    });

    it('keeps the closer sample rather than a farther one seen afterward', () => {
        const samples = [botsSample(550, 1, {'iron-ore': 1}), botsSample(50, 1, {'iron-ore': 99})];
        const rows = inventoryAtFailure([failedSettle(0, 1, 550)], samples);
        expect(rows[0].inventory?.inventory).toEqual({'iron-ore': 1});
    });

    it('returns null inventory when no sample names the failing bot', () => {
        const samples = [botsSample(50, 2, {})];
        const rows = inventoryAtFailure([failedSettle(0, 1, 550)], samples);
        expect(rows[0].inventory).toBeNull();
    });

    it('ignores non-bots samples', () => {
        const force: Sample = {
            kind: 'force',
            schema: 1,
            tick: 50,
            run: null,
            research: null,
            techs_unlocked: 0,
            production: {made: {}, consumed: {}},
            power: {generated_kw: 0, consumed_kw: 0, satisfaction: 1}
        };
        const rows = inventoryAtFailure([failedSettle(0, 1, 50)], [force]);
        expect(rows[0].inventory).toBeNull();
    });
});
