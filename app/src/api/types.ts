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
 * One frame file exactly as it exists on disk -- `crates/server/src/manage/frames.rs`.
 *
 * A frame's filename is a measurement, not a claim: the mod names it from
 * `game.tick` inside the game, and `tick`/`camera` here are only ever parsed
 * back out of that name. Both are `| null`, always present as keys, for a
 * name that does not fit the `tick-<digits>-<camera>.jpg` pattern -- such a
 * file is reported, never dropped from the list.
 */
export interface FrameEntry {
    /** Which `client<N>` directory this frame was captured by. */
    client: number;
    tick: number | null;
    camera: string | null;
    /** The filename exactly as it appears on disk; pass to `frameUrl`. */
    name: string;
    bytes: number;
}

/**
 * `GET /api/v1/frames` response.
 *
 * `clients` and `frames` together distinguish three states that must not be
 * conflated: no `clients` at all means the run has never happened; a
 * `clients` entry with no matching `frames` means capture has not produced
 * anything yet; both non-empty means frames are present. Never computed from
 * a start tick and a stride -- always a live directory listing, so a dropped
 * frame (multiplayer clients catching up do not honour `force_render`) shows
 * up as a genuine gap rather than being smoothed over.
 */
export interface FramesManifest {
    /** `client<N>` directories discovered under the workspace. */
    clients: number[];
    frames: FrameEntry[];
    /**
     * The opaque run identifier from `frames/run.json`, or `null` when capture
     * ran without one, the file is absent, or it could not be read.
     *
     * Opaque: compare it for equality and nothing else. `null` means
     * **unknown**, never *no match* — see `runIdCheck` in `@/api/frameJoin`.
     */
    run: string | null;
    /**
     * What each client's own sidecar says, in `clients` order.
     *
     * `run` is which run the manifest is about; this is which run each
     * client's directory belongs to. They diverge when a client sat out the
     * current run and still holds an older one's frames — the case worth
     * marking in the UI rather than mixing in silently.
     */
    client_runs: ClientRun[];
}

/** One client's answer to "which run do your frames belong to". */
export interface ClientRun {
    /** The `N` of the `client<N>` directory. */
    client: number;
    /** That directory's own run id; `null` is **unknown**, never *no match*. */
    run: string | null;
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
    frames: number | null;
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

/** One frame copied into a run archive. */
export interface ArchivedFrame {
    /** The bot that captured it. Bots and clients are 1:1. */
    bot: number;
    /** `game.tick` at capture, or `null` when the filename does not parse. */
    tick: number | null;
    camera: string | null;
    /** Path relative to the run directory. */
    file: string;
}

/** `GET /api/v1/runs/{id}/frames` response. */
export interface RunFramesResponse {
    frames: ArchivedFrame[];
}

/** One thing a bot did, placed on the tick axis. */
export interface Lane {
    bot: number;
    /**
     * The action id. **Not unique across a run** -- ids restart with every
     * plan, so this identifies an entry only together with `bot` and
     * `from_tick`.
     */
    id: number;
    /** What the plan called it, e.g. `mine 4 iron-ore`. */
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
          bots: number[];
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
    | {kind: 'frame'; bot: number; camera: string; file: string}
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
    | {kind: 'run_finished'; outcome: string; elapsed_ticks: number}
    /** A kind this build does not know. The server never emits it, but a
     *  future variant decodes to this rather than failing to parse. */
    | {kind: 'unknown'};

/**
 * One line of `events.jsonl`.
 *
 * Mirrors `factorio_bot_core::record::Event`: the `#[serde(flatten)]` of
 * `EventKind` into `Event` is why the server publishes this as an `allOf` of
 * `EventKind` and `{tick, wall_ms}` rather than a single flat object.
 */
export type Event = EventKind & {
    tick: number;
    wall_ms: number;
};

/** `GET /api/v1/runs/{id}/events` response. */
export interface EventsResponse {
    events: Event[];
    /** Lines that did not parse -- in practice the truncated last line of a crashed run. */
    skipped: number;
}
