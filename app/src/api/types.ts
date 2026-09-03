/**
 * Hand-written mirrors of the server types that are *not* generated into
 * `models/types.ts`.
 *
 * The generator only sees `crates/core` (the types deriving `TypeScriptify`),
 * and everything below lives in `crates/server` instead: the request and
 * response bodies of the management routes. They are written by hand, but
 * they are **not** unchecked. A chain of three guards ties them to what the
 * server publishes:
 *
 * 1. `crates/server/tests/openapi.rs` keeps `openapi.snapshot.json` equal to
 *    the `/openapi.json` the real router serves, so the snapshot cannot rot.
 * 2. `openapi.contract.spec.ts` asserts the snapshot against a table of what
 *    the client assumes each schema looks like.
 * 3. That table is *typed against the interfaces below*
 *    (`objectContract<InstanceStatus>`, `enumContract<JobStatus>`), so `tsc`
 *    fails under `pnpm run lint` if a name here and a row there disagree in
 *    either direction.
 *
 * So renaming a field in `crates/server` and mirroring it through the
 * snapshot and the contract table, but *not* here, does not compile -- which
 * is the failure this chain exists to force, because the alternative is code
 * that compiles and reads `undefined` in the browser.
 *
 * What the chain does not cover, so that this comment does not overstate it:
 *
 * - **The JSON type of a field.** The contract table states `type: 'integer'`
 *   or `'string'` as its own claim; it is pinned to the spec but not to the
 *   `number`/`string` written here. Nullability *is* pinned both ways.
 * - **`OutputStream`.** The job event stream is `text/event-stream` with no
 *   schema in the document, so there is nothing for the table to check it
 *   against. Nothing imports it yet either.
 * - **Application `code` values.** `code: 2` is a value, not a schema; it is
 *   pinned on the server side by the three `not_started`/`not_running` tests
 *   in `crates/server/tests/`.
 */

/** `GET /api/v1/instance` -- `crates/server/src/manage/instance.rs`. */
export interface InstanceStatus {
    started: boolean;
    /** A start accepted by POST /api/v1/instance/start is still running. */
    starting: boolean;
    client_count: number;
    server_port: number | null;
    rcon_port: number | null;
    /** Why the last start attempt failed, or `null`. */
    last_error: string | null;
}

/**
 * The `202` from `POST /api/v1/instance/start`.
 *
 * Accepted is not started: the start runs detached because a first-run
 * archive extraction takes minutes. Progress is read from `InstanceStatus`.
 */
export interface StartAccepted {
    accepted: boolean;
}

/** Body of `GET`/`PUT`/`POST /api/v1/scripts/file`. */
export interface ScriptContent {
    code: string;
}

/** `GET /api/v1/fs/exists` -- an arbitrary server path, not a script path. */
export interface ExistsResponse {
    exists: boolean;
}

/**
 * Body of `POST /api/v1/scripts/execute`.
 *
 * Exactly one of `path` or `code` must be set; the server answers 400 for
 * both and for neither. `language` is ignored when `path` is given and
 * defaults to `"lua"` otherwise, and `bot_count` defaults to the configured
 * `factorio.client_count`.
 */
export interface ExecuteRequest {
    path?: string;
    code?: string;
    language?: string;
    bot_count?: number;
}

/** The `202` from `POST /api/v1/scripts/execute`. */
export interface ExecuteAccepted {
    job_id: string;
}

export type JobStatus = 'running' | 'succeeded' | 'failed';

/** One script run, live or historical -- `crates/server/src/jobs.rs`. */
export interface Job {
    /** A decimal counter, serialised as a string so it never loses precision. */
    id: string;
    /** `null` for inline code from the editor, which has no path. */
    script: string | null;
    status: JobStatus;
    started_at_ms: number;
    finished_at_ms: number | null;
    stdout: string;
    stderr: string;
    /** Set only when `status` is `failed`. */
    error: string | null;
    /**
     * The run's replay document, already serialised as JSON, or `null` for a
     * run that has not produced one. Opaque here on purpose: the server does
     * not parse it either, so this module never types its shape.
     */
    replay: string | null;
}

/** Which of a job's two output buffers a line came from. */
export type OutputStream = 'stdout' | 'stderr';

