// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import ReplayScrubber from './ReplayScrubber.vue';
import {nextTick} from 'vue';
import {FramesManifest} from '@/api/types';
import {REALISTIC_REPLAY} from '@/api/replay.fixtures';
import {EMPTY_MANIFEST, MULTI_CAMERA_MANIFEST, OVERLAPPING_MANIFEST, OVERLAPPING_MANIFEST_LATE_START, UNRELATED_RUN_MANIFEST} from '@/api/frames.fixtures';
import {
    CLEAN_MANIFEST,
    CLEAN_TICKS,
    NO_TICKS,
    NO_VIDEO_MANIFEST,
    SKEWED_MANIFEST,
    STALLED_MANIFEST,
    STALLED_TICKS,
    UNSTOPPED_MANIFEST
} from '@/api/video.fixtures';

import '@/test/resizeObserverStub';

describe('ReplayScrubber -- honesty requirement 1: refuse the join when ranges do not overlap', () => {
    it('shows no frames at all when the replay and manifest tick ranges never overlap, and says why', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: UNRELATED_RUN_MANIFEST, tick: 200}
        });

        expect(wrapper.find('[data-testid="frame-image"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="frame-current"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="frame-stale"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="no-frame-state"]').exists()).toBe(false);

        const banner = wrapper.find('[data-testid="run-mismatch-banner"]');
        expect(banner.exists()).toBe(true);
        expect(banner.text()).toContain('different run');

        // The timeline itself is still rendered, per the brief: "Render the
        // timeline alone."
        expect(wrapper.find('[data-testid="step-list"]').exists()).toBe(true);
    });

    it('labels a passing (overlapping) check as partial -- overlap is a detector, not a guarantee', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 300}
        });
        const caveat = wrapper.find('[data-testid="run-match-caveat"]');
        expect(caveat.exists()).toBe(true);
        // Must not read as a guarantee: no wording that claims certainty.
        expect(caveat.text().toLowerCase()).not.toMatch(/confirmed|verified match/);
        expect(caveat.text().toLowerCase()).toMatch(/not a guarantee|partial/);
    });

    it('does not show the partial caveat when frames are refused outright -- that verdict is definitive, not hedged', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: UNRELATED_RUN_MANIFEST, tick: 200}
        });
        expect(wrapper.find('[data-testid="run-match-caveat"]').exists()).toBe(false);
    });

    it('has nothing to judge when there is no manifest yet, and renders no frame section at all', () => {
        const wrapper = mount(ReplayScrubber, {props: {replay: REALISTIC_REPLAY, manifest: null, tick: 200}});
        expect(wrapper.find('[data-testid="run-mismatch-banner"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="run-match-caveat"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="frame-axis"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="step-list"]').exists()).toBe(true);
    });

    it('cannot judge when the manifest carries no parseable frame ticks -- inconclusive is not treated as a mismatch', () => {
        const wrapper = mount(ReplayScrubber, {props: {replay: REALISTIC_REPLAY, manifest: EMPTY_MANIFEST, tick: 200}});
        expect(wrapper.find('[data-testid="run-mismatch-banner"]').exists()).toBe(false);
    });
});

describe('ReplayScrubber -- honesty requirement 2: "no frame for this moment" is a rendered state', () => {
    it('renders the explicit no-frame state when the scrubber sits before any capture', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST_LATE_START, tick: 50}
        });
        expect(wrapper.find('[data-testid="no-frame-state"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="frame-image"]').exists()).toBe(false);
    });

    it('is not an empty box: the no-frame state carries explanatory text', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST_LATE_START, tick: 50}
        });
        expect(wrapper.get('[data-testid="no-frame-state"]').text().length).toBeGreaterThan(0);
    });
});

describe('ReplayScrubber -- honesty requirement 3: a stale frame must say how stale', () => {
    it('shows the frame age in the DOM when the nearest capture is behind the scrubber', () => {
        // OVERLAPPING_MANIFEST has captures at 0, 300, 1200; scrubbing to 350
        // means the 300 capture is shown, 50 ticks old.
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 350}
        });
        expect(wrapper.find('[data-testid="frame-image"]').exists()).toBe(true);
        const stale = wrapper.get('[data-testid="frame-stale"]');
        expect(stale.text()).toContain('50');
    });

    it('does not call an exact-match frame stale', () => {
        // `tick` is on the SHIFTED axis, which counts from the run's first
        // observed tick — 100 for `REALISTIC_REPLAY`. So 200 here addresses the
        // frame whose filename says 300. Planned ticks count from zero while
        // `game.tick` does not, and the axis shows the two against one origin
        // so their positions are comparable; the lookup converts back, because
        // absolute is the only clock a filename knows.
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 200}
        });
        expect(wrapper.find('[data-testid="frame-stale"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="frame-current"]').exists()).toBe(true);
    });
});

