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
    /** The game port the server listens on; null means Factorio's default 34197. */
    factorio_port?: number | null;
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
 * One lane of a belt-like entity and what is on it. Mirrors `TransportLine` in
 * `crates/core/src/types.rs`.
 *
 * A belt is not an inventory: a `transport-belt` has two lanes, an
 * `underground-belt` four and a `splitter` eight, and which lane an item is on
 * is what decides whether an inserter can take it.
 */
export interface TransportLine {
    /**
     * `defines.transport_line`'s own name for this lane -- `left_line`,
     * `right_line`, `left_underground_line` and so on -- or `unmapped_<n>` for
     * an index the running game's `defines` could not name.
     */
    line: string;
    /**
     * What is riding on this lane, by item kind. Counts only: item positions
     * along the line are deliberately not carried.
     *
     * An empty array is an ordinary answer and means the lane is running
     * empty -- not that the lane is missing.
     */
    contents: InventoryItemWithQuality[];
}

/**
 * Which half of an underground-belt pair a `FactorioEntity` is. Mirrors
 * `crate::blueprint::UndergroundHalf` in `crates/core/src/blueprint.rs`,
 * carried onto `FactorioEntity::underground_half` (task 5) so the two halves
 * of a pair -- otherwise placed identically -- can be told apart.
 */
export type UndergroundHalf = 'input' | 'output';

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
    /**
     * What the machine has been given and not yet consumed -- a furnace's ore,
     * an assembler's ingredients, a lab's science.
     *
     * `null` and `[]` are different answers and both are real. `null` means
     * the entity has no input inventory at all (a belt, a chest, a tree) or
     * the record predates the field; `[]` means it has one and it is empty.
     * Without this field a furnace holding ore it is not smelting and a
     * furnace no ore ever reached looked identical from here.
     */
    input_inventory: InventoryItemWithQuality[] | null;
    /**
     * The lanes of a belt-like entity and what is riding on each, in
     * `defines.transport_line` order.
     *
     * `null` for anything that is not belt-connectable. A belt running empty
     * is *not* `null` -- it is a list of lanes with empty `contents`, and
     * separating those two is the whole reason the field exists.
     */
    transport_lines: TransportLine[] | null;
    /** Only present (non-null) for `entity_type: "resource"`. */
    amount: number | null;
    /** Only present for crafting machines. */
    recipe: string | null;
    ghost_name: string | null;
    ghost_type: string | null;
    /** Only present (non-null) for one half of an underground-belt pair. */
    underground_half: UndergroundHalf | null;
    /**
     * Which surface the entity stands on, by name (`LuaSurface.name`, unique
     * among surfaces; the *index* is reused after a deletion).
     *
     * `null` means the sender did not say -- **not** "Nauvis". A world dump or
     * run record written before 2026-09-06 lacks the field entirely, and while
     * every one of them is a Nauvis-only run, that is a fact about the mod's
     * `on_chunk_generated` guard, not about those bytes.
     *
     * Nothing renders it. This project is single-surface today; see
     * `docs/superpowers/notes/2026-09-06-surfaces-survey.md`.
     */
    surface: string | null;
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
    /**
     * The same figures per electric network, keyed by the smallest
     * sub-network id under each parent.
     *
     * The totals above hide the failure they are most often consulted about:
     * `generated_kw: 900` against `consumed_kw: 60` looks powered even when
     * the 900 kW is on one network and the machines that matter sit on an
     * island generating nothing. That island is its own entry here, and
     * `MachineSample.network` says which entry each machine is on.
     *
     * Published `required: false` only because `#[serde(default)]` lets a
     * pre-schema-2 archive decode without it; a live server always serialises
     * it, so it is declared present rather than optional -- the same reading
     * as `EventKind`'s `plan`, `reason` and `failure`.
     */
    networks: Record<string, NetworkPower>;
}

/** One electric network's own generation and demand. */
export interface NetworkPower {
    /**
     * Every electric *sub*-network under this parent, ascending. A machine
     * belongs here when its `MachineSample.network` is one of these. A closed
     * power switch puts several sub-networks under one parent, which is why
     * this is a set and not a single id -- and why the map key, the smallest
     * of them, is only a handle and never the thing to match on.
     */
    sub_ids: number[];
    generated_kw: number;
    consumed_kw: number;
    /** What consumers asked for, as opposed to what they got. */
    demanded_kw: number;
    /** Consumed over demanded, 1.0 when nothing is demanded. */
    satisfaction: number;
}