/** A node of `GET /api/v1/scripts/tree` -- mirrors `ScriptTreeNode` in `crates/core/src/types.rs`. */
export interface ScriptTreeNode {
    key: string;
    label: string;
    leaf: boolean;
    children: ScriptTreeNode[];
}

/** The `factorio` field of `GET /api/v1/settings` -- mirrors `FactorioSettings` in `crates/core/src/settings.rs`. */
export interface FactorioSettings {
    client_count: number;
    factorio_archive_path: string;
    map_exchange_string: string;
    rcon_pass: string;
    rcon_port: number;
    recreate: boolean;
    seed: string;
    workspace_path: string;
}

/** The `restapi` field of `GET /api/v1/settings` -- mirrors `RestApiSettings` in `crates/core/src/settings.rs`. */
export interface RestApiSettings {
    port: number;
    web_root: string | null;
}

/**
 * A 2D game-world coordinate. Mirrors `Position` in `crates/core/src/types.rs`.
 *
 * `y` increases DOWNWARD -- the same direction screen and SVG coordinates do
 * -- verified against a live capture (see `Rect` below). A view that flips
 * this axis to look "more like a normal graph" renders every position wrong
 * while looking entirely plausible.
 */
export interface Position {
    x: number;
    y: number;
}

/**
 * An axis-aligned bounding box, `left_top` to `right_bottom`. Mirrors `Rect`
 * in `crates/core/src/types.rs`.
 *
 * "Top" has the numerically SMALLER `y`: `crash-site-spaceship` in
 * `crates/core/tests/live-2.1.17-entities-spawn.json` has
 * `left_top.y = -9.296875` and `right_bottom.y = -1.5`. Do not swap them when
 * computing a rect's height, and do not negate `y` when placing one on
 * screen -- see `app/src/components/map/MapEntities.vue`.
 */
export interface Rect {
    left_top: Position;
    right_bottom: Position;
}

/**
 * One inventory slot, Factorio 2.0's quality-tagged format. Mirrors
 * `InventoryItemWithQuality` in `crates/core/src/types.rs`.
 */
export interface InventoryItemWithQuality {
    name: string;
    quality: string;
    count: number;
}

/**
 * One entity as `GET /api/v1/game/find-entities` reports it. Mirrors
 * `FactorioEntity` in `crates/core/src/types.rs`.
 *
 * `name`, `entity_type`, `position`, `bounding_box` and `direction` are
 * always present. Everything else is `Option<_>` on the Rust side with no
 * `skip_serializing_if`, so the server always sends the field -- present and
 * `null`, never omitted -- which is why these are typed `| null` rather than
 * `?`, matching `InstanceStatus` above.
 *
 * The map view (`app/src/components/map/`) reads only the required fields
 * plus `amount`; the inventory and ghost fields are declared here for
 * completeness with the server's schema but nothing renders them yet.
 */
export interface FactorioEntity {
    name: string;
    entity_type: string;
    position: Position;
    bounding_box: Rect;
    /** Factorio 2.x `defines.direction`: 0..=15, north at 0, clockwise. */
    direction: number;
    drop_position: Position | null;
    pickup_position: Position | null;
    output_inventory: InventoryItemWithQuality[] | null;
    fuel_inventory: InventoryItemWithQuality[] | null;
    /** Only present (non-null) for `entity_type: "resource"`. */
    amount: number | null;
    /** Only present for crafting machines. */
    recipe: string | null;
    ghost_name: string | null;
    ghost_type: string | null;
}

/**
 * One archived run, as it appears in a listing.
 *
 * Everything except `run_id` and `finished` is nullable because an unfinished
 * run genuinely lacks it. Present-and-null, never omitted: a caller must be
 * able to tell "this run never finished" from "this build does not report
 * outcomes".
 */
export interface RunSummary {
    run_id: string;
    /**
     * Whether the run reached `finish`. `false` means crashed *or* still
     * going -- the server cannot tell those apart and does not pretend to.
     */
    finished: boolean;
    /** Unix seconds. For "when was this", never for comparing two runs. */
    started_unix: number | null;
    finished_unix: number | null;
    outcome: string | null;
    /** A duration in game ticks, not the tick the run ended at. */
    elapsed_ticks: number | null;
    events: number | null;
    splits: number | null;
}

/** `GET /api/v1/runs` response. */
export interface RunsResponse {
    runs: RunSummary[];
}

