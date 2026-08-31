/**
 * Fixtures for the replay view's tests -- the *tracked*, Rust-generated ones.
 *
 * These two files are produced by `Replay::new` and pinned by a snapshot test
 * in `crates/executor/tests/`, so they are the real serialised shape rather
 * than a hand-written guess at it.
 *
 * They are run through `parseReplay` here, not assigned to `Replay` with a
 * type annotation or an `as` cast. A JSON import's string fields widen to
 * plain `string` in TypeScript, so an annotation would not compile (a bare
 * `string` is not assignable to `"act" | "walk"`) and a cast would compile by
 * simply not checking anything -- silently deleting the one property these
 * fixtures exist to provide. `parseReplay` is a runtime validator that checks
 * every discriminant against the literal values `replay.rs` can actually
 * produce, so THIS CALL SUCCEEDING is the check: if the tracked document ever
 * disagrees with this module's declared TypeScript shape, `parseReplay`
 * throws here and the whole test file fails, rather than a hand-written
 * mirror quietly drifting from what the executor emits.
 *
 * Do not hand-write a `Replay` fixture in this file. If a test needs a
 * variation the tracked documents do not cover, derive it from one of the two
 * below (see `ALL_PENDING_ATTEMPTED_REPLAY`) rather than inventing a new
 * document from scratch.
 */

import realisticData from '../../../crates/executor/tests/replay.snapshot.json';
import refusedData from '../../../crates/executor/tests/replay.refused.snapshot.json';
import {parseReplay, Replay} from './replay';

/**
 * A run that is partly observed -- the only interesting kind. Bot 1 walked
 * and mined under a working clock, then failed a craft on its third attempt
 * with only half a measurement (dispatch stamped, reply never arrived). Bot 2
 * lost track of its walk, never reached its next step, succeeded a step with
 * no clock at all, and is still `Running` a later one. `unmatched_walks`
 * carries one entry, for a bot 3 that has no step in this schedule at all.
 * Every absence in it is a different absence; see
 * `crates/executor/src/replay.rs`'s own `realistic()` test fixture, which the
 * Rust snapshot test that generated this file is built from.
 */
export const REALISTIC_REPLAY: Replay = parseReplay(realisticData);

/** The reason the refused fixture's run never started, read from the document itself. */
export const CIRCULAR_REFUSAL = (refusedData.refused as string | null) ?? '';

/**
 * A schedule refused before it was dispatched. Every row is `Pending` with
 * both ticks `null` -- byte-for-byte the same *rows* an attempted run that
 * measured nothing would produce. `refused` is the only field that tells the
 * two apart; see `replay.rs`'s module docs and `ALL_PENDING_ATTEMPTED_REPLAY`
 * below.
 */
export const REFUSED_REPLAY: Replay = parseReplay(refusedData);

/**
 * Not a tracked fixture -- there is no Rust producer for this exact shape,
 * because `run_into` never leaves `refused: null` on an all-`Pending`
 * document by accident; that is precisely the hazard the field exists to
 * distinguish from a genuine refusal. Derived from the tracked refused
 * document so the only difference from it is the one field under test.
 */
export const ALL_PENDING_ATTEMPTED_REPLAY: Replay = {
    ...REFUSED_REPLAY,
    refused: null
};

/** `Replay` documents above, pre-serialised, for tests that consume raw JSON text. */
export const REALISTIC_REPLAY_JSON = JSON.stringify(REALISTIC_REPLAY);
export const REFUSED_REPLAY_JSON = JSON.stringify(REFUSED_REPLAY);