/**
 * One machine's state at the sample's tick.
 *
 * Fields that do not apply to an entity's type are `null`: `recipe`,
 * `crafting`, `progress` and `products_finished` are declared by the Factorio
 * API for crafting machines only, so on a lab or a boiler the question does
 * not exist rather than having an answer.
 */
export interface MachineSample {
    /** Prototype name, e.g. `assembling-machine-1`. */
    name: string;
    /**
     * Entity type. The sampled set is exactly `assembling-machine`,
     * `furnace`, `mining-drill`, `lab`, `boiler`, `generator`, `container`
     * and `logistic-container` -- the machines a run's plan places, plus the
     * chests it feeds them from.
     */
    type: string;
    /** As the game reports it. A tile centre stays `-40.5`; nothing rounds. */
    position: Position;
    /**
     * `LuaEntity.status`, by name -- the game's own verdict on why this
     * machine is or is not running. `working`, `no_power`, `low_power`,
     * `no_ingredients`, `full_output`, `not_enough_space_in_output`,
     * `no_recipe`, `no_fuel`, `not_plugged_in_electric_network` and
     * `no_minable_resources` are the ones this project actually hits. A value
     * the mod could not name arrives as `unmapped_<n>`.
     */
    status: string | null;
    /**
     * `LuaEntity.electric_network_id`. `null` means connected to no electric
     * network at all -- for an assembling machine, that is the answer to
     * "did the pole we placed actually connect it".
     *
     * Join against `NetworkPower.sub_ids` in the same tick's force sample. An
     * id matching no sampled network means no *pole* on the force reached it,
     * since `PowerSample.networks` is enumerated from poles.
     */
    network: number | null;
    /**
     * What `get_recipe()` reports -- the only honest verdict on what a
     * machine is set to, because `set_recipe` returns the items it *removed*
     * rather than a success flag.
     */
    recipe: string | null;
    /** `is_crafting()`. Null on a non-crafting machine. */
    crafting: boolean | null;
    /** 0.0 to 1.0, rounded mod-side to a thousandth. */
    progress: number | null;
    /**
     * Lifetime completed crafts. Often the fastest read in the file: an
     * assembler still reporting `0` after twenty minutes did not produce,
     * whatever else its row says.
     */
    products_finished: number | null;
    /**
     * This machine's lifetime item count, in ITEMS rather than crafts: a
     * `copper-cable` craft yields two, so an assembler with
     * `products_finished: 3` reports `produced: 6`. Between two samples, the
     * difference is what this machine made -- which is what turns "who made
     * this output" from an inference into arithmetic.
     *
     * `null` for a machine that makes no items (lab, boiler, steam engine,
     * chest) and for a producer whose count cannot be obtained.
     * `produced_source` says which, always.
     */
    produced: number | null;
    /**
     * How `produced` was obtained: `game` (the game's own craft counter times
     * the recipe yield), `accumulated` (the mod counted it per tick from the
     * fall in the resource a drill is mining -- Factorio gives a drill no
     * counter at all), `unavailable` (a producer this cannot count, such as a
     * pumpjack on infinite crude oil) or `not-a-producer` (a lab, boiler,
     * engine or chest). `null` only for a run archived before the counters
     * existed.
     */
    produced_source: string | null;
    /**
     * Set when this drill shared its resource tile with another drill and
     * both were credited with the same fall: the count is then an upper bound
     * and the sum double-counts.
     */
    produced_shared: boolean | null;
    /**
     * For a mining drill, the resource under it -- what separates
     * `no_minable_resources` on a depleted patch from a drill that was never
     * placed over ore.
     */
    mining: string | null;
    /**
     * Ingredient inventory. Empty for a type that has none.
     *
     * The mod omits an empty inventory on the wire and the server restores it,
     * so these three are `required: false` in the spec but never actually
     * absent from a response.
     */
    input: Record<string, number>;
    /**
     * Output inventory, and for a chest its whole contents.
     *
     * Non-empty with `status: full_output` is a cell that produced and then
     * jammed; empty on the chest feeding a machine reporting
     * `no_ingredients` is a cell that ran dry. Opposite repairs.
     */
    output: Record<string, number>;
    /** Fuel inventory, for burners. Empty with `status: no_fuel` is the whole story. */
    fuel: Record<string, number>;
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
    /**
     * Per-machine state, on the same 300-tick beat as `force` and written from
     * the same handler -- so a machine's `network` joins to that tick's
     * `power.networks` without interpolating.
     *
     * `machines` is keyed by `unit_number`, not an array: the mod's
     * `table_to_json` cannot tell an empty array from an empty object, which
     * is why a `bots` line from a run with no connected players is
     * unparseable, and a run's first minutes legitimately have no machines.
     *
     * `truncated` counts machines seen but not written, when a base exceeds
     * the mod's per-line cap. Zero on every run so far; non-zero means this
     * line is a prefix, which a reader must be able to tell from "this is all
     * of them".
     */
    | {
          kind: 'machines';
          machines: Record<string, MachineSample>;
          truncated: number;
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
 * One step that is waiting rather than working -- carried on `EventKind`'s
 * `batch_progress` variant.
 *
 * The counters on that event name a bot and never the work. One run had an
 * action in flight for eleven minutes and the record could show frozen
 * counters, a bot id and `lost: 0`; identifying the action took a live rcon
 * query against the running game, which a run that dies mid-batch does not
 * offer. Nor could the record be joined to find it: a batch's
 * `action_dispatched` lines are written only after the batch finishes.
 *
 * The identity half (`id`, `bot`, `action`, `target`) is deliberately the
 * same as `action_dispatched`'s, so the two join on `id` with no second
 * vocabulary to learn.
 */
/**
 * What a bot's hands put into a machine or a chest, on an
 * `action_dispatched`.
 *
 * The planner's intent, not the game's answer: it comes from the plan's own
 * insert action, and the matching `action_settled` is what says whether the
 * game did it. A `take` carries none -- it moves material out of a machine
 * and into a bot.
 */
export interface Delivery {
    /** What was put in. */
    item: string;
    /** How many. A count of items, never of stacks. */
    count: number;
    /** The prototype name of what received it -- `stone-furnace`,
     *  `burner-mining-drill`, `wooden-chest`. The same coal buys different
     *  amounts of running in different machines, so a fuel delivery means
     *  nothing without it. */
    entity: string;
    /** Which inventory, in the planner's vocabulary (`fuel`,
     *  `furnace_source`, `assembler_input`, `chest`, ...) rather than
     *  Factorio's unified `defines.inventory` key. */
    slot: string;
}

export interface WaitingStep {
    /** The action id, joinable to `action_dispatched.id` and
     *  `PlannedStep.id`. `null` for a walk, which has no action id at all --
     *  a true absence, identified by `bot` + `step_index` instead. */
    id: number | null;
    /** Which walk this is in this bot's own slice of the schedule -- the same
     *  key `walk_dispatched.step_index` uses, and **not** an action id or an
     *  index into `plan_created.plan`. `null` for an action. */
    step_index: number | null;
    bot: number;
    /** The plan's own label, verbatim -- the same string
     *  `action_dispatched.action` and `PlannedStep.action` carry. For a walk,
     *  a description of the walk, since a walk has no label. */
    action: string;
    /** The planner's intent, with the same caveats as
     *  `action_dispatched.target`. `null` for a craft/research. */
    target: Position | null;
    /** What it is waiting for: `predecessor`, `background_conflict`,
     *  `lag_deadline`, `research`, `reply` or `walk`. The three that used to
     *  be indistinguishable are `predecessor` (blocked on another step),
     *  `lag_deadline` (serving machine time the plan asked for -- **not a
     *  fault**, however long) and `reply` (dispatched, and the game has not
     *  answered).
     *
     *  A `reply` past ~360,000 ms is a finding on its own: the RCON layer
     *  gives a dispatched action 360 seconds and then reports it lost, so a
     *  longer one means the time went into a path request, an out-of-reach
     *  move, or the placement retry loop inside the same call. */
    waiting_on: string;
    /** The step blocking this one, for `predecessor` and
     *  `background_conflict`. `null` for the rest. */
    blocked_by: number | null;
    /** For `lag_deadline`, the absolute `game.tick` the wait runs until, when
     *  a predecessor supplied a tick to anchor it to. Compare it to the
     *  event's own `tick` to see how much of the wait is left. */
    deadline_tick: number | null;
    /** Wall-clock milliseconds in this state, measured by the executor when
     *  it entered the state -- not the beat interval rounded, and not "since
     *  a heartbeat first noticed it". The clock restarts when the state
     *  changes. */
    waiting_ms: number;
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
    /**
     * The bot had no character to act with: dead and waiting to respawn, in
     * a cutscene, or under another controller. `ActionFailure.detail` carries
     * the mod's `<why>` clause, e.g. `dead, respawns in 587 ticks`.
     */
    | 'no_character'
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
    /** The bot had no character to walk with. Says nothing about the map: no path was searched. */
    | 'no_character'
    /**
     * The pathfinder found a route whose last waypoint is inside a collision
     * box the entity graph knows, so the walk was refused before dispatch.
     * The aim was wrong, not the map.
     */
    | 'destination_blocked'
    /**
     * The pathfinder refused the walk **and** every short hop from where the
     * character stands: the bot cannot leave its own tile, so the destination
     * was not what was unreachable. A `bot_benched` event sits beside it.
     */
    | 'boxed_in'
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
          /**
           * The world as it stood when a milestone closed, written to disk so
           * a later run can start from it rather than re-deriving twenty
           * minutes of world. Recorded only once the file is *finished*:
           * `game.server_save` returns long before the engine has written
           * anything.
           */
          kind: 'savepoint_written';
          milestone_index: number;
          /** Relative to the run directory, e.g. `savepoints/milestone-3.zip`. */
          file: string;
          bytes: number;
          /** Wall clock, not ticks: the engine writes outside the tick. */
          wrote_ms: number;
      }
    | {
          /**
           * A savepoint was asked for and did not arrive. Present so that a
           * milestone with no savepoint can be told apart from a run that
           * never asked for one.
           */
          kind: 'savepoint_failed';
          milestone_index: number;
          error: string;
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
          /**
           * A plan is being executed **right now**, and this is how far it has
           * got -- a heartbeat with counters, written on a fixed wall-clock
           * interval while the batch is in flight.
           *
           * Every other event about a plan's execution is written *after* the
           * whole batch finished, so between `plan_created` and the first
           * `action_dispatched` the record used to say nothing at all, for
           * however long the batch took -- a quarter of an hour is ordinary
           * for a four-bot plan. That made "executing a long plan perfectly
           * well" and "planned, then dispatched nothing, ever" identical from
           * outside, and one archived run was killed as hung while its bots
           * were demonstrably still crafting.
           *
           * It states **no verdict**: there is no `stalled` flag and no
           * threshold anywhere in the writer, because `dispatched: 0` on a
           * plan of 152 steps needs none -- it is unambiguous at any
           * duration. The interval decides the resolution of the answer,
           * never its correctness, and a batch shorter than one interval
           * writes none of these.
           */
          kind: 'batch_progress';
          /** Wall-clock milliseconds since the executor was handed this batch.
           *  Wall clock and not ticks, because a stopped game is exactly the
           *  case where the tick clock cannot say whether anything is
           *  happening. */
          elapsed_ms: number;
          /** How many actions the plan has in total -- the denominator for
           *  every count below. Actions only; walks are counted separately. */
          total: number;
          /** How many actions have been dispatched at least once. **The
           *  number the whole event is for**: `dispatched: 0` beside a
           *  `plan_created` with steps is a plan nothing is executing. */
          dispatched: number;
          /** Dispatched, no verdict yet. */
          in_flight: number;
          /** Reached a verdict of any kind. */
          settled: number;
          /** Of the settled, how many the game judged and refused. */
          failed: number;
          /** Of the settled, how many were acknowledged and never answered --
           *  kept apart from `failed` for the same reason `action_settled`
           *  keeps them apart. */
          lost: number;
          /** Walks are counted separately because a walk has no action id and
           *  appears in none of the counts above. Without them a batch whose
           *  every bot is walking reports `in_flight: 0` and reads as four
           *  idle bots. */
          walks_dispatched: number;
          walks_settled: number;
          /** Wall-clock milliseconds since `dispatched` last went up, or
           *  since the batch began when it never has. Measured against
           *  dispatches, not settles: a bot waiting out a modelled lag edge
           *  settles nothing and dispatches nothing, and this growing is the
           *  honest report of that rather than an accusation. */
          since_last_dispatch_ms: number;
          /** Which bots have an action in flight, ascending. Empty is not by
           *  itself a problem -- see `walks_dispatched`. */
          bots_in_flight: number[];
          /** What each waiting step is waiting for, longest wait first and
           *  capped at 8 entries. The counters above name a bot; this names
           *  the work, which is what an eleven-minute silence needed and did
           *  not have.
           *
           *  An empty list means nothing is waiting -- and is **contradicted**
           *  by `in_flight > 0`, since a dispatched action is by definition
           *  waiting for its reply. The two come from different writers, so
           *  that disagreement is a broken reporter rather than a quiet run. */
          waiting: WaitingStep[];
          /** How many were waiting before the list was capped. Equal to
           *  `waiting.length` when nothing was dropped. */
          waiting_total: number;
          /** How many walks came to rest outside the reach of the action they
           *  served and needed a corrective step, cumulative. A walk stops on
           *  the outer ring of its annulus, holding back a measured margin for
           *  the arrival itself; this says how often that margin was wrong.
           *  It states no verdict -- zero means it could be tightened, a
           *  rising number means it is too thin. */
          reach_corrections: number;
          /** The disclosure for the one act in this project a player cannot
           *  perform: how many times this batch asked the game to *create
           *  ground* (`generate_chunks`), how many chunks that made, and how
           *  many asks failed. A bot cannot walk into ungenerated ground --
           *  the pathfinder refuses -- so exploration asks for it explicitly,
           *  clamped to the reveal a character standing there would have got
           *  for free.
           *
           *  States no verdict. `ground_generate_calls: 0` is a run that never
           *  explored, which is every run before 2026-09-06. Calls with no
           *  chunks and no failures is ground that already existed; failures
           *  equal to calls is the verb not working. */
          ground_generate_calls: number;
          ground_generated_chunks: number;
          ground_generate_failures: number;
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
          /** What this action put INTO a machine or a chest, when it put
           *  anything in. `null` for a walk, a craft, a place, a `mine` or a
           *  `take` -- and absent on every run archived before the field
           *  existed, whose quantities live only in the prose of `action`. */
          delivery?: Delivery | null;
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
          /** The `defines.direction` the build was aimed with, when known:
           *  the excluded box is the prototype's box turned this way, which
           *  is a different shape for anything not square. `null` on lines
           *  written before it was carried. */
          direction: number | null;
          /** `'dispatch'` or `'pre_check'` -- see above. */
          source: string;
          /** The distinct names of the entities the game found in the tested
           *  collision box, sorted. Filled on both sources; empty means the
           *  game scanned the box and found no entity, which points at
           *  `tile`. Empty with `tile: null` on a `'dispatch'` line is the
           *  older shape, where nothing was asked. */
          blockers: string[];
          /** The tile under the refused centre, when the game named it. */
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
          /** Reachable ground in whole tiles of the game's own pathfinding
           *  grid; the tile the character stands on is always one of them. */
          pocket_tiles: number;
          /** How far the fill was allowed to look, in tiles. An enclosure
           *  wider than this window produces **no event**, so no events is
           *  not evidence that no bot was walled in. */
          searched_tiles: number;
      }
    | {
          /**
           * The game said this bot cannot move, and the next plan will not
           * send it anywhere. After a refused walk the executor asked the
           * pathfinder for a short hop in each of four directions from the
           * character, and every one was refused -- the game's own verdict
           * on the *character*, which neither `bot_enclosed` (a fill over
           * the occupancy model) nor a `no_path` walk (a verdict on one
           * destination) can give. Unlike `bot_enclosed`, this is acted on.
           */
          kind: 'bot_benched';
          bot: number;
          /** Where the character stood when every hop was refused. Observed. */
          position: Position;
          /** How many hops were asked for and refused. */
          refused_hops: number;
          /** How far each hop was aimed, in tiles. */
          hop_tiles: number;
      }
    | {
          /**
           * A benched bot can move again, and the next plan may send it.
           * `why` is `walked` when a walk for it succeeded, or `probed` when
           * the re-probe before a plan found a hop the game would path.
           */
          kind: 'bot_released';
          bot: number;
          /** Where the bench had been earned. */
          position: Position;
          why: string;
      }
    | {
          /**
           * A bot was walked clear of a placement that would otherwise have
           * sealed it in -- the `bot_enclosed` that did not happen. The
           * executor asks before every placement whether the footprint would
           * close the fill around the character about to build it, and walks
           * it to the nearest tile that stays open first; this is why a
           * `place` can be preceded by a walk the plan has no step for.
           */
          kind: 'bot_stepped_aside';
          bot: number;
          /** Where the character stood when it was about to place. Observed. */
          from: Position;
          /** The tile centre it was walked to instead. */
          to: Position;
          /** The entity that was about to be placed. */
          placing: string;
          /** Where it was about to be placed. */
          site: Position;
          /** `'enclosure'` (the placement would have walled the character
           *  in) or `'footprint'` (the character stood inside the
           *  placement's own collision box). */
          reason: string;
          /** How many tiles the character would have been left with; `0`
           *  for `'footprint'`. */
          pocket_tiles: number;
      }
    | {
          /**
           * A bot lost its character: the game's `on_player_died`. Until the
           * character respawns (`respawn_in` ticks, 600 by default) every
           * action for this bot is refused with `failure.kind ===
           * 'no_character'`; this is the line that says why.
           */
          kind: 'bot_died';
          bot: number;
          /** Where the character stood when it died. `null` when the mod could not read it. */
          position: Position | null;
          /** What killed it (`'medium-worm-turret'`), when the game named a cause. */
          cause: string | null;
          /** Its prototype type (`'turret'`, `'unit'`). `null` exactly when `cause` is. */
          cause_type: string | null;
          /** `ticks_to_respawn` the tick after death. `null` is "not readable", never "never". */
          respawn_in: number | null;
      }
    | {
          /** The character is back. Pairs with `bot_died` by `bot`. */
          kind: 'bot_respawned';
          bot: number;
          /** Where the new character stands -- the spawn point, ordinarily. */
          position: Position | null;
      }
    | {
          /**
           * The mod completed a Factorio 2.0 trigger technology on a
           * headless run because the force had already done what the
           * trigger names (a hand-crafted lab, a placed entity), once every
           * prerequisite was researched. Explains an `on_research_finished`
           * with no research ever started; absent on a client run, where
           * the game fires every trigger itself.
           */
          kind: 'research_trigger_emulated';
          technology: string;
          /** `'craft-item'` or `'build-entity'`. */
          trigger: string;
          /** The item a `craft-item` trigger counted; `null` for `build-entity`. */
          item: string | null;
          /** The entity a `build-entity` trigger counted; `null` for `craft-item`. */
          entity: string | null;
          needed: number;
          /** What the force had done when the sweep read it -- at least `needed`. */
          count: number;
      }
    | {
          /**
           * The mod discarded generated chunks because they are not on
           * Nauvis. This project supports exactly one surface: the world
           * model keys entities, resources and tiles by position alone, so a
           * second surface's chunk would merge into Nauvis with no error
           * anywhere, and `on_chunk_generated` drops it instead.
           *
           * Space Age is enabled in this workspace, so a row here is
           * possible, and it means the run planned against an incomplete
           * world -- every map fingerprint and resource distance from it
           * describes Nauvis only. No run so far has produced one.
           */
          kind: 'surface_chunk_dropped';
          /** By name. `LuaSurface.index` is reused after a deletion. */
          surface: string;
          /**
           * Chunks dropped for this surface **since the last flush**, not for
           * the run: the run total is the sum over every row naming it.
           */
          chunks: number;
          /**
           * One example: the top-left **tile** of this window's first dropped
           * chunk -- (-32, 64), not chunk (-1, 2).
           */
          first_left_top_x: number;
          first_left_top_y: number;
      }
    | {
          /**
           * The supervisor changed the roster it plans for: a bot absent
           * past the bounded respawn wait was dropped, or one dropped earlier
           * came back. Every later `plan_created.bots` shows the new roster;
           * this is the line that says why it changed. Removals and returns
           * only -- a bot the run did not start with is never added.
           */
          kind: 'roster_changed';
          /** The roster from here on, ascending. */
          bots: number[];
          left: number[];
          returned: number[];
          /** The supervisor's own sentence for why. */
          reason: string;
      }
    | {
          /**
           * How much ground the world model was given, against how much
           * ground a bot actually covered.
           *
           * The mod ingests every entity of every chunk the *engine
           * generates* and never consults the force's charted area, so the
           * model knows about ground no character has been near -- ore, water
           * and nests alike. `run-1788532631-48030`'s furthest bot reached
           * 63.8 tiles while the model it produced held crude oil at 380 and
           * 505. This event is the disclosure of that; it states no verdict.
           *
           * Written into the append-only event log rather than into
           * `provenance.json` (start-only) or the manifest (finish-only):
           * a killed run still got its free vision. One baseline at the run's
           * first event, a beat every 18,000 ticks, and one more at finish.
           * **The last one in the log is the run's answer.**
           */
          kind: 'vision_measured';
          /** The furthest any bot has been observed from the map origin, in
           *  tiles, Euclidean. `null` is "nobody has observed a bot
           *  position", **not** "no bot moved" -- `bot_samples` separates the
           *  two, and it is `null` on every run's baseline measurement. */
          travelled_tiles: number | null;
          /** Which bot was that far out. `null` exactly when
           *  `travelled_tiles` is. */
          travelled_bot: number | null;
          /** The tick that bot was seen there -- not this event's tick. */
          travelled_at_tick: number | null;
          /** How many bot positions this run has archived, the denominator
           *  behind `travelled_tiles`. Zero beside a null distance means the
           *  instrument said nothing. */
          bot_samples: number;
          /** The furthest thing the world model holds from the map origin, in
           *  tiles. `null` when the model holds nothing at all -- a world
           *  nothing has read yet, not a model reaching zero tiles. */
          model_tiles: number | null;
          /** What is out there, e.g. `'crude-oil'`. A distance alone reads as
           *  an abstraction. `null` exactly when `model_tiles` is. */
          model_furthest: string | null;
          model_furthest_position: Position | null;
          /** The census behind `model_tiles`. */
          model_resource_tiles: number;
          /** Counted apart from the resource tiles: a nest 500 tiles out and
           *  an ore tile 500 tiles out are the same disclosure but not the
           *  same finding. */
          model_enemy_structures: number;
          /** `model_tiles / travelled_tiles` -- how many times further the
           *  model sees than the furthest bot went. `null` whenever either
           *  half is unknown or no bot has left the origin, and **not**
           *  floored at 1. */
          unearned_ratio: number | null;
      }
    | {
          /** What one `goal.plan` cost: wall time, and what the game clock
           *  did while the planner thought. The planner is wall-clock work
           *  and a game left running through it was charged `60 * speed`
           *  ticks per second of it -- 334 ticks for automation at 1x,
           *  1,837 at 10x -- which was the whole of the "faster game, longer
           *  run" tax. `goal.plan` now stops the clock around expansion;
           *  this event is the receipt. Written by the plan itself, so a
           *  plan that raised (and has no `plan_created`) still shows its
           *  cost. */
          kind: 'planning_timed';
          /** Wall clock inside expansion and scheduling, pre-check round
           *  trips included. */
          planning_ms: number;
          /** Whether the clock was stopped for the duration. `false` on a
           *  build without RCON or when the pause request failed, in which
           *  case `tick_after - tick_before` says what it cost. */
          paused: boolean;
          /** Why the clock was left running, when it was: `"attached
           *  server, clock left running"` for a `--connect` / `--server` run
           *  whose game may be somebody's live session, or `"pause request
           *  failed"`. `null` whenever `paused` is true. */
          reason: string | null;
          /** `game.tick` when planning began; `null` when nobody could ask. */
          tick_before: number | null;
          /** `game.tick` when planning ended; `null` when nobody could ask. */
          tick_after: number | null;
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