/**
 * One milestone's timing -- the speedrun split.
 *
 * Measured in ticks, because two runs are compared on the game's clock and
 * never on wall time: a headless server and a graphical client with three
 * cameras do not run at the same speed, so wall time compares hardware.
 */
export interface Split {
    index: number;
    goal: string;
    started_tick: number;
    /** `null` when the run ended without closing this milestone. */
    ended_tick: number | null;
    /** `satisfied`, `stuck`, `stuck_silent`, `exhausted`, or `unfinished`. */
    outcome: string;
    /**
     * `ended_tick - started_tick`, or `null` while unfinished.
     *
     * Never subtract these yourself: a null minus a number is a plausible
     * zero, and zero looks exactly like a fast milestone.
     */
    elapsed_ticks: number | null;
}

/** `GET /api/v1/runs/{id}` response. */
export interface RunDetail {
    summary: RunSummary;
    /**
     * From `splits.json` when the run finished, otherwise derived from its
     * events -- so a crashed run still shows the milestones it got through.
     */
    splits: Split[];
}

/** One thing a bot did, placed on the tick axis. */
export interface Lane {
    bot: number;
    /**
     * The action id, or `null` for a lane that is not an action.
     *
     * **Not unique across a run** when present -- ids restart with every
     * plan, so this identifies an entry only together with `bot` and
     * `from_tick`.
     *
     * `null` means the lane has no action id, which today means it is a
     * **walk**: the scheduler emits a walk as its own step with no action id.
     * Borrowing the walk's step index for this field would put a number from
     * a different id space under a name that means action id.
     */
    id: number | null;
    /**
     * What the plan called it, e.g. `mine 4 iron-ore`.
     *
     * A walk has no label anywhere in the plan, so its lane's label is
     * composed by the server from the destination the schedule asked for --
     * `walk to (-23.5, 18.5)`.
     */
    action: string;
    from_tick: number;
    /**
     * `null` for an action dispatched and never settled.
     *
     * Drawn unterminated rather than dropped: the bot really did start it and
     * nothing came back. Dropping it would make a lost action look like one
     * that never happened.
     */
    to_tick: number | null;
    /** `null` while unterminated, otherwise the verdict the game gave. */
    status: string | null;
    error: string | null;
}

/** `GET /api/v1/runs/{id}/lanes` response. */
export interface RunLanesResponse {
    lanes: Lane[];
}

/** One bot's world state at the sample's tick. */
export interface BotSample {
    id: number;
    /** As the game reports it. A tile centre stays `-40.5`; nothing rounds. */
    position: Position;
    /** Item name to count. */
    inventory: Record<string, number>;
    /** Queue *length*, not its contents. */
    crafting_queue: number;
    mining: string | null;
}

/** What the force is researching, if anything. */
export interface ResearchSample {
    name: string;
    /** 0.0 to 1.0. */
    progress: number;
    /**
     * Always `null` today: no writer, mod-side or Rust-side, computes an
     * estimate yet. `null` means "not computed", not "the game reported
     * none".
     */
    eta_ticks: number | null;
}

/** Cumulative item counts, never per-interval. */
export interface ProductionSample {
    made: Record<string, number>;
    consumed: Record<string, number>;
}

export interface PowerSample {
    generated_kw: number;
    consumed_kw: number;
    /** Consumed over demanded. */
    satisfaction: number;
}

/**
 * The payload half of one `samples.jsonl` line, tagged by `kind`.
 *
 * Mirrors `factorio_bot_core::record::samples::SampleKind`, an internally
 * tagged Rust enum -- the server publishes it as an OpenAPI `oneOf`, one
 * member per variant, each carrying its own literal `kind`.
 */
export type SampleKind =
    | {kind: 'bots'; bots: BotSample[]}
    | {
          kind: 'force';
          /** Null when nothing is queued -- present and null, not absent. */
          research: ResearchSample | null;
          techs_unlocked: number;
          production: ProductionSample;
          power: PowerSample;
      }
    /** A kind this build does not know. The server never emits it, but a
     *  future variant decodes to this rather than failing to parse. */
    | {kind: 'unknown'};

/**
 * One line of `samples.jsonl`.
 *
 * Mirrors `factorio_bot_core::record::samples::Sample`: the `#[serde(flatten)]`
 * of `SampleKind` into `Sample` is why the server publishes this as an
 * `allOf` of `SampleKind` and `{tick, run}` rather than a single flat object.
 */
