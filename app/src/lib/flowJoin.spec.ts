import {describe, expect, it} from 'vitest';
import {flowAt, flowView} from './flowJoin';
import {FlowExport, MachineSample, Sample} from '@/api/types';

function pos(x: number, y: number) {
    return {x, y};
}

const FURNACE_FLOW: FlowExport = {
    tick: 6000,
    nodes: [
        {id: 0, position: pos(0.5, 0.5), name: 'burner-mining-drill', kind: 'mining-drill', recipe: null, miner_ore: 'iron-ore'},
        {id: 1, position: pos(1.5, 0.5), name: 'stone-furnace', kind: 'furnace', recipe: null, miner_ore: null}
    ],
    edges: [
        {from: 0, to: 1, lanes: [[{item: 'iron-ore', per_second: 0.5}]]}
    ]
};

// `MachinesSample` (the `Extract<Sample, {kind: 'machines'}>` variant) is
// not an exported name in `@/api/types` -- flowJoin.ts derives it locally,
// the same way machineTimeline.ts and runAttribution.ts already do. So this
// helper is typed as `Sample`, and every field `MachineSample` requires is
// filled in as a real object literal -- no `as unknown as ...` cast.
function machinesSample(tick: number, produced: number): Sample {
    const furnace: MachineSample = {
        name: 'stone-furnace',
        type: 'furnace',
        position: pos(1.5, 0.5),
        status: 'working',
        network: null,
        recipe: 'iron-plate',
        crafting: true,
        progress: 0.5,
        products_finished: produced,
        produced,
        produced_source: 'counter',
        produced_shared: null,
        mining: null,
        input: {},
        output: {},
        fuel: {}
    };
    return {
        kind: 'machines',
        schema: 1,
        tick,
        run: null,
        machines: {'1': furnace},
        truncated: 0
    };
}

describe('flowAt', () => {
    it('is the latest keyframe at or before the tick', () => {
        const a: FlowExport = {...FURNACE_FLOW, tick: 1000};
        const b: FlowExport = {...FURNACE_FLOW, tick: 5000};
        expect(flowAt([a, b], 4000)).toEqual(a);
        expect(flowAt([a, b], 6000)).toEqual(b);
    });

    it('is null before any keyframe exists', () => {
        expect(flowAt([{...FURNACE_FLOW, tick: 1000}], 500)).toBeNull();
    });

    it('is null for an empty flow history', () => {
        expect(flowAt([], 1000)).toBeNull();
    });
});

describe('flowView', () => {
    it('reports a node with no machine sample at its position as measured: null, never 0', () => {
        const view = flowView(FURNACE_FLOW, [], 6000);
        const drill = view.nodes.find((n) => n.id === 0)!;
        expect(drill.measuredPerMinute).toBeNull();
    });

    it('joins a node to its machine sample by position and computes a per-minute rate', () => {
        const samples: Sample[] = [machinesSample(3600, 10), machinesSample(6000, 34)];
        const view = flowView(FURNACE_FLOW, samples, 6000, 2);
        const furnace = view.nodes.find((n) => n.id === 1)!;
        // The nominal window is [6000 - 2*3600, 6000] = [-1200, 6000], clamped
        // to lo = 0. No `machines` sample exists at or before tick 0, so the
        // baseline falls back to the earliest sample (tick 3600, produced 10)
        // -- the fallback `machineRatePerMinuteAt` documents. The 24 items
        // (34 - 10) were made over the REAL observed span, tick 3600 to tick
        // 6000 -- 2400 ticks, i.e. 2400 / 3600 = 2/3 minute -- not over the
        // nominal 2-minute window. 24 / (2/3) = 36/min.
        expect(furnace.measuredPerMinute).toBe(36);
        expect(furnace.status).toBe('good'); // 'working' -> statusClass -> 'good'
    });

    it('agrees with the naive nominal-window formula when the baseline sample lands exactly at lo', () => {
        // cursorTick=7200, windowMinutes=1 -> lo = 3600, exactly the tick of
        // the baseline sample. The real observed span (3600 to 7200 = 3600
        // ticks = 1 minute) then coincides with the nominal window, so
        // dividing by either gives the same answer: (40-10)/1 = 30/min.
        const samples: Sample[] = [machinesSample(3600, 10), machinesSample(7200, 40)];
        const view = flowView(FURNACE_FLOW, samples, 7200, 1);
        const furnace = view.nodes.find((n) => n.id === 1)!;
        expect(furnace.measuredPerMinute).toBe(30);
    });

    it('reports measured as null, not Infinity or a fabricated number, when baseline and end sample coincide', () => {
        // Only one `machines` sample exists, so baseAt and endAt are the same
        // sample and the real elapsed span is zero.
        const samples: Sample[] = [machinesSample(50, 5)];
        const view = flowView(FURNACE_FLOW, samples, 100);
        const furnace = view.nodes.find((n) => n.id === 1)!;
        expect(furnace.measuredPerMinute).toBeNull();
    });

    it('derives model rate from the node\'s own outgoing edges, in items/minute', () => {
        const view = flowView(FURNACE_FLOW, [], 6000);
        const drill = view.nodes.find((n) => n.id === 0)!;
        expect(drill.primaryItem).toBe('iron-ore');
        expect(drill.modelPerMinute).toBe(30); // 0.5/s * 60
    });

    it('reports gap as null whenever either side is null or measured is zero', () => {
        const view = flowView(FURNACE_FLOW, [], 6000);
        const drill = view.nodes.find((n) => n.id === 0)!;
        expect(drill.gap).toBeNull(); // no measured sample at all
    });

    it('sums edge lanes into one totalPerMinute for stroke width', () => {
        const twoLane: FlowExport = {
            tick: 0,
            nodes: [
                {id: 0, position: pos(0, 0), name: 'transport-belt', kind: 'transport-belt', recipe: null, miner_ore: null},
                {id: 1, position: pos(1, 0), name: 'transport-belt', kind: 'transport-belt', recipe: null, miner_ore: null}
            ],
            edges: [{
                from: 0, to: 1,
                lanes: [[{item: 'iron-ore', per_second: 0.5}], [{item: 'copper-ore', per_second: 0.3}]]
            }]
        };
        const view = flowView(twoLane, [], 0);
        expect(view.edges[0].totalPerMinute).toBeCloseTo(48); // (0.5+0.3)*60
    });
});
