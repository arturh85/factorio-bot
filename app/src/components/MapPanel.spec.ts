// @vitest-environment jsdom
/**
 * These used to assert that a `<canvas>` element existed and nothing else --
 * jsdom's `getContext('2d')` returns `null` without the optional native
 * `canvas` package, so every number the old component drew was unobservable.
 * The panel is SVG now (see the header comment there for the element counts
 * that justify it), which means the geometry, the tooltip text and the legend
 * are all in the DOM and all asserted below. That is the point of the swap
 * that is worth as much as the readability: the map is now testable.
 *
 * World coordinates throughout: the projection is one `transform` on a group,
 * so a `d` or an `x` here is a game-world number and stays readable.
 */
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {Bounds, EntitySnapshot, MapRecord} from '@/api/types';
import MapPanel from './MapPanel.vue';

const BOUNDS: Bounds = {left: 0, top: 0, right: 48, bottom: 48};

function oreField(name: string, x0: number, y0: number, w: number, h: number): EntitySnapshot[] {
    const out: EntitySnapshot[] = [];
    for (let x = x0; x < x0 + w; x++) for (let y = y0; y < y0 + h; y++) {
        out.push({name, position: {x: x + 0.5, y: y + 0.5}, direction: 0});
    }
    return out;
}

const FURNACE: EntitySnapshot = {name: 'stone-furnace', position: {x: 20, y: 30}, direction: 0};

const RECORDS: MapRecord[] = [
    {tick: 5911, kind: 'placed', bot: 2, intent: FURNACE, actual: FURNACE, drift: null}
];

function mountPanel(overrides: Partial<InstanceType<typeof MapPanel>['$props']> = {}) {
    return mount(MapPanel, {
        props: {
            entities: [...oreField('iron-ore', 4, 4, 6, 5), FURNACE],
            bots: [{id: 1, position: {x: 20, y: 30}}],
            trail: {1: [{x: 2, y: 2}, {x: 10, y: 12}, {x: 20, y: 30}]},
            bounds: BOUNDS,
            records: RECORDS,
            ...overrides
        }
    });
}