export type Sample = SampleKind & {
    /**
     * The schema this line was written under. Kept as a real field -- not
     * just probed and discarded on read -- so an archived line still carries
     * its stamp and a future schema bump can refuse an old archive loudly
     * instead of silently losing data.
     */
    schema: number;
    tick: number;
    /**
     * The run id the mod stamped on this line, or `null` for a line written
     * before this field existed.
     */
    run: string | null;
};

/** `GET /api/v1/runs/{id}/samples` response. */
export interface RunSamplesResponse {
    samples: Sample[];
    /**
     * Lines that did not parse -- in practice `bots = {}` serialising to
     * `"{}"` rather than `"[]"` when no player is connected. Reported rather
     * than swallowed, matching `EventsResponse.skipped`.
     */
    skipped: number;
}

/** What one bot built or removed, or what a keyframe observed. */
export interface EntitySnapshot {
    name: string;
    /** Unrounded. A resource sits at a tile centre and stays there. */
    position: Position;
    /**
     * Factorio 2.0 uses 16 values. For an inserter this is the side it PICKS
     * UP from, not the side it drops into -- never invert or normalise it.
     */
    direction: number;
}

export interface Bounds {
    left: number;
    top: number;
    right: number;
    bottom: number;
}

/** An entity present on exactly one side of a keyframe comparison. */
export interface Divergence {
    entity: EntitySnapshot;
    /** Which side has it: `"game"` or `"model"`. */
    only_in: string;
}

/**
 * The payload half of one `map.jsonl` line, tagged by `kind`.
 *
 * Mirrors `factorio_bot_core::record::map::MapKind`, an internally tagged
 * Rust enum -- the server publishes it as an OpenAPI `oneOf`, one member per
 * variant, each carrying its own literal `kind`.
 */
export type MapKind =
    | {
          kind: 'placed';
          bot: number;
          /** What the executor asked for. */
          intent: EntitySnapshot;
          /** What the game reports it created. */
          actual: EntitySnapshot;
          /** Field names that differ, or null when they agree. */
          drift: string[] | null;
      }
    // Reserved, not yet produced: there is no `take_removal` seam in the
    // codebase, so nothing ever writes this variant today. Decodable from
    // day one so a future writer needs no client-side change.
    | {kind: 'removed'; bot: number; entity: EntitySnapshot}
    | {
          kind: 'keyframe';
          bounds: Bounds;
          /** What the game reports inside `bounds`. */
          game: EntitySnapshot[];
          /** What our `EntityGraph` believes is inside `bounds`. */
          model: EntitySnapshot[];
          /** Entities present in exactly one of them. */
          divergence: Divergence[];
      }
    /** A kind this build does not know. The server never emits it, but a
     *  future variant decodes to this rather than failing to parse. */
    | {kind: 'unknown'};

/**
 * One line of `map.jsonl`.
 *
 * Mirrors `factorio_bot_core::record::map::MapRecord`: the `#[serde(flatten)]`
 * of `MapKind` into `MapRecord` is why the server publishes this as an
 * `allOf` of `MapKind` and `{tick}` rather than a single flat object.
 */
export type MapRecord = MapKind & {
    tick: number;
};

/** `GET /api/v1/runs/{id}/map` response. */
export interface RunMapResponse {
    map: MapRecord[];
    /** Lines that did not parse. Reported rather than swallowed, matching
     *  `EventsResponse.skipped`. */
    skipped: number;
}

/**
 * One scheduled step, as the planner intended it -- carried on
 * `EventKind`'s `plan_created` variant so a run's record shows what was
 * planned, not only what happened.
 */
export interface PlannedStep {
    id: number;
    bot: number;
    /** What the plan called it, e.g. `mine 10 iron-ore` -- the same label a `Lane` carries. */
    action: string;
    /** Ids this step waits on. */
    deps: number[];
    /**
     * Ticks from the plan's *start*, not an absolute `game.tick`: a plan is
     * computed before it is dispatched and does not know its own origin. The
     * viewer converts planned ticks to observed ticks in exactly one place --
     * `observedOrigin()`.
     */
    planned_start: number;
    planned_duration: number;
}

