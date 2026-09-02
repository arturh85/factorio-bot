// @vitest-environment jsdom
/**
 * The frames panel after screenshot cameras were retired (2026-09-02).
 *
 * Video is the visual record now and a run captures no frames unless it asks,
 * so **zero frames is the normal case, not a failure**. The panel has to say
 * that in words. What it used to do instead was render its `<select>` with no
 * options and the message "This bot and camera captured no frames in this
 * run." -- which names a bot and a camera nobody chose, because there were
 * none to choose from, and reads as a picker that has gone wrong.
 */
import {beforeEach, describe, expect, it} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {mount} from '@vue/test-utils';
import RunsPage from './RunsPage.vue';
import {useRunsStore} from '@/store/runsStore';
import {ArchivedFrame, RunDetail} from '@/api/types';

beforeEach(() => {
    setActivePinia(createPinia());
});

const DETAIL: RunDetail = {
    summary: {
        run_id: 'run-1788365280-15443',
        finished: true,
        started_unix: 1788365280,
        finished_unix: 1788368000,
        outcome: 'success',
        elapsed_ticks: 162000,
        events: 400,
        frames: 0,
        splits: 3,
        samples: 500,
        map: 12
    } as RunDetail['summary'],
    splits: [
        {
            index: 1,
            goal: 'iron plates',
            started_tick: 100,
            ended_tick: 2000,
            elapsed_ticks: 1900,
            outcome: 'reached'
        } as RunDetail['splits'][number]
    ]
};

function frame(bot: number, tick: number, camera: string): ArchivedFrame {
    return {bot, tick, camera, file: `frames/${bot}/tick-${tick}-${camera}.jpg`};
}

function open(frames: ArchivedFrame[]) {
    const store = useRunsStore();
    store.$patch({detail: DETAIL, frames});
    return mount(RunsPage, {
        global: {stubs: {MapPanel: true, RouterLink: true}}
    });
}

describe('RunsPage frames panel', () => {
    it('says no frames were captured, and offers no picker, when there are none', () => {
        const wrapper = open([]);

        const none = wrapper.find('[data-testid="no-frames"]');
        expect(none.exists()).toBe(true);
        expect(none.text().toLowerCase()).toContain('no screenshots');
        // The picker is the part that reads as broken: a <select> with nothing
        // in it, above a message naming a bot and camera nobody chose.
        expect(wrapper.find('[data-testid="frame-picker"]').exists()).toBe(false);
    });

    it('still offers the picker for a run that did capture frames', () => {
        const wrapper = open([frame(1, 300, 'follow'), frame(1, 600, 'follow')]);

        expect(wrapper.find('[data-testid="frame-picker"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="no-frames"]').exists()).toBe(false);
    });

    it('keeps saying so when the frames listing itself failed, which is not the same thing', () => {
        // "None were captured" and "we could not find out" are different
        // answers, and the fetch failure has to win: it is the one that means
        // the panel does not know.
        const store = useRunsStore();
        store.$patch({detail: DETAIL, frames: [], frameError: 'frames unavailable — 500'});
        const wrapper = mount(RunsPage, {
            global: {stubs: {MapPanel: true, RouterLink: true}}
        });

        expect(wrapper.text()).toContain('frames unavailable');
        expect(wrapper.find('[data-testid="no-frames"]').exists()).toBe(false);
    });
});