describe('MapPanel', () => {
    it('draws 30 ore tiles as ONE outlined patch, not 30 cells', () => {
        const wrapper = mountPanel();
        const paths = wrapper.findAll('path');
        expect(paths).toHaveLength(1);
        // Tile centres 4.5..9.5 / 4.5..8.5 -> tile edges 4..10 / 4..9.
        expect(paths[0].attributes('d')).toBe('M4 4L10 4L10 9L4 9Z');
    });

    it('projects with a single group transform, so the shapes stay in world coordinates', () => {
        const wrapper = mountPanel();
        // 48 world tiles into a 480 viewport: scale 10, no letterboxing.
        expect(wrapper.find('svg > g').attributes('transform')).toBe('translate(0 0) scale(10)');
    });

    it('draws a marker for each non-resource entity, centred on its reported position', () => {
        const wrapper = mountPanel();
        const rect = wrapper.get('[data-testid^="map-feature-entity-"]');
        // A 7px marker at scale 10 is 0.7 tiles, floored to a whole tile so a
        // sub-tile entity is still visible.
        expect(Number(rect.attributes('x'))).toBeCloseTo(19.5);
        expect(Number(rect.attributes('y'))).toBeCloseTo(29.5);
        expect(Number(rect.attributes('width'))).toBeCloseTo(1);
    });

    it('draws each bot as a dot at its own position', () => {
        const wrapper = mountPanel();
        const circle = wrapper.get('[data-testid="map-feature-bot-1"]');
        expect(circle.attributes('cx')).toBe('20');
        expect(circle.attributes('cy')).toBe('30');
    });

    it('draws a trail through every sampled position', () => {
        const wrapper = mountPanel();
        const trail = wrapper.get('[data-testid="map-feature-trail-1"]');
        expect(trail.get('.map-panel__trail-line').attributes('points')).toBe('2,2 10,12 20,30');
    });

    it('gives a trail a hairline to read and a separate fat target to hit', () => {
        // One fat translucent line cannot be both, and drawing it that way
        // smears a band across the map.
        const wrapper = mountPanel();
        const trail = wrapper.get('[data-testid="map-feature-trail-1"]');
        const hit = trail.get('.map-panel__trail-hit');
        expect(hit.attributes('stroke')).toBe('none');
        // Hit-tested regardless of paint -- an unpainted stroke is not
        // `visiblePainted` and would otherwise be untouchable.
        expect(hit.attributes('pointer-events')).toBe('stroke');
        expect(Number(hit.attributes('stroke-width'))).toBeCloseTo(1.6); // 16px at scale 10
        expect(trail.get('.map-panel__trail-line').attributes('pointer-events')).toBe('none');
    });

    it('names each patch on the map, so a colour does not have to be looked up', () => {
        const wrapper = mountPanel();
        const labels = wrapper.findAll('.map-panel__patch-label');
        expect(labels).toHaveLength(1);
        expect(labels[0].text()).toBe('iron-ore');
        // Centred on the patch and not a second tab stop for the same thing.
        expect(labels[0].attributes('x')).toBe('7');
        expect(labels[0].attributes('aria-hidden')).toBe('true');
        expect(labels[0].attributes('pointer-events')).toBe('none');
    });

    it('marks the origin so a trail reads as going somewhere relative to spawn', () => {
        const wrapper = mountPanel();
        expect(wrapper.findAll('.map-panel__origin line')).toHaveLength(2);
    });

    it('reports an empty map rather than drawing an empty viewport', () => {
        // A blank map and "nothing was built yet" look identical, and one of
        // them is a bug.
        const wrapper = mount(MapPanel, {props: {entities: [], bots: [], trail: {}, bounds: null}});
        expect(wrapper.text()).toContain('Nothing placed yet');
        expect(wrapper.find('svg').exists()).toBe(false);
    });

    it('reacts to bounds arriving by swapping the text for the map', async () => {
        const wrapper = mount(MapPanel, {
            props: {entities: [FURNACE], bots: [], trail: {}, bounds: null as Bounds | null}
        });
        expect(wrapper.find('svg').exists()).toBe(false);
        await wrapper.setProps({bounds: BOUNDS});
        expect(wrapper.find('svg').exists()).toBe(true);
        expect(wrapper.text()).not.toContain('Nothing placed yet');
    });

    describe('tooltips', () => {
        it('shows nothing until something is inspected', () => {
            expect(mountPanel().find('[data-testid="map-tooltip"]').exists()).toBe(false);
        });

        it('names an ore patch and its tile count on hover', async () => {
            const wrapper = mountPanel();
            await wrapper.get('path').trigger('pointerenter');
            const tooltip = wrapper.get('[data-testid="map-tooltip"]');
            expect(tooltip.text()).toContain('iron-ore patch');
            expect(tooltip.text()).toContain('30 tiles');
        });

        it('attributes a placed entity to its bot and tick', async () => {
            const wrapper = mountPanel();
            await wrapper.get('[data-testid^="map-feature-entity-"]').trigger('pointerenter');
            expect(wrapper.get('[data-testid="map-tooltip"]').text()).toContain('placed by bot 2 at tick 5911');
        });

        it('says so when no placement was recorded, rather than inventing one', async () => {
            const wrapper = mountPanel({records: []});
            await wrapper.get('[data-testid^="map-feature-entity-"]').trigger('pointerenter');
            expect(wrapper.get('[data-testid="map-tooltip"]').text()).toContain('no placement recorded');
        });

        it('opens on a tap, which is the only gesture a touch screen has', async () => {
            const wrapper = mountPanel();
            await wrapper.get('[data-testid="map-feature-bot-1"]').trigger('click');
            expect(wrapper.get('[data-testid="map-tooltip"]').text()).toContain('bot 1');
        });

        it('opens on keyboard focus, so the map is reachable by Tab alone', async () => {
            const wrapper = mountPanel();
            await wrapper.get('[data-testid="map-feature-trail-1"]').trigger('focus');
            expect(wrapper.get('[data-testid="map-tooltip"]').text()).toContain('bot 1 trail');
        });

        it('makes every shape focusable and named for assistive tech', () => {
            const wrapper = mountPanel();
            const shapes = wrapper.findAll('[data-testid^="map-feature-"]');
            expect(shapes.length).toBe(4); // patch, entity, trail, bot
            for (const shape of shapes) {
                expect(shape.attributes('tabindex')).toBe('0');
                expect(shape.attributes('role')).toBe('button');
                expect(shape.attributes('aria-label')).toBeTruthy();
                // Native hover tooltip as well, for free.
                expect(shape.find('title').exists()).toBe(true);
            }
        });

        it('announces the tooltip rather than only displaying it', async () => {
            const wrapper = mountPanel();
            await wrapper.get('path').trigger('pointerenter');
            expect(wrapper.get('[data-testid="map-tooltip"]').attributes('aria-live')).toBe('polite');
        });

        it('keeps a keyboard-focused tooltip up when the pointer leaves the map', async () => {
            // Focus outranks the pointer: otherwise tabbing to a shape and
            // then nudging the mouse would silently drop what you selected.
            const wrapper = mountPanel();
            await wrapper.get('[data-testid="map-feature-bot-1"]').trigger('focus');
            await wrapper.get('[data-testid="map-svg"]').trigger('pointerleave');
            expect(wrapper.find('[data-testid="map-tooltip"]').exists()).toBe(true);
        });

        it('closes a hover tooltip when the pointer leaves the map', async () => {
            const wrapper = mountPanel();
            await wrapper.get('path').trigger('pointerenter');
            await wrapper.get('[data-testid="map-svg"]').trigger('pointerleave');
            expect(wrapper.find('[data-testid="map-tooltip"]').exists()).toBe(false);
        });

        it('closes when the focused shape is blurred', async () => {
            const wrapper = mountPanel();
            const bot = wrapper.get('[data-testid="map-feature-bot-1"]');
            await bot.trigger('focus');
            await bot.trigger('blur');
            expect(wrapper.find('[data-testid="map-tooltip"]').exists()).toBe(false);
        });

        it('ignores a blur from a shape that is not the focused one', async () => {
            const wrapper = mountPanel();
            await wrapper.get('[data-testid="map-feature-bot-1"]').trigger('focus');
            await wrapper.get('path').trigger('blur');
            expect(wrapper.get('[data-testid="map-tooltip"]').text()).toContain('bot 1');
        });

        it('dismisses on Escape', async () => {
            const wrapper = mountPanel();
            const bot = wrapper.get('[data-testid="map-feature-bot-1"]');
            await bot.trigger('focus');
            await bot.trigger('keydown', {key: 'Escape'});
            expect(wrapper.find('[data-testid="map-tooltip"]').exists()).toBe(false);
        });

        it('anchors the tooltip below a feature near the top edge and above one further down', async () => {
            const wrapper = mount(MapPanel, {
                props: {
                    entities: [],
                    bots: [{id: 1, position: {x: 24, y: 2}}, {id: 2, position: {x: 24, y: 40}}],
                    trail: {},
                    bounds: BOUNDS
                }
            });
            await wrapper.get('[data-testid="map-feature-bot-1"]').trigger('pointerenter');
            expect(wrapper.get('[data-testid="map-tooltip"]').attributes('style')).toContain('translate(-50%, 1.25rem)');
            await wrapper.get('[data-testid="map-feature-bot-2"]').trigger('pointerenter');
            expect(wrapper.get('[data-testid="map-tooltip"]').attributes('style')).toContain('calc(-100% - 1.25rem)');
        });

        it('keeps the tooltip clear of the left and right edges', async () => {
            const wrapper = mount(MapPanel, {
                props: {
                    entities: [],
                    bots: [{id: 1, position: {x: 0, y: 24}}],
                    trail: {},
                    bounds: BOUNDS
                }
            });
            await wrapper.get('[data-testid="map-feature-bot-1"]').trigger('pointerenter');
            expect(wrapper.get('[data-testid="map-tooltip"]').attributes('style')).toContain('left: 15%');
        });
    });

    describe('legend', () => {
        it('lists one row per thing on the map, with the count it stands for', () => {
            const wrapper = mountPanel();
            const legend = wrapper.get('[data-testid="map-legend"]');
            expect(legend.text()).toContain('iron-ore');
            expect(legend.text()).toContain('30 tiles in 1 patch');
            expect(legend.text()).toContain('stone-furnace');
            expect(legend.text()).toContain('1 on map');
            expect(legend.text()).toContain('bot 1');
        });

        it('paints each swatch in the colour of the shape it stands for', () => {
            const wrapper = mountPanel();
            const swatch = wrapper.get('[data-testid="legend-resource:iron-ore"] .legend__swatch');
            const patchFill = wrapper.get('path').attributes('fill');
            expect(swatch.attributes('style')).toContain(patchFill);
        });

        it('distinguishes rows by shape as well as by colour', () => {
            const wrapper = mountPanel();
            expect(wrapper.find('[data-testid="legend-resource:iron-ore"] .legend__swatch--patch').exists()).toBe(true);
            expect(wrapper.find('[data-testid="legend-entity:stone-furnace"] .legend__swatch--entity').exists()).toBe(true);
            expect(wrapper.find('[data-testid="legend-bot:1"] .legend__swatch--bot').exists()).toBe(true);
        });

        it('renders no legend for a map with nothing on it', () => {
            const wrapper = mount(MapPanel, {props: {entities: [], bots: [], trail: {}, bounds: BOUNDS}});
            expect(wrapper.find('[data-testid="map-legend"]').exists()).toBe(false);
        });
    });
});
