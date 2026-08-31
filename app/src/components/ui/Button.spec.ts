// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Button from './Button.vue';

describe('Button', () => {
    it('renders its slot inside a non-submitting button', () => {
        const wrapper = mount(Button, {slots: {default: 'Run'}});
        expect(wrapper.element.tagName).toBe('BUTTON');
        // Without an explicit type, a button inside a form submits it.
        expect(wrapper.attributes('type')).toBe('button');
        expect(wrapper.text()).toBe('Run');
    });

    it('paints the danger variant and not the primary one', () => {
        const wrapper = mount(Button, {props: {variant: 'danger'}, slots: {default: 'Stop'}});
        expect(wrapper.classes()).toContain('bg-danger');
        expect(wrapper.classes()).not.toContain('bg-brand');
    });

    it('lets a caller-supplied background override the variant background', () => {
        const wrapper = mount(Button, {props: {variant: 'primary', class: 'bg-success'}});
        // Concatenating instead of merging leaves both, and which one wins
        // then depends on Tailwind's emit order rather than on the caller.
        expect(wrapper.classes()).toContain('bg-success');
        expect(wrapper.classes()).not.toContain('bg-brand');
    });

    it('carries the disabled attribute and swallows the click when disabled', async () => {
        const wrapper = mount(Button, {props: {disabled: true}, slots: {default: 'Running ...'}});
        expect(wrapper.attributes('disabled')).toBeDefined();
        await wrapper.trigger('click');
        expect(wrapper.emitted('click')).toBeUndefined();
    });

    it('emits click when enabled', async () => {
        const wrapper = mount(Button, {slots: {default: 'Run'}});
        await wrapper.trigger('click');
        expect(wrapper.emitted('click')).toHaveLength(1);
    });
});
