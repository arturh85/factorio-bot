// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import CursorBar from './CursorBar.vue';

const scale = {from: 3242, to: 25224};

describe('CursorBar', () => {
    it('shows the tick and the game time, and seeks on input', async () => {
        const w = mount(CursorBar, {props: {scale, cursor: 21242, playing: false, rate: 300}});
        expect(w.text()).toContain('tick 21,242');
        expect(w.text()).toContain('5:00');
        await w.get('input[type="range"]').setValue('4000');
        expect(w.emitted('seek')?.[0]).toEqual([4000]);
    });
    it('toggles play and changes the step', async () => {
        const w = mount(CursorBar, {props: {scale, cursor: 3242, playing: true, rate: 300}});
        expect(w.get('button').text()).toBe('Pause');
        await w.get('button').trigger('click');
        expect(w.emitted('toggle')).toHaveLength(1);
        await w.get('select').setValue('1800');
        expect(w.emitted('rate')?.[0]).toEqual([1800]);
    });
});
