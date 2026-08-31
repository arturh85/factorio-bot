// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import ReplayStepRow from './ReplayStepRow.vue';
import {REALISTIC_REPLAY} from '@/api/replay.fixtures';
import {ReplayStep} from '@/api/replay';

const AXIS_CEILING = 400;

/** Steps from the shared fixture, addressed by what makes each one interesting. */
const FAILED_STEP = REALISTIC_REPLAY.steps[2]; // Failed, attempt 3, half-observed
const LOST_STEP = REALISTIC_REPLAY.steps[3]; // Lost
const PENDING_STEP = REALISTIC_REPLAY.steps[4]; // Pending, both ticks null
const BELIEVED_STEP = REALISTIC_REPLAY.steps[0]; // walk, believed, fully observed
const MEASURED_SUCCESS_STEP = REALISTIC_REPLAY.steps[1]; // act, measured, fully observed
const UNOBSERVED_SUCCESS_STEP = REALISTIC_REPLAY.steps[5]; // Success, both ticks null

function renderRow(step: ReplayStep) {
    return mount(ReplayStepRow, {props: {step, axisCeiling: AXIS_CEILING}});
}

/**
 * The element carrying a row's observation, whichever shape it took: a full
 * bar when both ticks are known, or a marker when only one is. Both fixture
 * steps used below (`FAILED_STEP`, `LOST_STEP`) are half-observed -- the
 * dispatch is known, the reply never arrived -- which is a real, common shape
 * for both statuses and not something these tests should have to special-case.
 */
function observedElement(wrapper: ReturnType<typeof renderRow>) {
    const bar = wrapper.find('[data-testid="observed-bar"]');
    return bar.exists() ? bar : wrapper.get('[data-testid="observed-marker"]');
}

describe('ReplayStepRow -- Lost vs Failed', () => {
    it('gives Lost and Failed rows different accessible labels', () => {
        const failed = renderRow(FAILED_STEP);
        const lost = renderRow(LOST_STEP);

        const failedLabel = observedElement(failed).attributes('aria-label');
        const lostLabel = observedElement(lost).attributes('aria-label');

        expect(failedLabel).toBeTruthy();
        expect(lostLabel).toBeTruthy();
        expect(failedLabel).not.toBe(lostLabel);
    });

    it('gives Lost and Failed rows different visual treatment (class and status attribute)', () => {
        const failed = renderRow(FAILED_STEP);
        const lost = renderRow(LOST_STEP);

        const failedBar = observedElement(failed);
        const lostBar = observedElement(lost);

        expect(failedBar.attributes('data-status')).toBe('Failed');
        expect(lostBar.attributes('data-status')).toBe('Lost');
        expect(failedBar.classes().join(' ')).not.toBe(lostBar.classes().join(' '));
    });

    it('Failed carries the error message; Lost carries no error', () => {
        const failed = renderRow(FAILED_STEP);
        const lost = renderRow(LOST_STEP);

        expect(failed.text()).toContain('not enough iron-plate');
        expect(lost.text()).not.toContain('not enough iron-plate');
    });
});

describe('ReplayStepRow -- absent observation draws nothing, not a zero-length bar', () => {
    it('draws no observed bar at all for a Pending step with both ticks null', () => {
        const wrapper = renderRow(PENDING_STEP);
        expect(wrapper.find('[data-testid="observed-bar"]').exists()).toBe(false);
    });

    it('still draws the planned bar for a step that was never observed', () => {
        const wrapper = renderRow(PENDING_STEP);
        expect(wrapper.find('[data-testid="planned-bar"]').exists()).toBe(true);
    });

    it('draws no observed bar for a Success step with no clock at all', () => {
        const wrapper = renderRow(UNOBSERVED_SUCCESS_STEP);
        expect(wrapper.find('[data-testid="observed-bar"]').exists()).toBe(false);
        // The absence must not be silently dropped either -- something must
        // tell a reader this step succeeded with no timing at all.
        expect(wrapper.text().toLowerCase()).toMatch(/no (clock|timing|measurement)/);
    });

    it('draws a partial marker, not a full bar, when only the dispatch tick is known', () => {
        // FAILED_STEP: observed_start_tick=340, observed_end_tick=null.
        const wrapper = renderRow(FAILED_STEP);
        expect(wrapper.find('[data-testid="observed-bar"]').exists()).toBe(false);
        expect(wrapper.find('[data-testid="observed-marker"]').exists()).toBe(true);
    });
});

describe('ReplayStepRow -- evidence', () => {
    it('surfaces the believed reason as visible/attached text for a walk row', () => {
        const wrapper = renderRow(BELIEVED_STEP);
        const mark = wrapper.get('[data-testid="evidence-mark"]');
        expect(mark.attributes('title')).toContain('arrival is not');
    });

    it('renders no evidence caveat for a measured row', () => {
        const wrapper = renderRow(MEASURED_SUCCESS_STEP);
        expect(wrapper.find('[data-testid="evidence-mark"]').exists()).toBe(false);
    });

    /**
     * `Believed` is an epistemics label on an otherwise normal row, not an
     * outcome. A walk row is `Believed` on every single walk, which is the
     * whole reason it must read as quiet and informational rather than as an
     * alarm: styled like a failure, the common case would look like it is
     * constantly going wrong. `Failed` and `Lost` are outcomes and are
     * allowed to draw the eye; this must not borrow their colour to do it.
     */
    it('does not style the believed marker with the same classes as a Failed or Lost observed bar', () => {
        const believed = renderRow(BELIEVED_STEP); // Success, but Believed
        const failed = renderRow(FAILED_STEP);
        const lost = renderRow(LOST_STEP);

        const believedClasses = believed.get('[data-testid="evidence-mark"]').classes();
        const failedClasses = observedElement(failed).classes();
        const lostClasses = observedElement(lost).classes();

        for (const alarmClass of [...failedClasses, ...lostClasses]) {
            expect(believedClasses).not.toContain(alarmClass);
        }
        // Not just "different from the alarms" -- must not use the alarm
        // palette tokens at all, including ones neither row above happens to
        // use.
        expect(believedClasses.join(' ')).not.toMatch(/danger|warn/);
    });

    it('still colours a Believed row by its own status -- evidence does not override the outcome', () => {
        // BELIEVED_STEP succeeded and is fully observed; it must still show a
        // normal success bar underneath the quiet evidence marker.
        const wrapper = renderRow(BELIEVED_STEP);
        const bar = wrapper.get('[data-testid="observed-bar"]');
        expect(bar.attributes('data-status')).toBe('Success');
    });
});

describe('ReplayStepRow -- retries', () => {
    it('shows the attempt number when a step was retried', () => {
        const wrapper = renderRow(FAILED_STEP);
        expect(wrapper.text()).toContain('3');
        expect(wrapper.find('[data-testid="attempt-count"]').exists()).toBe(true);
    });

    it('shows no attempt marker for a first-attempt success', () => {
        const wrapper = renderRow(MEASURED_SUCCESS_STEP);
        expect(wrapper.find('[data-testid="attempt-count"]').exists()).toBe(false);
    });
});