describe('ReplayScrubber -- honesty requirement 4: gaps in the frame sequence stay visible', () => {
    it('marks every known frame tick on the axis, so the distance between them shows a gap', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 700}
        });
        const marks = wrapper.findAll('[data-testid="frame-tick-mark"]');
        expect(marks.map((m) => m.attributes('data-tick'))).toEqual(['0', '300', '1200']);
    });

    it('a scrubber sitting inside a dropped-frame gap shows the stale frame before it, not a fabricated current one', () => {
        // Gap is 300 -> 1200 in absolute ticks (900, versus the 300-tick
        // cadence). Scrubber ticks are shifted by the run's origin of 100, so
        // 600 here is absolute 700, and the nearest frame at 300 is 400 ticks
        // old — the true age, never shrunk to make the gap look smaller.
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 600}
        });
        expect(wrapper.find('[data-testid="frame-current"]').exists()).toBe(false);
        const stale = wrapper.get('[data-testid="frame-stale"]');
        expect(stale.text()).toContain('400');
    });
});

describe('ReplayScrubber -- axis and scrubber plumbing', () => {
    it('renders one row per step beneath the frame section, via the existing ReplayView', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 300}
        });
        expect(wrapper.findAllComponents({name: 'ReplayStepRow'})).toHaveLength(REALISTIC_REPLAY.steps.length);
    });

    it('emits an updated tick when the scrubber moves', async () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 300}
        });
        const slider = wrapper.findComponent({name: 'Slider'});
        expect(slider.exists()).toBe(true);
        await slider.vm.$emit('update:modelValue', 10);
        expect(wrapper.emitted('update:tick')).toEqual([[10]]);
    });
});

describe('ReplayScrubber -- camera selection', () => {
    /**
     * With several cameras the scrubber must show one, and say which.
     * Merging them would put one camera's picture under another's name — a
     * frame that is real, current, and not of the thing the label claims.
     */
    it('offers only the cameras that produced frames, and shows one of them', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: MULTI_CAMERA_MANIFEST, tick: 0}
        });
        const options = wrapper.get('[data-testid="camera-select"]').findAll('option');
        expect(options.map((o) => o.text())).toEqual(['area', 'bot-1', 'follow']);
    });

    it('shows the selected camera\'s frame, not another camera\'s', async () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: MULTI_CAMERA_MANIFEST, tick: 0}
        });
        const select = wrapper.get('[data-testid="camera-select"]');

        // Asserted on the image's `src`, not on the caption: the caption says
        // which tick, and the whole point here is which *camera's* picture is
        // loaded at that tick. Two cameras share the tick, so only the URL
        // distinguishes them.
        const src = () => wrapper.get('[data-testid="frame-image"]').attributes('src') ?? '';

        await select.setValue('bot-1');
        expect(src()).toContain('bot-1');
        expect(src()).not.toContain('follow');

        await select.setValue('follow');
        expect(src()).toContain('follow');
        expect(src()).not.toContain('bot-1');
    });

    /**
     * `area` has a frame at tick 0 and none at 300; `follow` has both. Selecting
     * `area` at 300 must show the stale state rather than silently borrowing
     * `follow`'s frame — cameras are independent captures and a gap in one is
     * not filled by another.
     */
    it('a per-camera gap is a gap, not another camera\'s frame', async () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: MULTI_CAMERA_MANIFEST, tick: 300}
        });
        await wrapper.get('[data-testid="camera-select"]').setValue('area');
        expect(wrapper.find('[data-testid="frame-current"]').exists()).toBe(false);
        expect(wrapper.get('[data-testid="frame-stale"]').text()).toContain('300');
    });

    it('offers no selector when there is only one camera', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 0}
        });
        expect(wrapper.find('[data-testid="camera-select"]').exists()).toBe(false);
    });
});

describe('stale client marking', () => {
    /**
     * A client that sat out this run keeps the previous run's frames, and they
     * list in the manifest looking exactly like current ones. The picker says
     * so instead of presenting them as this run's.
     */
    it('marks the client whose frames belong to another run', async () => {
        const manifest: FramesManifest = {
            clients: [1, 2],
            run: null,
            client_runs: [
                {client: 1, run: 'job-9'},
                {client: 2, run: 'job-4'}
            ],
            frames: [
                {client: 1, tick: 300, camera: 'follow', name: 'tick-0000000300-follow.jpg', bytes: 10},
                {client: 2, tick: 300, camera: 'follow', name: 'tick-0000000300-follow.jpg', bytes: 10}
            ]
        };
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest, jobId: 'job-9'}
        });
        await nextTick();

        const options = wrapper.find('[data-testid="client-select"]').findAll('option');
        expect(options.map((o) => o.text())).toEqual([
            'client 1',
            'client 2 — frames from another run'
        ]);
    });

    it('marks nothing when the run is unknown', async () => {
        // No jobId: the two clients still disagree, but nothing here knows
        // which is current, and a guess would be worse than silence.
        const manifest: FramesManifest = {
            clients: [1, 2],
            run: null,
            client_runs: [
                {client: 1, run: 'job-9'},
                {client: 2, run: 'job-4'}
            ],
            frames: [
                {client: 1, tick: 300, camera: 'follow', name: 'tick-0000000300-follow.jpg', bytes: 10},
                {client: 2, tick: 300, camera: 'follow', name: 'tick-0000000300-follow.jpg', bytes: 10}
            ]
        };
        const wrapper = mount(ReplayScrubber, {props: {replay: REALISTIC_REPLAY, manifest}});
        await nextTick();

        const options = wrapper.find('[data-testid="client-select"]').findAll('option');
        expect(options.map((o) => o.text())).toEqual(['client 1', 'client 2']);
    });
});