/**
 * Why a milestone needed no work -- carried on `EventKind`'s
 * `milestone_satisfied` variant.
 *
 * A milestone can close after zero iterations for two entirely different
 * reasons: the world already met the goal, or the planner returned an empty
 * plan that the supervisor's "an empty plan means satisfied" rule then
 * reported as success. `unknown` is a run recorded before this field
 * existed, and must never be read as either of the other two -- that would
 * be guessing exactly what this field exists to stop guessing.
 */
export type SatisfiedReason = 'already_satisfied' | 'plan_empty' | 'unknown';

/**
 * How an `ActionFailure` failed, coarse enough to group by in a query.
 *
 * `partial_transfer` is deliberately not `rejected`: a rejection moved
 * nothing, while a partial transfer already moved some of what was asked for
 * and left the rest behind. `ActionFailure.detail` then carries both counts
 * and the item, e.g. `moved 18 of 20 iron-plate`.
 */
export type FailureKind =
    | 'missing_item'
    | 'unreachable'
    | 'blocked'
    | 'partial_transfer'
    | 'rejected'
    | 'timeout'
    | 'other';

/**
 * A structured failure, carried *beside* `EventKind`'s `action_settled.error`
 * string rather than instead of it: the string is what a person reads, `kind`
 * is what a query groups by.
 */
export interface ActionFailure {
    kind: FailureKind;
    detail: string | null;
}

/**
 * Why a walk did not arrive.
 *
 * A separate vocabulary from `FailureKind`: every distinction here is about
 * the pathfinder, and none of `FailureKind`'s substantive members means
 * anything for a walk.
 *
 * `no_path` against `pathfinder_busy` is the distinction the record exists to
 * keep. `no_path` means the pathfinder searched and found nothing -- a fact
 * about the destination that repeating the walk will not change.
 * `pathfinder_busy` means it never searched (the request queue was full, or
 * the answer never came), so nothing was learned and asking again is the
 * right move.
 */
export type WalkFailureKind =
    | 'no_path'
    | 'pathfinder_busy'
    /**
     * Re-paths kept succeeding and the bot kept not arriving, until the mod
     * gave up. **Archive only**: the mod no longer re-paths for itself, and
     * this variant's successor is `stalled`.
     */
    | 'repath_limit'
    /**
     * A leg stopped progressing and the walk was abandoned. Reached only after
     * the Rust side has already spent its retry budget asking the game for
     * fresh paths, so it says the walking was stuck, not that the map is.
     */
    | 'stalled'
    /** No verdict ever arrived -- pairs with `status: 'lost'`. */
    | 'timeout'
    | 'other';

/**
 * A structured walk failure, carried *beside* `walk_settled.error` rather
 * than instead of it: the string is what a person reads, `kind` is what a
 * query groups by.
 *
 * Both positions are **observed**, taken from the message the mod wrote at
 * the instant it gave up -- not from `on_player_changed_position`, which
 * fires per tile crossed and so leaves a parked bot's position up to a tile
 * stale.
 */
export interface WalkFailure {
    kind: WalkFailureKind;
    /** Where the character actually stood when the mod gave up. `null` when the wording named no position. */
    from: Position | null;
    /**
     * The destination the walk was really steering at: the last waypoint of
     * the path the *game* returned.
     *
     * **Not the same as `walk_settled.to`**, which is what the schedule asked
     * for. The difference is the whole point of recording it -- an archived
     * run's three walk failures all have this land strictly inside the
     * collision box of a furnace the same run had built, up to 1.2 tiles from
     * the destination that was requested.
     */
    destination: Position | null;
}

/**
 * What happened, tagged by `kind`.
 *
 * Mirrors `factorio_bot_core::record::EventKind`, an internally tagged Rust
 * enum -- the server publishes it as an OpenAPI `oneOf`, one member per
 * variant, each carrying its own literal `kind`.
 *
 * `plan`, `reason` and `failure` are declared present (not `?`) rather than
 * optional: the server always serialises them for anything it writes today.
 * They publish as `required: false` only because `#[serde(default)]` lets an
 * *old* record on disk -- written before the field existed -- still
 * deserialise, which is a fact about reading old files, not about what a
 * live server sends.
 */
