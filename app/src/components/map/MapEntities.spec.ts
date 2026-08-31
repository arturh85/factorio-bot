// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import MapEntities from './MapEntities.vue';
import {FactorioEntity} from '@/api/types';
import {SPAWN_ENTITIES} from '@/api/game.fixtures';

function entity(overrides: Partial<FactorioEntity>): FactorioEntity {
    return {
        name: 'x',
        entity_type: 'container',
        position: {x: 0, y: 0},
        bounding_box: {left_top: {x: -1, y: -1}, right_bottom: {x: 1, y: 1}},
        direction: 0,
        drop_position: null,
        pickup_position: null,
        output_inventory: null,
        fuel_inventory: null,
        amount: null,
        recipe: null,
        ghost_name: null,
        ghost_type: null,
        ...overrides
    };
}

function render(entities: FactorioEntity[], center = {x: 0, y: 0}, radius = 50) {
    return mount(MapEntities, {props: {entities, center, radius}});
}

describe('MapEntities -- orientation (rule 1: y increases downward, never flip it)', () => {
    /**
     * Two fixture entities at different y -- not invented, the real
     * `crash-site-spaceship` (top area, negative/smaller y) and
     * `stone-furnace` (spawn floor, larger y) from
     * `crates/core/tests/live-2.1.17-entities-spawn.json`. A later "fix" that
     * negates or swaps a y coordinate anywhere in the render path flips their
     * relative order and fails this test, even though the map would still
     * render *something* plausible-looking.
     */
    it('renders the entity with the larger y LOWER on screen (larger SVG y) than one with smaller y', () => {
        const higher = SPAWN_ENTITIES.find(e => e.name === 'crash-site-spaceship')!; // y around -6
        const lower = SPAWN_ENTITIES.find(e => e.name === 'stone-furnace')!; // y = 2

        expect(higher.position.y).toBeLessThan(lower.position.y); // sanity on the fixture itself

        const wrapper = render([higher, lower], {x: 0, y: 0}, 60);
        const higherRect = wrapper.get(`[data-testid="entity-${higher.name}"]`);
        const lowerRect = wrapper.get(`[data-testid="entity-${lower.name}"]`);

        const higherY = Number(higherRect.attributes('y'));
        const lowerY = Number(lowerRect.attributes('y'));

        expect(higherY).toBeLessThan(lowerY);
    });

    it('does not negate y when building the SVG viewBox', () => {
        // A viewBox with a negated/flipped y for center {x:10, y:20} radius 5
        // would start at y = -25 (if someone "fixed" the sign) instead of the
        // correct y = 15.
        const wrapper = render([], {x: 10, y: 20}, 5);
        const svg = wrapper.get('svg');
        const viewBox = svg.attributes('viewBox');
        expect(viewBox).toBe('5 15 10 10');
    });
});

describe('MapEntities -- rect derives from bounding_box, not position plus a guessed size', () => {
    it('uses bounding_box corners for x/y/width/height, ignoring how far bounding_box is from position', () => {
        // position is nowhere near the bounding box -- if a rect were drawn
        // from position plus an assumed size, it would not land here.
        const e = entity({
            name: 'offset-entity',
            position: {x: 100, y: 100},
            bounding_box: {left_top: {x: -3, y: -4}, right_bottom: {x: -1, y: -2}}
        });
        const wrapper = render([e], {x: 0, y: 0}, 10);
        const rect = wrapper.get('[data-testid="entity-offset-entity"]');

        expect(Number(rect.attributes('x'))).toBeCloseTo(-3);
        expect(Number(rect.attributes('y'))).toBeCloseTo(-4);
        expect(Number(rect.attributes('width'))).toBeCloseTo(2);
        expect(Number(rect.attributes('height'))).toBeCloseTo(2);
    });

    it('gives two entities with the same position but different bounding boxes different rect sizes', () => {
        const small = entity({
            name: 'small',
            position: {x: 5, y: 5},
            bounding_box: {left_top: {x: 4.5, y: 4.5}, right_bottom: {x: 5.5, y: 5.5}}
        });
        const large = entity({
            name: 'large',
            position: {x: 5, y: 5},
            bounding_box: {left_top: {x: 0, y: 0}, right_bottom: {x: 10, y: 10}}
        });
        const wrapper = render([small, large], {x: 5, y: 5}, 10);

        const smallWidth = Number(wrapper.get('[data-testid="entity-small"]').attributes('width'));
        const largeWidth = Number(wrapper.get('[data-testid="entity-large"]').attributes('width'));
        expect(smallWidth).toBeCloseTo(1);
        expect(largeWidth).toBeCloseTo(10);
        expect(smallWidth).not.toBe(largeWidth);
    });
});

describe('MapEntities -- identification', () => {
    it('puts the entity name in a <title> so hovering identifies it', () => {
        const e = entity({name: 'iron-ore'});
        const wrapper = render([e]);
        const rect = wrapper.get('[data-testid="entity-iron-ore"]');
        expect(rect.find('title').text()).toBe('iron-ore');
    });

    it('colours rects by entity_type: two different types get different fills', () => {
        const a = entity({name: 'a', entity_type: 'resource'});
        const b = entity({name: 'b', entity_type: 'furnace'});
        const wrapper = render([a, b]);
        const fillA = wrapper.get('[data-testid="entity-a"]').attributes('fill');
        const fillB = wrapper.get('[data-testid="entity-b"]').attributes('fill');
        expect(fillA).toBeTruthy();
        expect(fillB).toBeTruthy();
        expect(fillA).not.toBe(fillB);
    });

    it('gives two entities of the same entity_type the same fill', () => {
        const a = entity({name: 'a', entity_type: 'resource'});
        const b = entity({name: 'b', entity_type: 'resource'});
        const wrapper = render([a, b]);
        const fillA = wrapper.get('[data-testid="entity-a"]').attributes('fill');
        const fillB = wrapper.get('[data-testid="entity-b"]').attributes('fill');
        expect(fillA).toBe(fillB);
    });
});

describe('MapEntities -- renders one rect per entity', () => {
    it('renders exactly as many rects as entities given', () => {
        const wrapper = render(SPAWN_ENTITIES);
        expect(wrapper.findAll('rect[data-testid^="entity-"]')).toHaveLength(SPAWN_ENTITIES.length);
    });

    it('renders nothing for an empty entity list, without erroring', () => {
        const wrapper = render([]);
        expect(wrapper.findAll('rect[data-testid^="entity-"]')).toHaveLength(0);
    });
});
