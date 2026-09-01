/**
 * The pure map from game-world coordinates to canvas pixel coordinates.
 *
 * Kept out of `MapPanel.vue` on purpose: jsdom's `HTMLCanvasElement` has no
 * real 2D context (`getContext('2d')` returns `null` there without the
 * optional native `canvas` package), so a component test can assert that a
 * `<canvas>` exists but never assert on the numbers it draws. This function
 * is the part that actually can be tested, and the part most likely to be
 * wrong in a way a screenshot wouldn't catch -- a stretched or off-centre
 * world.
 */
import {Bounds, Position} from '@/api/types';

export interface Projection {
    scale: number;
    offsetX: number;
    offsetY: number;
}

/**
 * A projection that fits `bounds` inside a `width`x`height` canvas.
 *
 * Uses one scale for both axes -- `Math.min` of the two -- so a wide, short
 * base is never stretched into a square; the shorter axis gets letterboxed
 * instead, centred in the leftover space. Degenerate input (a zero-area
 * `bounds`, or a zero-sized canvas) returns the identity-ish `{scale: 1,
 * offsetX: 0, offsetY: 0}` rather than dividing by zero.
 */
export function projectionFor(bounds: Bounds, width: number, height: number): Projection {
    const worldWidth = bounds.right - bounds.left;
    const worldHeight = bounds.bottom - bounds.top;
    if (worldWidth <= 0 || worldHeight <= 0 || width <= 0 || height <= 0) {
        return {scale: 1, offsetX: 0, offsetY: 0};
    }
    const scale = Math.min(width / worldWidth, height / worldHeight);
    const offsetX = (width - worldWidth * scale) / 2 - bounds.left * scale;
    const offsetY = (height - worldHeight * scale) / 2 - bounds.top * scale;
    return {scale, offsetX, offsetY};
}

/**
 * Projects one world position to canvas pixel coordinates.
 *
 * Never rounds: a tile centre such as `-40.5` must stay a tile centre after
 * scaling, not snap to a whole canvas pixel.
 */
export function project(projection: Projection, position: Position): Position {
    return {
        x: position.x * projection.scale + projection.offsetX,
        y: position.y * projection.scale + projection.offsetY
    };
}