/**
 * The video half. `REALISTIC_REPLAY`'s observed ticks start at 100, so
 * `observedOrigin()` is 100 and a shifted tick `t` on this axis is absolute
 * `game.tick` `t + 100` -- which is the clock the recording's samples are in.
 * `CLEAN_TICKS` spans absolute 100-250, i.e. shifted 0-150.
 */
describe('ReplayScrubber -- the video half', () => {
    const video = {
        videoManifest: CLEAN_MANIFEST,
        videoTicks: CLEAN_TICKS,
        videoSrc: '/api/v1/video/file?run=run-1'
    };

    it('renders nothing at all for a run that recorded no video', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: EMPTY_MANIFEST, tick: 0,
                videoManifest: NO_VIDEO_MANIFEST, videoTicks: NO_TICKS, videoSrc: null}
        });
        expect(wrapper.find('[data-testid="video-section"]').exists()).toBe(false);
    });

    it('renders the element with its source when the cursor is inside the recording', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: EMPTY_MANIFEST, tick: 30, ...video}
        });
        const element = wrapper.find('[data-testid="video-element"]');
        expect(element.exists()).toBe(true);
        expect(element.attributes('src')).toBe('/api/v1/video/file?run=run-1');
        expect(wrapper.find('[data-testid="video-out-of-range"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="video-clock-unknown"]').exists()).toBe(false);
    });

    it('says the tick is outside the recording rather than showing second zero', () => {
        // Shifted 200 is absolute 300, past the recording's last sample at 250.
        // Clamping would park the element on a frame of a different moment.
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: EMPTY_MANIFEST, tick: 200, ...video}
        });
        expect(wrapper.find('[data-testid="video-out-of-range"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="video-element"]').exists()).toBe(false);
    });

    it('covers the picture -- not merely captions it -- when the cursor is in a stall', () => {
        // The strongest requirement in the design. Shifted 45 is absolute 145,
        // which falls between two samples separated by a gap line. The video
        // does have a picture at the interpolated position; it is a picture of
        // some other moment, and leaving it visible with a caption beside it is
        // fabricated continuity.
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: EMPTY_MANIFEST, tick: 45,
                videoManifest: STALLED_MANIFEST, videoTicks: STALLED_TICKS,
                videoSrc: '/api/v1/video/file'}
        });
        const cover = wrapper.find('[data-testid="video-clock-unknown"]');
        expect(cover.exists()).toBe(true);
        expect(cover.classes()).toContain('absolute');
        expect(cover.classes()).toContain('inset-0');
    });

    it('marks the whole run unverified when the recording\'s rate did not check out', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: EMPTY_MANIFEST, tick: 30,
                videoManifest: SKEWED_MANIFEST, videoTicks: CLEAN_TICKS, videoSrc: '/v.mp4'}
        });
        expect(wrapper.find('[data-testid="video-clock-unverified"]').exists()).toBe(true);
    });

    it('reports a recorder that outlived its run as a defect', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: EMPTY_MANIFEST, tick: 30,
                videoManifest: UNSTOPPED_MANIFEST, videoTicks: CLEAN_TICKS, videoSrc: '/v.mp4'}
        });
        const kinds = wrapper.findAll('[data-testid="video-defect"]')
            .map((node) => node.attributes('data-kind'));
        expect(kinds).toContain('unstopped');
    });

    it('shows the video even when the frame join is refused', () => {
        // A run can record video and no frames. Hiding the recording because
        // there is nothing to judge the *frames* against would lose the only
        // artefact it has.
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: UNRELATED_RUN_MANIFEST, tick: 30, ...video}
        });
        expect(wrapper.find('[data-testid="frame-section"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="video-element"]').exists()).toBe(true);
    });

    it('survives the cursor moving, which is what schedules a seek', async () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: EMPTY_MANIFEST, tick: 30, ...video}
        });
        await wrapper.setProps({tick: 60});
        await nextTick();
        expect(wrapper.find('[data-testid="video-element"]').exists()).toBe(true);
    });
});
