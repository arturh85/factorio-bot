// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {CLEAN_MANIFEST, CLEAN_TICKS, NO_VIDEO_MANIFEST} from '@/api/video.fixtures';
import {FlowExport} from '@/api/types';
import MapPanel from '@/components/MapPanel.vue';
import RunSidePanel from './RunSidePanel.vue';

const base = {
    runId: 'run-1', cursor: 100, entities: [], bots: [], trail: {}, records: [], bounds: null, mapError: null,
    fills: new Map(), videoTicks: null, videoError: null, flow: null, flowError: null, samples: []
};

describe('RunSidePanel', () => {
    it('shows only the map tab when the run recorded no video', () => {
        const w = mount(RunSidePanel, {props: {...base, video: NO_VIDEO_MANIFEST}});
        expect(w.findAll('[role="tab"]').map((t) => t.text())).toEqual(['Map']);
        expect(w.find('video').exists()).toBe(false);
    });
    it('offers a video tab when a recording exists and switches to it', async () => {
        const w = mount(RunSidePanel, {props: {...base, video: CLEAN_MANIFEST, videoTicks: CLEAN_TICKS}});
        const tabs = w.findAll('[role="tab"]');
        expect(tabs.map((t) => t.text())).toEqual(['Map', 'Video']);
        await tabs[1].trigger('click');
        expect(w.find('video').exists()).toBe(true);
        expect(w.get('video').attributes('src')).toBe('/api/v1/runs/run-1/video/file');
    });
    it('hands the map the highlighted position key', () => {
        // The machine band selects; the map is where that selection is seen.
        // This panel is the only thing between them.
        const entities = [{name: 'stone-furnace', position: {x: 10, y: 12}, direction: 0}];
        const w = mount(RunSidePanel, {props: {...base, video: NO_VIDEO_MANIFEST, entities, bounds: {left: 0, top: 0, right: 48, bottom: 48}, highlight: '10,12'}});
        expect(w.findComponent(MapPanel).props('highlight')).toBe('10,12');
        expect(w.findAll('rect[data-highlighted="true"]')).toHaveLength(1);
    });
    it('shows the map error in place of the map', () => {
        const w = mount(RunSidePanel, {props: {...base, video: NO_VIDEO_MANIFEST, mapError: 'entity map unavailable — x'}});
        expect(w.text()).toContain('entity map unavailable');
    });
    it('shows a Flow tab and mounts FlowPanel when it is selected', async () => {
        const FLOW: FlowExport = {
            tick: 100,
            nodes: [{id: 0, position: {x: 0.5, y: 0.5}, name: 'burner-mining-drill', kind: 'mining-drill', recipe: null, miner_ore: 'iron-ore'}],
            edges: []
        };
        const w = mount(RunSidePanel, {props: {...base, video: NO_VIDEO_MANIFEST, flow: FLOW}});
        const tabs = w.findAll('[role="tab"]');
        expect(tabs.map((t) => t.text())).toEqual(['Map', 'Flow']);
        await tabs[1].trigger('click');
        expect(w.findAll('circle.flow-node')).toHaveLength(1);
    });
    it('offers no Flow tab when there is neither a flow keyframe nor a flow error', () => {
        const w = mount(RunSidePanel, {props: {...base, video: NO_VIDEO_MANIFEST}});
        expect(w.findAll('[role="tab"]').map((t) => t.text())).toEqual(['Map']);
    });
});
