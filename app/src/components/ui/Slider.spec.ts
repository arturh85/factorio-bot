// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {SliderRoot} from 'reka-ui';
import Slider from './Slider.vue';

import '@/test/resizeObserverStub';

describe('Slider', () => {
    it('hands reka-ui the single-element array it expects', () => {
        const wrapper = mount(Slider, {props: {modelValue: 4, min: 0, max: 16, label: 'Client Instances'}});
        expect(wrapper.findComponent(SliderRoot).props('modelValue')).toEqual([4]);
    });

    it('unwraps reka-ui array updates back to a number', async () => {
        const wrapper = mount(Slider, {props: {modelValue: 4, min: 0, max: 16}});
        await wrapper.findComponent(SliderRoot).vm.$emit('update:modelValue', [9]);
        // Forwarding the array unchanged would emit [[9]] here.
        expect(wrapper.emitted('update:modelValue')).toEqual([[9]]);
    });

    it('renders a focusable thumb with the slider role', () => {
        const wrapper = mount(Slider, {props: {modelValue: 4, min: 0, max: 16}});
        expect(wrapper.find('[role="slider"]').exists()).toBe(true);
    });

    it('passes the bounds through to reka-ui', () => {
        const wrapper = mount(Slider, {props: {modelValue: 4, min: 2, max: 16, step: 2}});
        expect(wrapper.findComponent(SliderRoot).props('min')).toBe(2);
        expect(wrapper.findComponent(SliderRoot).props('max')).toBe(16);
        expect(wrapper.findComponent(SliderRoot).props('step')).toBe(2);
    });
});
