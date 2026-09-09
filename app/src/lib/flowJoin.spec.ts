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
        // 24 items over the trailing 2-minute window = 12/min.
        expect(furnace.measuredPerMinute).toBe(12);
        expect(furnace.status).toBe('good'); // 'working' -> statusClass -> 'good'
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
