// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Toaster from './Toaster.vue';
import {useToast} from '@/composables/useToast';

beforeEach(() => useToast().clear());
afterEach(() => useToast().clear());

describe('Toaster', () => {
    it('renders one panel per queued message, newest last', async () => {
        const wrapper = mount(Toaster);
        useToast().add({summary: 'first', life: 0});
        useToast().add({summary: 'second', detail: 'with detail', life: 0});
        await wrapper.vm.$nextTick();

        const toasts = wrapper.findAll('[data-testid="toast"]');
        expect(toasts.length).toBe(2);
        expect(toasts[1].text()).toContain('with detail');
    });

    it('colours an error differently from a success', async () => {
        const wrapper = mount(Toaster);
        useToast().add({severity: 'error', summary: 'boom', life: 0});
        await wrapper.vm.$nextTick();
        expect(wrapper.find('[data-testid="toast"]').classes()).toContain('border-danger');
        expect(wrapper.find('[data-testid="toast"]').classes()).not.toContain('border-success');
    });

    it('drops the message whose dismiss button is pressed', async () => {
        const wrapper = mount(Toaster);
        useToast().add({summary: 'first', life: 0});
        useToast().add({summary: 'second', life: 0});
        await wrapper.vm.$nextTick();

        await wrapper.findAll('[aria-label="Dismiss"]')[0].trigger('click');
        const toasts = wrapper.findAll('[data-testid="toast"]');
        expect(toasts.length).toBe(1);
        expect(toasts[0].text()).toContain('second');
    });

    it('announces politely so a screen reader reads new messages', () => {
        const wrapper = mount(Toaster);
        expect(wrapper.attributes('aria-live')).toBe('polite');
    });
});
