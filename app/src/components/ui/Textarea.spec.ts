// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Textarea from './Textarea.vue';

describe('Textarea', () => {
    it('renders the model value', () => {
        const wrapper = mount(Textarea, {props: {modelValue: '/server-save'}});
        expect(wrapper.element.tagName).toBe('TEXTAREA');
        expect((wrapper.element as HTMLTextAreaElement).value).toBe('/server-save');
    });

    it('emits what the user typed', async () => {
        const wrapper = mount(Textarea, {props: {modelValue: ''}});
        await wrapper.setValue('/c game.print(1)');
        expect(wrapper.emitted('update:modelValue')).toEqual([['/c game.print(1)']]);
    });

    it('merges a caller height over its own', () => {
        const wrapper = mount(Textarea, {props: {modelValue: '', class: 'min-h-64'}});
        expect(wrapper.classes()).toContain('min-h-64');
        expect(wrapper.classes()).not.toContain('min-h-24');
    });
});
