// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Checkbox from './Checkbox.vue';

describe('Checkbox', () => {
    it('reports the model value through aria-checked', () => {
        const wrapper = mount(Checkbox, {props: {modelValue: true, label: 'Recreate Level'}});
        expect(wrapper.find('[role="checkbox"]').attributes('aria-checked')).toBe('true');
    });

    it('emits the flipped value when the box is clicked', async () => {
        const wrapper = mount(Checkbox, {props: {modelValue: false, label: 'Recreate Level'}});
        await wrapper.find('[role="checkbox"]').trigger('click');
        expect(wrapper.emitted('update:modelValue')).toEqual([[true]]);
    });

    it('wires the label to the control so clicking the text toggles it', () => {
        const wrapper = mount(Checkbox, {props: {modelValue: false, label: 'Enable Autostart'}});
        const label = wrapper.find('label');
        expect(label.text()).toBe('Enable Autostart');
        expect(label.attributes('for')).toBe(wrapper.find('[role="checkbox"]').attributes('id'));
        expect(label.attributes('for')).toBeTruthy();
    });
});
