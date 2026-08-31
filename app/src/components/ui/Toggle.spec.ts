// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Toggle from './Toggle.vue';

function mountToggle(modelValue: boolean) {
    return mount(Toggle, {props: {modelValue, onLabel: 'Recreate Level', offLabel: 'Use existing Level'}});
}

describe('Toggle', () => {
    it('shows the off label and reports aria-pressed false when unset', () => {
        const wrapper = mountToggle(false);
        expect(wrapper.text()).toBe('Use existing Level');
        expect(wrapper.attributes('aria-pressed')).toBe('false');
    });

    it('shows the on label and reports aria-pressed true when set', () => {
        const wrapper = mountToggle(true);
        expect(wrapper.text()).toBe('Recreate Level');
        expect(wrapper.attributes('aria-pressed')).toBe('true');
    });

    it('emits the flipped value when clicked', async () => {
        const wrapper = mountToggle(false);
        await wrapper.trigger('click');
        expect(wrapper.emitted('update:modelValue')).toEqual([[true]]);
    });

    it('emits false when clicked while set', async () => {
        const wrapper = mountToggle(true);
        await wrapper.trigger('click');
        expect(wrapper.emitted('update:modelValue')).toEqual([[false]]);
    });
});