export type EventKind =
    | {
          kind: 'run_started';
          run_id: string;
          bots: number[];
          /** Present-and-null when unknown, never absent. */
          seed: number | null;
          factorio: string | null;
          git: string | null;
      }
    | {kind: 'milestone_started'; index: number; goal: string}
    | {
          kind: 'milestone_satisfied';
          index: number;
          iterations: number;
          reason: SatisfiedReason;
      }
    | {
          kind: 'milestone_stuck';
          index: number;
          outcome: string;
          best_steps: number | null;
          last_error: string | null;
      }
    | {
          kind: 'plan_created';
          milestone_index: number;
          steps: number;
          makespan: number;
          /**
           * The roster the planner expanded this plan against -- every bot it
           * was allowed to give work to, not the bots it happened to use.
           * `null` when the caller did not state one.
           *
           * Never the bots in the steps (that hides the bot a reader is
           * asking about) and never the roster the *process* was started with
           * (that names bots the plan was never made for -- one archived run
           * says `[1, 2]` for a plan made for `[2]` alone). Only whatever
           * called the planner knows, so a run whose driver did not say gets
           * `null` rather than a plausible substitute.
           */
          bots: number[] | null;
          /** The steps the planner actually produced, in enough detail to draw the DAG. */
          plan: PlannedStep[];
      }
    | {
          kind: 'action_dispatched';
          id: number;
          bot: number;
          action: string;
          /** Where the plan sent this action -- the planner's intent, not the
           *  game's resolution. `null` for `craft`/`research`, which act on
           *  no location; always present for `mine`/`place`/`insert`/`remove`. */
          target: Position | null;
      }
    | {
          kind: 'action_settled';
          id: number;
          bot: number;
          status: string;
          /** `null` when the game never reported a dispatch tick to subtract from. */
          elapsed_ticks: number | null;
          error: string | null;
          /** The same failure, classified. `null` on success. */
          failure: ActionFailure | null;
      }
    | {
          /**
           * A bot was sent walking -- the walking half of
           * `action_dispatched`, and written only when the game stamped a
           * dispatch tick for it.
           *
           * Walking is most of a run's wall clock, and no walk reached the
           * event log at all before this variant existed: a run could fail
           * three walks and leave one `milestone_stuck.last_error` behind,
           * with the rest only in a server log the next run overwrites.
           */
          kind: 'walk_dispatched';
          bot: number;
          /**
           * Which walk this is: its index in **this bot's own slice** of the
           * schedule, in schedule order.
           *
           * A walk has no action id, so `(bot, step_index)` is the only thing
           * that names one. It is neither an action id nor an index into
           * `plan_created.plan` (which is indexed over every bot's steps and
           * omits walks entirely) -- joining it to either produces confident
           * nonsense.
           */
          step_index: number;
          /**
           * Where the **schedule** sent the bot. An intent, not an arrival:
           * no arrival position is recorded, because the only observation of
           * one is up to a tile stale. It is also routinely a position the
           * bot cannot stand on -- arrival means within the step's radius of
           * it, never on it.
           */
          to: Position;
          /**
           * Plan-relative ticks, **not** a `game.tick` -- the same two clocks
           * `PlannedStep.planned_start` keeps apart, converted in exactly one
           * place (`observedOrigin()`).
           */
          planned_start: number;
          /** How long the scheduler expected the walk to take. Carried nowhere else: `plan_created.plan` holds only steps that have an action id. */
          planned_duration: number;
      }
    | {
          /**
           * A walk reached a verdict. Every walk that reached one gets
           * exactly one of these, whether or not the game stamped a tick.
           *
           * `failed` and `lost` are never collapsed: `failed` is the game
           * refusing the walk, `lost` is the game acknowledging it and never
           * answering -- a bot that may still be walking as far as anyone
           * knows.
           */
          kind: 'walk_settled';
          bot: number;
          /** The same `(bot, step_index)` identity as `walk_dispatched`. */
          step_index: number;
          /**
           * Where the schedule sent the bot, repeated here because a walk the
           * game never acknowledged has no `walk_dispatched` line and a walk
           * has no label anywhere in the record.
           */
          to: Position;
          status: string;
          /** `null` when the walk was not timed at both ends -- a duration nobody measured, not a duration of zero. */
          elapsed_ticks: number | null;
          error: string | null;
          /** The same failure, classified. `null` on success. */
          failure: WalkFailure | null;
      }
    | {
          /**
           * A bot was moved by `player.teleport` rather than by walking.
           * `on_player_changed_position` fires identically for a teleport and
           * a walked step, so this is the only signal that distinguishes
           * them -- a walk duration recorded before this variant existed may
           * include one or more silent teleports.
           */
          kind: 'teleport';
          bot: number;
          /** e.g. `walk_stuck`, `revive_ghost_blocked`, `place_blueprint_blocked`. */
          reason: string;
          from: Position;
          to: Position;
          distance: number;
          /** The walk action this happened during. `null` for the two
           *  blueprint/ghost-revive sites, which have no dispatched action. */
          action_id: number | null;
      }
    | {
          /**
           * The game refused a build, and the planner has stopped offering
           * that site. From this tick to the end of the run, every plan
           * excludes `entity`'s collision box centred at `position` -- so
           * this is the line that explains a planner which suddenly prefers
           * a further-away tile.
           *
           * `source` says when the game was asked, and the two are not
           * interchangeable. `'dispatch'` means a bot flew there and tried to
           * build: there is an `action_settled` failure at about the same
           * tick carrying the game's own wording. `'pre_check'` means the
           * planner asked before committing, so **no action for this site was
           * ever created** and there is no `action_settled` line to look for.
           */
          kind: 'placement_refused';
          /** The item the bot was holding, and the name the excluded
           *  collision box is looked up under. */
          entity: string;
          /** The centre the build was aimed at. The excluded region is the
           *  whole collision box centred here, not this one tile. */
          position: Position;
          /** `'dispatch'` or `'pre_check'` -- see above. */
          source: string;
          /** The distinct names of the entities the game found in the tested
           *  collision box, sorted. Always empty for `'dispatch'`, which has
           *  no way to ask. Empty for `'pre_check'` means no entity was in
           *  the footprint at all, which points at `tile`. */
          blockers: string[];
          /** The tile under the refused centre. `null` for `'dispatch'`. */
          tile: string | null;
      }
    | {
          /**
           * A character cannot reach open ground from where it stands.
           *
           * A flood fill over the occupancy model, started where the bot
           * stood, closed without reaching open ground. Every placement
           * around it was individually clear of the bot; the *set* formed a
           * wall, which is why no per-placement check could see it.
           *
           * Always sits beside a failed walk: the check only runs once the
           * game's own pathfinder has refused that bot a route from that
           * spot, so this is the diagnosis of that failure rather than a
           * second report of it. Nothing acts on it -- it changes no plan and
           * moves no bot.
           *
           * `run-1788432181-42528` had two of four bots frozen for 77% of the
           * run and produced no event like this, which is why it exists.
           */
          kind: 'bot_enclosed';
          bot: number;
          /** Where the character stood -- *observed*, unlike a walk's `to`. */
          position: Position;
          /** Reachable ground in square tiles of configuration space
           *  (obstacles grown by the character's own box), so smaller than
           *  the floor area a person would measure by eye. */
          pocket_tiles: number;
          /** How far the fill was allowed to look, in tiles. An enclosure
           *  wider than this window produces **no event**, so no events is
           *  not evidence that no bot was walled in. */
          searched_tiles: number;
      }
    | {kind: 'run_finished'; outcome: string; elapsed_ticks: number}
    /** A kind this build does not know. The server never emits it, but a
     *  future variant decodes to this rather than failing to parse. */
    | {kind: 'unknown'};

