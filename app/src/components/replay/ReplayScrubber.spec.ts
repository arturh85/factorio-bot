// @vitest-environment jsdom
/**
 * The scrubber after the per-camera screenshots were retired (2026-09-02).
 *
 * Everything that judged and rendered *frames* is gone with the feature: the
 * run-mismatch banner, the partial-match caveat, the client and camera
 * pickers, the frame axis and the `frame-current` / `frame-stale` states. What
 * survives is the scrubber itself and the video half -- and the video's three
 * states are deliberately **not** the frame states renamed, which is what most
 * of this file now pins.
 */
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import ReplayScrubber from './ReplayScrubber.vue';
import {nextTick} from 'vue';
import {REALISTIC_REPLAY} from '@/api/replay.fixtures';
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

describe('ReplayScrubber -- axis and scrubber plumbing', () => {
    it('renders one row per step, via the existing ReplayView', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, tick: 300}
        });
        expect(wrapper.findAllComponents({name: 'ReplayStepRow'})).toHaveLength(REALISTIC_REPLAY.steps.length);
    });

    it('emits an updated tick when the scrubber moves', async () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, tick: 300}
        });
        const slider = wrapper.findComponent({name: 'Slider'});
        expect(slider.exists()).toBe(true);
        await slider.vm.$emit('update:modelValue', 10);
        expect(wrapper.emitted('update:tick')).toEqual([[10]]);
    });

    it('still offers the scrubber for a run with no video at all', () => {
        // The scrubber used to live inside the frame section, so a run with
        // nothing to show lost the control that drives the step rows too.
        const wrapper = mount(ReplayScrubber, {props: {replay: REALISTIC_REPLAY, tick: 0}});
        expect(wrapper.findComponent({name: 'Slider'}).exists()).toBe(true);
    });

    it('has no trace of the retired screenshot panel', () => {
        const wrapper = mount(ReplayScrubber, {props: {replay: REALISTIC_REPLAY, tick: 0}});
        for (const id of ['frame-section', 'frame-axis', 'frame-image', 'frame-current',
            'frame-stale', 'no-frame-state', 'camera-select', 'client-select',
            'run-mismatch-banner', 'run-match-caveat']) {
            expect(wrapper.find(`[data-testid="${id}"]`).exists()).toBe(false);
        }
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
            props: {replay: REALISTIC_REPLAY, tick: 0,
                videoManifest: NO_VIDEO_MANIFEST, videoTicks: NO_TICKS, videoSrc: null}
        });
        expect(wrapper.find('[data-testid="video-section"]').exists()).toBe(false);
    });

    it('renders the element with its source when the cursor is inside the recording', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, tick: 30, ...video}
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
            props: {replay: REALISTIC_REPLAY, tick: 200, ...video}
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
            props: {replay: REALISTIC_REPLAY, tick: 45,
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
            props: {replay: REALISTIC_REPLAY, tick: 30,
                videoManifest: SKEWED_MANIFEST, videoTicks: CLEAN_TICKS, videoSrc: '/v.mp4'}
        });
        expect(wrapper.find('[data-testid="video-clock-unverified"]').exists()).toBe(true);
    });

    it('reports a recorder that outlived its run as a defect', () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, tick: 30,
                videoManifest: UNSTOPPED_MANIFEST, videoTicks: CLEAN_TICKS, videoSrc: '/v.mp4'}
        });
        const kinds = wrapper.findAll('[data-testid="video-defect"]')
            .map((node) => node.attributes('data-kind'));
        expect(kinds).toContain('unstopped');
    });

    it('survives the cursor moving, which is what schedules a seek', async () => {
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, tick: 30, ...video}
        });
        await wrapper.setProps({tick: 60});
        await nextTick();
        expect(wrapper.find('[data-testid="video-element"]').exists()).toBe(true);
    });
});
