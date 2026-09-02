/**
 * Where the map's camera points, and — just as important — when it is
 * allowed to move.
 *
 * The panel used to frame `boundsAt(map, cursor)`: the bounds recorded on the
 * latest keyframe, which describe **entities**. A bot that walks past the
 * built area therefore walks out of the picture, and a viewer watching a live
 * run sees a still frame while the run is at its busiest. That is the bug this
 * file exists to fix, and the fix is not "make the box tighter" — a box
 * recomputed from scratch on every cursor tick twitches continuously while
 * scrubbing or playing, which is a worse picture than a frozen one.
 *
 * So there are two halves here and they pull against each other:
 *
 *   1. **What should be visible** (`contentBox`) — everything the chosen mode
 *      is responsible for, bots and their trails included. A trail running off
 *      the edge is the same failure as a bot running off the edge: it is the
 *      part of the picture that says where the bot came from.
 *   2. **Whether to move** (`cameraFrame`) — the previous frame is kept until
 *      the content actually leaves it, so a bot crossing the map changes
 *      nothing on screen until it approaches an edge. Movement is the
 *      exception, not the per-tick default.
 *
 * Everything below is pure: same inputs, same frame. The caller owns the
 * `previous` frame and hands it back in, which is what makes the hysteresis
 * testable instead of hidden in a component's local state.
 *
 * World coordinates and tiles throughout. Positions arrive as Factorio
 * reports them — a resource sits at a tile *centre*, `-40.5`, never `-41` —
 * and nothing here floors a coordinate. Only the frame's own edges are
 * quantised, and they are quantised to a 16-tile grid where a half tile
 * cannot survive to shift anything.
 */
import {Bounds, Position} from '@/api/types';

/**
 * `fit` shows the built world and the bots together; `follow` shows the bots
 * and drops the rest.
 *
 * Two modes rather than one clever rule, because the two answer different
 * questions and no single frame answers both. "Where is everything?" wants
 * the base in shot even when a bot is 300 tiles away prospecting. "What is
 * that bot doing?" wants the bot big, and the base is then a distraction
 * whose only contribution is to shrink the subject.
 */
export type CameraMode = 'fit' | 'follow';

/**
 * How close content may get to the frame's edge before the frame moves, in
 * tiles.
 *
 * The trigger, not the padding — see `CAMERA_LEAD`. It is not zero because a
 * bot dot is drawn 6 screen pixels wide: a bot exactly on the edge is already
 * half outside, and by the time it is *past* the edge the viewer has already
 * lost it, which is the complaint this file answers.
 */
export const CAMERA_MARGIN = 8;

/**
 * How much room a re-frame leaves around the content, in tiles.
 *
 * Four times the trigger margin, and the gap between the two is the whole
 * anti-jitter budget: a bot has to cross 24 tiles of dead zone before the
 * picture moves again. Framing to the trigger margin instead — re-framing to
 * exactly the box that just failed the test — would put the content back on
 * the edge it just reached, and the camera would then step forward with
 * every few tiles the bot walked. That is the twitch, and it is caused by
 * where the camera moves *to*, not by how often it is asked.
 *
 * The cost is honest: every frame is 32 tiles roomier than it strictly needs
 * to be. That is a fixed, predictable amount of empty ground, which is much
 * cheaper to look at than a moving one.
 */
export const CAMERA_LEAD = 32;

/**
 * Frame edges are snapped outward to this grid, in tiles.
 *
 * This is the anti-jitter mechanism that survives even a re-frame: two
 * different content boxes inside the same 16-tile cell produce the *same*
 * frame, so sub-cell motion cannot move the picture at all. 16 is half a
 * Factorio chunk, which keeps the numbers recognisable when reading a frame
 * out of a test failure.
 */
export const CAMERA_GRID = 16;

/**
 * The smallest frame, in tiles.
 *
 * Without a floor, four bots standing on the same tile at the start of a run
 * would be framed into a few square tiles and the map would read as an
 * enormous close-up of nothing -- and every step any of them took would then
 * be an enormous movement. 96 tiles is three chunks; at the panel's 480px
 * viewport that is 5 px per tile, and a placed entity is drawn at a floor of
 * 7 px, so nothing disappears at the bottom of this range.
 */
export const CAMERA_MIN_SPAN = 96;

/**
 * How much slack the previous frame may carry before it is worth zooming
 * back in.
 *
 * The frame is kept while the content fits, so after a bot returns from a
 * long trip the frame would otherwise stay stretched around a journey nobody
 * is making any more. Shrinking as soon as there is *any* slack would undo
 * the hysteresis — the frame would track the content down tile by tile —
 * so it takes a factor of two before the frame tightens. Because the tightened
 * frame is `desired`, and `desired` already contains the content plus
 * `CAMERA_LEAD` -- which is four times the escape trigger -- a shrink cannot
 * immediately re-trigger a grow: the content has to actually move, and move a
 * long way, to escape again. That is what stops it oscillating.
 */
export const CAMERA_SHRINK_SLACK = 2;

/**
 * Above this span, in tiles, `fit` is showing more world than it can show
 * usefully.
 *
 * Nothing *vanishes* at that point — the panel draws bots and entity markers
 * at a fixed size in screen pixels, so they stay hittable however far out the
 * camera pulls — but 256 tiles across a 480px viewport is under 2 px per tile
 * and the base has stopped being legible as a base. The frame does not
 * silently switch modes on the viewer's behalf; it reports the strain and the
 * panel offers the other mode.
 */
export const CAMERA_STRAIN_SPAN = 256;