/**
 * One line of `events.jsonl`.
 *
 * Mirrors `factorio_bot_core::record::Event`: the `#[serde(flatten)]` of
 * `EventKind` into `Event` is why the server publishes this as an `allOf` of
 * `EventKind` and `{tick}` rather than a single flat object.
 *
 * There used to be a `wall_ms` here too. It was stamped when `record()` was
 * *called*, but `record.actions()`/`record.teleports()`/`record.refusals()`
 * are each called once per supervisor "ran" transition -- after an entire
 * multi-bot plan has finished executing -- so every event written in one
 * such call got the wall clock reading from the moment that whole batch was
 * flushed, not the moment each event actually happened. See
 * `factorio_bot_core::record::Event`'s doc comment for the full account and
 * `docs/superpowers/notes/2026-09-02-inventory-shortfall.md` for the
 * `33780 -> 738866` / `10-tick` finding that prompted the removal. Nothing
 * here ever read it.
 */
export type Event = EventKind & {
    tick: number;
};

/** `GET /api/v1/runs/{id}/events` response. */
export interface EventsResponse {
    events: Event[];
    /** Lines that did not parse -- in practice the truncated last line of a crashed run. */
    skipped: number;
}

// --- video: the host-side recording of a run -----------------------------
//
// Mirrors `factorio_bot_core::record::video`. The opt-in visual record of a
// run, and since the per-camera screenshots were retired (2026-09-02) the only
// one. See `docs/superpowers/specs/2026-09-02-video-capture-design.md`.

