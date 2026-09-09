/**
 * A hand-written mirror of `crates/executor/src/replay.rs`'s `Replay`.
 *
 * That module is the authority for this shape and for the absence semantics
 * below -- read its doc comments before touching this file. The short version:
 *
 * - `planned_*` fields are never absent; a schedule always knows where it put
 *   a step.
 * - `observed_*` fields are `null`, never `0`, when the game never said.
 *   `Ticks | null`, not `Ticks`, everywhere an observation can be missing.
 * - There is deliberately no observed makespan anywhere in this shape.
 *   `replay.rs` refuses to compute one because doing so means picking a
 *   definition (last reply? last dispatch? last step with any tick at all?),
 *   and that choice belongs to whoever renders the document, not to the
 *   document itself. See `replayMakespanCeiling` below for the one derived
 *   number this module *does* allow -- an axis maximum, not a claimed total.
 * - `refused: string | null`. `null` means the run was attempted; a string is
 *   what the run refuser said, and its presence means *nothing was
 *   dispatched at all* -- not "attempted and observed nothing".
 * - `evidence` distinguishes a claim that was *measured* from one the
 *   executor merely *believes*; a walk's arrival is always the latter. See
 *   `crates/executor/src/replay.rs`'s `WALK_BELIEF` for why.
 *
 * A tracked `Replay` JSON snapshot is coming from the Rust side so these types
 * can be checked against a real document. Until then this file is the
 * contract, and `replay.fixtures.ts` is the one place fixtures live so that
 * swapping the fixture source is a one-line change, not an edit to every spec.
 */

/** A tick count. Plain `number` on the wire; never fractional, never negative. */
export type Ticks = number;

export interface Position {
    x: number;
    y: number;
}

/**
 * What a row is a row of. Internally tagged (`{"kind": "act", ...}` /
 * `{"kind": "walk", ...}`), matching `ReplayStepKind`'s `#[serde(tag =
 * "kind", rename_all = "snake_case")]` in `replay.rs`.
 */
export type ReplayStepKind =
    | {kind: 'act'; action: number; label: string}
    | {kind: 'walk'; to: Position};

/**
 * Whether a row's observation means what it says.
 *
 * `{"kind": "measured"}` or `{"kind": "believed", "why": "..."}` -- mirrors
 * `Evidence`'s internal tagging in `replay.rs`. A `measured` row is
 * structurally incapable of carrying a `why`; a `believed` row cannot omit
 * one. Never write your own text for `why` -- it travels with the document.
 */
export type Evidence =
    | {kind: 'measured'}
    | {kind: 'believed'; why: string};

/**
 * What is known about a row.
 *
 * `Lost` and `Failed` are deliberately distinct: `Lost` means this run will
 * never learn the outcome, `Failed` means a verdict arrived and it was bad.
 * Collapsing the two in a view is the exact mistake `replay.rs`'s tests exist
 * to catch on the Rust side -- do not repeat it here.
 *
 * `Abandoned` is a third thing again: never dispatched, because a
 * predecessor did not succeed or the bot's walk halted it. The game never saw
 * the row, so it has no ticks -- like `Pending` -- but unlike `Pending` the
 * run did reach it and decided against it, and the row's error names why.
 */
export type ReplayStatus = 'Pending' | 'Running' | 'Success' | 'Failed' | 'Lost' | 'Abandoned';

/** One scheduled step, paired with whatever the run learned about it. */
export interface ReplayStep {
    /** Position in the schedule. Stable identity for selection. */
    index: number;
    bot: number;
    /** Position among this bot's own steps, in schedule order. */
    bot_step_index: number;
    what: ReplayStepKind;
    /** Where the scheduler put this step. Never observed, never absent. */
    planned_start_tick: Ticks;
    planned_end_tick: Ticks;
    /** `null` when the game never said. Never zero-as-unknown. */
    observed_start_tick: Ticks | null;
    observed_end_tick: Ticks | null;
    status: ReplayStatus;
    /**
     * Which attempt this row describes, counting from 1, or `null` when no
     * attempt was made at all. Always `null` for walk rows.
     */
    attempt_number: number | null;
    evidence: Evidence;
    error: string | null;
}

/** A walk observation that no scheduled step claims. Normally never seen. */
export interface UnmatchedWalk {
    bot: number;
    bot_step_index: number;
}

/** A schedule paired with an execution log, as `Replay::new` produces it. */
export interface Replay {
    /** The *planned* end of the whole run. There is no observed counterpart. */
    planned_makespan: Ticks;
    /**
     * `null`: the run was attempted. A string: the run was refused before it
     * started, dispatched nothing, and this is what the refusal said.
     *
     * Without this field, a refused run and an attempted run that measured
     * nothing produce byte-identical all-`Pending` documents -- one
     * unremarkable, one alarming. Never render them the same way.
     */
    refused: string | null;
    steps: ReplayStep[];
    /**
     * Walk observations whose `(bot, bot_step_index)` matched no scheduled
     * step. Normally empty; non-empty only when a log was paired with the
     * wrong schedule. Report it, never hide it.
     */
    unmatched_walks: UnmatchedWalk[];
}

