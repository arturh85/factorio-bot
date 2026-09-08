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

/**
 * The map's own SVG.
 *
 * Every shape lookup goes through this, because the camera control's icons
 * are SVGs with `<path>`s of their own and a bare `find('path')` picks one of
 * those up instead of an ore patch.
 */
const MAP = '[data-testid="map-svg"]';

/**
 * What the camera frames for `BOUNDS` with no bot outside it: 0..48 padded by
 * the 32-tile lead is -32..80, a 112-tile square. Every projected number in
 * this file is derived from that, not from `BOUNDS` -- the panel does not
 * frame the keyframe's entity bounds any more, which is the bug that made a
 * bot walking away disappear.
 */
const FRAME_SPAN = 112;
const SCALE = 480 / FRAME_SPAN;

/** The group transform the panel applied, parsed back into numbers. */
function projectionOf(wrapper: ReturnType<typeof mountPanel>) {
    const transform = wrapper.get(`${MAP} > g`).attributes('transform') ?? '';
    const parsed = /translate\((-?[\d.]+) (-?[\d.]+)\) scale\(([\d.]+)\)/.exec(transform);
    if (parsed === null) throw new Error(`unparsable transform: ${transform}`);
    return {offsetX: Number(parsed[1]), offsetY: Number(parsed[2]), scale: Number(parsed[3])};
}

/** Where a world position actually lands in the 480x480 viewport. */
function onScreen(wrapper: ReturnType<typeof mountPanel>, position: {x: number; y: number}) {
    const {offsetX, offsetY, scale} = projectionOf(wrapper);
    return {x: position.x * scale + offsetX, y: position.y * scale + offsetY};
}