/** Where a recording got to. */
export type VideoStatus =
    /**
     * **In an archived run this is a defect, not a state.** It means the run
     * finished and nobody stopped the encoder, so the recording is of unknown
     * completeness and its clock never got its second calibration pair.
     */
    | 'recording'
    | 'stopped'
    /** Did not exit when asked. The file is still playable -- the container is fragmented. */
    | 'killed'
    /** The encoder stopped advancing while the run was live, e.g. its window disappeared. */
    | 'died'
    /** Never started; `reason` says why. The run went on without it. */
    | 'failed'
    /** Stopped early to leave the disk to the run's own records. */
    | 'stopped_low_disk';

/**
 * One `(host clock, encoder clock)` observation.
 *
 * The first pair fixes the video's zero; the second, taken at stop, fixes the
 * *rate*, which is a check rather than a parameter. One pair cannot detect a
 * capture that dropped frames at all, which is why there are two.
 */
export interface Calibration {
    /** Milliseconds since the recorder's epoch — the same epoch `TickSample.w` counts from. */
    host_wall_ms: number;
    /** ffmpeg's own `out_time_us`, in milliseconds. */
    out_time_ms: number;
}

/** `video.json`: what the encoder was asked for, what it did, and how it ended. */
export interface VideoRecord {
    run: string;
    file: string;
    /** The geometry `xwininfo` **observed**, which is what was actually captured. */
    width: number;
    height: number;
    /**
     * The geometry that was *asked for*. A window manager may refuse or adjust
     * it — a tiling compositor certainly will unless the window is floated —
     * and that is a warning on the run, not a failure.
     */
    requested_width: number;
    requested_height: number;
    fps: number;
    status: VideoStatus;
    reason: string | null;
    ffmpeg_exit: number | null;
    calibration: Calibration[];
    /** `null` when there are not two pairs to compare — **unknown**, never "fine". */
    rate_ok: boolean | null;
    window_id: string | null;
}

/** The observed span of a recording's clock. */
export interface TickRange {
    from: number;
    to: number;
}

/** `GET /api/v1/video` and `GET /api/v1/runs/{id}/video` response. */
export interface VideoManifest {
    /**
     * Which run this recording belongs to, from `video/run.json`.
     *
     * Opaque: compare it for equality and nothing else. `null` is
     * **unknown** — the recorder ran without an id, or the sidecar is absent
     * or unreadable — and never *no match*, because absence is not evidence
     * of a mismatch.
     */
    run: string | null;
    /** `null` when no recorder ever wrote here, which is every run by default. */
    video: VideoRecord | null;
    bytes: number | null;
    samples: number;
    skipped: number;
    /**
     * What `runTimeline.ts` feeds into its `drawn` set -- the span the axis
     * starts drawing at. `null` when the clock observed nothing.
     */
    tick_range: TickRange | null;
}

/** What one `ticks.jsonl` line is. */
export type TickKind =
    | 'sample'
    | 'start'
    /**
     * **No tick was observed here.** The only thing that can tell a stalled
     * game from a running one: a video has no null, so while the game stalls
     * the recorder keeps writing frames of the last drawn image.
     */
    | 'gap'
    | 'stop';

/**
 * One clock sample. Short keys because the file this mirrors has thousands of
 * lines and its only job is to be a table.
 */
export interface TickSample {
    /** `game.tick`. Absent on a `gap` line, and only there. */
    t?: number | null;
    /** Milliseconds since the recorder's epoch, stamped at the send/receive midpoint. */
    w: number;
    /** Always present, including for an ordinary `sample`. */
    k: TickKind;
    /** Why, for a gap. */
    reason?: string | null;
}

/** `GET /api/v1/video/ticks` and `GET /api/v1/runs/{id}/video/ticks` response. */
export interface VideoTicksResponse {
    samples: TickSample[];
    skipped: number;
}
