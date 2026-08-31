// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import ReplayView from './ReplayView.vue';
import {STATUS_VISUAL} from './statusVisual';
import {ReplayStatus} from '@/api/replay';
import {
    ALL_PENDING_ATTEMPTED_REPLAY,
    CIRCULAR_REFUSAL,
    REALISTIC_REPLAY,
    REFUSED_REPLAY
} from '@/api/replay.fixtures';

describe('ReplayView -- refused vs. attempted-but-unmeasured', () => {
    /**
     * The distinction `Replay::refused` exists for. Both documents have every
     * row `Pending` with every tick `null` -- byte-for-byte identical rows --
     * and mean opposite things: one never started, the other started and
     * measured nothing. A view that captions them the same way, or leaves one
     * uncaptioned, has thrown away the one field that tells them apart.
     */
    it('captions a refused run differently from an attempted run that measured nothing', () => {
        const refused = mount(ReplayView, {props: {replay: REFUSED_REPLAY}});
        const attempted = mount(ReplayView, {props: {replay: ALL_PENDING_ATTEMPTED_REPLAY}});

        const refusedCaption = refused.find('[data-testid="refused-banner"]');
        const attemptedCaption = attempted.find('[data-testid="attempted-no-data-banner"]');

        expect(refusedCaption.exists()).toBe(true);
        expect(attemptedCaption.exists()).toBe(true);
        // Not the same banner rendered twice under different names.
        expect(refused.find('[data-testid="attempted-no-data-banner"]').exists()).toBe(false);
        expect(attempted.find('[data-testid="refused-banner"]').exists()).toBe(false);
        expect(refusedCaption.text()).not.toBe(attemptedCaption.text());
    });

    it('shows the refusal reason verbatim, not a generic message', () => {
        const wrapper = mount(ReplayView, {props: {replay: REFUSED_REPLAY}});
        expect(wrapper.text()).toContain(CIRCULAR_REFUSAL);
    });

    it('greys the plan when refused, but not when merely unmeasured', () => {
        const refused = mount(ReplayView, {props: {replay: REFUSED_REPLAY}});
        const attempted = mount(ReplayView, {props: {replay: ALL_PENDING_ATTEMPTED_REPLAY}});

        const refusedPlan = refused.get('[data-testid="step-list"]');
        const attemptedPlan = attempted.get('[data-testid="step-list"]');
        expect(refusedPlan.classes().join(' ')).not.toBe(attemptedPlan.classes().join(' '));
    });

    it('renders no caption at all for a normal, partly-measured run', () => {
        const wrapper = mount(ReplayView, {props: {replay: REALISTIC_REPLAY}});
        expect(wrapper.find('[data-testid="refused-banner"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="attempted-no-data-banner"]').exists()).toBe(false);
    });
});

describe('ReplayView -- legend and unmatched walks', () => {
    it('always shows the legend note about internal move-asides, even on a normal run', () => {
        const wrapper = mount(ReplayView, {props: {replay: REALISTIC_REPLAY}});
        expect(wrapper.find('[data-testid="legend-note"]').exists()).toBe(true);
    });

    it('shows the legend note on a refused run too', () => {
        const wrapper = mount(ReplayView, {props: {replay: REFUSED_REPLAY}});
        expect(wrapper.find('[data-testid="legend-note"]').exists()).toBe(true);
    });

    it('reports unmatched walks rather than hiding them', () => {
        // REALISTIC_REPLAY carries a real unmatched walk from the tracked fixture.
        expect(REALISTIC_REPLAY.unmatched_walks.length).toBeGreaterThan(0);
        const wrapper = mount(ReplayView, {props: {replay: REALISTIC_REPLAY}});
        const banner = wrapper.get('[data-testid="unmatched-walks-banner"]');
        for (const walk of REALISTIC_REPLAY.unmatched_walks) {
            expect(banner.text()).toContain(String(walk.bot));
        }
    });

    it('shows no unmatched-walks banner when there are none', () => {
        const wrapper = mount(ReplayView, {props: {replay: ALL_PENDING_ATTEMPTED_REPLAY}});
        expect(wrapper.find('[data-testid="unmatched-walks-banner"]').exists()).toBe(false);
    });
});

describe('ReplayView -- steps and axis', () => {
    it('renders one row per step, in schedule order', () => {
        const wrapper = mount(ReplayView, {props: {replay: REALISTIC_REPLAY}});
        expect(wrapper.findAllComponents({name: 'ReplayStepRow'})).toHaveLength(REALISTIC_REPLAY.steps.length);
    });

    it('labels the time axis as ticks, never as a duration or makespan', () => {
        const wrapper = mount(ReplayView, {props: {replay: REALISTIC_REPLAY}});
        const axis = wrapper.get('[data-testid="axis-label"]');
        expect(axis.text().toLowerCase()).toContain('tick');
        expect(axis.text().toLowerCase()).not.toMatch(/makespan|duration/);
    });
});

describe('ReplayView -- no document yet', () => {
    it('shows a neutral empty state when there is no replay at all', () => {
        const wrapper = mount(ReplayView, {props: {replay: null}});
        expect(wrapper.find('[data-testid="step-list"]').exists()).toBe(false);
        expect(wrapper.text().length).toBeGreaterThan(0);
    });

    it('surfaces a parse error distinctly, when one is given', () => {
        const wrapper = mount(ReplayView, {props: {replay: null, parseError: 'replay document invalid at $.steps[0].status'}});
        expect(wrapper.text()).toContain('$.steps[0].status');
    });

    /**
     * The legend is derived from `STATUS_VISUAL`, and this asserts the
     * derivation rather than the contents: every status that has a visual
     * must appear, so adding one to the table and forgetting the key is a
     * failure here. Written against the table's own keys, not a hand-written
     * list -- a list would be the mirror the derivation exists to avoid.
     */
    it('keys every status that has a visual, without restating them', () => {
        const wrapper = mount(ReplayView, {props: {replay: REALISTIC_REPLAY}});
        const legend = wrapper.get('[data-testid="status-legend"]');

        // Iterating STATUS_VISUAL, not LEGEND_ENTRIES. Iterating the latter
        // would derive both sides of this check from the same source, so
        // dropping a status from the legend would also drop it from what the
        // test looks for and the test would still pass. That is the exact
        // defect this project documented hours before this test was written,
        // and the first draft of it had the bug.
        const withVisuals = (Object.entries(STATUS_VISUAL) as [ReplayStatus, {label: string}][])
            .filter(([status]) => status !== 'Pending');

        for (const [status, entry] of withVisuals) {
            expect(
                wrapper.find(`[data-testid="legend-${status}"]`).exists(),
                `${status} has a visual but no legend entry`
            ).toBe(true);
            expect(legend.text()).toContain(entry.label);
        }
        // Pending is deliberately absent: it draws no observed bar, so a
        // swatch for it would be an empty box beside the word "pending".
        expect(wrapper.find('[data-testid="legend-Pending"]').exists()).toBe(false);
    });
});