function isVisible(point: {x: number; y: number}): boolean {
    return point.x >= 0 && point.x <= 480 && point.y >= 0 && point.y <= 480;
}

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
        const paths = wrapper.findAll(`${MAP} path`);
        expect(paths).toHaveLength(1);
        // Tile centres 4.5..9.5 / 4.5..8.5 -> tile edges 4..10 / 4..9.
        expect(paths[0].attributes('d')).toBe('M4 4L10 4L10 9L4 9Z');
    });

    it('projects with a single group transform, so the shapes stay in world coordinates', () => {
        const wrapper = mountPanel();
        // The camera's 112-tile square into a 480 viewport, with world -32
        // landing on viewport 0. Square frame, so no letterboxing.
        expect(wrapper.get(`${MAP} > g`).attributes('transform')).toBe(
            `translate(${32 * SCALE} ${32 * SCALE}) scale(${SCALE})`
        );
    });

    it('draws a marker for each non-resource entity, centred on its reported position', () => {
        const wrapper = mountPanel();
        const rect = wrapper.get('[data-testid^="map-feature-entity-"]');
        // A 7px marker at this scale is 1.63 tiles: markers are sized in
        // screen pixels, which is why pulling the camera out does not turn
        // them into specks.
        const side = 7 / SCALE;
        expect(Number(rect.attributes('width'))).toBeCloseTo(side);
        expect(Number(rect.attributes('x'))).toBeCloseTo(20 - side / 2);
        expect(Number(rect.attributes('y'))).toBeCloseTo(30 - side / 2);
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
        expect(Number(hit.attributes('stroke-width'))).toBeCloseTo(16 / SCALE); // 16 screen px
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
        expect(wrapper.find(MAP).exists()).toBe(false);
    });

    it('reacts to bounds arriving by swapping the text for the map', async () => {
        const wrapper = mount(MapPanel, {
            props: {entities: [FURNACE], bots: [], trail: {}, bounds: null as Bounds | null}
        });
        expect(wrapper.find(MAP).exists()).toBe(false);
        await wrapper.setProps({bounds: BOUNDS});
        expect(wrapper.find(MAP).exists()).toBe(true);
        expect(wrapper.text()).not.toContain('Nothing placed yet');
    });

    describe('tooltips', () => {
        it('shows nothing until something is inspected', () => {
            expect(mountPanel().find('[data-testid="map-tooltip"]').exists()).toBe(false);
        });

        it('names an ore patch and its tile count on hover', async () => {
            const wrapper = mountPanel();
            await wrapper.get(`${MAP} path`).trigger('pointerenter');
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
            await wrapper.get(`${MAP} path`).trigger('pointerenter');
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
            await wrapper.get(`${MAP} path`).trigger('pointerenter');
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
            await wrapper.get(`${MAP} path`).trigger('blur');
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
            // The bot has to be near the frame's edge for the clamp to do
            // anything, and the camera keeps content 32 tiles clear of the
            // edge whenever it re-frames -- so this is a bot that has walked
            // to x = -24 inside a frame the camera decided not to move.
            const wrapper = mount(MapPanel, {
                props: {
                    entities: [],
                    bots: [{id: 1, position: {x: 10, y: 24}}],
                    trail: {},
                    bounds: BOUNDS
                }
            });
            await wrapper.setProps({bots: [{id: 1, position: {x: -24, y: 24}}]});
            await wrapper.get('[data-testid="map-feature-bot-1"]').trigger('pointerenter');
            expect(wrapper.get('[data-testid="map-tooltip"]').attributes('style')).toContain('left: 15%');
        });
    });

    describe('camera', () => {
        it('keeps a bot in view after it walks outside the built area', () => {
            // The complaint this exists for: the keyframe's bounds are 0..48,
            // and framing them put this bot off the picture entirely, so the
            // map looked frozen while the run was at its busiest.
            const far = {x: 200, y: -20};
            const wrapper = mount(MapPanel, {
                props: {entities: [FURNACE], bots: [{id: 1, position: far}], trail: {}, bounds: BOUNDS}
            });
            expect(isVisible(onScreen(wrapper, far))).toBe(true);
            // ...and fit mode still shows the base it walked away from.
            expect(isVisible(onScreen(wrapper, {x: 20, y: 30}))).toBe(true);
        });

        it('keeps a trail that leads off the built area in view too', () => {
            const wrapper = mount(MapPanel, {
                props: {
                    entities: [FURNACE],
                    bots: [{id: 1, position: {x: 20, y: 30}}],
                    trail: {1: [{x: -160, y: -40}, {x: 20, y: 30}]},
                    bounds: BOUNDS
                }
            });
            expect(isVisible(onScreen(wrapper, {x: -160, y: -40}))).toBe(true);
        });

        it('does not move the picture while a bot walks about inside the frame', async () => {
            // Re-deriving a tight box every cursor tick is what makes a map
            // twitch continuously while playing, which is worse than a still
            // frame. Nothing left the frame here, so nothing moves.
            const wrapper = mountPanel();
            const before = projectionOf(wrapper);
            await wrapper.setProps({bots: [{id: 1, position: {x: 40, y: 44}}]});
            expect(projectionOf(wrapper)).toEqual(before);
        });

        it('re-frames once the bot reaches the edge', async () => {
            const wrapper = mountPanel();
            const before = projectionOf(wrapper);
            const edge = {x: 76, y: 10};
            await wrapper.setProps({bots: [{id: 1, position: edge}]});
            expect(projectionOf(wrapper).scale).not.toBe(before.scale);
            expect(isVisible(onScreen(wrapper, edge))).toBe(true);
        });

        it('offers both framings as buttons, each reachable by Tab', () => {
            const wrapper = mountPanel();
            const group = wrapper.get('[role="group"]');
            expect(group.attributes('aria-label')).toBe('Map camera');
            const fit = wrapper.get('[data-testid="camera-fit"]');
            const follow = wrapper.get('[data-testid="camera-follow"]');
            // Native buttons: focusable and activatable by Enter and Space
            // without this component implementing a key handler at all.
            expect(fit.element.tagName).toBe('BUTTON');
            expect(follow.element.tagName).toBe('BUTTON');
            expect(fit.attributes('aria-pressed')).toBe('true');
            expect(follow.attributes('aria-pressed')).toBe('false');
        });

        it('zooms to the bots, dropping the base, when the viewer asks it to', async () => {
            const far = {x: 400, y: 10};
            const wrapper = mount(MapPanel, {
                props: {entities: [FURNACE], bots: [{id: 1, position: far}], trail: {}, bounds: BOUNDS}
            });
            const fitted = projectionOf(wrapper).scale;
            await wrapper.get('[data-testid="camera-follow"]').trigger('click');
            expect(wrapper.get('[data-testid="camera-follow"]').attributes('aria-pressed')).toBe('true');
            expect(wrapper.get('[data-testid="camera-fit"]').attributes('aria-pressed')).toBe('false');
            expect(projectionOf(wrapper).scale).toBeGreaterThan(fitted);
            expect(isVisible(onScreen(wrapper, far))).toBe(true);
            // The base is genuinely out of shot now. That is the trade the
            // viewer just chose, not a bug.
            expect(isVisible(onScreen(wrapper, {x: 20, y: 30}))).toBe(false);
        });

        it('says when fit has been stretched too far, and offers the way out', async () => {
            const wrapper = mount(MapPanel, {
                props: {entities: [FURNACE], bots: [{id: 1, position: {x: 400, y: 10}}], trail: {}, bounds: BOUNDS}
            });
            expect(wrapper.get('[data-testid="camera-strain"]').text()).toContain('Follow bots');
            await wrapper.get('[data-testid="camera-follow"]').trigger('click');
            expect(wrapper.find('[data-testid="camera-strain"]').exists()).toBe(false);
        });

        it('does not nag about strain on an ordinary map', () => {
            expect(mountPanel().find('[data-testid="camera-strain"]').exists()).toBe(false);
        });

        it('draws the bots before the first keyframe, rather than reporting an empty map', () => {
            // `bounds` is null until a keyframe lands, but a sampled bot is
            // something to show, and "no keyframe yet" is not "nothing here".
            const wrapper = mount(MapPanel, {
                props: {entities: [], bots: [{id: 1, position: {x: 12, y: 12}}], trail: {}, bounds: null}
            });
            expect(wrapper.find(MAP).exists()).toBe(true);
            expect(wrapper.get('[data-testid="map-feature-bot-1"]').attributes('cx')).toBe('12');
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
            const patchFill = wrapper.get(`${MAP} path`).attributes('fill');
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

    describe('fills', () => {
        it('fills a machine with its status colour when a fill is given for its position', () => {
            const entities: EntitySnapshot[] = [{name: 'stone-furnace', position: {x: 10, y: 12}, direction: 0}];
            const w = mountPanel({entities, fills: new Map([['10,12', 'no_fuel']])});
            const rect = w.get(`${MAP} rect[data-entity="stone-furnace"]`);
            expect(rect.attributes('fill')).toBe('var(--color-status-serious)');
            expect(rect.find('title').text()).toContain('no_fuel');
        });
    });
});