/** Why the returned frame is what it is. Reported so a test can assert the policy, not just the numbers. */
export type CameraReason =
    /** No usable previous frame. */
    | 'initial'
    /** The viewer switched modes; the old frame described a different question. */
    | 'mode-changed'
    /** The content still fits, so the picture does not move. */
    | 'kept'
    /** Something left the frame — the case the whole file exists for. */
    | 'escaped'
    /** The content pulled far enough in that the frame was wasting the viewport. */
    | 'shrank';

export interface CameraFrame {
    mode: CameraMode;
    /**
     * The framed area, always **square**.
     *
     * The panel's viewport is square and `projectionFor` letterboxes anything
     * that is not, which would mean the frame and the visible area were two
     * different rectangles — and then "does the content fit in the previous
     * frame" would be answering about the wrong one. Squaring here keeps the
     * frame and the picture the same thing.
     */
    bounds: Bounds;
    reason: CameraReason;
    /** The frame is stretched past what it can usefully show. See `CAMERA_STRAIN_SPAN`. */
    strained: boolean;
}

export interface CameraInput {
    mode: CameraMode;
    /** The keyframe's entity bounds, or null before the first keyframe. */
    world: Bounds | null;
    /** Every bot's position at the cursor. */
    bots: Position[];
    /** Every bot's recent positions. Framed as content, not decoration. */
    trails: Position[][];
    /** The frame this panel is currently showing, or null on the first pass. */
    previous: CameraFrame | null;
}

function boundsOf(points: Position[], seed: Bounds | null): Bounds | null {
    let box = seed === null ? null : {...seed};
    for (const point of points) {
        if (box === null) {
            box = {left: point.x, top: point.y, right: point.x, bottom: point.y};
            continue;
        }
        box.left = Math.min(box.left, point.x);
        box.top = Math.min(box.top, point.y);
        box.right = Math.max(box.right, point.x);
        box.bottom = Math.max(box.bottom, point.y);
    }
    return box;
}

/**
 * Everything the mode is responsible for keeping on screen, unpadded.
 *
 * `follow` falls back to the world when there is nothing to follow. A run
 * with no `bots` samples — an archive from before sampling, or a cursor
 * parked before the first sample — would otherwise answer an empty panel for
 * a map that has plenty to show, and "there are no bots yet" is not the same
 * claim as "there is no map".
 */
export function contentBox(input: CameraInput): Bounds | null {
    const points = [...input.bots, ...input.trails.flat()];
    if (input.mode === 'follow') {
        return boundsOf(points, null) ?? input.world;
    }
    return boundsOf(points, input.world);
}

function padded(box: Bounds, margin: number): Bounds {
    return {
        left: box.left - margin,
        top: box.top - margin,
        right: box.right + margin,
        bottom: box.bottom + margin
    };
}

function snapOut(box: Bounds, grid: number): Bounds {
    return {
        left: Math.floor(box.left / grid) * grid,
        top: Math.floor(box.top / grid) * grid,
        right: Math.ceil(box.right / grid) * grid,
        bottom: Math.ceil(box.bottom / grid) * grid
    };
}

/**
 * The frame this content wants, ignoring whatever is on screen now.
 *
 * Pad by `CAMERA_LEAD`, snap outward to the grid, floor the span, then square
 * it about the snapped centre. Order matters: squaring last is what
 * guarantees the result is square, and snapping before squaring is what makes
 * the centre a multiple of half a grid cell, so every edge lands on an exact
 * number and no rounding error can creep into a comparison against the
 * previous frame.
 */
export function desiredFrame(content: Bounds): Bounds {
    const box = snapOut(padded(content, CAMERA_LEAD), CAMERA_GRID);
    const span =
        Math.ceil(Math.max(box.right - box.left, box.bottom - box.top, CAMERA_MIN_SPAN) / CAMERA_GRID) *
        CAMERA_GRID;
    const centreX = (box.left + box.right) / 2;
    const centreY = (box.top + box.bottom) / 2;
    const half = span / 2;
    return {
        left: centreX - half,
        top: centreY - half,
        right: centreX + half,
        bottom: centreY + half
    };
}

function contains(outer: Bounds, inner: Bounds): boolean {
    return (
        outer.left <= inner.left &&
        outer.top <= inner.top &&
        outer.right >= inner.right &&
        outer.bottom >= inner.bottom
    );
}

/** A frame's span in tiles. Square, so one number describes it. */
export function spanOf(bounds: Bounds): number {
    return Math.max(bounds.right - bounds.left, bounds.bottom - bounds.top);
}

/**
 * The frame to show now, given what is on screen and what has to be visible.
 *
 * Returns null only when there is genuinely nothing to frame — no keyframe
 * bounds, no bots, no trail — which the panel reports as an empty map rather
 * than drawing an empty viewport.
 */
export function cameraFrame(input: CameraInput): CameraFrame | null {
    const content = contentBox(input);
    if (content === null) return null;

    const desired = desiredFrame(content);
    const finish = (bounds: Bounds, reason: CameraReason): CameraFrame => ({
        mode: input.mode,
        bounds,
        reason,
        strained: spanOf(bounds) > CAMERA_STRAIN_SPAN
    });

    const previous = input.previous;
    if (previous === null) return finish(desired, 'initial');
    if (previous.mode !== input.mode) return finish(desired, 'mode-changed');

    // The margin is inside the test on purpose: a bot exactly on the previous
    // frame's edge is drawn as half a dot and reads as already gone. It has to
    // re-frame while it still has room, not once it is clipped.
    if (!contains(previous.bounds, padded(content, CAMERA_MARGIN))) {
        return finish(desired, 'escaped');
    }
    if (spanOf(previous.bounds) > spanOf(desired) * CAMERA_SHRINK_SLACK) {
        return finish(desired, 'shrank');
    }
    return finish(previous.bounds, 'kept');
}
