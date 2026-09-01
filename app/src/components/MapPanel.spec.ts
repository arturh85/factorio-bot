// @vitest-environment jsdom
/**
 * jsdom has no real 2D canvas context (`getContext('2d')` returns `null`
 * without the optional native `canvas` package), so these assert on
 * structure -- a `<canvas>` exists, or the empty-state text renders -- not
 * on anything drawn. The coordinate transform that would otherwise need
 * pixel assertions lives in `@/lib/mapProjection.ts` and is tested there.
 */
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import MapPanel from './MapPanel.vue';

describe('MapPanel', () => {
    it('renders a canvas sized to the bounds it is given', () => {
        const wrapper = mount(MapPanel, {
            props: {
                entities: [{name: 'stone-furnace', position: {x: -12, y: 8}, direction: 0}],
                bots: [{id: 1, position: {x: 0, y: 0}}],
                trail: {1: [{x: -1, y: 0}, {x: 0, y: 0}]},
                bounds: {left: -64, top: -64, right: 64, bottom: 64}
            }
        });
        expect(wrapper.find('canvas').exists()).toBe(true);
    });

    it('reports an empty map rather than drawing an empty canvas', () => {
        // A blank canvas and "nothing was built yet" look identical, and one
        // of them is a bug.
        const wrapper = mount(MapPanel, {
            props: {entities: [], bots: [], trail: {}, bounds: null}
        });
        expect(wrapper.text()).toContain('Nothing placed yet');
    });

    it('does not render a canvas when there are no bounds', () => {
        const wrapper = mount(MapPanel, {
            props: {entities: [], bots: [], trail: {}, bounds: null}
        });
        expect(wrapper.find('canvas').exists()).toBe(false);
    });

    it('does not throw when the map has entities, bots and trails but no null-context canvas to draw with', () => {
        // jsdom's getContext('2d') returns null; the component must tolerate
        // that rather than crash on a null context.
        expect(() =>
            mount(MapPanel, {
                props: {
                    entities: [
                        {name: 'stone-furnace', position: {x: -12, y: 8}, direction: 0},
                        {name: 'transport-belt', position: {x: -40.5, y: -48.5}, direction: 4}
                    ],
                    bots: [{id: 1, position: {x: 0, y: 0}}, {id: 2, position: {x: 3, y: 3}}],
                    trail: {1: [{x: -1, y: 0}, {x: 0, y: 0}], 2: [{x: 3, y: 3}]},
                    bounds: {left: -64, top: -64, right: 64, bottom: 64}
                }
            })
        ).not.toThrow();
    });

    it('reacts to bounds changing from null to set by swapping text for canvas', async () => {
        const wrapper = mount(MapPanel, {
            props: {entities: [], bots: [], trail: {}, bounds: null as null | {left: number; top: number; right: number; bottom: number}}
        });
        expect(wrapper.find('canvas').exists()).toBe(false);
        await wrapper.setProps({bounds: {left: -8, top: -8, right: 8, bottom: 8}});
        expect(wrapper.find('canvas').exists()).toBe(true);
        expect(wrapper.text()).not.toContain('Nothing placed yet');
    });
});
