// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Card from './Card.vue';

describe('Card', () => {
    it('renders the title prop as the card heading', () => {
        const wrapper = mount(Card, {props: {title: 'Seed'}, slots: {default: '<p>body</p>'}});
        expect(wrapper.find('h2').text()).toBe('Seed');
        expect(wrapper.text()).toContain('body');
    });

    it('renders no heading at all when there is no title', () => {
        const wrapper = mount(Card, {slots: {default: '<p>body</p>'}});
        expect(wrapper.find('h2').exists()).toBe(false);
    });

    it('lets the title slot carry markup the prop could not', () => {
        const wrapper = mount(Card, {
            props: {title: 'ignored'},
            slots: {title: '<a href="https://factorio.com/download">download</a>'}
        });
        expect(wrapper.find('h2 a').attributes('href')).toBe('https://factorio.com/download');
        expect(wrapper.text()).not.toContain('ignored');
    });
});
