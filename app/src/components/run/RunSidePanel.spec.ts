// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {CLEAN_MANIFEST, CLEAN_TICKS, NO_VIDEO_MANIFEST} from '@/api/video.fixtures';
import RunSidePanel from './RunSidePanel.vue';

const base = {runId: 'run-1', cursor: 100, entities: [], bots: [], trail: {}, records: [], bounds: null, mapError: null, fills: new Map(), videoTicks: null, videoError: null};

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
    it('shows the map error in place of the map', () => {
        const w = mount(RunSidePanel, {props: {...base, video: NO_VIDEO_MANIFEST, mapError: 'entity map unavailable — x'}});
        expect(w.text()).toContain('entity map unavailable');
    });
});
