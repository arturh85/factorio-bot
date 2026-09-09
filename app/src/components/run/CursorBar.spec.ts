// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import CursorBar from './CursorBar.vue';

const scale = {from: 3242, to: 25224};

describe('CursorBar', () => {
    it('shows the tick and the game time, and seeks on input', async () => {
        const w = mount(CursorBar, {props: {scale, clock: scale, cursor: 21242, playing: false, rate: 300}});
        expect(w.text()).toContain('tick 21,242');
        expect(w.text()).toContain('5:00');
        await w.get('input[type="range"]').setValue('4000');
        expect(w.emitted('seek')?.[0]).toEqual([4000]);
    });
    it('shows a focus ring on every control, not only the button', () => {
        // Keyboard reach was already there; the ring that says where the
        // keyboard IS was on the button alone.
        const w = mount(CursorBar, {props: {scale, clock: scale, cursor: 3242, playing: false, rate: 300}});
        for (const sel of ['button', 'input[type="range"]', 'select']) {
            expect(w.get(sel).classes()).toContain('focus-visible:ring-focus');
        }
    });
    it('toggles play and changes the step', async () => {
        const w = mount(CursorBar, {props: {scale, clock: scale, cursor: 3242, playing: true, rate: 300}});
        expect(w.get('button').text()).toBe('Pause');
        await w.get('button').trigger('click');
        expect(w.emitted('toggle')).toHaveLength(1);
        await w.get('select').setValue('1800');
        expect(w.emitted('rate')?.[0]).toEqual([1800]);
    });
    it('reads game time off the analysis clock, not the trimmed axis', () => {
        const w = mount(CursorBar, {props: {scale: {from: 3417, to: 25224}, clock: {from: 3242, to: 25224}, cursor: 21242, playing: false, rate: 300}});
        expect(w.text()).toContain('5:00'); // (21242-3242)/60 = 300 s; off the axis it would read 4:57
    });
});