/**
 * Thrown by {@link parseReplay} naming the exact field that did not fit.
 *
 * A JSON import's string fields widen to `string` in TypeScript, so nothing
 * short of a runtime check can tell `"act"` from `"acct"` or confirm `status`
 * is one of the five real values -- a type annotation alone would accept
 * either. This is that check: every leaf of the document is inspected against
 * the literal values `replay.rs` can actually produce, and a mismatch fails
 * loudly, at the one place a malformed or drifted document can still be
 * caught, rather than reaching a template as `undefined`.
 */
export class ReplayShapeError extends Error {
    constructor(path: string, detail: string) {
        super(`replay document invalid at ${path}: ${detail}`);
        this.name = 'ReplayShapeError';
    }
}

function fail(path: string, detail: string): never {
    throw new ReplayShapeError(path, detail);
}

function asObject(value: unknown, path: string): Record<string, unknown> {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        fail(path, `expected an object, got ${JSON.stringify(value)}`);
    }
    return value as Record<string, unknown>;
}

function asArray(value: unknown, path: string): unknown[] {
    if (!Array.isArray(value)) {
        fail(path, `expected an array, got ${JSON.stringify(value)}`);
    }
    return value;
}

function asNumber(value: unknown, path: string): number {
    if (typeof value !== 'number') {
        fail(path, `expected a number, got ${JSON.stringify(value)}`);
    }
    return value;
}

function asNumberOrNull(value: unknown, path: string): number | null {
    return value === null ? null : asNumber(value, path);
}

function asString(value: unknown, path: string): string {
    if (typeof value !== 'string') {
        fail(path, `expected a string, got ${JSON.stringify(value)}`);
    }
    return value;
}

function asStringOrNull(value: unknown, path: string): string | null {
    return value === null ? null : asString(value, path);
}

/** Narrows `value` to one of `options`, or fails naming what it actually was. */
function asOneOf<T extends string>(value: unknown, options: readonly T[], path: string): T {
    if (typeof value !== 'string' || !(options as readonly string[]).includes(value)) {
        fail(path, `expected one of ${options.join(' | ')}, got ${JSON.stringify(value)}`);
    }
    return value as T;
}

const STATUSES = ['Pending', 'Running', 'Success', 'Failed', 'Lost', 'Abandoned'] as const;

function parseEvidence(value: unknown, path: string): Evidence {
    const obj = asObject(value, path);
    const kind = asOneOf(obj.kind, ['measured', 'believed'] as const, path + '.kind');
    return kind === 'measured'
        ? {kind: 'measured'}
        : {kind: 'believed', why: asString(obj.why, path + '.why')};
}

function parsePosition(value: unknown, path: string): Position {
    const obj = asObject(value, path);
    return {x: asNumber(obj.x, path + '.x'), y: asNumber(obj.y, path + '.y')};
}

function parseWhat(value: unknown, path: string): ReplayStepKind {
    const obj = asObject(value, path);
    const kind = asOneOf(obj.kind, ['act', 'walk'] as const, path + '.kind');
    return kind === 'act'
        ? {kind: 'act', action: asNumber(obj.action, path + '.action'), label: asString(obj.label, path + '.label')}
        : {kind: 'walk', to: parsePosition(obj.to, path + '.to')};
}

function parseStep(value: unknown, path: string): ReplayStep {
    const obj = asObject(value, path);
    return {
        index: asNumber(obj.index, path + '.index'),
        bot: asNumber(obj.bot, path + '.bot'),
        bot_step_index: asNumber(obj.bot_step_index, path + '.bot_step_index'),
        what: parseWhat(obj.what, path + '.what'),
        planned_start_tick: asNumber(obj.planned_start_tick, path + '.planned_start_tick'),
        planned_end_tick: asNumber(obj.planned_end_tick, path + '.planned_end_tick'),
        observed_start_tick: asNumberOrNull(obj.observed_start_tick, path + '.observed_start_tick'),
        observed_end_tick: asNumberOrNull(obj.observed_end_tick, path + '.observed_end_tick'),
        status: asOneOf(obj.status, STATUSES, path + '.status'),
        attempt_number: obj.attempt_number === null ? null : asNumber(obj.attempt_number, path + '.attempt_number'),
        evidence: parseEvidence(obj.evidence, path + '.evidence'),
        error: asStringOrNull(obj.error, path + '.error')
    };
}

function parseUnmatchedWalk(value: unknown, path: string): UnmatchedWalk {
    const obj = asObject(value, path);
    return {
        bot: asNumber(obj.bot, path + '.bot'),
        bot_step_index: asNumber(obj.bot_step_index, path + '.bot_step_index')
    };
}

