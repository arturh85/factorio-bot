// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Input from './Input.vue';

describe('Input', () => {
    it('renders the model value into the field', () => {
        const wrapper = mount(Input, {props: {modelValue: '/tmp/workspace'}});
        expect((wrapper.element as HTMLInputElement).value).toBe('/tmp/workspace');
    });

    it('emits what the user typed', async () => {
        const wrapper = mount(Input, {props: {modelValue: ''}});
        await wrapper.setValue('abc');
        expect(wrapper.emitted('update:modelValue')).toEqual([['abc']]);
    });

    it('marks itself invalid for assistive tech and paints the danger border', () => {
        const wrapper = mount(Input, {props: {modelValue: '/nope', invalid: true}});
        expect(wrapper.attributes('aria-invalid')).toBe('true');
        expect(wrapper.classes()).toContain('border-danger');
        expect(wrapper.classes()).not.toContain('border-divider');
    });

    it('carries no aria-invalid when valid', () => {
        const wrapper = mount(Input, {props: {modelValue: '/tmp'}});
        expect(wrapper.attributes('aria-invalid')).toBeUndefined();
        expect(wrapper.classes()).toContain('border-divider');
    });

    it('passes native attributes through to the input element', () => {
        const wrapper = mount(Input, {props: {modelValue: '7492'}, attrs: {type: 'number', max: '65535'}});
        expect(wrapper.attributes('type')).toBe('number');
        expect(wrapper.attributes('max')).toBe('65535');
    });
});
