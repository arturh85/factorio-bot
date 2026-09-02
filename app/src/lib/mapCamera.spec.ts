/**
 * The framing policy, asserted as numbers.
 *
 * Every expectation here is hand-computed from the constants, because the
 * whole point of putting the camera in a pure function was that "the bot is
 * off screen" and "the map twitches while playing" stop being things you can
 * only see by watching a run and become things a test can state.
 */
import {describe, expect, it} from 'vitest';
import {Bounds, Position} from '@/api/types';
import {
    CAMERA_MIN_SPAN,
    CAMERA_STRAIN_SPAN,
    CameraFrame,
    CameraInput,
    cameraFrame,
    contentBox,
    desiredFrame,
    spanOf
} from './mapCamera';

const WORLD: Bounds = {left: 0, top: 0, right: 48, bottom: 48};

function input(overrides: Partial<CameraInput> = {}): CameraInput {
    return {mode: 'fit', world: WORLD, bots: [], trails: [], previous: null, ...overrides};
}

function frame(overrides: Partial<CameraInput> = {}): CameraFrame {
    const result = cameraFrame(input(overrides));
    if (result === null) throw new Error('expected a frame');
    return result;
}

/** Is the point inside the frame at all -- the minimum bar this file exists for. */
function holds(bounds: Bounds, point: Position): boolean {
    return (
        point.x >= bounds.left && point.x <= bounds.right && point.y >= bounds.top && point.y <= bounds.bottom
    );
}

describe('desiredFrame', () => {
    it('pads the content by the lead and snaps every edge outward to the 16-tile grid', () => {
        // 0..48 padded by 32 is -32..80, already on the grid: a 112-tile
        // square that needs no further squaring.
        expect(desiredFrame(WORLD)).toEqual({left: -32, top: -32, right: 80, bottom: 80});
    });

    it('never frames tighter than the minimum span, so a huddle of bots is not a close-up of nothing', () => {
        const result = desiredFrame({left: 10, top: 10, right: 11, bottom: 11});
        expect(spanOf(result)).toBe(CAMERA_MIN_SPAN);
        expect(result).toEqual({left: -40, top: -40, right: 56, bottom: 56});
    });

    it('squares a wide box about its centre rather than letterboxing it', () => {
        // -32..240 wide (272) against -32..80 tall (112).
        const result = desiredFrame({left: 0, top: 0, right: 200, bottom: 48});
        expect(result).toEqual({left: -32, top: -112, right: 240, bottom: 160});
        expect(result.right - result.left).toBe(result.bottom - result.top);
    });

    it('gives two content boxes inside the same grid cell the same frame', () => {
        // This is the anti-jitter mechanism on its own, before any hysteresis:
        // sub-cell motion cannot move the picture at all.
        const a = desiredFrame({left: 10.2, top: 10.2, right: 10.2, bottom: 10.2});
        const b = desiredFrame({left: 10.9, top: 10.9, right: 10.9, bottom: 10.9});
        expect(a).toEqual(b);
        expect(a).toEqual({left: -40, top: -40, right: 56, bottom: 56});
    });

    it('recovers whole-tile edges from a tile centre without drifting half a tile', () => {
        // A resource -- and a bot standing on it -- is at -40.5, never -41.
        // The half tile has to survive into the frame's contents and out of
        // the frame's own edges.
        const result = desiredFrame({left: -40.5, top: -48.5, right: -40.5, bottom: -48.5});
        expect(result).toEqual({left: -88, top: -104, right: 8, bottom: -8});
        expect(holds(result, {x: -40.5, y: -48.5})).toBe(true);
    });
});

describe('contentBox', () => {
    it('unions the world with the bots in fit mode', () => {
        expect(contentBox(input({bots: [{x: 200, y: -20}]}))).toEqual({
            left: 0,
            top: -20,
            right: 200,
            bottom: 48
        });
    });

    it('includes every trail point: a trail off the edge is a bot off the edge', () => {
        expect(contentBox(input({trails: [[{x: -60, y: 4}, {x: 10, y: 4}]]}))).toEqual({
            left: -60,
            top: 0,
            right: 48,
            bottom: 48
        });
    });

    it('drops the world in follow mode, keeping only the bots and their trails', () => {
        expect(
            contentBox(input({mode: 'follow', bots: [{x: 200, y: 200}], trails: [[{x: 180, y: 190}]]}))
        ).toEqual({left: 180, top: 190, right: 200, bottom: 200});
    });

    it('falls back to the world when follow mode has nothing to follow', () => {
        // "No bots sampled yet" is not the same claim as "no map".
        expect(contentBox(input({mode: 'follow'}))).toEqual(WORLD);
    });

    it('reports nothing to frame when there is no world and no bot', () => {
        expect(contentBox(input({world: null}))).toBeNull();
        expect(cameraFrame(input({world: null}))).toBeNull();
    });

    it('frames the bots alone before the first keyframe has arrived', () => {
        const result = frame({world: null, bots: [{x: 12, y: 12}]});
        expect(holds(result.bounds, {x: 12, y: 12})).toBe(true);
        expect(spanOf(result.bounds)).toBe(CAMERA_MIN_SPAN);
    });
});

