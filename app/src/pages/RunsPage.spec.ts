// @vitest-environment jsdom
/**
 * The frames panel after screenshot cameras were retired (2026-09-02).
 *
 * Video is the visual record now and almost no run captures frames, so the
 * panel explaining its own emptiness appeared on every run page. It is gone:
 * a run with no frames renders nothing here at all.
 *
 * Two things still have to hold. A run that DID capture frames keeps its
 * picker, because those are the tick-addressable record. And a listing that
 * FAILED still shows its error -- "we could not find out" is not "there were
 * none", and the panel must not go quiet on the one case where it does not
 * know.
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
    it('renders nothing at all when there are no frames', () => {
        const wrapper = open([]);

        // Not a message, not an empty picker: nothing. Screenshot cameras are
        // retired, so this is every run, and a paragraph about a retired
        // feature on every run page is worse than silence.
        expect(wrapper.find('.frame').exists()).toBe(false);
        expect(wrapper.find('[data-testid="frame-picker"]').exists()).toBe(false);
        expect(wrapper.text().toLowerCase()).not.toContain('no screenshots');
    });

    it('still offers the picker for a run that did capture frames', () => {
        const wrapper = open([frame(1, 300, 'follow'), frame(1, 600, 'follow')]);

        expect(wrapper.find('[data-testid="frame-picker"]').exists()).toBe(true);
        expect(wrapper.find('.frame').exists()).toBe(true);
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
        // The panel exists in this case precisely because it has something to
        // say; the empty case is the one that renders nothing.
        expect(wrapper.find('.frame').exists()).toBe(true);
    });
});