/**
 * Validates and narrows an already-`JSON.parse`d value into a `Replay`.
 *
 * This is the one place a malformed or drifted document is caught: every
 * discriminant (`status`, `evidence.kind`, `what.kind`) is checked against
 * the literal values `replay.rs` can actually emit, and every tick is
 * checked as a number or `null` -- never coerced, never defaulted. Throws
 * {@link ReplayShapeError} naming the exact field on any mismatch, rather
 * than returning a document a view would partially trust.
 *
 * Used for two purposes with one implementation: production code calls it
 * (via {@link parseReplayJson}) on whatever the server sent, and the fixture
 * module calls it directly on the tracked, Rust-generated snapshot documents
 * -- where this call succeeding, unmodified, *is* the check that this
 * module's declared types have not drifted from what the executor emits.
 */
export function parseReplay(input: unknown): Replay {
    const obj = asObject(input, '$');
    const steps = asArray(obj.steps, '$.steps').map((s, i) => parseStep(s, `$.steps[${i}]`));
    const unmatchedWalks = asArray(obj.unmatched_walks, '$.unmatched_walks')
        .map((w, i) => parseUnmatchedWalk(w, `$.unmatched_walks[${i}]`));
    return {
        planned_makespan: asNumber(obj.planned_makespan, '$.planned_makespan'),
        refused: asStringOrNull(obj.refused, '$.refused'),
        steps,
        unmatched_walks: unmatchedWalks
    };
}

/**
 * Parses a replay document from the raw JSON text `subscribeJobEvents` and
 * `Job.replay` both hand over verbatim.
 *
 * Never throws: a malformed or truncated document is an `{error}` result,
 * not an exception a mounting component would have to catch, and not a
 * document silently missing fields that a view would then read as
 * `undefined` and treat as falsy zero. `JSON.parse` failures and
 * {@link ReplayShapeError}s are reported the same way, through the same path.
 */
export function parseReplayJson(json: string): {replay: Replay} | {error: string} {
    let value: unknown;
    try {
        value = JSON.parse(json);
    } catch (err) {
        return {error: 'not valid JSON: ' + (err instanceof Error ? err.message : String(err))};
    }
    try {
        return {replay: parseReplay(value)};
    } catch (err) {
        return {error: err instanceof Error ? err.message : String(err)};
    }
}

/**
 * A time axis maximum honest enough to draw: the planned makespan, extended
 * to cover any observed tick that runs past it.
 *
 * This is NOT an observed makespan -- `replay.rs` refuses to compute one on
 * purpose, because doing so means choosing between last-reply, last-dispatch
 * and last-step-with-any-tick. This function makes no such claim: it never
 * asserts that the run ended at the number it returns, only that nothing
 * observed falls after it, so a caller drawing an axis has somewhere honest
 * to stop. Callers must still label the axis "ticks", not "duration" or
 * "makespan".
 */
export function replayAxisCeiling(replay: Replay): Ticks {
    let max = replay.planned_makespan;
    const origin = observedOrigin(replay);
    for (const step of replay.steps) {
        for (const tick of [step.observed_start_tick, step.observed_end_tick]) {
            if (tick !== null) {
                const shifted = tick - origin;
                if (shifted > max) {
                    max = shifted;
                }
            }
        }
    }
    return max;
}

/**
 * The tick the observed timeline is measured from.
 *
 * **Planned and observed ticks are in different origins**, which is not
 * obvious and was drawn wrongly before this existed. `planned_start_tick`
 * comes from a `Schedule` built before anything ran, so it counts from zero at
 * run start. `observed_start_tick` is `game.tick` — an absolute clock that was
 * already at 60,551 when this run's first step was dispatched.
 *
 * Rendering both against one origin put every planned bar in the first 1.4% of
 * the axis and every observed bar at the far right, so the two rows looked
 * like a comparison and their positions meant nothing. Durations were the only
 * honest reading, and nothing said so.
 *
 * The origin is the earliest observed tick in the run: the first moment the
 * game acknowledged anything. **It is an anchor, not run start** — the run
 * began some ticks earlier, when the script was launched — so a shifted
 * observed timeline says "this much later than the first dispatch", not "this
 * much after the run began". The two differ by however long setup took, and
 * nothing in the document records that.
 *
 * `0` when nothing was observed, which makes the shift a no-op and leaves a
 * plan-only replay drawn exactly as before.
 */
export function observedOrigin(replay: Replay): Ticks {
    let origin: Ticks | null = null;
    for (const step of replay.steps) {
        for (const tick of [step.observed_start_tick, step.observed_end_tick]) {
            if (tick !== null && (origin === null || tick < origin)) {
                origin = tick;
            }
        }
    }
    return origin ?? 0;
}
