// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import ReplayScrubber from './ReplayScrubber.vue';
import {REALISTIC_REPLAY} from '@/api/replay.fixtures';
import {
    EMPTY_MANIFEST,
    OVERLAPPING_MANIFEST,
    OVERLAPPING_MANIFEST_LATE_START,
    UNRELATED_RUN_MANIFEST
} from '@/api/frames.fixtures';

// jsdom does not implement ResizeObserver, which reka-ui's SliderRoot (used
// for the scrubber) needs on mount to measure the track.
class ResizeObserverStub {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
}
globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;

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
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 300}
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
        // Gap is 300 -> 1200 (900 ticks, versus the normal 300-tick cadence).
        // At 700 the true age of the nearest frame (300) is 400.
        const wrapper = mount(ReplayScrubber, {
            props: {replay: REALISTIC_REPLAY, manifest: OVERLAPPING_MANIFEST, tick: 700}
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
