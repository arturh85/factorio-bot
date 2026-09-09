// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import FlowPanel from './FlowPanel.vue';
import {FlowExport} from '@/api/types';

const FLOW: FlowExport = {
    tick: 100,
    nodes: [
        {id: 0, position: {x: 0.5, y: 0.5}, name: 'burner-mining-drill', kind: 'mining-drill', recipe: null, miner_ore: 'iron-ore'},
        {id: 1, position: {x: 2.5, y: 0.5}, name: 'stone-furnace', kind: 'furnace', recipe: null, miner_ore: null}
    ],
    edges: [{from: 0, to: 1, lanes: [[{item: 'iron-ore', per_second: 0.5}]]}]
};

describe('FlowPanel', () => {
    it('draws one mark per node and one line per edge', () => {
        const w = mount(FlowPanel, {props: {flow: FLOW, flowError: null, samples: [], cursor: 100}});
        expect(w.findAll('circle.flow-node')).toHaveLength(2);
        expect(w.findAll('line.flow-edge')).toHaveLength(1);
    });

    it('scales an edge\'s stroke width by its modelled rate', () => {
        const w = mount(FlowPanel, {props: {flow: FLOW, flowError: null, samples: [], cursor: 100}});
        const line = w.get('line.flow-edge');
        expect(Number(line.attributes('stroke-width'))).toBeGreaterThan(0);
    });

    it('titles a node with model, measured and gap wording -- the wording is the claim', () => {
        const w = mount(FlowPanel, {props: {flow: FLOW, flowError: null, samples: [], cursor: 100}});
        const drill = w.findAll('circle.flow-node')[0];
        expect(drill.find('title').text()).toContain('model');
        expect(drill.find('title').text()).toContain('measured');
    });

    it('shows a one-line reason instead of an empty plot when flow is null', () => {
        const w = mount(FlowPanel, {props: {flow: null, flowError: null, samples: [], cursor: 100}});
        expect(w.findAll('circle.flow-node')).toHaveLength(0);
        expect(w.text().toLowerCase()).toContain('no flow');
    });

    it('shows the fetch-failure reason, not an empty plot, when flowError is set', () => {
        const w = mount(FlowPanel, {props: {flow: null, flowError: 'this server does not provide /flow', samples: [], cursor: 100}});
        expect(w.text()).toContain('this server does not provide /flow');
    });
});