describe('cameraFrame', () => {
    it('keeps a bot in view that has walked far outside the built area', () => {
        const bot = {x: 200, y: -20};
        const result = frame({bots: [bot]});
        expect(result.reason).toBe('initial');
        expect(holds(result.bounds, bot)).toBe(true);
        // ...and the base is still in shot, which is what fit mode means.
        expect(holds(result.bounds, {x: 0, y: 48})).toBe(true);
    });

    it('does not move while the content stays inside the frame', () => {
        const first = frame({bots: [{x: 10, y: 10}]});
        const second = frame({bots: [{x: 40, y: 40}], previous: first});
        expect(second.reason).toBe('kept');
        expect(second.bounds).toEqual(first.bounds);
    });

    it('re-frames when a bot approaches the edge, before it is clipped', () => {
        const first = frame();
        expect(first.bounds.right).toBe(80);
        // 76 is still inside the frame, but within the 8-tile trigger margin
        // of its edge -- a dot drawn there is already half outside.
        const second = frame({bots: [{x: 76, y: 10}], previous: first});
        expect(second.reason).toBe('escaped');
        expect(second.bounds.right).toBeGreaterThan(first.bounds.right);
        expect(holds(second.bounds, {x: 76, y: 10})).toBe(true);
    });

    it('re-frames for a trail that leads off the edge as readily as for a bot', () => {
        const first = frame();
        const second = frame({trails: [[{x: -90, y: 10}, {x: 10, y: 10}]], previous: first});
        expect(second.reason).toBe('escaped');
        expect(holds(second.bounds, {x: -90, y: 10})).toBe(true);
    });

    it('leaves the content well clear of the edge it just re-framed for', () => {
        // Re-framing to the box that just failed the trigger would put the bot
        // straight back on the edge, and the camera would then creep forward
        // every few tiles. It gets the full lead instead.
        const first = frame();
        const second = frame({bots: [{x: 76, y: 10}], previous: first});
        expect(second.bounds.right - 76).toBeGreaterThanOrEqual(32);
    });

    it('zooms back in only once the frame is more than twice the span it needs', () => {
        const wide = frame({bots: [{x: 400, y: 10}]});
        expect(spanOf(wide.bounds)).toBe(464);
        const back = frame({previous: wide});
        expect(back.reason).toBe('shrank');
        expect(back.bounds).toEqual({left: -32, top: -32, right: 80, bottom: 80});
    });

    it('holds a frame that is merely a little roomier than it needs to be', () => {
        const roomy = frame({bots: [{x: 70, y: 10}]});
        expect(spanOf(roomy.bounds)).toBe(144);
        // 144 is not more than twice the 112 the world alone would ask for.
        expect(frame({previous: roomy}).reason).toBe('kept');
    });

    it('cannot oscillate: the frame it shrinks to already holds the content plus its lead', () => {
        const wide = frame({bots: [{x: 400, y: 10}]});
        const shrunk = frame({previous: wide});
        expect(shrunk.reason).toBe('shrank');
        // Same inputs again, one step later: nothing moved, so nothing moves.
        const settled = frame({previous: shrunk});
        expect(settled.reason).toBe('kept');
        expect(settled.bounds).toEqual(shrunk.bounds);
    });

    it('discards the previous frame when the viewer switches modes', () => {
        const fit = frame({bots: [{x: 200, y: 200}]});
        const follow = frame({mode: 'follow', bots: [{x: 200, y: 200}], previous: fit});
        expect(follow.reason).toBe('mode-changed');
        expect(spanOf(follow.bounds)).toBe(CAMERA_MIN_SPAN);
        expect(holds(follow.bounds, {x: 200, y: 200})).toBe(true);
        // Follow is the mode that makes a distant bot big again: in the fit
        // frame that same bot was one part in three.
        expect(spanOf(follow.bounds)).toBeLessThan(spanOf(fit.bounds));
    });

    it('reports strain when fit is stretched past what it can usefully show', () => {
        expect(frame().strained).toBe(false);
        const stretched = frame({bots: [{x: 400, y: 10}]});
        expect(spanOf(stretched.bounds)).toBeGreaterThan(CAMERA_STRAIN_SPAN);
        expect(stretched.strained).toBe(true);
        // ...and following that same bot is not strained, which is the point
        // of offering the choice rather than picking one.
        expect(frame({mode: 'follow', bots: [{x: 400, y: 10}]}).strained).toBe(false);
    });

    it('moves a handful of times, not every step, while a bot walks 200 tiles', () => {
        // The jitter test. 100 cursor steps of 2 tiles each: every step must
        // keep the bot in view, and almost none of them may move the picture.
        let current: CameraFrame | null = null;
        let moves = 0;
        for (let step = 0; step <= 100; step++) {
            const bot = {x: step * 2, y: 10};
            const next = frame({bots: [bot], trails: [[{x: 0, y: 10}, bot]], previous: current});
            if (current !== null && next.reason !== 'kept') moves++;
            expect(holds(next.bounds, bot)).toBe(true);
            current = next;
        }
        // Exactly four, and deterministically four: this is a pure function
        // over a fixed walk, so the number is the policy rather than a
        // tolerance. 100 steps, 4 movements, and the bot on screen for all of
        // them.
        expect(moves).toBe(4);
    });
});
