// @vitest-environment jsdom
/**
 * What the run page renders now that the per-camera screenshots are gone.
 *
 * The frames panel was removed with the feature (retired 2026-09-02): a
 * `<details class="frame">` block with a (bot, camera) picker, an `<img>` and
 * a "no frame yet at this tick" state. None of it exists any more, and the
 * first test here pins that -- it is the one assertion that would catch the
 * panel being reintroduced by a revert.
 *
 * The rest covers what took its place as the page's visual record: the video
 * panel, which renders only for a run that actually recorded one. There is
 * deliberately no empty state for it, because a panel explaining its own
 * emptiness on every run page is exactly what the frames panel had become.
 */
import {beforeEach, describe, expect, it} from 'vitest';
import {createPinia, setActivePinia} from 'pinia';
import {mount} from '@vue/test-utils';
import RunsPage from './RunsPage.vue';
import {useRunsStore} from '@/store/runsStore';
import {RunDetail, VideoManifest} from '@/api/types';

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

const VIDEO: VideoManifest = {
    run: 'run-1788365280-15443',
    video: {
        run: 'run-1788365280-15443',
        file: 'video/run.mp4',
        width: 700,
        height: 854,
        requested_width: 700,
        requested_height: 854,
        fps: 30,
        status: 'stopped',
        reason: null,
        ffmpeg_exit: 0,
        calibration: [],
        rate_ok: true,
        window_id: '0x1'
    },
    bytes: 290 * 1048576,
    samples: 120,
    skipped: 0,
    tick_range: {from: 100, to: 2000}
};

function open(patch: {video?: VideoManifest} = {}) {
    const store = useRunsStore();
    store.$patch({detail: DETAIL, ...patch});
    return mount(RunsPage, {
        global: {stubs: {MapPanel: true, RouterLink: true}}
    });
}

describe('RunsPage', () => {
    it('has no screenshot panel at all -- the cameras are retired', () => {
        const wrapper = open();

        // Not a message, not an empty picker: nothing. A paragraph about a
        // retired feature on every run page is worse than silence.
        expect(wrapper.find('.frame').exists()).toBe(false);
        expect(wrapper.find('[data-testid="frame-picker"]').exists()).toBe(false);
        expect(wrapper.text().toLowerCase()).not.toContain('screenshot');
    });

    it('renders the splits it was given', () => {
        expect(open().text()).toContain('iron plates');
    });

    it('renders no video panel for a run that recorded none', () => {
        // Same rule the frames panel broke: absence renders nothing, rather
        // than a panel explaining that there is nothing.
        expect(open().find('.video').exists()).toBe(false);
    });

    it('renders the player for a run that did record one', () => {
        const wrapper = open({video: VIDEO});

        expect(wrapper.find('.video').exists()).toBe(true);
        expect(wrapper.find('.video__player').exists()).toBe(true);
        expect(wrapper.text()).toContain('700x854');
    });
});
