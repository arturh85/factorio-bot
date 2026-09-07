use crate::action::InventorySlot;
use crate::error::PlannerError;
use crate::goal::Holder;
use crate::ids::{ActionId, BotId, ChainId, ItemId, Ticks};
use crate::method::produce::DrainPolicy;
use crate::method::util::rotated_collision_box;
use factorio_bot_core::constants::BOT_FORCE;
use factorio_bot_core::dashmap::DashMap;
use factorio_bot_core::factorio::util::{add_to_rect, calculate_distance};
use factorio_bot_core::factorio::world::{FactorioSurface, WalkRefusal};
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::types::{
    Direction, FactorioEntity, FactorioEntityPrototype, FactorioTechnology, FactorioTile,
    HandMiningObstacle, PlayerId, Pos, Position, Rect, ResourcePatch,
    VANILLA_CHARACTER_RESOURCE_CATEGORIES,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// How far two collision boxes may reach into each other before it counts.
///
/// Factorio lets boxes that merely touch along an edge coexist, and the
/// prototype numbers are binary fractions (a stone furnace is ±0.69921875),
/// so exact touching really happens. A 1/512 tile is finer than the 1/256 the
/// game stores positions at, so nothing this admits is a collision the game
/// would see; it only keeps float noise from reading as one.
const TOUCH_SLACK: f64 = 1. / 512.;

/// The literal `FactorioEntity::name` every ghost reports, regardless of what
/// it is a ghost OF -- verified live against Factorio 2.1.17. The entity it
/// will become lives in `FactorioEntity::ghost_name` instead; see
/// [`PlanState::ghosts_named_any`].
pub const GHOST_ENTITY_NAME: &str = "entity-ghost";

/// Half either side of a vanilla `character`'s collision box, used only when
/// the world's own `entity_prototypes` carries no `character` entry at all.
///
/// No real game omits its own player character from that table — this exists
/// for the hand-built fixtures that do, the same situation
/// `character_mining_speed`'s `VANILLA_CHARACTER_MINING_SPEED` fallback
/// covers. The number itself is not guessed: it is the `collision_box` of the
/// `character` entry in `crates/core/tests/entity-prototype-fixtures.json`,
/// which was captured off a live Factorio 2.1 game (`±0.19921875`, the same
/// source the `stone-furnace` and `assembling-machine-1` numbers quoted
/// elsewhere in this file come from).
const VANILLA_CHARACTER_COLLISION_HALF_SIDE: f64 = 0.19921875;

/// Half the side of the tile a resource entity occupies.
///
/// A tile spans one unit and the ore is reported at its *centre* — every real
/// resource sits at `(-40.5, -48.5)`, never `(-41, -49)` — so every point of
/// the tile is within this of the position the planner passes around. Used to
/// turn "a character must not stand on that tile" into a distance between two
/// tile centres; see [`PlanState::mining_tile_separation`].
const TILE_HALF_SIDE: f64 = 0.5;

/// How much more than one stack a crafting machine's *input* slot accepts, in
/// items, in vanilla 2.1.
///
/// An empirical constant of this game version, named as one. Measured on a
/// live 2.1.17 instance by creating a machine, inserting 1000–5000 of an
/// ingredient into `defines.inventory.crafter_input` and recording what was
/// accepted: iron-ore, copper-ore, stone and solid-fuel all took **70** on a
/// stack of 50; iron-plate **120** on a stack of 100; copper-cable **220** on a
/// stack of 200.
///
/// **The margin is additive, not proportional** — 50→70, 100→120, 200→220.
/// Proportional would have made the stack of 200 answer 240. It was the same
/// +20 across every machine, every recipe and every ingredient amount tried,
/// which is why it is one number here rather than a table.
///
/// Two caveats belong with it. Every measurement was on a *freshly created,
/// empty* machine, so whether a mid-craft machine answers the same is
/// unverified; and [`PlanState::slot_capacity`] uses this only to make a
/// transfer *smaller*, so being wrong costs an extra trip rather than a
/// failed action. Do not read it as a production-stop threshold: what makes a
/// machine stop is a different question with a different answer per machine
/// type (a stone furnace stops at a full output stack, an
/// assembling-machine-1 at three or four items).
pub const INPUT_OVERLOAD: u32 = 20;

/// A vanilla character's `resource_reach_distance`, used when no bot in the
/// roster reports a plausible one.
///
/// 2.7 is what live 2.1.17 reports (`crates/core/tests/live_2_1_payloads.rs`
/// pins it), and it is the same number `mine_step_aside_waypoint` in
/// `mods/BotBridge/control.lua` falls back to.
const VANILLA_RESOURCE_REACH: f64 = 2.7;

/// How far from an entity [`PlanState::electric_supply_kw`] looks for the
/// **first** poles that might power it, in tiles.
///
/// This is the seed of a traversal, not the extent of the search. Once a pole
/// is found the search continues along the wire from it (see
/// [`PlanState::electric_entities`]), so a generator any number of poles away
/// is reachable. `EntityGraph` still offers no "every entity" query, and this
/// keeps the common case — a plant beside the thing it powers — to a single
/// disc.
///
/// # Why it stopped being the whole search
///
/// It was 64 tiles and it was the entire extent, honestly documented as "a
/// bound, not a physical limit". Then oil arrived. **A pole run longer than 64
/// tiles carried power that [`crate::Condition::Powered`] could not see**, and
/// the scheduler checks the same condition, so it was a real ceiling and not a
/// verification artefact. Measured on seed 31337: a well at ~52 tiles planned;
/// at ~121, ~130 and ~140 it refused; at ~281 it refused for want of water.
/// Seed 31337's crude oil is 256–384 tiles out, so oil could not be planned at
/// all.
///
/// **Raising the constant was the obvious fix and the wrong one.** It trades a
/// false refusal for a full-map scan on every condition check, and any
/// constant is wrong for some map: 64 is wrong for this oil, and its
/// replacement is wrong for the next map. **The bound was a disc around the
/// consumer, while the thing that carries power is the wire.** The connectivity
/// pass below was never the defect — a generator ten poles away was simply
/// never a candidate for it to find.
const POWER_SEARCH_RADIUS: f64 = 64.;

/// Half the side of a pole's supply area, **from the pole's own prototype**.
///
/// `FactorioEntityPrototype::supply_area_distance` landed on 2026-09-06 and
/// this is its first reader. It is the field the game itself uses, so a mod
/// that retunes `small-electric-pole`, or ships a pole this crate has never
/// heard of, now gets its own number instead of ours — which is the whole
/// point, and is the owner's standing rule that a rate or a reach is *worked
/// out from data* rather than copied.
///
/// # Two things it is not, and both would be silent if got wrong
///
/// **A pole's convention is half the side, and a beacon's is not.** The same
/// prototype field answers for both `ElectricPole` and `Beacon`, and it means
/// different things on each: `2.5` on a small pole is a 5x5 supply *square*,
/// while `3` on a beacon is 3 tiles *beyond its 3x3 footprint*, i.e. 9x9. So
/// this gates on `entity_type == "electric-pole"` before believing the number
/// — without that gate a beacon standing in a plan would read as supplying
/// 6x6 of power, having never been wired to anything. The beacon convention
/// lives in [`crate::method::util::BeaconGeometry`] and deliberately shares no
/// helper with this one.
///
/// **An absent field is unknown, not zero.** A pre-2026-09-06 dump — which is
/// every archived world this project has, `workspace/scripts/map.json`
/// included — carries no `supply_area_distance` at all, and a pole that
/// supplies nothing is a different fact from a pole nobody asked. So the
/// answer falls back to [`vanilla_pole_supply_half_extent`], which is what
/// the field *would have said* on those maps, and a name that table does not
/// know answers `None` — no coverage, which under-credits rather than
/// over-credits, the direction every table in this file errs in.
///
/// A prototype the world does not carry at all is `None` for the same reason.
fn pole_supply_half_extent(
    prototypes: &DashMap<String, FactorioEntityPrototype>,
    name: &str,
) -> Option<f64> {
    let prototype = prototypes.get(name)?;
    if prototype.entity_type != "electric-pole" {
        return None;
    }
    prototype
        .supply_area_distance
        .or_else(|| vanilla_pole_supply_half_extent(name))
}

/// What [`pole_supply_half_extent`] answers for a **vanilla pole on a world
/// whose sender predates `supply_area_distance`**, from base 2.1.17's
/// `base/prototypes/entity/entities.lua` in this repo's `workspace/data`.
///
/// Not a table anybody should extend. It exists because deleting it would
/// blind the planner on every dump taken before 2026-09-06 — the entire
/// offline measurement basis, including the seed-31337 t=0 map — and a
/// planner that silently stops seeing poles is worse than one carrying four
/// numbers it can point at a file for. Every world recorded from here on
/// overrides it before it is ever consulted, and
/// `a_pole_prototype_without_the_field_falls_back_to_vanilla` is what pins
/// that this path is reached only when the sender said nothing.
///
/// A name it does not know contributes no coverage at all.
fn vanilla_pole_supply_half_extent(name: &str) -> Option<f64> {
    match name {
        // 5x5 supply area.
        "small-electric-pole" => Some(2.5),
        // 7x7.
        "medium-electric-pole" => Some(3.5),
        // 4x4 — a big pole is for spanning distance, not for covering ground.
        "big-electric-pole" => Some(2.0),
        // 18x18.
        "substation" => Some(9.0),
        _ => None,
    }
}

/// How far a pole of `name` can throw a wire, in tiles, read off **its own
/// prototype** — `FactorioEntityPrototype::maximum_wire_distance`.
///
/// Two poles are wired when their centres are within the **smaller** of their
/// two reaches, which is the game's rule and is why this is a per-pole number
/// rather than one constant. What this answers is one pole's contribution to
/// that minimum, never a verdict about a pair.
///
/// **The `entity_type == "electric-pole"` gate is not caution, it is the
/// difference between 4 poles and 98.** `get_max_wire_distance()` is the
/// maximum over *every* wire kind, so a machine reports its **circuit** wire
/// distance: measured over all 1,028 prototypes of a live 2.1.17 game, four
/// report a pole's copper span and 94 more report 9, 10 or 30 tiles of
/// circuit reach — `stone-furnace`, `wooden-chest`, `power-switch`,
/// `agricultural-tower`. Without the gate a network wires itself through an
/// assembling machine.
///
/// **`Some(0.0)` is not the same as absent.** The mod sends the game's own
/// zero for an entity with no wires, and that zero is believed: only `None` —
/// *the sender did not say* — falls back to [`vanilla_pole_wire_reach`].
///
/// # This is the table that had drifted
///
/// It was a hand-kept table of four vanilla names until 2026-09-07, when the
/// four were checked against the game's own data for the first time and
/// `big-electric-pole` read **30.0** against the game's **32**. Factorio 2.0
/// moved the value and nothing noticed, **because a hand-kept table of game
/// data is only ever read by code that agrees with it**: a legal big-pole span
/// read as a broken network, silently. The number was corrected in place that
/// day, and the field it should have come from was shipped through the mod
/// immediately after — which is this function.
fn pole_wire_reach(
    prototypes: &DashMap<String, FactorioEntityPrototype>,
    name: &str,
) -> Option<f64> {
    let prototype = prototypes.get(name)?;
    if prototype.entity_type != "electric-pole" {
        return None;
    }
    prototype
        .maximum_wire_distance
        .or_else(|| vanilla_pole_wire_reach(name))
}

/// What [`pole_wire_reach`] answers for a **vanilla pole on a world whose
/// sender predates `maximum_wire_distance`**, from base 2.1.17's
/// `base/prototypes/entity/entities.lua` in this repo's `workspace/data`.
///
/// Not a table anybody should extend, and the twin of
/// [`vanilla_pole_supply_half_extent`] in every respect — including *why it
/// survives*. Deleting the supply shim was measured rather than reasoned
/// about: with it gone and nothing else changed, all three offline goals
/// refused to expand at all, because every archived world predates the field
/// and so every pole supplied nothing — and the refusal blamed **the water**,
/// one layer downstream, with no mention of poles. Every dump this project
/// owns reads `None` here for exactly the same reason.
///
/// A world that declares the field overrides this before it is ever consulted;
/// a pole name it does not know answers `None`, which is no reach at all and
/// under-credits rather than over-credits.
fn vanilla_pole_wire_reach(name: &str) -> Option<f64> {
    match name {
        "small-electric-pole" => Some(7.5),
        "medium-electric-pole" => Some(9.0),
        // 32, not the 30 of Factorio 1.x.
        "big-electric-pole" => Some(32.0),
        "substation" => Some(18.0),
        _ => None,
    }
}

/// The `entity_type`s whose [`generation_kw`] this planner will credit.
///
/// **The gate is a determinism gate, not a completeness gate**, and it is the
/// reason reading `max_energy_production` off the prototype did not silently
/// switch solar on. `LuaEntityPrototype::get_max_energy_production()` answers
/// for a `solar-panel` too — with its **noon** figure — and for an
/// `accumulator` with its discharge limit. Crediting either would make the
/// same plan feasible or not according to what time of day the run started,
/// which is exactly the trap CLAUDE.md records as "the same blueprint runs or
/// does not run depending on when the run starts". A planner whose output must
/// be identical for identical inputs cannot credit a number that is not.
///
/// **The daylight channel does not lift this gate, and that is deliberate.**
/// [`PlanState::solar_average_kw`] now derives a panel's *average* output,
/// which is a function of surface constants and so is perfectly deterministic
/// — the determinism objection is answered. What is not answered is storage: an
/// array credited its average keeps a base alive only if the accumulators to
/// carry the night are actually standing, and
/// [`PlanState::accumulators_per_panel`] says how many that is without anything
/// yet checking that they exist. Crediting the average here before that check
/// exists would turn a *false refusal* into a base that dies at midnight, which
/// is the wrong direction to be wrong in. The two accessors are the arithmetic
/// a solar arm of `method::power` needs; wiring them into supply is that arm's
/// work, not this table's.
///
/// These three are steady while fuelled: a steam engine's 900 kW is the same
/// at every hour of every day. `generator` covers `steam-engine` and
/// `steam-turbine`, `burner-generator` and `fusion-generator` the Space Age
/// pair. A type not listed falls back to [`vanilla_generation_kw`] and thence
/// to `None` — no generation, which refuses rather than promises, the
/// direction every table in this file errs in.
const DETERMINISTIC_GENERATOR_TYPES: [&str; 3] =
    ["generator", "burner-generator", "fusion-generator"];

/// What a generator contributes to a network, in kW, when it is running,
/// **from its own prototype**.
///
/// `FactorioEntityPrototype::max_energy_production` is
/// `get_max_energy_production()`, a **method** on the 2.1.17 runtime API. It
/// is the only route to the number: a `steam-engine` carries no output figure
/// anywhere, at the data stage or on the runtime prototype. Its 900 kW is
/// `fluid_usage_per_tick * 60 * heat_capacity * (maximum_temperature -
/// default_temperature) * effectivity`, and `LuaEntityPrototype` exposes
/// `maximum_temperature` and `effectivity` but **not**
/// `fluid_usage_per_tick` -- doclint-allow: a Factorio data-stage name whose
/// absence from this tree is precisely the claim being made. So the physics
/// cannot be reassembled out here, and the runtime's own answer is what the
/// mod forwards.
///
/// Nameplate capacity, not observed output: nothing in `FactorioSurface` says
/// whether a steam engine has steam. See [`PlanState::electric_supply_kw`].
///
/// Gated on [`DETERMINISTIC_GENERATOR_TYPES`], and falling back to
/// [`vanilla_generation_kw`] for a world whose sender predates the field —
/// which is every dump this project has archived.
fn generation_kw(prototypes: &DashMap<String, FactorioEntityPrototype>, name: &str) -> Option<f64> {
    let from_prototype = prototypes.get(name).and_then(|prototype| {
        if !DETERMINISTIC_GENERATOR_TYPES.contains(&prototype.entity_type.as_str()) {
            return None;
        }
        // A zero is a generator that produces nothing, which is not a fact
        // worth crediting and is indistinguishable here from a runtime that
        // declined to answer.
        prototype.max_energy_production_kw().filter(|kw| *kw > 0.)
    });
    from_prototype.or_else(|| vanilla_generation_kw(name))
}

/// What [`generation_kw`] answers for a **vanilla generator on a world whose
/// sender predates `max_energy_production`**, from base 2.1.17's
/// `base/prototypes/entity/entities.lua` in this repo's `workspace/data`.
///
/// The counterpart of [`vanilla_pole_supply_half_extent`] and kept for the
/// same measured reason: deleting that one, with nothing else changed, made
/// **all three offline goals refuse to expand at all** — and the refusal
/// blamed the water, one layer downstream, with no mention of poles. Every
/// archived world here, `workspace/scripts/map.json` included, predates every
/// electrical field, so a fallback is the difference between a planner and a
/// planner that has gone blind.
///
/// Not a table anybody should extend. A name it does not know contributes
/// nothing.
///
/// **`steam-turbine`'s 5,800 is this project's number and the game's is
/// 5,820** — `1.0 * 60 * 0.2 * (500 - 15) * 1.0` — a 0.34% under-credit in an
/// entity nothing builds yet. Left as it was found rather than corrected,
/// because this table's whole job is to say what a pre-field world *would have
/// answered*, and every world it applies to was planned against 5,800.
/// `steam-engine`'s 900 is exact.
fn vanilla_generation_kw(name: &str) -> Option<f64> {
    match name {
        "steam-engine" => Some(900.0),
        "steam-turbine" => Some(5800.0),
        _ => None,
    }
}

/// What a headroom question is *about*, and therefore what
/// [`PlanState::electric_demand_kw_excluding`] must not charge against its own
/// answer.
///
/// # Why an exclusion exists at all
///
/// A headroom test asks "is there room for this draw", and the draw it names
/// is a draw that is *about to be* placed. The moment any of it is already in
/// the plan overlay — and it always is, because a placement is created in a
/// fork before the check that decides whether to keep it — the ledger charges
/// it twice: once as standing demand and once as the `kw` being asked for. The
/// predicate then answers differently on the second evaluation of an identical
/// plan, which is a non-idempotent condition in a supervisor loop.
///
/// # Why a region, and not a list of positions
///
/// [`Consumer`](Excluded::Consumer) was the whole of this type for as long as
/// every caller sited **one machine**. A caller siting a *block* — a decoded
/// blueprint whose 179 entities are placed together and whose `kw` is their
/// sum — has N things to exclude, and stating them as N positions has two
/// costs a rectangle does not: the list has to be carried inside every
/// [`Condition`](crate::action::Condition) built from it, cloned onto every
/// precondition; and it is a list of *what has been placed so far*, so it
/// changes as the block goes down, which is exactly the idempotence this type
/// exists to protect.
///
/// A block's ground is a rectangle known before the first entity is placed and
/// unchanged by placing them, so [`Ground`](Excluded::Ground) is O(1), stable
/// under a partial build, and states the honest predicate: *the network can
/// supply this draw beyond what is drawn by consumers that are not mine.*
///
/// It is sound only because the ground is the caller's own: `BuildBlock`
/// refuses with `PlannerError::BlockGroundOccupied` before emitting anything
/// if the footprint carries a foreign entity, so nothing inside the rectangle
/// is somebody else's draw. A caller whose region may contain consumers its
/// `kw` does not account for must not use this variant — it would over-state
/// headroom, which is the one direction this file's tables never err in.
#[derive(Clone, Copy, Debug, Default)]
pub enum Excluded<'a> {
    /// Charge every consumer on the network. What a question asked about
    /// ground nobody is about to build on wants.
    #[default]
    Nothing,
    /// One consumer, matched **by tile** through [`Pos`] — the machine being
    /// asked about. The tile match is how
    /// [`PlanState::create_entity`](PlanState::create_entity) keys the
    /// overlay, and two consumers cannot stand on one tile anyway.
    Consumer(&'a Position),
    /// Every consumer standing inside a rectangle — the ground a block is
    /// about to occupy, whose entire draw the question already names.
    Ground(&'a Rect),
}

impl Excluded<'_> {
    /// Whether a consumer standing at `position` is one the question is about.
    ///
    /// The rectangle test is **inclusive** on all four edges, unlike
    /// [`Rect::contains`], which is strict — and that pairs with the ground
    /// being drawn from footprints rather than from positions
    /// (`method::power::occupied_ground`). Either alone is enough and both are
    /// kept, because what they prevent is silent.
    ///
    /// **Measured, not reasoned**, by breaking each in turn against a 48-inserter
    /// block: a footprint-drawn ground survives a strict test (every position is
    /// half a collision box inside its own edge) and a positions-drawn ground
    /// survives an inclusive one (every outermost position is exactly on the
    /// edge). Break **both** and the whole perimeter is charged — 18 of 48
    /// inserters in that fixture, 234 kW of double count — which is a leak that
    /// grows with the block's edge and shows up as an inexplicable refusal.
    fn covers(&self, position: &Position) -> bool {
        match self {
            Excluded::Nothing => false,
            Excluded::Consumer(pos) => Pos::from(*pos) == Pos::from(position),
            Excluded::Ground(rect) => {
                position.x() >= rect.left_top.x()
                    && position.x() <= rect.right_bottom.x()
                    && position.y() >= rect.left_top.y()
                    && position.y() <= rect.right_bottom.y()
            }
        }
    }
}

/// One electric network as seen from a patch of ground: the poles near it, the
/// wire components they form, and which of those components reach the ground.
///
/// Built once by [`PlanState::electric_network`] and asked twice — by
/// [`PlanState::electric_supply_kw`] and [`PlanState::electric_demand_kw`] —
/// so that the two agree, by construction, about what "the same network"
/// means. Two independent walks would be two notions of it, and a demand
/// subtracted from a supply computed over a different network is worse than no
/// demand at all.
struct ElectricNetwork {
    /// Every entity [`PlanState::electric_entities`] reached by following the
    /// wire out from the ground asked about, in tile order.
    nearby: Vec<FactorioEntity>,
    /// Each pole's supply box, in that same order.
    poles: Vec<Rect>,
    /// Union-find over `poles`, by index, already path-compressed as it was
    /// built.
    parent: Vec<usize>,
    /// The component roots whose supply areas meet the ground asked about.
    supplying: BTreeSet<usize>,
}

impl ElectricNetwork {
    /// Read-only find. No compression, because `carries` takes `&self` so both
    /// callers can iterate `nearby` while asking — and a pole row is a handful
    /// of entries that the construction pass has already flattened.
    fn root(&self, mut i: usize) -> usize {
        while self.parent[i] != i {
            i = self.parent[i];
        }
        i
    }

    /// Is `footprint` covered by a pole of a component that reaches the
    /// ground this network was built for?
    ///
    /// The predicate both a generator and a consumer are tested with, which is
    /// what makes a kilowatt of supply and a kilowatt of demand commensurable.
    fn carries(&self, footprint: &Rect) -> bool {
        self.poles
            .iter()
            .enumerate()
            .filter(|(_, supply)| boxes_overlap(supply, footprint))
            .any(|(index, _)| self.supplying.contains(&self.root(index)))
    }
}

/// The pessimistic duty-cycle draw of a basic electric inserter, in kW.
///
/// **Not a prototype field, and the only entry in [`consumer_kw`] that is not
/// one.** Vanilla gives an `inserter` a 0.4 kW idle `drain` plus 5 kJ per
/// movement and 5 kJ per rotation (`base/prototypes/entity/entities.lua`);
/// what a network budget needs is what a *busy* one costs, and that is a duty
/// cycle rather than a field. 13 kW is the figure
/// `docs/superpowers/specs/2026-09-03-starter-factory-design.md` §4.2 chose,
/// with its reason stated there: budgeting the 0.4 kW drain for an inserter
/// that never stops swinging is the same error as counting coverage as
/// capacity, one table down.
///
/// The faster inserters below scale it by their own per-swing energy against
/// this one's 5 kJ, so there is one duty cycle in this file and not four.
const INSERTER_DUTY_KW: f64 = 13.0;

/// What a consumer draws from an electric network, in kW, **from its own
/// prototype**.
///
/// The other half of [`generation_kw`], and what
/// [`PlanState::electric_demand_kw`] sums.
/// `FactorioEntityPrototype::electric_energy_usage` is the runtime's
/// `energy_usage` **attribute** (not a method, unlike
/// `get_supply_area_distance` and `get_max_energy_production` — the two shapes
/// were checked against this install's `runtime-api.json` rather than
/// recalled), converted from joules per tick by
/// `FactorioEntityPrototype::energy_usage_kw`.
///
/// # Two gates, and neither is optional
///
/// **The mod sends this field only for a prototype with an electric energy
/// source.** A `stone-furnace`'s `energy_usage` is 90 kW *of coal*; charged
/// against an electric budget it is a number in the wrong units that every
/// test would agree with. That is why the fallback below leaves burner
/// machines out rather than zeroing them, and the gate lives upstream where
/// the energy source is visible.
///
/// **An inserter is electric and still has no figure**, so the four inserter
/// rows below are reached through the ordinary absent-field fallback rather
/// than by an exception. Measured on all 1,028 prototypes of a live 2.1.17
/// game: 28 carry `energy_usage` and **not one reports 0**. An `inserter`
/// passes the electric gate and the attribute is simply absent on it — its
/// cost is `energy_per_movement` and `energy_per_rotation`, per swing. What a
/// budget needs is what a *busy* one costs, which is a duty cycle rather than
/// a prototype field. The `> 0` filter below is kept as a guard against a
/// modded prototype that says zero, and it has never fired in vanilla.
/// Sending the per-swing energies and deriving the four inserter rows from
/// them is the follow-up that would delete the last non-prototype number here.
///
/// # It was checked before it was replaced, and it had not drifted
///
/// All 11 checkable rows of [`vanilla_consumer_kw`] matched the live game
/// exactly on 2026-09-07 — unlike [`vanilla_pole_wire_reach`], the neighbour
/// table that had silently drifted 30 against 32. So the maintenance half of this change
/// found no bug, and the milestone arithmetic that rests on `electric-furnace`
/// = 180 kW stands as written.
///
/// # The correctness half is what the table could not have: the names it never
/// knew
///
/// This table's unknown name errs **towards permitting** — a consumer it does
/// not carry draws nothing, so an unmodelled machine on the network is
/// headroom that is not there — and its own doc below has always said so. The
/// live game names **17 electric consumers it never carried**, including
/// several this project can already place: `small-lamp` at 5 kW (the
/// `FurnaceLine` fixture stands three of them and was budgeting zero for all
/// three), `recycler` at 180, `roboport` at 50, `pump` at 29, and the three
/// combinators at 1 each. The prototype path charges every one of them without
/// anybody having to have thought of it, which is the whole argument for
/// deriving rather than tabulating.
fn consumer_kw(prototypes: &DashMap<String, FactorioEntityPrototype>, name: &str) -> Option<f64> {
    let from_prototype = prototypes
        .get(name)
        .and_then(|prototype| prototype.energy_usage_kw())
        .filter(|kw| *kw > 0.);
    from_prototype.or_else(|| vanilla_consumer_kw(name))
}

/// What [`consumer_kw`] answers for a **vanilla consumer on a world whose
/// sender predates `electric_energy_usage`**, and for the inserters, whose
/// draw is not a prototype field at all.
///
/// Every figure except [`INSERTER_DUTY_KW`] is the prototype's own
/// `energy_usage`, read off
/// `base/prototypes/entity/entities.lua` and
/// `base/prototypes/entity/mining-drill.lua` in this repo's `workspace/data`
/// (base 2.1.17). It kept its numbers for the same reason
/// [`vanilla_pole_supply_half_extent`] keeps its: **every archived world here
/// predates the field**, `workspace/scripts/map.json` — the seed-31337 t=0 dump
/// the offline baselines are all measured on — included. Deleting the pole
/// fallback with nothing else changed made all three offline goals refuse to
/// expand at all, blaming the water; this one would blind the demand ledger
/// the same way, and every world recorded from here on overrides it before it
/// is ever consulted.
///
/// [`delivery_offset`] is the last supply-side table still waiting for a field
/// of its own (`vector_to_place_result`); [`pole_wire_reach`] stopped waiting
/// on 2026-09-07, when `maximum_wire_distance` began crossing the bridge.
///
/// # The direction an unknown name errs in, which is not the usual one
///
/// Every other table in this file answers `None` for a name it does not know
/// and thereby **under**-credits: an unknown pole covers nothing, an unknown
/// generator makes nothing, an unknown machine delivers into nothing, and each
/// of those refuses a plan rather than promising one. This table is the
/// opposite: a consumer it does not name draws **nothing**, so an unmodelled
/// machine on the network is headroom that is not there. That is the unsafe
/// direction and it is stated rather than hidden — it is why the table names
/// every electric consumer this planner can place, and why the residual is
/// listed in `docs/superpowers/notes/2026-09-03-red-science-automated.md`
/// rather than treated as closed.
///
/// **Burner machines are deliberately absent**, and they are absent rather
/// than zero-valued for a reason: a stone furnace draws 90 kW *of coal*, not
/// of electricity, and an entry for it here would be a number in the wrong
/// units that every test would agree with. So are `offshore-pump` (its
/// `energy_source` is `type = "void"`, whatever its 60 kW `energy_usage`
/// says), `boiler` and `steam-engine`. The prototype path above is absent for
/// exactly the same set, because the mod gates on an *electric* energy source
/// — so the two halves agree by construction rather than by care.
fn vanilla_consumer_kw(name: &str) -> Option<f64> {
    match name {
        "assembling-machine-1" => Some(75.0),
        "assembling-machine-2" => Some(150.0),
        "assembling-machine-3" => Some(375.0),
        "electric-mining-drill" => Some(90.0),
        "pumpjack" => Some(90.0),
        "lab" => Some(60.0),
        "electric-furnace" => Some(180.0),
        "chemical-plant" => Some(210.0),
        "oil-refinery" => Some(420.0),
        "radar" => Some(300.0),
        "beacon" => Some(480.0),
        // 5 kJ a movement and 5 kJ a rotation: the duty cycle itself.
        "inserter" | "long-handed-inserter" => Some(INSERTER_DUTY_KW),
        // 7 kJ each, so 7/5 of the same cycle.
        "fast-inserter" => Some(INSERTER_DUTY_KW * 7. / 5.),
        // 20 kJ each, so four times it.
        "bulk-inserter" => Some(INSERTER_DUTY_KW * 20. / 5.),
        _ => None,
    }
}

/// Is `name` a consumer that draws its [`consumer_kw`] only once a recipe is
/// set on it?
///
/// The crafting machines of the table above: each has an `energy_usage` it
/// draws while crafting and a drain of a few kW otherwise, and with no recipe
/// it never crafts. A lab, a drill, an inserter and a radar have no recipe to
/// set and draw whenever they have work. By name rather than by
/// `entity_type`, because entities this crate's own methods create in the
/// overlay carry the prototype's type and entities a test creates by hand
/// often carry none, and the ledger has to charge both the same.
fn takes_a_recipe(name: &str) -> bool {
    matches!(
        name,
        "assembling-machine-1"
            | "assembling-machine-2"
            | "assembling-machine-3"
            | "chemical-plant"
            | "oil-refinery"
    )
}

/// Where a machine puts what it produces, as a north-frame offset from its own
/// position.
///
/// Vanilla's `vector_to_place_result`, read off
/// `base/prototypes/entity/mining-drill.lua` in this repo's `workspace/data`
/// (`{-0.35, -1.3}` for the burner drill, `{0, -1.85}` for the electric one)
/// and written down here for the same reason as
/// [`vanilla_pole_supply_half_extent`], [`generation_kw`] and `power.rs`'s
/// fluid-connection tables: **`FactorioEntityPrototype` carries no such field
/// and the mod does not send one.** Sending `vector_to_place_result` is what
/// deletes the one that is left; `supply_area_distance`, `energy_usage` and
/// `maximum_wire_distance` have all landed, so [`pole_supply_half_extent`],
/// [`consumer_kw`] and [`pole_wire_reach`] now derive.
///
/// A machine this table does not name delivers into nothing at all, which
/// refuses rather than over-credits — the same direction
/// [`vanilla_pole_supply_half_extent`] and `collides_with_water` choose for an
/// unknown name.
///
/// **The inserters are the pair with [`pickup_offset`], and neither is useful
/// without the other.** An inserter's `direction` names the side it *picks up*
/// from, so its drop is the *opposite* tile — the single most expensive thing
/// to get backwards in this whole design, because a backwards inserter places
/// 100 %, passes every geometry check, and moves nothing. The two offsets are
/// therefore written as one rule turned by one direction rather than as four
/// hand-written cases, and they are checked against two measurements CLAUDE.md
/// made **in a running game** rather than against each other:
/// `direction = 12` ("west") moves items west to east, and a row fed from a
/// belt to its north uses `direction = 0` at both ends. Both are asserted in
/// this file's `inserter_geometry_tests`.
///
/// `None` for a half-diagonal direction, because [`Position::turn`] names no
/// rotation for one and no machine stands on one.
fn delivery_offset(name: &str, direction: Direction) -> Option<Position> {
    let north = match name {
        "burner-mining-drill" => (-0.35, -1.3),
        "electric-mining-drill" => (0., -1.85),
        // The far side from the pickup: `+y` is south, and north-facing means
        // "picks up from the north".
        name => (0., inserter_reach(name)?),
    };
    Position::new(north.0, north.1).turn(direction)
}

/// How far an inserter reaches, in tiles, on **each** side of itself.
///
/// A vanilla inserter swings between the tile in front of it and the tile
/// behind it; a long-handed one skips a tile on both sides. There is no
/// prototype field for it that reaches this crate — `FactorioEntityPrototype`
/// carries none of `pickup_position`, `insert_position` or
/// `energy_per_movement` — so this is the fifth hand-written table in this
/// file and it goes in the same follow-up.
///
/// `None` for anything that is not an inserter, which is what keeps the pickup
/// half of [`PlanState::delivers_into`] an *inserter's* claim: a stone furnace
/// does not reach out and take from the chest beside it.
fn inserter_reach(name: &str) -> Option<f64> {
    match name {
        "inserter"
        | "burner-inserter"
        | "fast-inserter"
        | "bulk-inserter"
        | "filter-inserter"
        | "stack-inserter"
        | "stack-filter-inserter" => Some(1.),
        "long-handed-inserter" => Some(2.),
        _ => None,
    }
}

/// Where an inserter standing at its own origin picks **up** from, as a
/// north-frame offset turned by `direction`.
///
/// **The direction points here.** `direction = 0` picks up one tile *north*
/// and drops one tile south; `direction = 12` (west) picks up one tile west
/// and drops one east, which is CLAUDE.md's "direction 12 is what moves items
/// west to east". Both of that file's measured cases fall out of these two
/// lines and [`delivery_offset`]'s inserter arm, which is why they are written
/// as one rule and not as a case table.
///
/// `None` for anything [`inserter_reach`] does not name.
fn pickup_offset(name: &str, direction: Direction) -> Option<Position> {
    Position::new(0., -inserter_reach(name)?).turn(direction)
}

/// Above this, a reported `resource_reach_distance` is not a character's.
///
/// A *player* with no character reports `f64::MAX` here — the game's way of
/// saying "unbounded" — and feeding that into a separation would make every
/// tile on the map crowd every other one and refuse every mining plan. The
/// mod's `mine_step_aside_waypoint` guards the same value with the same
/// bound, for the same reason.
const MAX_PLAUSIBLE_RESOURCE_REACH: f64 = 1000.;

/// Half the `character` collision box on each axis, as `base` reports it.
///
/// Falls back to [`VANILLA_CHARACTER_COLLISION_HALF_SIDE`] on both axes when
/// the world carries no `character` prototype — see that constant for why a
/// world can lack one. Read rather than hardcoded because it is prototype data
/// a mod can change, exactly like `character_mining_speed`'s.
fn character_half_box(base: &FactorioSurface) -> (f64, f64) {
    base.entity_prototypes
        .get("character")
        .map(|p| {
            let b = &p.collision_box;
            (b.width() / 2., b.height() / 2.)
        })
        .unwrap_or((
            VANILLA_CHARACTER_COLLISION_HALF_SIDE,
            VANILLA_CHARACTER_COLLISION_HALF_SIDE,
        ))
}

/// The furthest a character's centre can be from a resource tile's centre
/// while still standing on that tile.
///
/// Two axis-aligned boxes overlap only while the gap on *both* axes is under
/// the sum of their half-sides, so the extreme separation at which they still
/// touch is corner to corner — that hypotenuse. It is the supremum of
/// [`PlanState::character_stands_on_tile`], and a unit test says so.
fn tile_occupancy_radius(base: &FactorioSurface) -> f64 {
    let (half_x, half_y) = character_half_box(base);
    (TILE_HALF_SIDE + half_x).hypot(TILE_HALF_SIDE + half_y)
}

/// Do two collision boxes share ground? Touching along an edge does not count.
fn boxes_overlap(a: &Rect, b: &Rect) -> bool {
    a.left_top.x() < b.right_bottom.x() - TOUCH_SLACK
        && b.left_top.x() < a.right_bottom.x() - TOUCH_SLACK
        && a.left_top.y() < b.right_bottom.y() - TOUCH_SLACK
        && b.left_top.y() < a.right_bottom.y() - TOUCH_SLACK
}

/// The one-tile box covering `tile`. Tile `(x, y)` spans `[x, x+1)`.
fn tile_area(tile: &Pos) -> Rect {
    Rect::new(
        &Position::new(tile.0 as f64, tile.1 as f64),
        &Position::new(tile.0 as f64 + 1., tile.1 as f64 + 1.),
    )
}

/// Every tile `area` reaches into, in a fixed order.
///
/// The slack keeps a box that stops exactly on a tile boundary from claiming
/// the tile beyond it, which is the same edge case `boxes_overlap` handles.
fn tiles_under(area: &Rect) -> Vec<Pos> {
    let x0 = (area.left_top.x() + TOUCH_SLACK).floor() as i32;
    let x1 = (area.right_bottom.x() - TOUCH_SLACK).floor() as i32;
    let y0 = (area.left_top.y() + TOUCH_SLACK).floor() as i32;
    let y1 = (area.right_bottom.y() - TOUCH_SLACK).floor() as i32;
    let mut out = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            out.push(Pos(x, y));
        }
    }
    out
}

/// How much ore a tile is assumed to hold when **nobody has said**.
///
/// A fallback, no longer the answer. `EntityGraph::resource_amount` reports
/// what the game said is left in a tile -- the mod has always sent
/// `entity.amount` for a resource, and `FactorioEntity::amount` has always
/// carried it -- so this stands in only where that is `None`: a hand-built
/// fixture, whose `FactorioEntity::new_resource` sets no amount, or a resource
/// that reached the graph from a blueprint rather than from the game.
///
/// It used to be the answer for every tile on every map, and
/// `workspace/runs/run-1788334911-41961` is what that cost. Rung 6 asked one
/// bot for 50 iron ore from a tile the model said held 500; the tile held 14,
/// the bot mined it dry, and the mod reported `the target iron-ore was gone
/// before mining finished -- something else mined it first` -- which was true
/// about the disappearance and wrong about the cause, because there was no
/// something else. Five of the run's six iron mines died that way, on six
/// *different* tiles, against a roster the scheduler had put a single bot in.
///
/// The number itself is not a measurement and never was: it is large enough
/// that a fixture's ore never runs out mid-test, which is all a fixture needs.
/// A real tile near a twenty-run-old spawn holds single digits.
pub const DEFAULT_RESOURCE_PER_TILE: u32 = 500;

/// A bot's simulated state during planning.
#[derive(Clone, Debug)]
pub struct BotState {
    pub position: Position,
    pub inventory: BTreeMap<ItemId, u32>,
    pub build_distance: f64,
    pub reach_distance: f64,
    pub resource_reach_distance: f64,
}

impl Default for BotState {
    fn default() -> Self {
        BotState {
            position: Position::new(0., 0.),
            inventory: BTreeMap::new(),
            build_distance: 10.0,
            reach_distance: 10.0,
            resource_reach_distance: 3.0,
        }
    }
}

/// One tile this plan has committed to a mining action, and *when* it is held.
///
/// # The time axis, and why it is a runner rather than a tick range
///
/// Two mining actions have to be kept apart in space only if they can happen
/// at the same **time** — the whole reason for
/// [`PlanState::mining_tile_separation`] is that a bot mining one tile stands
/// on the tiles around it, and standing there stops *somebody else* mining
/// them. A tile the first bot has finished with and walked away from is free.
///
/// A plan being expanded has no clock: who runs an action and when is
/// [`crate::schedule`]'s decision, taken after the whole network exists. But
/// the scheduler makes one guarantee that expansion can read off the network
/// it is building, and it is exactly the guarantee needed here:
///
/// * a bot runs **one action at a time** (`free_at` in `schedule`, which every
///   chosen action advances to its own end), and
/// * a chain with an **owner** is offered to that bot and to no other — the
///   owner tier in `schedule`'s `candidate_tiers` is a single-bot tier with no
///   fallback, so an owner is a hard constraint, not a preference — and a
///   chain *without* one is still bound to a single bot the moment its first
///   action is placed (`chain_binding`), so its own actions are serial too,
///   even though nothing says which bot they are serial on.
///
/// So two claims made inside chains owned by the *same* bot — or inside the
/// *same* unowned chain — are provably disjoint in time, whatever the schedule
/// turns out to be, and they need no separation from each other at all. That
/// is what `runner` records: not a tick, but **whose serial timeline the claim
/// sits on**. See [`ClaimRunner`] for the two ways that can be known.
///
/// `None` means the claim was made outside any chain at all, where the action
/// stays individually assignable and could land on any bot at any time. Such a
/// claim conflicts with everything, itself included. Conservative by
/// construction: an unknown runner is never treated as a match, not even
/// against another unknown one.
///
/// # What this relaxes, and what it does not
///
/// Whole-tile exclusivity ([`PlanState::is_resource_claimed`]) stays global as
/// a *fact*: the tile is claimed, and every other runner is refused it. What
/// changed on 2026-09-06 is who "every other" means for the runner that
/// already holds it. Exclusivity is not a simultaneity rule — it existed
/// because the planner could not know what a tile really holds (see
/// [`DEFAULT_RESOURCE_PER_TILE`]), so a second draw would spend an invented
/// number. Where the world *states* the tile's amount, that reason is absent,
/// and [`PlanState::claim_yields_to`] lets the claim's own runner draw again
/// against `resource_available`. A tile the world said nothing about keeps
/// the old rule verbatim.
#[derive(Clone, Debug)]
struct MiningClaim {
    /// The tile's centre, kept beside its flooring `Pos` key so a distance is
    /// never measured to a rounded-down position. See the
    /// [`claimed`](PlanState#structfield.claimed) field.
    centre: Position,
    /// The bot whose serial timeline this claim sits on, or `None` when the
    /// runner was not known where the claim was made.
    runner: Option<ClaimRunner>,
}

/// Whose serial timeline a mining claim sits on.
///
/// Two spellings of the same guarantee, because the scheduler makes it twice
/// over:
///
/// * [`ClaimRunner::Bot`] — the claim was made inside a chain with an
///   **owner**. `schedule` offers an owned chain to that bot and to no other
///   (the owner tier in `candidate_tiers` is a single-bot tier with no
///   fallback), so every chain that bot owns runs on it, and a bot runs one
///   action at a time.
/// * [`ClaimRunner::Chain`] — the claim was made inside a chain with no
///   owner, which `converges` opens because "who runs it stays the
///   scheduler's decision". The bot is unknown, but `chain_binding` still
///   welds the whole chain to *one* of them, so the actions inside it are
///   still serial with each other.
///
/// `Bot` is the wider statement and subsumes `Chain` for the chains it owns;
/// they are never mixed, because a chain either has an owner where it was
/// opened or does not. Two claims match only when they are the same variant
/// carrying the same id, so an unowned chain never matches a bot even if the
/// scheduler later happens to put them together — the conservative direction,
/// and the only one that is sound without the schedule in hand.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClaimRunner {
    /// Every chain this bot owns, all of it serial on that bot.
    Bot(BotId),
    /// One unowned chain, serial on whichever bot the scheduler binds it to.
    Chain(ChainId),
}

/// Do two claim runners provably name the same serial timeline?
///
/// `None` is *unknown*, never a wildcard and never a match — not even against
/// another `None`. Two claims neither of which knows its runner may well end
/// up on two different bots at the same moment, which is the case the
/// separation exists for. Written as one function because both the crowding
/// predicate and the tile walk in [`crate::method::util`] have to answer it
/// the same way or they stop agreeing about which tiles are free.
fn same_runner(a: Option<ClaimRunner>, b: Option<ClaimRunner>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x == y)
}

/// A container or machine the world has reported contents for, and which the
/// plan may therefore take those contents out of.
///
/// # What counts as a buffer, and where that is decided
///
/// **Not here.** This crate believes whatever
/// [`FactorioSurface::observed_inventories`] tells it, minus two checks it can
/// make locally (below). The decision about *which* containers the game is
/// ever asked about belongs to whoever issues the RCON query --
/// `crates/core`'s `Planner::refresh_buffers` -- because that is the code that
/// knows whether an entity is one the bots built or one a person put there.
/// Putting a whitelist in the planner as well would be two policies that can
/// disagree, and the planner's copy would be the one nobody updates.
///
/// The two checks that *are* local, because they are about consistency rather
/// than policy:
///
/// * **The entity is still there, under the same name.** A reading is keyed by
///   tile, and a tile can be cleared and rebuilt. `from_world` drops a reading
///   whose position no longer holds an entity of the name the reading was
///   taken from.
/// * **The entity has a withdrawable output slot** ([`withdraw_slot`]). A
///   reading for something with no such slot describes an inventory no
///   `ActionKind::Remove` this planner emits can address.
///
/// # Output only, never fuel
///
/// [`ObservedInventory`](factorio_bot_core::factorio::world::ObservedInventory)
/// carries both `output` and `fuel`, and only `output` reaches here. Coal in a
/// burning furnace is not a buffer, it is a machine's consumable: taking it
/// out stalls the furnace the plan may still be waiting on, and the amount
/// recoverable is a partially-burnt fuel slot rather than a count anybody
/// planned. The fuel reading is kept in `crates/core` because it is what the
/// game answered, and dropping data at the boundary is worse than carrying it.
#[derive(Clone, Debug, PartialEq)]
pub struct Buffer {
    /// The entity's name, as both the reading and the world agree it is.
    pub name: String,
    /// The entity's centre, unrounded -- what an `ActionKind::Remove` must
    /// name and what a distance is measured to. The `Pos` key floors.
    pub position: Position,
    /// Which of the entity's inventories the contents were read out of, and
    /// so which one a `Remove` has to address.
    pub slot: InventorySlot,
    /// What is in it, less whatever this plan has already taken.
    pub contents: BTreeMap<ItemId, u32>,
}

/// Which inventory of an entity of this type a plan may withdraw from, if any.
///
/// Keyed on the *type* rather than the name, because the type is what decides
/// which inventory `LuaEntity::get_output_inventory()` returned -- a chest's
/// whole inventory, a furnace's result slot, an assembler's output slot -- and
/// the reading in `ObservedInventory::output` came from exactly that call.
/// Keying on the name would need a table of every container in the game.
///
/// `None` for everything else, which is the honest answer rather than a
/// default: an entity type this does not name is one whose contents no
/// `ActionKind::Remove` the planner emits knows how to address.
fn withdraw_slot(entity_type: &str) -> Option<InventorySlot> {
    match entity_type {
        "furnace" => Some(InventorySlot::FurnaceResult),
        "container" | "logistic-container" => Some(InventorySlot::Chest),
        "assembling-machine" => Some(InventorySlot::AssemblerOutput),
        _ => None,
    }
}

/// A batch this plan has queued into a machine, and what a further batch would
/// have to wait for.
///
/// See [`machine_queue`](PlanState#structfield.machine_queue) for what an entry
/// promises and why one is sometimes withheld.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineQueue {
    /// What the queued batch smelts. A batch of a *different* item may not
    /// queue behind it.
    pub item: ItemId,
    /// The action that leaves the machine empty — the take of the newest batch
    /// queued into it. A later batch's inserts must be ordered after this.
    pub release: ActionId,
    /// The bot whose timeline `release` sits on: the taker of the newest
    /// batch, or `None` for a cell's furnace, whose take is timed from a
    /// drill rather than from anyone's hands.
    ///
    /// This is what lets a smelt tell *its own* queue from somebody else's.
    /// Queueing behind a batch the same bot takes costs that bot nothing it
    /// was not already paying — its actions are serial — while queueing
    /// behind another bot's puts the wait on that bot's timeline, which the
    /// expansion cannot see. See `method::have::adoptable_furnaces`.
    pub taker: Option<BotId>,
    /// Machine time this plan has already queued into it, across every batch.
    /// Not a start tick and not a finish tick: nothing in this crate knows when
    /// an action runs until `schedule` says so. It exists only to be compared
    /// with another machine's, so that a bank spreads across the least-loaded
    /// furnaces instead of piling onto the nearest one.
    pub queued: Ticks,
}

/// The eight compass directions, as unit vectors, with their names.
///
/// Written out rather than derived from `cos`/`sin`: the planner's determinism
/// rule is about floats, and a table of constants cannot differ between two
/// builds of the same source the way a libm call can. Named in Factorio's
/// frame, where `y` grows southward, so `(0, -1)` is north.
const COMPASS: [(&str, f64, f64); 8] = [
    ("east", 1., 0.),
    ("south-east", D, D),
    ("south", 0., 1.),
    ("south-west", -D, D),
    ("west", -1., 0.),
    ("north-west", -D, -D),
    ("north", 0., -1.),
    ("north-east", D, -D),
];
const D: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Where the tile tree runs out: seventeen probes of the ground around a
/// point, and which of them found nothing.
///
/// Built by [`PlanState::charting`]; read by `crate::score`, which reports
/// it beside a map's resource distances, and by [`ChartingSummary`], which
/// turns it into the sentence a `NotCharted` refusal carries. **A blind probe
/// says the chunk was never written out, not that nothing stands there** --
/// see [`PlanState::charting`] for what a probe is.
///
/// **This is the field that decides whether a map score means anything**,
/// and it exists because the failure it guards against is silent:
/// `EntityGraph` holds charted chunks, so a map whose iron is at 400 tiles and
/// a map whose iron has not been looked at produce the *same* report -- "no
/// iron-ore within the radius" -- and only this tells them apart. The same
/// silence is what `PlannerError::NotCharted` names for a plan.
///
/// # What t=0 actually charts
///
/// Nothing in `mods/BotBridge` calls `force.chart`. The mod replays
/// `surface.get_chunks()` once at `whoami("server")` (`initial_discovery`,
/// `control.lua:827`), one chunk per tick, and after that only reacts to
/// `on_chunk_generated`. So what a t=0 dump knows is **the chunks the save was
/// created with**, which for a plain `--create` is the generated spawn region:
/// measured off `workspace/server-log.txt`, 418 chunks in a 20x20 core block,
/// tiles spanning `[-320, 320)` on both axes -- 409,600 tiles.
///
/// `crate::score::DEFAULT_SEARCH_RADIUS` is 256, and every point inside a disc
/// of radius 256 has `|x| <= 256 < 320`, so **the whole default search disc
/// fits inside the region a t=0 dump has already charted**. That is what makes
/// scoring a fresh map worth doing at all, and it is a fact about one measured
/// save rather than a guarantee -- which is exactly why this is probed per
/// dump and reported, instead of being asserted in a comment. It is also why
/// a `NotCharted` refusal on a t=0 dump usually reports the disc as fully
/// covered and points *beyond* the radius: the resource is not inside the
/// generated spawn region, and the ground past it has never been generated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartingScore {
    /// Probe points that landed on ground the model has a tile for.
    pub covered: usize,
    /// Probe points tried.
    pub probes: usize,
    /// The probes that found nothing: the directions this score is blind in.
    /// Listed rather than counted because "blind to the north-east" and "blind
    /// everywhere past half the radius" are different findings. In probe
    /// order -- the origin, then the compass at half the radius, then at the
    /// full radius -- so the first entry is also the nearest.
    pub blind: Vec<Position>,
}

impl ChartingScore {
    /// Whether every probe found charted ground.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.blind.is_empty() && self.probes > 0
    }
}

/// The nearest probe that found no ground: the direction charting ends
/// soonest in, and how far out that is.
#[derive(Clone, Debug, PartialEq)]
pub struct Frontier {
    /// A compass name, or `here` when the origin itself is uncharted.
    pub direction: &'static str,
    pub position: Position,
    pub distance: f64,
}

/// What a plan can see of the map around a point, as a sentence.
///
/// This is the payload of [`PlannerError::NotCharted`]: which resources the
/// model holds at all, how much of the disc around `origin` is charted, and
/// where charted ground ends. It exists so that a refusal says "unexplored"
/// with a direction attached rather than "absent" with nothing -- piece 1 of
/// `docs/superpowers/specs/2026-09-04-exploration-design.md`.
///
/// `seen` is the resource census the fingerprint already computes: tiles per
/// resource name, charted anywhere in the model. Tiles rather than patches,
/// for the reason [`PlanState::resource_patches`] documents about the flood
/// fill.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartingSummary {
    pub origin: Position,
    pub radius: f64,
    pub score: ChartingScore,
    pub frontier: Option<Frontier>,
    pub seen: BTreeMap<String, usize>,
}

impl std::fmt::Display for ChartingSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.seen.is_empty() {
            write!(f, "the plan sees no resource tile at all")?;
        } else {
            let seen: Vec<String> = self
                .seen
                .iter()
                .map(|(name, tiles)| format!("{name} ({tiles} tiles)"))
                .collect();
            write!(f, "the plan sees {}", seen.join(", "))?;
        }
        write!(
            f,
            "; charted ground covers {} of {} probes within {:.0} tiles of {}",
            self.score.covered, self.score.probes, self.radius, self.origin
        )?;
        match &self.frontier {
            Some(Frontier {
                direction: "here", ..
            }) => write!(f, ", and the ground under the origin itself is uncharted"),
            Some(frontier) => write!(
                f,
                ", ending soonest {:.0} tiles {} at {}",
                frontier.distance, frontier.direction, frontier.position
            ),
            None => write!(f, ", so the uncharted ground is beyond that radius"),
        }
    }
}

/// What is standing on a piece of ground, when something is.
///
/// The naming half of [`PlanState::occupant_of`]. A refusal that says
/// "obstructed" tells whoever reads it nothing they can act on; one that
/// names the tile and what is on it distinguishes "move the block", "clear a
/// tree", "step off the footprint" and "the game already refused here"
/// without a run.
///
/// Ordered exactly as `occupant_of` checks: a footprint covered by two of
/// these reports the first, which is deliberate -- the alternative is
/// reporting all of them and making the message longer without making it
/// more useful.
///
/// **The order puts what is named ahead of what is anonymous**, and that is
/// the whole lesson of
/// `docs/superpowers/notes/2026-09-06-a-failed-placement-blames-a-tree.md`.
/// The blocking-box source is the only one that cannot say what it found, so
/// it is asked **last**: an entity has a name, a character has a player id,
/// and a refusal carries the game's own list of what stood in the box it
/// judged. Until 2026-09-06 the anonymous source was asked third of five, so
/// a tile that was *also* a refused footprint reported
/// "occupied by a tree, cliff, rock or unit" -- a confident wrong label, over
/// a source that knew the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Occupant {
    /// An entity, planned or already standing, by its prototype name.
    Entity(String),
    /// A `player_collidable` tile.
    Water,
    /// Something with a collision box that no source in this model names.
    ///
    /// `blocked_tree` keeps an `is_minable` flag and no name at all, so a
    /// name here would be invented rather than read. `minable` is that one
    /// bit and the only thing this variant honestly knows: `true` is a tree
    /// or a rock (`FactorioEntity::is_minable` is exactly "type is `tree` or
    /// `simple-entity`"), `false` is **anything else** with a box the entity
    /// tree does not hold -- a cliff, a unit, a corpse, an item on the ground.
    ///
    /// It used to be a bare `Terrain` whose message recited
    /// "a tree, cliff, rock or unit", four things it had not read, of which
    /// a reader takes the first. That message named a tree at a tile the game
    /// said held nothing but ore.
    Terrain { minable: bool },
    /// A character. `on_roster` is the difference between "somebody else is
    /// standing there" and "one of the bots you are planning for is standing
    /// on the ground you asked it to build on" -- the second is the case a
    /// researcher building a block near their own bots hits, and it is
    /// cleared by walking, not by moving the block.
    Character { player: PlayerId, on_roster: bool },
    /// A footprint the game itself already refused a build at, this run,
    /// **with the evidence the game attached to that refusal**.
    ///
    /// `entity` is what was being built; `blockers` is what the mod found
    /// standing in the box it had just had judged, and `tile` the ground
    /// under the refused centre (`PlacementRefusal::blockers` / `tile`,
    /// filled by `describe_footprint` in the mod and read back by
    /// `note_placement_refusal`). None of it is reconstructed from this
    /// side's model -- a refusal is the game disagreeing with the model, so
    /// the model's opinion of the site is the one thing that cannot explain
    /// it.
    ///
    /// An empty `blockers` with a `tile` means the game scanned the box and
    /// found no entity in it; empty *and* no tile is the signature of a
    /// refusal from a mod that appended nothing, i.e. "not asked". The
    /// `Display` says which, rather than reading either as "clear".
    Refused {
        entity: String,
        blockers: Vec<String>,
        tile: Option<String>,
    },
    /// The world has no prototype for the entity, so its footprint cannot be
    /// sized at all. Not "clear": the same case
    /// [`PlanState::is_area_free_facing`] answers `false` for.
    Unknown,
}

impl std::fmt::Display for Occupant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Occupant::Entity(name) => write!(f, "occupied by {name}"),
            Occupant::Water => write!(f, "water"),
            Occupant::Terrain { minable: true } => {
                write!(f, "occupied by a tree or rock")
            }
            Occupant::Terrain { minable: false } => write!(
                f,
                "occupied by something with a collision box that this model cannot name -- \
                 a cliff, a unit, or anything else the entity tree does not hold"
            ),
            Occupant::Character {
                player,
                on_roster: true,
            } => write!(
                f,
                "character {player}, one of this plan's own bots, is standing on it"
            ),
            Occupant::Character {
                player,
                on_roster: false,
            } => write!(f, "character {player} is standing on it"),
            Occupant::Refused {
                entity,
                blockers,
                tile,
            } => {
                write!(f, "a footprint the game already refused a {entity} at")?;
                match (blockers.is_empty(), tile.as_deref()) {
                    (false, Some(tile)) => {
                        write!(f, ", where it found {} on tile {tile}", blockers.join(", "))
                    }
                    (false, None) => write!(f, ", where it found {}", blockers.join(", ")),
                    (true, Some(tile)) => write!(
                        f,
                        ", where it found no entity at all on tile {tile}, so the ground itself \
                         is the answer"
                    ),
                    (true, None) => {
                        write!(f, ", and it named nothing it found there")
                    }
                }
            }
            Occupant::Unknown => write!(f, "an entity this world has no prototype for"),
        }
    }
}

/// One footprint the game refused a build at, kept with what the game said
/// about it.
///
/// The planner's own copy of `FactorioSurface::placement_refusals`, reduced
/// to the geometry it has to test against ([`Self::area`], the prototype's
/// box turned the way the build was aimed) plus the three things the refusal
/// itself carried and this crate used to discard: the entity that was
/// refused, and the mod's observation of what stood in the box.
///
/// Kept because a refusal that cannot say what it is about is unreadable
/// exactly when it matters. A `BuildBlock` replan that refuses one tile of a
/// partial build has to tell a reader whether to clear something, move the
/// block, or wait for a bot to walk off, and the game already answered that
/// question at the moment it said no.
#[derive(Debug, Clone, PartialEq)]
pub struct RefusedFootprint {
    /// The box the game actually tested: the prototype's collision box,
    /// turned the way the build was aimed, centred where it was aimed.
    pub area: Rect,
    /// The entity the game refused, by prototype name.
    pub entity: String,
    /// What the mod found standing in that box, deduplicated and sorted by
    /// name. Empty means it looked and found no entity.
    pub blockers: Vec<String>,
    /// The tile under the refused centre, when the reply named one. `None`
    /// together with an empty `blockers` is "the mod appended nothing", not
    /// "the ground is clear".
    pub tile: Option<String>,
}

/// The world at a point in a hypothetical plan.
///
/// `base` is shared and never mutated; every difference lives in the overlay
/// fields, so `fork` costs the overlay rather than a world copy.
#[derive(Clone)]
pub struct PlanState {
    base: Arc<FactorioSurface>,
    /// How freely a fragment may wait on a cell this plan already stood.
    ///
    /// Set by [`crate::plan_best`], which builds one plan under each policy
    /// and keeps the shorter schedule. It lives here rather than in
    /// `ExpansionCtx` because `Method::applicable` is handed a `PlanState`
    /// and nothing else, and `applicable` and `expand` must answer from the
    /// same policy or a method claims a goal it then refuses.
    drain_policy: DrainPolicy,
    /// Set the moment a decision taken under [`PlanState::drain_policy`]
    /// would have come out differently under another policy.
    ///
    /// Written by `Drain::new` -- the one place a policy is ever read -- and
    /// read by [`crate::plan_best`] after a *complete* expansion, where a
    /// `false` is a proof rather than a guess: expansion is a deterministic
    /// function of this state, so if every drain decision the whole
    /// expansion took was policy-free, the next policy's expansion is
    /// identical action for action and there is nothing to learn by running
    /// it. That is what buys back the second pass on the goals where the
    /// policies never disagree, which is most of them.
    ///
    /// Shared through `fork`/`clone` on purpose: an expansion forks its state
    /// constantly (`expand` alone forks twice, once to rehearse), and a probe
    /// that did not survive a fork would observe nothing. `Arc<AtomicBool>`
    /// rather than `Cell` only so `PlanState` stays `Send + Sync`; the
    /// expansion is single-threaded, so the ordering is immaterial and the
    /// observation is deterministic.
    ///
    /// It records *that* a policy mattered, never which plan is better --
    /// nothing here may decide a plan, or the arbiter would stop being the
    /// finished schedule.
    policy_probe: Arc<AtomicBool>,
    bots: BTreeMap<BotId, BotState>,
    /// Bots `from_world` was asked for that `base` has no player for.
    ///
    /// Every bot in here got a `BotState::default()` instead of the world's
    /// real inventory and reach distances — an empty inventory and guessed
    /// reach limits, not "this bot has none of that yet". Today that never
    /// happens on the production path: `initiate_missing_players_with_default_inventory`
    /// (`crates/core/src/plan/planner.rs`) seeds every player id the run's
    /// roster names before `from_world` ever sees it, and every planner
    /// fixture that constructs a `PlanState` directly relies on the very
    /// fallback this records (`fixture_world()` never has players, so every
    /// existing test is "unknown" by this definition).
    ///
    /// That is exactly why `from_world` cannot turn this into a hard error
    /// without also rewriting every one of those fixtures, several of which
    /// (`tests/scheduling.rs`, `tests/red_science.rs`) are pinned byte-for-byte.
    /// So the substitution stays, but stops being silent: a caller that names
    /// a `BotId` the world does not have — the per-bot goal API this was
    /// flagged against — can check `unknown_bots()` and refuse to build a plan
    /// against invented reach distances, which `crates/scripting_lua` (out of
    /// this crate's scope) is where that refusal belongs.
    unknown_bots: BTreeSet<BotId>,
    /// Entities added by the plan, keyed by tile.
    added: BTreeMap<Pos, FactorioEntity>,
    /// Positions whose base-world entity the plan has removed.
    removed: BTreeSet<Pos>,
    /// Ore taken from a tile by the plan, subtracted from the base amount.
    consumed: BTreeMap<Pos, u32>,
    /// Tiles this plan has already committed to a mining action.
    ///
    /// `consumed` says *how much* the plan has taken from a tile; this says
    /// *that the tile is spoken for*, and the two answer different questions.
    /// Counting alone was not enough: [`DEFAULT_RESOURCE_PER_TILE`] is 500, so
    /// four bots each asked to mine ten iron ore all found the same nearest
    /// tile still holding hundreds and all four were sent to it. That is the
    /// defect this field exists for — the run that reached milestone 4 died on
    /// `the target stone was gone before mining finished, something else mined
    /// it first`, which is two bots on one tile seen from the game's side.
    ///
    /// So a tile is committed **whole**, not by the amount taken from it. The
    /// planner cannot know what a tile really holds (see
    /// [`DEFAULT_RESOURCE_PER_TILE`]), so "500 covers both shares" is a
    /// modelling assumption, not a fact, and it is exactly the assumption that
    /// failed. Whole-tile commitment needs no such assumption: one tile, one
    /// mining action, and the finite-amount question stops mattering between
    /// actions because there is only ever one.
    ///
    /// It costs almost nothing in locality. Candidate tiles are ordered by
    /// distance, so the second claimant takes the *next* nearest tile — one
    /// tile further on, inside the same patch — rather than a different patch.
    ///
    /// **Whole, against everybody else; by the amount, against itself.** The
    /// defect above is four *different* bots on one tile, and that is what
    /// exclusivity answers, unchanged. What it also did, until 2026-09-06, was
    /// bar the claim's **own** runner — so a plan spent a tile per shortfall
    /// per bot, and a deep goal on a 940-tile field claimed 324 tiles that
    /// still held 134,734 ore between them and then refused a share of six.
    /// Where the world states a tile's amount there is no assumption left to
    /// protect and [`PlanState::claim_yields_to`] lets that one runner draw
    /// again; where it states nothing, the paragraph above stands word for
    /// word. See [`MiningClaim`] for why one runner's two draws cannot collide.
    ///
    /// **A claim covers the ground around the tile, not only the tile.** Whole-
    /// tile exclusivity stopped two bots being sent to one ore and did nothing
    /// about the second bot *standing* on the first one's ore: run
    /// `run-1788313837-06402` spread four bots across four adjacent tiles and
    /// lost six of thirteen mines to `could not start mining for 301 ticks:
    /// another character is standing on the iron-ore`. So a claim also
    /// excludes every tile within [`PlanState::mining_tile_separation`] of it
    /// ([`PlanState::is_resource_crowded`]).
    ///
    /// Read through [`PlanState::resource_unclaimed`], never by the physical
    /// queries: [`PlanState::resource_available`] and
    /// `Condition::ResourceAvailable` still report what the ground holds,
    /// because a claim is a fact about *this plan*, not about the world the
    /// executor will meet.
    /// The tile centre is kept as the value, not rebuilt from the key.
    /// `Pos` floors, so `From<&Pos> for Position` hands back `(-41, -49)` for
    /// a tile whose ore really sits at `(-40.5, -48.5)`. That is harmless
    /// while a claim is only ever tested for equality, and is off by up to a
    /// tile the moment a *distance* is measured to it — which
    /// [`PlanState::is_resource_crowded`] does.
    claimed: BTreeMap<Pos, MiningClaim>,
    /// Machines this plan has already committed a batch of work to.
    ///
    /// The entity-side twin of [`claimed`](PlanState#structfield.claimed), and
    /// it exists for the same reason. A furnace is a *serial* machine with one
    /// source slot and one result slot: two smelts that both load it are two
    /// batches queued behind each other, and each of them models its own wait
    /// as though it had the furnace to itself. For two different ores it is
    /// worse than a bad estimate — a furnace holding iron ore refuses copper —
    /// and the plan would still validate, still schedule, and fail in the game.
    ///
    /// So a furnace a smelt adopts, or places for itself, is committed **whole
    /// for the whole plan**, exactly as a mining claim commits a tile whole.
    ///
    /// **A commitment is no longer permanent**, and that is what
    /// [`machine_queue`](PlanState#structfield.machine_queue) adds. The
    /// membership here still says "not idle, do not adopt this as though it
    /// were free"; a later smelt that is willing to *queue* behind the batch
    /// already in it asks that map instead, and gets the action id it has to
    /// wait for. Everything that asks about ground
    /// ([`PlanState::machine_committed_near`]) reads this set and is unchanged
    /// by reuse: the tile stays taken however many batches run on it.
    ///
    /// Keyed by `Pos`, which floors — sound here because every entry is an
    /// entity position tested only for equality, never for distance.
    committed_machines: BTreeSet<Pos>,
    /// Machines a later batch in *this* plan may queue behind, and what it
    /// costs to do so.
    ///
    /// # The defect this closes
    ///
    /// [`committed_machines`](PlanState#structfield.committed_machines) is a
    /// commitment that was never released, so a plan needed **as many furnaces
    /// as it had `Smelt` goals**. Measured on `run-1788497495-79997`: 28
    /// independent hand-smelts for 276 iron ore and 46 copper, several
    /// smelting a single ore, each with a furnace of its own. Cross-plan reuse
    /// worked — a later plan adopts what the last one left standing — so each
    /// epoch added ~13 rather than starting over, and *within* a plan there was
    /// no reuse at all.
    ///
    /// That is a space cost as much as a stone cost. Red science alone placed
    /// 42 stone furnaces on seed `31337`, spread `x −18..33, y −49..−12`, on a
    /// map whose iron ore is 18.4 tiles from spawn — so they landed on and
    /// around the very patch the green-science cell then needed, and `Producing`
    /// refused with *no room for a iron-ore cell within 12 tiles of the patch*.
    ///
    /// # What an entry promises
    ///
    /// [`MachineQueue::release`] is an action that leaves the machine **empty**:
    /// its source slot smelted out and its result slot taken. A later smelt may
    /// therefore state an ordering edge from it to its own inserts and load the
    /// machine with anything. An entry is written **only** when that is
    /// provable at expansion time — see `method::have::smelt_steps`, which
    /// withholds one for a bank slot whose take is capped below what the slot
    /// produces, or whose ore inserts do not match the runs it was sized for.
    /// A machine with no entry is committed and never reused, which is exactly
    /// the old behaviour.
    ///
    /// [`MachineQueue::item`] is what the queued batch smelts, and reuse is
    /// restricted to a smelt of the same item. The drain argument above is an
    /// argument about a *model*: `bank_coal` is explicitly an approximation
    /// that "ignores partial burns", so a furnace can come up a plate short of
    /// what the plan believed. Same item, that is a shortfall the replan sees;
    /// a different item, it is an insert the game refuses outright. The narrow
    /// rule costs almost nothing in practice, because
    /// `method::have::adoptable_furnaces` scopes its search to the *ore patch*
    /// and two ores are two patches.
    ///
    /// [`MachineQueue::queued`] is the smelting time this plan has already put
    /// into the machine, and is what makes several reusable furnaces spread
    /// rather than pile onto the nearest one.
    ///
    /// Keyed by `Pos` for the same reason and with the same soundness argument
    /// as `committed_machines`.
    machine_queue: BTreeMap<Pos, MachineQueue>,
    /// Machine time this plan has queued into each machine, **across every
    /// batch and every smelt**, which is what [`MachineQueue::queued`] reports.
    ///
    /// Kept apart from `machine_queue` because the two have different
    /// lifetimes. A smelt that adopts a queued furnace *unqueues* it while it
    /// emits (its entry names the previous smelt's release, which a third
    /// smelt must not read) and queues it again behind its own take. The total
    /// used to live only on the entry, so that cycle reset it to the newest
    /// batch alone, and "least-loaded first" compared the last batch each
    /// furnace ran rather than the queue on it. On green that put twenty-six
    /// batches from all four bots onto one furnace while two others stood
    /// with one long batch each -- see `method::have::adoptable_furnaces`.
    machine_load: BTreeMap<Pos, Ticks>,
    /// Bot-ticks this expansion has already committed each bot to: the
    /// nominal durations of every action emitted under a chain the bot owns,
    /// summed. Written by the driver's `run_steps` through
    /// [`Self::note_planned_ticks`] -- the one place every action passes --
    /// and read by [`Self::planned_ticks`]. See that method for what it
    /// deliberately leaves out.
    planned_ticks: BTreeMap<BotId, Ticks>,
    /// Buffers this plan is *filling*, whose contents are therefore already
    /// spoken for.
    ///
    /// A stockpile deposits into a chest and then withdraws the whole lot in
    /// one `Remove` (see [`crate::method::have::Stockpile`]). Between those two
    /// emissions the chest holds items, and `Withdraw` -- which is registered
    /// ahead of everything and asks only whether *some* buffer holds the item
    /// -- would happily plan a second bot to take them straight back out. That
    /// is not a slow plan: the stockpile's own take then asks for more than the
    /// chest still holds and [`PlanState::take_from_buffer`] refuses the whole
    /// expansion. It was measured as
    /// `the buffer at [-57.5, 13.5] holds 3 copper-ore, and the plan wants 10`.
    ///
    /// So a committed buffer is hidden from [`PlanState::buffers_holding`],
    /// which is `Withdraw`'s only door, and stays visible to
    /// [`PlanState::buffered`], which is what `Condition::BufferHas` and the
    /// stockpile's own arithmetic read. Reserved rather than emptied, for the
    /// same reason every other ledger here reserves: the items really are in
    /// the chest, and only *other* goals need to be told they are taken.
    ///
    /// Keyed by `Pos`, which floors -- sound for the same reason
    /// `committed_machines` is: every entry is an entity position tested only
    /// for equality.
    stockpiled: BTreeSet<Pos>,
    /// Whose serial timeline a claim made **now** sits on, and whose timeline
    /// a crowding question is being asked *for*.
    ///
    /// Driver-set, saved and restored exactly where `ExpansionCtx::chain` is
    /// (see `method::mod::expand_goal` and the `Step::Owned` arm of
    /// `run_steps`), because it is the same fact: a chain with an owner runs
    /// wholly on that bot, and `None` outside such a chain.
    ///
    /// See [`MiningClaim::runner`] for what it buys and why it is sound.
    claim_runner: Option<ClaimRunner>,
    /// The one force this plan acts for, or `None` if `base` carries no force
    /// by that name.
    ///
    /// Chosen once, here, and read by everything that asks a question about
    /// technology — `is_researched` and `technology` both. That single
    /// selection site is the point. Before it there were two: `is_researched`
    /// answered "yes" if *any* force had the technology researched, while the
    /// cost of that same technology was read from whichever force sorted
    /// first. Each was individually defensible and together they disagreed
    /// about who we are planning for, so a world where `alpha` has not
    /// researched automation and `zeta` has made the planner silently emit
    /// nothing for `Researched("automation")` — refusing to research something
    /// the acting force does not have.
    ///
    /// The planner has no concept of acting for several forces at once, and
    /// inventing one to fix this would be speculative. So it acts for exactly
    /// one, and the type says so: there is nowhere else to make the choice and
    /// nothing else to disagree with.
    ///
    /// **Which force: [`BOT_FORCE`], by name.** It used to be the
    /// alphabetically first, on the stated premise that "every world this
    /// plans against has a single force, so the tie-break decides nothing in
    /// practice". That premise was false from the first research completion of
    /// every run. `writeout_forces` in `mods/BotBridge/control.lua` emits all
    /// of `game.forces`, so `FactorioSurface::forces` gains `enemy` and
    /// `neutral` the moment the mod re-sends them, and `min()` returns
    /// **`enemy`** — a force that never researches anything. In run 30
    /// (`workspace/runs/run-1788365280-15443/`) that happened at tick 26,449,
    /// which is milestones 6 and 7 in full: the `player` force had finished
    /// `automation-science-pack` by tick 53,485 and `enemy` had not, so all
    /// five milestone-7 plans re-derived its trigger and crafted a second lab
    /// for a technology the force already had.
    ///
    /// It was also *inconsistently* wrong, which is worse than uniformly
    /// wrong: the mod's `collect_recipes` hardcodes `game.forces["player"]`,
    /// so recipe gating read the right force while every technology question
    /// read `enemy`, and one `PlanState` gave two different answers about the
    /// same game. Naming the force is what makes the two agree.
    ///
    /// Reproducibility was the sort's whole justification and a name keeps it:
    /// a lookup by key does not depend on the `DashMap`'s hash seed either.
    force: Option<String>,
    /// Technologies the plan has completed.
    ///
    /// Keyed by bare technology name, which is unambiguous precisely because a
    /// `PlanState` acts for one force: the overlay cannot mean a different
    /// force's copy of the technology than `force` above names. If the planner
    /// ever learns to act for several, this has to become `(force, tech)` at
    /// the same time — the two are one decision, not two.
    researched: BTreeSet<String>,
    /// Holdings expansion has already promised to an action it is about to
    /// emit, keyed the way [`Holder`] keys a shortfall: per bot for
    /// `Holder::Bot`/`Holder::Share`, and once for the roster as a whole for
    /// `Holder::Anyone`.
    ///
    /// This is *not* inventory. Nothing here has been spent — the items are
    /// still in the bot's hands, and [`PlanState::inventory_count`] and
    /// [`PlanState::lose`] both still see them, which is what keeps the
    /// emitted plan's own arithmetic (and `Condition::HasItem`'s check of it)
    /// reading the real simulated inventory. A reservation only answers a
    /// different question: how much is left over for some *other* goal to
    /// count towards itself. See [`PlanState::available`].
    reserved: BTreeMap<BotId, BTreeMap<ItemId, u32>>,
    /// The `Holder::Anyone` half of `reserved`, which names no bot.
    reserved_by_anyone: BTreeMap<ItemId, u32>,
    /// Half the diagonal of the largest collision box among `base`'s known
    /// entity prototypes, or `0.` if it carries none.
    ///
    /// Computed once from `base` in [`PlanState::from_world`] rather than on
    /// every [`PlanState::is_area_clear`] call: `base` never changes after
    /// construction (see the struct doc), so the value cannot go stale, and
    /// this avoids rescanning the prototype table for every candidate
    /// placement a search considers. `entity_prototypes` is a `DashMap` with
    /// no defined iteration order, but a maximum over its values does not
    /// depend on that order, so this stays deterministic.
    max_prototype_half_diagonal: f64,
    /// How far apart two tiles must be before two *different* mining actions
    /// may claim them. See [`PlanState::mining_tile_separation`] for the
    /// derivation; computed once in [`PlanState::from_world`] because neither
    /// `base` nor the roster changes after construction.
    mining_tile_separation: f64,
    /// The collision box of every character standing on the surface, keyed by
    /// player id — roster bots included.
    ///
    /// A character is a physical obstacle: BotBridge builds with
    /// `build_check_type = manual` (`rcon_place_entity`,
    /// `mods/BotBridge/control.lua`), and that check collides with characters
    /// like any other entity. `EntityGraph` never sees one — `add` inserts a
    /// whitelist of *factory* entity types — so without this field the planner
    /// has no source for them at all and the ground under an idle bot reads as
    /// open.
    ///
    /// Run `run-1788319014-01846` is what that costs. Rung 4 left two bots
    /// parked where their last rung-2 mine had put them 14 000 ticks earlier;
    /// bot 2 sat at `(-18.2421875, 51.28125)`, the planner sited a stone
    /// furnace at `[-19, 51]` and then `[-18, 51]` five times over, and the
    /// game refused every one. The mod's own message says which character it
    /// was: it answers `§player_blocks_placement§` when the *acting* player is
    /// inside the footprint and the generic `can_place_entity said 'no'`
    /// otherwise, and the run got the generic one every time.
    ///
    /// # Why the roster is not excluded
    ///
    /// It was, for exactly one run. The first version of this field held only
    /// the characters the plan's roster did *not* name, on the reasoning that
    /// `base.players` is merely where a roster bot *started*, that the plan
    /// moves it, and that the acting bot is kept out of its own footprint by
    /// [`crate::action::Condition::AtPosition`]'s `min_radius` (see
    /// [`PlanState::placement_clearance`]) in any case.
    ///
    /// Run `run-1788322836-81715` falsified the middle step. Its roster was
    /// the whole connected game — `rcon.players()` returned `[2, 3, 4]` — so
    /// the filter emptied the field of every bot that mattered and left only
    /// the phantom at the origin. Rung 4 scheduled all 114 of its steps onto
    /// bot 2; bots 3 and 4 got none. Bot 3 was standing at
    /// `(-15.328125, -58.2890625)` where rung 2's copper had left it, the
    /// planner sited a stone furnace at `[-16, -58]` — whose box spans
    /// `[-16.8, -15.2] x [-58.8, -57.2]`, over bot 3 by a third of a tile —
    /// and the game refused it three times before the milestone gave up. A
    /// roster bot is not a bot the plan will move; it is a bot the plan *may*
    /// move, and which bots a plan actually moves is not known until
    /// `schedule` has assigned the work.
    ///
    /// Nor does the plan move one during expansion: `BotState::position` is
    /// seeded from `base.players` and never advances (`method::have`'s furnace
    /// siting says so in place, and is why it anchors on the ore instead of on
    /// the bot). So there is no second, fresher position to prefer — the
    /// observed one is the only position the planner has for any bot, roster
    /// or not, and it is a fact about the ground rather than a narrative.
    ///
    /// The acting bot is therefore blocked from the tile it currently stands
    /// on, which is over-conservative: `min_radius` will walk it clear before
    /// the placement runs. That costs one ring of [`crate::method::util::free_area_near`]'s
    /// outward search and picks a neighbouring tile. The other direction costs
    /// a milestone: a refused placement is replanned to the same site, refused
    /// again, and the run reports `stuck`. Exempting the actor is not even
    /// available as a middle course, because a site chosen while exempting the
    /// bot expansion had in hand is a site the *scheduler* may hand to a
    /// different bot, which reproduces the bug in a narrower form.
    ///
    /// Ordered by player id rather than collected straight off the `players`
    /// `DashMap`, whose iteration order moves with the hash seed.
    ///
    /// **Caveat, and it is not this field's to fix.**
    /// `Planner::initiate_missing_players_with_default_inventory`
    /// (`crates/core/src/plan/planner.rs`) invents a `FactorioPlayer` for
    /// every requested bot id the game has no player for, and a default one
    /// sits at `(0, 0)` with plausible-looking reach distances — nothing here
    /// can tell it from a real bot parked at the origin. Both runs above asked
    /// for four bots and got three clients, so player 1 was such a phantom,
    /// and it now shadows a ~0.4-tile box at the origin unconditionally rather
    /// than only when a roster filter happened to admit it. That is a small,
    /// bounded false refusal — `free_area_near` steps to the next ring — and
    /// it is the price of not silently believing a real parked bot is not
    /// there. The real fix belongs upstream, in not inventing the player, or
    /// in marking an invented one so consumers can tell.
    characters: BTreeMap<PlayerId, Rect>,
    /// Footprints the *game* has already refused a build at, this run.
    ///
    /// The sixth occupancy source, and the only one that is not a model of
    /// the ground: the five above are the planner's belief about what is
    /// there, while this is a verdict the game handed down about a specific
    /// box. It exists because the belief has been wrong four times in a row
    /// for four different reasons -- a forest, a non-roster character, a
    /// roster character, and a fourth still unidentified -- and because until
    /// now nothing carried the verdict from the run that earned it to the
    /// plan that came next. Run `run-1788323755-24892` re-chose one refused
    /// tile twice in a single milestone.
    ///
    /// # What is excluded, and why it is the box rather than the tile
    ///
    /// The whole footprint the game tested, not the tile at its centre. What
    /// the game said is "an entity of *this box* centred *here* cannot be
    /// built", so the sound inference is that something in that box blocks a
    /// build -- and nothing narrower is available, because the refusal names
    /// no cause and no coordinate. Excluding only the exact tile would let
    /// the next plan pick a neighbour whose box covers the same unknown
    /// blocker, and `free_area_near`'s rings would then walk into it one tile
    /// at a time, spending an iteration per step against a stall limit of
    /// three. Excluding the box steps clear in one move.
    ///
    /// It over-excludes when the blocker sits in a corner of the box. That is
    /// the same trade the `characters` field above documents, in the same
    /// direction: a few extra tiles of `free_area_near`'s 625-candidate
    /// window against a milestone.
    ///
    /// # It narrows the site, not the search
    ///
    /// Nothing here touches `FREE_TILE_SEARCH_RADIUS` or the ring order.
    /// A refusal makes specific candidates unavailable and lets the existing
    /// outward walk find the next one, so a refused site costs the *nearest*
    /// alternative and never a wider search than the one already run.
    ///
    /// # Believed for the whole run, unless the game's own evidence retires it
    ///
    /// Read from `FactorioSurface::placement_refusals` on every
    /// [`PlanState::from_world`]. That ledger is still never expired -- it is
    /// the record of what the game said -- but this field drops the entries
    /// whose recorded blockers are *only* transients (a character, a ghost):
    /// see `PlacementRefusal::names_only_transient_blockers`, which keeps
    /// everything else, an empty blocker list included, because an absent
    /// observation is unknown and not clear.
    ///
    /// Without that, a footprint refused because a bot happened to be standing
    /// on it was fenced off for the rest of the run, and a `BuildBlock` that
    /// lost one placement could never be finished by a replan -- the property
    /// `goal.built` exists for. Purity survives it: the rule is a pure
    /// function of the refusal's own recorded fields, this is still an input
    /// read once at construction like every other field, and two `PlanState`s
    /// built from the same world and roster still expand to the same plan.
    ///
    /// It runs identically offline and live, deliberately. A rule that
    /// re-asked the game when one was attached and used the record otherwise
    /// would make `factorio-bot plan --world <dump>` and a live replan
    /// disagree about the same ledger, and that difference would be
    /// discovered at 2am rather than read here.
    ///
    /// # It carries the game's own evidence, not just the box
    ///
    /// `PlacementRefusal` has always held what the mod found in the box it
    /// judged (`blockers`, `tile`) and the name of the entity that was
    /// refused; this field kept only the rectangle and threw the rest away,
    /// so the best a refusal could say was *"a footprint the game already
    /// refused a build at"*. That is a true sentence about the wrong half of
    /// what is known. See [`RefusedFootprint`].
    ///
    /// Sorted by geometry rather than kept in arrival order, so the field
    /// does not depend on the sequence the game happened to refuse things in.
    refused: Vec<RefusedFootprint>,
    /// What the world last saw in each container and machine, **less whatever
    /// this plan has already taken out of it**, keyed by tile.
    ///
    /// The other half of a handover. `smelt_steps` has one bot load a furnace
    /// and another unload it; if the second bot never arrives and the plan is
    /// remade, the plates are sitting in that furnace and the ore they came
    /// from is gone from the ground. Without this field the replan sees an
    /// empty-handed roster and asks the world to mine ore that no longer
    /// exists. See [`crate::method::have::Withdraw`], which is the method that
    /// spends it.
    ///
    /// # It is an overlay, and the subtraction is what makes it one
    ///
    /// Read once in [`PlanState::from_world`], exactly like `refused` and
    /// every other field, so purity holds: two `PlanState`s built from the
    /// same world expand identically. [`PlanState::take_from_buffer`] then
    /// decrements it as each `Remove` is emitted, so a second goal in the same
    /// plan cannot count the same plates towards itself -- the shared-
    /// intermediate defect `reserved` exists for, in a different ledger.
    ///
    /// # Ordered, because the order is load-bearing
    ///
    /// A `BTreeMap<Pos, _>`, filled from
    /// [`FactorioSurface::observed_inventories`], which sorts before it hands
    /// anything over. A buffer overlay iterated in hash order would move
    /// emission order, which fixes `ActionId` allocation, which fixes
    /// `schedule`'s `(end, ActionId, BotId)` tie-break -- a correctness bug,
    /// not a style one.
    ///
    /// # Empty in every fixture, and that is the inertness proof
    ///
    /// Nothing writes to `FactorioSurface::inventories` unless a caller pulls
    /// contents over RCON, so every existing test world has none of these and
    /// `Withdraw` claims nothing. That is why registering a new method ahead
    /// of `Smelt` and `Mine` moved no makespan.
    buffers: BTreeMap<Pos, Buffer>,
    /// What each burner machine was last seen holding in its fuel slot, by
    /// tile -- the `fuel` half of the same readings `buffers` is the `output`
    /// half of.
    ///
    /// # A credit, not a buffer
    ///
    /// Nothing withdraws fuel: coal in a running machine is that machine's
    /// consumable, and taking it out stalls whatever the plan may be waiting
    /// on (see [`Buffer`]). What a reading *is* good for is not paying twice.
    /// A cell standing from an earlier plan with coal still in its drill and
    /// its furnace is topped up by [`crate::method::produce::PlaceDrill`] for
    /// what it is asked for, less this, so the top-up is what the job needs
    /// and not what the job needs plus what is already burning. Read through
    /// [`PlanState::fuelled`]; never decremented, because nothing in a plan
    /// spends it -- the game does.
    ///
    /// # Read once, like every other reading
    ///
    /// Seeded in [`PlanState::from_world`] from
    /// [`FactorioSurface::observed_inventories`], under the same guard as
    /// `buffers`: the entity the reading names must still be the entity
    /// standing on that tile. Empty in every fixture and in every offline
    /// dump, since `world.dump` never refreshes inventories -- so offline a
    /// standing cell is topped up in full, which is the safe direction.
    fuel: BTreeMap<Pos, BTreeMap<ItemId, u32>>,
    /// What each machine was last seen holding in its **input** slot, by tile
    /// — a furnace's ore, an assembler's ingredients — and `None` for a tile
    /// whose entity has no input slot at all.
    ///
    /// # Not a buffer, and deliberately not counted as material
    ///
    /// The third of the three readings one `inventory_contents_at` reply
    /// carries, beside `buffers` (`output`) and `fuel`. Like fuel and unlike
    /// output, **nothing withdraws from here**: [`withdraw_slot`] maps a
    /// furnace to its *result* slot, which is the only inventory an
    /// `ActionKind::Remove` this planner emits can address, so ore in an input
    /// slot never becomes material a plan may spend. Adding it to `buffers`
    /// would be a double-spend of exactly the shape this crate has already
    /// paid for once.
    ///
    /// # What it is for: a machine that is BUSY looks idle without it
    ///
    /// A `stone-furnace` mid-smelt — ore in, nothing out yet — is
    /// indistinguishable from a furnace no ore ever reached, once the input
    /// slot is dropped. `crate::method::have`'s `adoptable_furnaces` asks
    /// [`PlanState::holds_buffer`] whether a standing furnace is free and
    /// therefore reads the second answer for the first. **This field is the
    /// evidence that separates them; changing what `adoptable_furnaces` does
    /// with it is a planner policy decision and is deliberately not taken
    /// here.** See [`PlanState::holds_input`].
    ///
    /// # `None` is not an empty map
    ///
    /// A `wooden-chest` is queried by `refresh_buffers` alongside furnaces and
    /// has no input inventory whatsoever; the value is `None` for it and
    /// `Some(empty)` for a furnace standing empty. Both answer "no ore here",
    /// and they answer differently about the machine.
    ///
    /// # Read once, like every other reading
    ///
    /// Seeded in [`PlanState::from_world`] under the same guard as `buffers`
    /// and `fuel`: the entity the reading names must still be the entity
    /// standing on that tile. Never decremented — nothing in a plan spends it,
    /// the machine does. Empty in every fixture and in every offline dump,
    /// since `world.dump` never refreshes inventories.
    input: BTreeMap<Pos, Option<BTreeMap<ItemId, u32>>>,
    /// Walks the game's pathfinder searched for and did not find, this run.
    ///
    /// The `refused` field's twin for *getting somewhere* rather than for
    /// *building somewhere*, and read the same way: once, in
    /// [`PlanState::from_world`], off a ledger `crates/core` keeps for the
    /// life of the world. It answers exactly one question, through
    /// [`PlanState::is_walk_refused`] — "has this bot already been told there
    /// is no route from where it stands to there".
    ///
    /// # Why it is not folded into the occupancy sources
    ///
    /// `refused` becomes a `Rect` and joins the five things that make ground
    /// unbuildable, so every site search steps around it without knowing it
    /// exists. This cannot work that way. A refused walk excludes no ground:
    /// the destination is fine, the route is what is missing, and the fact is
    /// about a *pair* of points, one of which is a bot. So it stays a list of
    /// facts and is consulted where both points are in hand, which is
    /// [`crate::schedule::schedule`]'s candidate loop — the one place that
    /// knows which bot is being sent where.
    ///
    /// # Determinism
    ///
    /// Sorted by `(player, from, to)` with `total_cmp`, exactly as `refused`
    /// is sorted by geometry and for the same reason: the field must not
    /// depend on the order the game happened to refuse things in. It is read
    /// once at construction like every other field, so two `PlanState`s built
    /// from the same world still plan identically.
    ///
    /// # Empty in every fixture
    ///
    /// Nothing writes to `FactorioSurface::walk_refusals` unless a real walk was
    /// refused by a real game, so every existing test world has none of these
    /// and scheduling is bit-for-bit what it was.
    refused_walks: Vec<WalkRefusal>,
    /// Bots that cannot reach open ground from where they are standing, with
    /// the size of the pocket each is sealed into, in square tiles of
    /// configuration space.
    ///
    /// # Two independent witnesses, and why both are required
    ///
    /// A row is here only when *both* of these hold at the moment the plan is
    /// built:
    ///
    /// * `FactorioSurface::enclosures` — written by
    ///   `crates/executor::walk_memory::note_enclosure` — carries an
    ///   observation for this player within
    ///   [`WalkRefusal::SAME_PLACE_TOLERANCE`] of where the bot is *now*. That
    ///   ledger is only ever written when the game's own pathfinder has
    ///   already refused a route from that spot, so it is the game's evidence,
    ///   not ours.
    /// * [`PlanState::escape_from`] run against *this* state agrees, right
    ///   now. The ledger is append-only and never drained, so on its own it
    ///   would keep a bot excluded long after somebody mined the tree that
    ///   trapped it. Re-asking is what un-excludes a bot without requiring it
    ///   to move first — which it cannot do, that being the condition.
    ///
    /// [`crate::enclosure::Escape::Unknown`] is never folded into this: an
    /// unmodelled window is not proof a bot is stuck, and the whole of the
    /// rest of this type treats "I could not tell" as a reason to do the
    /// ordinary thing.
    ///
    /// # What reads it
    ///
    /// Two callers, and both are asking the *same* question — "may this bot be
    /// given work that no other bot could ever take over?" — at the two places
    /// such work is created:
    ///
    /// * [`crate::method::have::even_shares`], for a gathering share; and
    /// * [`crate::method::pick_chain_actor`], for the `chain_actor` a caller
    ///   hands [`crate::method::expand`], which is what a goal naming no holder
    ///   is both sized against and (through the `Holder::Share(chain_actor)`
    ///   bills the methods state) welded to.
    ///
    /// Nothing else, and in particular nothing at schedule time. A walled-in
    /// bot is still a full member of the roster everywhere else — `schedule`
    /// may still pick it, `Holder::Bot` may still name it, and every free
    /// action still ranks it. What it must not get is a *share*: a share opens
    /// a chain that names it as owner (`crates/planner/src/method/mod.rs`), and
    /// `schedule` treats an owner as a hard constraint with no fallback tier,
    /// so a share handed to a walled-in bot is work no other bot can ever pick
    /// up. That is what `run-1788449752-46541` measured — bots 2 and 3 frozen
    /// at one spot for 75 000 ticks, still being sized six iron ore each on
    /// every replan. The chain actor is the same trap by the other road, and it
    /// was left open by that fix on purpose: the top-level chain is not a
    /// share, so the share filter never sees it.
    ///
    /// # Determinism
    ///
    /// A `BTreeMap`, keyed by `BotId`, built from a ledger read once at
    /// construction — the same discipline `refused_walks` keeps, and for the
    /// same reason. Empty in every fixture: nothing writes
    /// `FactorioSurface::enclosures` unless a real game refused a real walk.
    walled_in: BTreeMap<BotId, f64>,
    /// Bots the *game* has said cannot move from where they stand, with the
    /// position each was benched at.
    ///
    /// # The witness is the game, and there is no second one
    ///
    /// `FactorioSurface::benches` is written by `crates/executor`'s
    /// `walk_memory::note_mobility` when the pathfinder has refused a walk
    /// **and** refused a short hop in every direction from the character.
    /// That is the game reasoning from the character's own collision box at
    /// its own position, which is exactly what [`Self::walled_in`]'s fill
    /// cannot do: the fill models no characters and seeds from a tile
    /// centre. In `run-1788614781-38058` bot 6 stood overlapping a furnace,
    /// the fill said open, `walled_in` stayed empty, and the bot was given a
    /// gathering share -- and so a walk -- on seven consecutive plans.
    ///
    /// So no fill is asked to agree here. What keeps the bench honest is
    /// that it is a fact about a *position*: a row applies only while the
    /// bot still stands within [`WalkRefusal::SAME_PLACE_TOLERANCE`] of
    /// where it was earned. A bot pushed clear by a step-aside or a
    /// recovery is not the bot the game answered for, and is back in every
    /// split on the next plan without anyone having to lift the row. The
    /// executor also lifts it outright when a walk for that bot succeeds or
    /// a re-probe before the plan finds a hop the game will path.
    ///
    /// # What reads it
    ///
    /// Everything [`Self::walled_in`] gates -- shares
    /// ([`crate::method::have::even_shares`]) and the chain actor
    /// ([`crate::method::pick_chain_actor`]) -- and one thing it does not:
    /// [`crate::schedule`] refuses a benched bot every candidate pairing
    /// whose action would need it to walk. A walled-in bot keeps its
    /// membership because the fill might be wrong in the bot's favour; a
    /// benched bot has the game's word that it cannot go, and a plan that
    /// sends it anyway is the defect this field exists to end. It may still
    /// act where it stands.
    benched: BTreeMap<BotId, Position>,
    /// How much of each raw item this expansion has so far sent to be
    /// *gathered* -- picked off a tile or swung out of a rock -- summed over
    /// every `Have`/`Produced` that reached `Mine` or `Chop`. Written by those
    /// two methods' `expand` and read against [`Self::gathering_forecast`]
    /// by [`Self::gathering_ahead`].
    ///
    /// Per bot, because a swing's surplus lands in one bot's inventory and
    /// serves only that bot's later fragments: the demand a rock is judged
    /// against is the gathering bot's, not the roster's. The reader
    /// (`crate::method::have::chop_beats_mining`) is asked through
    /// `Method::applicable`, which has no actor but does have the goal's
    /// holder, and a `Have`'s holder is the bot that will gather it.
    gathering_recorded: BTreeMap<(BotId, ItemId), u32>,
    /// What a *rehearsal* of the same expansion gathered in total, item by
    /// item -- see [`crate::method::expand`]. Empty during the rehearsal
    /// itself and in every state built directly, so a method that reads
    /// [`Self::gathering_ahead`] sees zero and answers for the fragment in
    /// front of it, exactly as it did before the forecast existed.
    gathering_forecast: BTreeMap<(BotId, ItemId), u32>,
}

impl PlanState {
    pub fn from_world(base: Arc<FactorioSurface>, bots: &[BotId]) -> PlanState {
        let mut map = BTreeMap::new();
        let mut unknown_bots = BTreeSet::new();
        for id in bots {
            let state = match base.players.get(&id.0) {
                Some(player) => BotState {
                    position: player.position.clone(),
                    inventory: player.main_inventory.clone(),
                    build_distance: player.build_distance as f64,
                    reach_distance: player.reach_distance as f64,
                    resource_reach_distance: player.resource_reach_distance,
                },
                None => {
                    unknown_bots.insert(*id);
                    BotState::default()
                }
            };
            map.insert(*id, state);
        }
        let max_prototype_half_diagonal = base
            .entity_prototypes
            .iter()
            .map(|entry| {
                let b = &entry.value().collision_box;
                (b.width() / 2.).hypot(b.height() / 2.)
            })
            .fold(
                0.0_f64,
                |acc, d| if d.total_cmp(&acc).is_gt() { d } else { acc },
            );
        // By name, never by sort. See the `force` field's own doc for the run
        // that established what the sort actually selected.
        let force = base.forces.get(BOT_FORCE).map(|entry| entry.key().clone());
        // The roster's worst case, not each bot's own: a tile is claimed by
        // one action and has to keep *every* other bot off it, so the bound
        // that matters is the largest reach anybody in the roster swings from.
        // `f64::MAX` — what a player with no character reports — is not a
        // character's reach and is dropped rather than propagated.
        let reach = map
            .values()
            .map(|bot| bot.resource_reach_distance)
            .filter(|reach| *reach > 0. && *reach <= MAX_PLAUSIBLE_RESOURCE_REACH)
            .fold(None, |acc: Option<f64>, reach| {
                Some(match acc {
                    Some(best) if best.total_cmp(&reach).is_ge() => best,
                    _ => reach,
                })
            })
            .unwrap_or(VANILLA_RESOURCE_REACH);
        let mining_tile_separation = reach + tile_occupancy_radius(&base);
        // Every character on the surface, roster or not. The roster used to be
        // filtered out here; see the `characters` field doc for the run that
        // showed a roster bot is not a bot the plan is going to move.
        let (half_x, half_y) = character_half_box(&base);
        let characters: BTreeMap<PlayerId, Rect> = base
            .players
            .iter()
            .map(|player| {
                let p = &player.value().position;
                (
                    *player.key(),
                    Rect::new(
                        &Position::new(p.x() - half_x, p.y() - half_y),
                        &Position::new(p.x() + half_x, p.y() + half_y),
                    ),
                )
            })
            .collect();
        // Sites the game refused, turned into the boxes it refused them at.
        // The prototype lookup is the same one `collision_area_facing` makes,
        // under the same name the planner used when it chose the site and
        // turned the same way the build was aimed, so the box excluded here
        // is exactly the box that was offered and turned down. A refusal that
        // carries no direction (a ledger written before it was recorded) is
        // read north-facing, which is what every entry was read as until
        // then; a direction no building stands on falls back the same way
        // rather than to a guess.
        let mut refused: Vec<RefusedFootprint> = base
            .placement_refusals()
            .iter()
            // Expired here, and only here. A refusal whose own recorded
            // evidence names nothing but transients -- a character, a ghost --
            // is not a fact about the ground, and believing it for the rest of
            // the run is what stopped a partial `BuildBlock` from ever being
            // finished by a replan: `goal.built` re-derives the entities not
            // yet standing, and one permanently fenced footprint refuses the
            // whole block. See
            // `PlacementRefusal::names_only_transient_blockers`, which is
            // careful in the other direction too: an empty blocker list is
            // kept, because "the mod appended nothing" is not "nothing was
            // there".
            .filter(|refusal| !refusal.names_only_transient_blockers())
            .map(|refusal| {
                let facing = refusal
                    .direction
                    .and_then(Direction::from_u8)
                    .unwrap_or(Direction::North);
                let area = base
                    .entity_prototypes
                    .get(&refusal.entity)
                    .map(|proto| {
                        let box_ = rotated_collision_box(&proto.collision_box, facing)
                            .unwrap_or_else(|| proto.collision_box.clone());
                        add_to_rect(&box_, &refusal.position)
                    })
                    // No prototype: the one thing that can be said without
                    // inventing a size is that the game refused a build
                    // centred here. A unit box around that centre is the
                    // floor, not a guess at the entity -- it under-excludes,
                    // which is the safe direction for a fallback that should
                    // never fire (the planner only ever places names the
                    // world has prototypes for).
                    .unwrap_or_else(|| {
                        Rect::new(
                            &Position::new(refusal.position.x - 0.5, refusal.position.y - 0.5),
                            &Position::new(refusal.position.x + 0.5, refusal.position.y + 0.5),
                        )
                    });
                // Copied whole, not summarised: what the game found is the
                // one description of this ground that was not written by the
                // model the refusal contradicts.
                RefusedFootprint {
                    area,
                    entity: refusal.entity.clone(),
                    blockers: refusal.blockers.clone(),
                    tile: refusal.tile.clone(),
                }
            })
            .collect();
        refused.sort_by(|a, b| {
            a.area
                .left_top
                .x
                .total_cmp(&b.area.left_top.x)
                .then(a.area.left_top.y.total_cmp(&b.area.left_top.y))
                .then(a.area.right_bottom.x.total_cmp(&b.area.right_bottom.x))
                .then(a.area.right_bottom.y.total_cmp(&b.area.right_bottom.y))
                // Two refusals at the same box are still two refusals -- of
                // different entities, or of the same one with different
                // evidence -- so the tie is broken by what they say rather
                // than left to arrival order, which is the one thing this
                // sort exists to remove.
                .then_with(|| a.entity.cmp(&b.entity))
                .then_with(|| a.blockers.cmp(&b.blockers))
                .then_with(|| a.tile.cmp(&b.tile))
        });
        // Walks the game searched for and did not find. Taken whole rather
        // than reduced to geometry the way `refused` is: a refused walk names
        // a bot, a place it stood and a place it could not get to, and
        // dropping any of the three would answer a question nobody asked.
        let mut refused_walks: Vec<WalkRefusal> = base.walk_refusals();
        refused_walks.sort_by(|a, b| {
            a.player
                .cmp(&b.player)
                .then(a.from.x.total_cmp(&b.from.x))
                .then(a.from.y.total_cmp(&b.from.y))
                .then(a.to.x.total_cmp(&b.to.x))
                .then(a.to.y.total_cmp(&b.to.y))
        });
        // What the world last saw inside each container and machine, kept only
        // where the reading and the world still agree about what is standing
        // there. See the `buffers` field for what a buffer is and where the
        // decision about which ones to observe actually lives.
        let mut buffers: BTreeMap<Pos, Buffer> = BTreeMap::new();
        let mut fuel: BTreeMap<Pos, BTreeMap<ItemId, u32>> = BTreeMap::new();
        let mut input: BTreeMap<Pos, Option<BTreeMap<ItemId, u32>>> = BTreeMap::new();
        for (tile, observed) in base.observed_inventories() {
            // `is_empty` covers all three inventories, so a furnace holding
            // nothing but ore is kept rather than skipped as "nothing here" --
            // which is the whole point of reading the input slot.
            if observed.is_empty() {
                continue;
            }
            // A reading is keyed by tile, and a tile can be cleared and
            // rebuilt. Believing a reading whose entity is gone -- or whose
            // entity is now something else -- would plan a `Remove` against
            // an entity the game will not find, and `EntityAt` would only
            // catch the first of those two.
            let Some(entity) = base
                .entity_graph
                .entity_at(&observed.position)
                .and_then(|id| base.entity_graph.entity_by_id(id))
            else {
                continue;
            };
            if entity.name != observed.name {
                continue;
            }
            if !observed.fuel.is_empty() {
                fuel.insert(tile.clone(), observed.fuel);
            }
            // Stored whenever the game answered about this entity at all, empty
            // slot and absent slot alike: `None` here means "this entity has no
            // input inventory", and a tile with no entry at all means "nobody
            // asked". Folding the first into the second would lose the
            // distinction one map lower down.
            input.insert(tile.clone(), observed.input);
            if observed.output.is_empty() {
                continue;
            }
            let Some(slot) = withdraw_slot(&entity.entity_type) else {
                continue;
            };
            buffers.insert(
                tile,
                Buffer {
                    name: observed.name,
                    position: observed.position,
                    slot,
                    contents: observed.output,
                },
            );
        }
        let mut state = PlanState {
            base,
            bots: map,
            unknown_bots,
            added: Default::default(),
            removed: Default::default(),
            consumed: Default::default(),
            claimed: Default::default(),
            committed_machines: Default::default(),
            machine_queue: Default::default(),
            machine_load: Default::default(),
            planned_ticks: Default::default(),
            stockpiled: Default::default(),
            claim_runner: None,
            force,
            researched: Default::default(),
            reserved: Default::default(),
            reserved_by_anyone: Default::default(),
            max_prototype_half_diagonal,
            mining_tile_separation,
            characters,
            refused,
            buffers,
            fuel,
            input,
            refused_walks,
            walled_in: BTreeMap::new(),
            benched: BTreeMap::new(),
            gathering_recorded: BTreeMap::new(),
            gathering_forecast: BTreeMap::new(),
            drain_policy: DrainPolicy::default(),
            policy_probe: Arc::new(AtomicBool::new(false)),
        };
        state.walled_in = state.find_walled_in();
        state.benched = state.find_benched();
        state
    }

    /// The benches that still apply: one per roster bot the game has benched
    /// and that is still standing where the bench was earned -- see the
    /// [`PlanState::benched`] field for why the position is the test and
    /// why no fill is consulted.
    fn find_benched(&self) -> BTreeMap<BotId, Position> {
        let benches = self.base.benches();
        if benches.is_empty() {
            return BTreeMap::new();
        }
        let mut out = BTreeMap::new();
        for (bot, state) in &self.bots {
            let here = benches.iter().find(|bench| {
                bench.player == bot.0
                    && (bench.at.x - state.position.x)
                        .hypot(bench.at.y - state.position.y)
                        .total_cmp(&WalkRefusal::SAME_PLACE_TOLERANCE)
                        .is_le()
            });
            if let Some(bench) = here {
                out.insert(*bot, bench.at.clone());
            }
        }
        out
    }

    /// Record that `bot` has been sent to gather `need` of `item` by hand --
    /// off a tile or off a rock. Called by `Mine::expand` and `Chop::expand`,
    /// once each, at the head of their expansion, with the chain actor.
    pub fn note_gathering(&mut self, bot: BotId, item: &str, need: u32) {
        if need == 0 {
            return;
        }
        let entry = self
            .gathering_recorded
            .entry((bot, item.to_string()))
            .or_insert(0);
        *entry = entry.saturating_add(need);
    }

    /// How much of `item` `whose` is still going to gather after what it has
    /// gathered so far, according to the rehearsal's forecast; zero with no
    /// forecast, and zero once the recorded total has caught up with it.
    ///
    /// A named holder reads its own bot's ledger; `Holder::Anyone` reads
    /// the roster's, which over-states what any one bot will gather and so
    /// errs towards the rock -- the direction a goal nobody in particular
    /// owns can afford, since it is scattered into per-bot shares before any
    /// gathering method sees it.
    ///
    /// This is what lets a gathering decision be made over the plan's
    /// demand for the item rather than over one fragment of it: a
    /// `Have { coal, 1 }` for a furnace's fuel is one of a dozen such
    /// fragments, and priced alone none of them pays for a rock that covers
    /// them all.
    pub fn gathering_ahead(&self, whose: &Holder, item: &str) -> u32 {
        let ahead = |bot: BotId| {
            self.gathering_forecast
                .get(&(bot, item.to_string()))
                .copied()
                .unwrap_or(0)
                .saturating_sub(
                    self.gathering_recorded
                        .get(&(bot, item.to_string()))
                        .copied()
                        .unwrap_or(0),
                )
        };
        match whose {
            Holder::Bot(bot) | Holder::Share(bot) => ahead(*bot),
            Holder::Anyone => self
                .bots
                .keys()
                .map(|bot| ahead(*bot))
                .fold(0u32, u32::saturating_add),
        }
    }

    /// Everything [`Self::note_gathering`] has recorded, by bot and item.
    pub fn gathering_recorded(&self) -> BTreeMap<(BotId, ItemId), u32> {
        self.gathering_recorded.clone()
    }

    /// Install a rehearsal's totals as this state's forecast. Replaces any
    /// forecast already set; the recorded ledger is untouched.
    pub fn set_gathering_forecast(&mut self, forecast: BTreeMap<(BotId, ItemId), u32>) {
        self.gathering_forecast = forecast;
    }

    /// Which of this state's bots are sealed into a pocket where they stand.
    ///
    /// Run once, from [`PlanState::from_world`], on a state that is complete
    /// except for this field. See the field's own doc for why the answer needs
    /// two witnesses rather than one.
    ///
    /// # Cost
    ///
    /// One flood fill per bot the *game* has already reported walled in at
    /// (about) its current spot — which is zero for every fixture in this
    /// workspace and for every healthy run, because the ledger this reads is
    /// only ever written from a `failed to path find`. It is deliberately not
    /// "a fill per bot": that would be four fills on every plan for a question
    /// nobody has a reason to ask.
    fn find_walled_in(&self) -> BTreeMap<BotId, f64> {
        let observed = self.base.enclosures();
        if observed.is_empty() {
            return BTreeMap::new();
        }
        let mut out = BTreeMap::new();
        for (bot, state) in &self.bots {
            let reported = observed.iter().any(|found| {
                found.player == bot.0
                    && (found.at.x - state.position.x)
                        .hypot(found.at.y - state.position.y)
                        .total_cmp(&WalkRefusal::SAME_PLACE_TOLERANCE)
                        .is_le()
            });
            if !reported {
                continue;
            }
            // The second witness. `Open` and `Unknown` both mean "do not
            // exclude"; only a fill that closes inside the window does.
            if let crate::enclosure::Escape::Enclosed { pocket_tiles } =
                self.escape_from(&state.position)
            {
                out.insert(*bot, pocket_tiles);
            }
        }
        out
    }

    pub fn fork(&self) -> PlanState {
        self.clone()
    }

    /// How freely a fragment may wait on a standing cell -- see
    /// [`DrainPolicy`] and [`crate::plan_best`].
    pub fn drain_policy(&self) -> DrainPolicy {
        self.drain_policy
    }

    /// The same state under another drain policy. Consuming, so a policy is
    /// chosen once for a whole expansion rather than drifting inside one.
    #[must_use]
    pub fn with_drain_policy(mut self, policy: DrainPolicy) -> PlanState {
        self.drain_policy = policy;
        self
    }

    /// The same state with a fresh, unset [`PlanState::policy_probe`], so an
    /// expansion's observation is its own rather than an earlier run's.
    ///
    /// Consuming and fresh-allocating for the same reason `with_drain_policy`
    /// is consuming: the probe belongs to one expansion, and a probe carried
    /// in from a caller would answer for a run nobody here made.
    #[must_use]
    pub fn with_fresh_policy_probe(mut self) -> PlanState {
        self.policy_probe = Arc::new(AtomicBool::new(false));
        self
    }

    /// Record that the decision just taken would have differed under another
    /// [`DrainPolicy`]. Called by `Drain::new` and nowhere else.
    pub(crate) fn note_policy_divergence(&self) {
        self.policy_probe.store(true, Ordering::Relaxed);
    }

    /// Whether any drain decision taken against this state (or any fork of
    /// it, since the probe is shared) depended on the drain policy.
    ///
    /// **Only meaningful after a complete expansion.** A `false` read
    /// part-way through says the policy has not mattered *yet*, which proves
    /// nothing about the rest -- see the field's doc.
    pub fn drain_policy_mattered(&self) -> bool {
        self.policy_probe.load(Ordering::Relaxed)
    }

    pub fn base(&self) -> &Arc<FactorioSurface> {
        &self.base
    }

    /// Bots passed to [`PlanState::from_world`] that `base` had no player
    /// for, and so were seeded with `BotState::default()` — an empty
    /// inventory and guessed reach distances — instead of the world's own
    /// data. Empty whenever every requested bot was a real player, which is
    /// every production call today; see the field doc for why this is a
    /// detectable flag rather than a hard error.
    pub fn unknown_bots(&self) -> &BTreeSet<BotId> {
        &self.unknown_bots
    }

    /// Technology `name` as the acting force defines it.
    ///
    /// The only way to reach a `FactorioTechnology` from a `PlanState`, so
    /// that a caller cannot read one force's cost while `is_researched`
    /// answers about another's.
    pub fn technology(&self, name: &str) -> Option<FactorioTechnology> {
        let force = self.force.as_deref()?;
        self.base
            .forces
            .get(force)
            .and_then(|entry| entry.technologies.get(name).cloned())
    }

    /// The acting force's `manual_mining_speed_modifier`, or `0.` when the
    /// world does not report one.
    ///
    /// Read through the acting force for the same reason `technology` is: the
    /// planner acts for exactly one force, and a mining rate taken from a
    /// different force than the research questions are answered against would
    /// be the same silent disagreement that field doc describes.
    ///
    /// `0.` is the game's own default for an unmodified force, so a world
    /// with no forces at all (every planner fixture) reads as "no bonus"
    /// rather than as an error.
    pub fn manual_mining_speed_modifier(&self) -> f64 {
        let Some(force) = self.force.as_deref() else {
            return 0.;
        };
        self.base
            .forces
            .get(force)
            .and_then(|entry| entry.manual_mining_speed_modifier.as_deref().copied())
            .map(f64::from)
            .unwrap_or(0.)
    }

    /// Every technology the acting force defines, in name order.
    ///
    /// Ordered because `recipe -> technology` lookups scan this and must pick
    /// the same answer on every run; the underlying map is a `BTreeMap`, so the
    /// order is the data's, not the hash seed's.
    pub fn technology_names(&self) -> Vec<String> {
        let Some(force) = self.force.as_deref() else {
            return Vec::new();
        };
        self.base
            .forces
            .get(force)
            .map(|entry| entry.technologies.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// How many of `item` fit in one stack, from the world's own
    /// `item_prototypes`.
    ///
    /// `None` means the world carries no prototype for `item`, which on the
    /// production path never happens — `workspace/scripts/map.json` carries
    /// all 342 of them — and in this crate's fixtures happens constantly. See
    /// [`PlanState::slot_capacity`] for what a caller must do with that.
    pub fn stack_size(&self, item: &str) -> Option<u32> {
        self.base.item_prototypes.get(item).map(|p| p.stack_size)
    }

    /// The most of `item` one machine's `slot` can hold at once.
    ///
    /// **The planner otherwise models every machine inventory as unbounded**,
    /// and sizes a load, a batch and a take from what the *goal* needs while
    /// the game sizes them from what a *slot* holds. Where the two disagree
    /// the game wins: silently on the way in (a full output slot puts a
    /// furnace in `full_output` and it stops smelting) and loudly on the way
    /// out (`tried to remove 141 iron-plate but removed 100`). This is the one
    /// function that knows the difference; see
    /// `docs/superpowers/specs/2026-09-04-world-model-divergence-design.md`.
    ///
    /// The rules are measured against a live Factorio 2.1.17 instance, by
    /// creating an entity, inserting 1000–5000 of the item into the named
    /// `defines.inventory` and recording what was accepted:
    ///
    /// * **Outputs are one slot of exactly `stack_size`** — stone-furnace /
    ///   iron-plate 100, assembling-machine-1 / iron-gear-wheel 100,
    ///   assembling-machine-1 / electronic-circuit 200. The number tracks the
    ///   *item*, not the machine, which is why this is read per item rather
    ///   than tabulated.
    /// * **`Fuel` is one slot of exactly `stack_size`** — stone-furnace /
    ///   coal 50, burner-mining-drill / coal 50.
    /// * **Inputs are `stack_size` + [`INPUT_OVERLOAD`]** — iron-ore,
    ///   copper-ore, stone and solid-fuel 70 (stack 50); iron-plate 120
    ///   (stack 100); copper-cable 220 (stack 200).
    /// * **`LabInput` is one stack per science type** — lab /
    ///   automation-science-pack 200.
    /// * **`Chest` is `slots × stack_size` and cannot be computed here.**
    ///   `FactorioEntityPrototype` carries no inventory size, so this answers
    ///   `None` for a chest. A wooden chest holds 1600 iron-plate and the
    ///   largest chest transfer in any plan measured so far is 37, so nothing
    ///   comes near it; sending the slot count is the follow-up.
    ///
    /// **Capacity comes from prototypes and never from the game**, and that is
    /// settled rather than chosen: the furnace whose take failed was placed by
    /// the *same plan* that planned to empty it, so at expansion time there was
    /// no entity to ask. Nor is `get_insertable_count` a usable oracle — on the
    /// bench it answered 50 for an input slot that `insert` then filled to 70.
    ///
    /// # `None` means "unknown, therefore unbounded"
    ///
    /// Never a guessed number. Several of this crate's fixtures build worlds
    /// with no `item_prototypes` at all and pin their plans byte-for-byte, so
    /// a fallback constant would silently re-size every one of them. A caller
    /// must treat `None` as "do not split", which is exactly today's
    /// behaviour, and the `warn!` below is here because a silently uncapped
    /// path is the failure mode this accessor exists to end.
    pub fn slot_capacity(&self, slot: InventorySlot, item: &str) -> Option<u32> {
        if slot == InventorySlot::Chest {
            // Known-unknowable rather than missing, so no warning: see above.
            return None;
        }
        let Some(stack) = self.stack_size(item) else {
            factorio_bot_core::tracing::warn!(
                item,
                slot = ?slot,
                "no item prototype, so this slot is planned as unbounded"
            );
            return None;
        };
        Some(match slot {
            InventorySlot::FurnaceSource | InventorySlot::AssemblerInput => {
                stack.saturating_add(INPUT_OVERLOAD)
            }
            InventorySlot::FurnaceResult
            | InventorySlot::AssemblerOutput
            | InventorySlot::Fuel
            | InventorySlot::LabInput => stack,
            // Returned above; matched rather than `_` so a new variant is a
            // compile error here and not a silently wrong capacity.
            InventorySlot::Chest => return None,
        })
    }

    pub fn bot_ids(&self) -> Vec<BotId> {
        self.bots.keys().copied().collect()
    }

    pub fn bot(&self, id: BotId) -> Option<&BotState> {
        self.bots.get(&id)
    }

    pub fn inventory_count(&self, id: BotId, item: &str) -> u32 {
        self.bots
            .get(&id)
            .and_then(|b| b.inventory.get(item))
            .copied()
            .unwrap_or(0)
    }

    /// Every item any bot holds, with the roster-wide total for each.
    pub fn item_totals(&self) -> BTreeMap<ItemId, u32> {
        let mut out: BTreeMap<ItemId, u32> = BTreeMap::new();
        for bot in self.bots.values() {
            for (item, count) in &bot.inventory {
                *out.entry(item.clone()).or_insert(0) += *count;
            }
        }
        out
    }

    /// Sum across every bot — the meaning of `Holder::Anyone`.
    pub fn total_count(&self, item: &str) -> u32 {
        self.bots
            .values()
            .map(|b| b.inventory.get(item).copied().unwrap_or(0))
            .sum()
    }

    /// How much of `item` is left for a *new* goal to count towards itself:
    /// what `whose` holds, less what expansion has already promised to an
    /// action it is about to emit.
    ///
    /// This — not `inventory_count`/`total_count` — is the question
    /// "is this goal already satisfied" has to ask. Asking the raw holding
    /// double-counts an intermediate two sub-goals of one recipe both draw on:
    /// a lab's own ten gears are visible to the sub-goal that produces its
    /// transport belts, which then spends two of them and leaves the lab craft
    /// short. Reserving is what stops a holding being claimed twice.
    ///
    /// A per-bot reservation lowers the roster total too, because the items it
    /// names are a known bot's and so genuinely spoken for. An `Anyone`
    /// reservation does *not* lower any one bot's figure, because it names no
    /// bot: nothing says which of them holds the promised items, and guessing
    /// would refuse work a bot can really do.
    pub fn available(&self, whose: &Holder, item: &str) -> u32 {
        match whose {
            Holder::Anyone => {
                let promised: u32 = self
                    .reserved
                    .values()
                    .map(|held| held.get(item).copied().unwrap_or(0))
                    .sum::<u32>()
                    .saturating_add(self.reserved_by_anyone.get(item).copied().unwrap_or(0));
                self.total_count(item).saturating_sub(promised)
            }
            Holder::Bot(id) | Holder::Share(id) => self
                .inventory_count(*id, item)
                .saturating_sub(self.reserved_for(*id, item)),
        }
    }

    fn reserved_for(&self, id: BotId, item: &str) -> u32 {
        self.reserved
            .get(&id)
            .and_then(|held| held.get(item))
            .copied()
            .unwrap_or(0)
    }

    /// Promise `count` of `item` to `whose`, hiding it from [`available`] until
    /// [`PlanState::release`] gives it back. Reservations add up, so two
    /// promises of the same item hide both.
    ///
    /// [`available`]: PlanState::available
    pub fn reserve(&mut self, whose: &Holder, item: &str, count: u32) {
        let ledger = match whose {
            Holder::Anyone => &mut self.reserved_by_anyone,
            Holder::Bot(id) | Holder::Share(id) => self.reserved.entry(*id).or_default(),
        };
        let entry = ledger.entry(item.to_string()).or_insert(0);
        *entry = entry.saturating_add(count);
    }

    /// Undo one [`PlanState::reserve`]. Saturating rather than fallible: a
    /// release is always paired with a reserve by the caller that made it, so
    /// there is no failure for a caller to handle, and clamping at zero cannot
    /// invent stock the way wrapping would.
    pub fn release(&mut self, whose: &Holder, item: &str, count: u32) {
        let ledger = match whose {
            Holder::Anyone => &mut self.reserved_by_anyone,
            Holder::Bot(id) | Holder::Share(id) => self.reserved.entry(*id).or_default(),
        };
        let Some(entry) = ledger.get_mut(item) else {
            return;
        };
        *entry = entry.saturating_sub(count);
        if *entry == 0 {
            ledger.remove(item);
        }
    }

    pub fn gain(&mut self, id: BotId, item: &str, count: u32) {
        let bot = self.bots.entry(id).or_default();
        *bot.inventory.entry(item.to_string()).or_insert(0) += count;
    }

    pub fn lose(&mut self, id: BotId, item: &str, count: u32) -> Result<(), PlannerError> {
        let available = self.inventory_count(id, item);
        if available < count {
            return Err(PlannerError::InsufficientItems {
                bot: id,
                item: item.to_string(),
                required: count,
                available,
            });
        }
        let bot = self.bots.get_mut(&id).ok_or(PlannerError::UnknownBot(id))?;
        let entry = bot.inventory.entry(item.to_string()).or_insert(0);
        *entry -= count;
        if *entry == 0 {
            bot.inventory.remove(item);
        }
        Ok(())
    }

    pub fn set_position(&mut self, id: BotId, position: Position) {
        self.bots.entry(id).or_default().position = position;
    }

    /// Does this plan know of any buffer at all?
    ///
    /// The cheap guard [`crate::method::have::Withdraw::applicable`] asks
    /// first. Every fixture in this crate, and every world nobody has pulled
    /// container contents into, answers `false` here -- so the whole
    /// withdrawal path costs one `BTreeMap::is_empty` on the plans that have
    /// nothing to withdraw, which is all of them until a caller refreshes.
    pub fn has_buffers(&self) -> bool {
        !self.buffers.is_empty()
    }

    /// How much `item` the burner machine on `position`'s tile was last seen
    /// holding in its fuel slot.
    ///
    /// Zero for a machine nobody has asked the game about, which is every
    /// machine offline and every machine outside the list `refresh_buffers`
    /// queries -- and zero is the answer that makes a caller *bring* the fuel,
    /// so an unread slot costs a few coal and never a stalled machine. See
    /// the [`fuel`](PlanState#structfield.fuel) field.
    pub fn fuelled(&self, position: &Position, item: &str) -> u32 {
        self.fuel
            .get(&Pos::from(position))
            .and_then(|slot| slot.get(item))
            .copied()
            .unwrap_or(0)
    }

    /// Is the machine on `position`'s tile holding something it has been given
    /// and not yet turned into anything -- ore in a furnace, ingredients in an
    /// assembler?
    ///
    /// `false` for three different situations, and a caller that needs to tell
    /// them apart wants [`PlanState::input_reading`] rather than this: nobody
    /// asked the game about this tile, the entity has no input slot at all
    /// (every chest), or it has one and it is standing empty.
    ///
    /// # What this answers that [`PlanState::holds_buffer`] cannot
    ///
    /// A furnace mid-smelt has ore in and nothing out, so `holds_buffer` says
    /// `false` for it -- the same answer it gives for a furnace no ore ever
    /// reached. `crate::method::have`'s `adoptable_furnaces` uses that answer
    /// to decide a standing furnace is free to load, and so treats a busy
    /// furnace as idle. **This function is the evidence; acting on it is a
    /// planner policy change that has to be measured live** (an offline dump's
    /// `inventories` is always empty, so no baseline here can move), and it is
    /// deliberately left to whoever owns that decision -- excluding a busy
    /// furnace makes a plan place another one instead, which costs stone.
    pub fn holds_input(&self, position: &Position) -> bool {
        self.input
            .get(&Pos::from(position))
            .and_then(Option::as_ref)
            .is_some_and(|slot| slot.values().any(|count| *count > 0))
    }

    /// The raw input reading for `position`'s tile, with `absent`, `empty` and
    /// `never asked` all still distinct.
    ///
    /// `None` -- nobody asked the game about this tile.
    /// `Some(None)` -- it answered, and the entity has no input inventory.
    /// `Some(Some(map))` -- it answered with the contents, possibly empty.
    ///
    /// Kept beside the verdict [`PlanState::holds_input`] rather than replaced
    /// by it, so a caller that needs to distinguish "this machine cannot hold
    /// ore" from "it can and does not" still has the evidence.
    pub fn input_reading(&self, position: &Position) -> Option<&Option<BTreeMap<ItemId, u32>>> {
        self.input.get(&Pos::from(position))
    }

    /// How much of `item` the machine on `position`'s tile was last seen
    /// holding in its input slot.
    ///
    /// Zero for a tile nobody asked about, for an entity with no input slot
    /// and for an empty one alike -- all three mean "no `item` is waiting in
    /// there", which is what a caller counting ore wants. Use
    /// [`PlanState::input_reading`] when the difference matters.
    pub fn input_held(&self, position: &Position, item: &str) -> u32 {
        self.input
            .get(&Pos::from(position))
            .and_then(Option::as_ref)
            .and_then(|slot| slot.get(item))
            .copied()
            .unwrap_or(0)
    }

    /// Is anything at all still sitting in the buffer on `position`'s tile?
    ///
    /// The item-blind form of [`PlanState::buffered`], for a caller asking
    /// whether a machine is *idle* rather than whether it holds some
    /// particular thing — a furnace with five plates in its result slot is
    /// spoken for whatever those plates are, because `Withdraw` may already
    /// have planned to take them.
    pub fn holds_buffer(&self, position: &Position) -> bool {
        self.buffers
            .get(&Pos::from(position))
            .is_some_and(|buffer| buffer.contents.values().any(|count| *count > 0))
    }

    /// How much of `item` the buffer standing on `position`'s tile still
    /// holds, as far as this plan is concerned.
    ///
    /// Zero for a tile with no buffer, which is the same answer as a buffer
    /// holding none of it -- deliberately, because both mean "nothing to take
    /// from here" and a caller has `entity_at` if it needs to tell them apart.
    pub fn buffered(&self, position: &Position, item: &str) -> u32 {
        self.buffers
            .get(&Pos::from(position))
            .and_then(|buffer| buffer.contents.get(item))
            .copied()
            .unwrap_or(0)
    }

    /// Every buffer holding at least one `item`, nearest `from` first.
    ///
    /// Ordered by `(distance, x, y, name)` with `total_cmp`, the same total
    /// order [`PlanState::nearest_supply_anchor`] uses and for the same
    /// reason: emission order fixes `ActionId` allocation, which fixes
    /// `schedule`'s `(end, ActionId, BotId)` tie-break. Two buffers equally
    /// far away must resolve the same way on every run, and a distance alone
    /// does not do that.
    ///
    /// **No radius.** A distant buffer is a longer walk, and `schedule`
    /// already prices a walk at `WALK_TILES_PER_TICK`; refusing one would
    /// charge the same distance twice, once as ticks and once as a veto. It
    /// would also be the more dangerous error: the alternative to withdrawing
    /// is making the items again, and the whole reason this overlay exists is
    /// that making them again may be *impossible* -- the ore they came from is
    /// gone from the ground. Under-withdrawing strands materials permanently;
    /// over-withdrawing costs a walk. The bound that does exist is which
    /// containers a caller asked the game about in the first place, and it
    /// lives there rather than here (see the [`Buffer`] type).
    pub fn buffers_holding(&self, from: &Position, item: &str) -> Vec<Buffer> {
        let mut out: Vec<(f64, Buffer)> = self
            .buffers
            .values()
            .filter(|buffer| !self.stockpiled.contains(&Pos::from(&buffer.position)))
            .filter(|buffer| buffer.contents.get(item).copied().unwrap_or(0) > 0)
            .map(|buffer| (calculate_distance(&buffer.position, from), buffer.clone()))
            .collect();
        out.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.position.x.total_cmp(&b.1.position.x))
                .then(a.1.position.y.total_cmp(&b.1.position.y))
                .then(a.1.name.cmp(&b.1.name))
        });
        out.into_iter().map(|(_, buffer)| buffer).collect()
    }

    /// Spend `count` of `item` out of the buffer on `position`'s tile.
    ///
    /// **Fallible, and it must stay fallible.** Taking more than a buffer
    /// holds is not a rounding question, it is the plan having counted the
    /// same plates twice -- so it is an error with the numbers in it rather
    /// than a saturating subtraction that would leave the second `Remove`
    /// standing in the plan and let the *game* discover the shortfall. The
    /// game does discover it, and loudly (`rcon_remove_from_inventory`
    /// complains when it moves fewer items than asked, and
    /// `judge_transfer_reply` reads any complaint as a failed action) -- but a
    /// plan that is arithmetically wrong should fail at the planner, not four
    /// minutes later on a bot that has walked there.
    /// Put `count` of `item` into the buffer on `position`'s tile, creating
    /// the buffer if this is the first thing the plan has put there.
    ///
    /// **Infallible, where [`PlanState::take_from_buffer`] is not**, and the
    /// asymmetry is the point. Taking more than a buffer holds is the plan
    /// having counted the same items twice, which is an arithmetic error worth
    /// an error value. Adding to one cannot be wrong in that way: whether the
    /// depositing bot really has the items is a separate question, asked and
    /// answered by the `Condition::HasItem` on the action carrying the
    /// [`Effect::BufferGain`] and by the `Effect::LoseItem` beside it.
    ///
    /// `name` and `slot` are taken from the caller rather than looked up,
    /// because a chest the plan placed a moment ago has no reading behind it
    /// -- see [`Effect::BufferGain`](crate::action::Effect::BufferGain). They
    /// are only used when the entry is created; a later deposit into a buffer
    /// the world reported keeps the name and slot that reading came with,
    /// which is what a `Remove` against it has to address.
    ///
    /// Capacity is not modelled. A wooden chest holds sixteen stacks and the
    /// bills this crate writes are far smaller, so a limit here would be a
    /// number with no case behind it; the game refusing an insert is a
    /// transfer failure the executor already reports.
    pub fn stock_buffer(
        &mut self,
        position: &Position,
        name: &str,
        slot: InventorySlot,
        item: &str,
        count: u32,
    ) {
        if count == 0 {
            return;
        }
        let buffer = self
            .buffers
            .entry(Pos::from(position))
            .or_insert_with(|| Buffer {
                name: name.to_string(),
                position: position.clone(),
                slot,
                contents: BTreeMap::new(),
            });
        *buffer.contents.entry(item.to_string()).or_insert(0) += count;
    }

    /// Mark the buffer on `position`'s tile as one this plan is filling.
    ///
    /// See [`stockpiled`](PlanState#structfield.stockpiled). Idempotent, and
    /// never undone within an expansion: the deposits it protects are emitted
    /// once and consumed once.
    pub fn commit_stockpile(&mut self, position: &Position) {
        self.stockpiled.insert(Pos::from(position));
    }

    pub fn take_from_buffer(
        &mut self,
        position: &Position,
        item: &str,
        count: u32,
    ) -> Result<(), PlannerError> {
        let key = Pos::from(position);
        let available = self.buffered(position, item);
        if available < count {
            return Err(PlannerError::BufferShort {
                item: item.to_string(),
                position: position.to_string(),
                required: count,
                available,
            });
        }
        let Some(buffer) = self.buffers.get_mut(&key) else {
            // Unreachable while `available >= count` and `count > 0`; a
            // zero-count take of a tile with no buffer lands here and is a
            // no-op, which is the right answer for it.
            return Ok(());
        };
        let entry = buffer.contents.entry(item.to_string()).or_insert(0);
        *entry -= count;
        if *entry == 0 {
            buffer.contents.remove(item);
        }
        Ok(())
    }

    pub fn entity_at(&self, position: &Position) -> Option<FactorioEntity> {
        let key = Pos::from(position);
        if let Some(entity) = self.added.get(&key) {
            return Some(entity.clone());
        }
        if self.removed.contains(&key) {
            return None;
        }
        self.base
            .entity_graph
            .entity_at(position)
            .and_then(|id| self.base.entity_graph.entity_by_id(id))
    }

    /// The ground an entity of `name` would cover with its centre at `position`,
    /// in world coordinates.
    ///
    /// Read from the game's own `collision_box` prototype — never assumed and
    /// never per-entity constants. A stone furnace is ±0.69921875 and an
    /// assembling machine ±1.19921875; the planner has no business knowing
    /// either number.
    ///
    /// `None` when the world has no prototype under that name. Callers treat
    /// that as *not placeable* rather than guessing a size: a guess that is too
    /// small is exactly the defect this function exists to remove, and the only
    /// names the planner ever places come from the world's own prototypes and
    /// recipes, so a miss means the world carries no prototype data at all.
    /// Failing to plan is recoverable; planning a placement the game refuses,
    /// which costs the bot its whole remaining slice, is not.
    pub fn collision_area(&self, name: &str, position: &Position) -> Option<Rect> {
        self.collision_area_facing(name, position, Direction::North)
    }

    /// [`collision_area`](Self::collision_area) for an entity that stands
    /// facing `direction` rather than north.
    ///
    /// A boiler is 3x2 tiles facing north and 2x3 facing east, so the ground a
    /// placement claims is a function of its direction and not only of its
    /// name. Every `Place` this planner emitted before the power plant carried
    /// direction 0, which is why one function sufficed until now.
    ///
    /// `None` for an unknown prototype, exactly as
    /// [`collision_area`](Self::collision_area), and also for a half-diagonal
    /// direction: no building stands on one, and guessing a box for it would
    /// be inventing a footprint.
    pub fn collision_area_facing(
        &self,
        name: &str,
        position: &Position,
        direction: Direction,
    ) -> Option<Rect> {
        let proto = self.base.entity_prototypes.get(name)?;
        let box_ = rotated_collision_box(&proto.collision_box, direction)?;
        Some(add_to_rect(&box_, position))
    }

    /// Does `name`'s prototype collide with water tiles?
    ///
    /// **The offshore pump does not**, and that single exception is why this
    /// exists. Its `collision_mask` is object and train layers only — the
    /// vanilla prototype says so in a comment in place, "collide just with
    /// object-layer and train-layer which don't collide with water" — so it is
    /// the one building in this plan that may stand with its box over a lake.
    /// Since `fa8dabf3` made water solid, every water tile is a blocking box
    /// in `blocked_tree`, and a pump sited on a real shoreline has its body
    /// over two of them. Without this the plant refuses every site it finds.
    ///
    /// Read from the prototype rather than named here, so it is the game's
    /// answer and not this crate's. **Both spellings of the layer**, because
    /// the two captures in this repo disagree: the entity-prototype fixture
    /// says `water-tile` and `player-layer` (Factorio 1.x names) while the live
    /// 2.1.17 snapshot says `water_tile` and `player`. Matching one of them
    /// would make this true in tests and false in a run, or the reverse.
    ///
    /// A prototype with **no** mask at all collides: an unstated mask is not a
    /// licence to build in a lake.
    pub fn collides_with_water(&self, name: &str) -> bool {
        match self.base.entity_prototypes.get(name) {
            Some(proto) => match &proto.collision_mask {
                Some(layers) => layers
                    .iter()
                    .any(|layer| layer == "water-tile" || layer == "water_tile"),
                None => true,
            },
            None => true,
        }
    }

    /// Is `name` a machine that has to stand **on** a resource to work?
    ///
    /// **This is a question about the machine, not about the ground, and it
    /// decides no placement.** A drill that is not on ore mines nothing; that
    /// is what this answers, and it is why `method::blueprint`'s
    /// `drills_are_fed` and `nearest_ore_seed` ask it. Whether ore *blocks* a
    /// placement is a different question with a different answer, and until
    /// `ore-does-not-block` this predicate was used for both — see
    /// [`occupant_of`](Self::occupant_of), which no longer counts ore at all.
    ///
    /// The conflation was expensive and silent, because it failed safe: ore
    /// refused legal ground, so it surfaced as `NoRoute` and `NoSiteFound`
    /// rather than as a factory that would not build. A belt route across a
    /// patch was refused; a cell near any patch was refused a site; and
    /// `MinerLine` could not site at any radius, because its own belt-and-pole
    /// corridor ran over the ore its drills need.
    ///
    /// **Read from the prototype's `entity_type`, and deliberately not from
    /// its `collision_mask`.** The mask says only what the entity collides
    /// with, and needing ore underfoot is not a collision — a drill's 2.1.17
    /// mask is `is_lower_object`, `is_object`, `water_tile`, `item`, `object`,
    /// `player`, `meltable`, character for character a stone furnace's.
    ///
    /// `entity_type` is the game's own classification and both captures in
    /// this repo agree on it, unlike the mask layer names: `mining-drill` in
    /// the 1.x entity-prototype fixture and in the live 2.1.17 snapshot alike.
    /// It admits `burner-mining-drill`, `electric-mining-drill`,
    /// `big-mining-drill` and `pumpjack` — every one of which stands on a
    /// resource, crude oil included.
    ///
    /// An entity the world has no prototype for answers `false`: an unknown
    /// name is not something this can vouch for, and the callers all read a
    /// `true` as licence to site *at* ore.
    pub fn stands_on_resources(&self, name: &str) -> bool {
        match self.base.entity_prototypes.get(name) {
            Some(proto) => proto.entity_type == "mining-drill",
            None => false,
        }
    }

    /// The `resource_category` of the resource prototype `name`, or `None`
    /// when there is no such resource or the capture predates the field.
    ///
    /// The two `None`s are told apart by [`PlanState::is_resource`]; a
    /// caller that needs to say *why* it cannot answer asks both.
    pub fn resource_category(&self, name: &str) -> Option<String> {
        self.base
            .entity_prototypes
            .get(name)
            .filter(|proto| proto.entity_type == "resource")
            .and_then(|proto| proto.resource_category.clone())
    }

    /// What mining the prototype `name` yields, by product name in lexical
    /// order (`mine_result` is a `BTreeMap`). Empty for a prototype the world
    /// does not know or one that yields nothing.
    pub fn mine_products(&self, name: &str) -> Vec<String> {
        self.base
            .entity_prototypes
            .get(name)
            .and_then(|proto| proto.mine_result.clone())
            .map(|products| products.into_keys().collect())
            .unwrap_or_default()
    }

    /// Whether `name` is a `resource` prototype -- something that comes out
    /// of the ground -- whatever else the capture knows about it.
    pub fn is_resource(&self, name: &str) -> bool {
        self.base
            .entity_prototypes
            .get(name)
            .is_some_and(|proto| proto.entity_type == "resource")
    }

    /// The `mining-drill` prototypes whose `resource_categories` list
    /// `category`, by name in lexical order.
    ///
    /// This is the game's own rule for what mines what -- a drill works a
    /// resource iff the resource's category is among the drill's -- read off
    /// the prototype table the same way [`PlanState::hand_mining_obstacle`]
    /// reads the character's. Sorted so the answer depends on the data and
    /// not on the map's iteration order. A drill captured before the mod
    /// sent `resource_categories` lists nothing and is never returned: an
    /// absent answer is read as "not said", never as "mines everything".
    pub fn extractors_for(&self, category: &str) -> Vec<String> {
        let mut drills: Vec<String> = self
            .base
            .entity_prototypes
            .iter()
            .filter(|proto| proto.entity_type == "mining-drill")
            .filter(|proto| {
                proto
                    .resource_categories
                    .as_ref()
                    .is_some_and(|categories| categories.iter().any(|c| c == category))
            })
            .map(|proto| proto.name.clone())
            .collect();
        drills.sort();
        drills
    }

    /// Names of every `resource` prototype this world knows, in lexical
    /// order.
    ///
    /// The mirror of [`PlanState::extractors_for`]'s filter over the same
    /// table: that one walks every `mining-drill` prototype for a given
    /// category, this walks every `resource` prototype regardless of
    /// category. It exists so a caller can invert the resource -> category ->
    /// drill relation -- "which resources can THIS drill extract" -- without
    /// hardcoding which resource names a map might carry (iron-ore,
    /// copper-ore, coal, stone, crude-oil, uranium-ore, and whatever a mod
    /// adds). Sorted for the same reason `extractors_for` sorts: the answer
    /// must depend on the data, not on `DashMap`'s iteration order.
    pub fn resource_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .base
            .entity_prototypes
            .iter()
            .filter(|proto| proto.entity_type == "resource")
            .map(|proto| proto.name.clone())
            .collect();
        names.sort();
        names
    }

    /// The water tile nearest `from`, or `None` if there is none within
    /// `max_radius`.
    ///
    /// A straight delegation to `EntityGraph`, and it stays one on purpose:
    /// **the plan overlay has no tiles.** Nothing this planner emits creates
    /// or removes terrain, so there is no `added`/`removed` pass to make here
    /// and no way for a plan to disagree with the world about where the lake
    /// is. Ordering, the Euclidean/Manhattan trap and the half-tile between a
    /// tile's corner and its centre are all settled there; see
    /// `EntityGraph::nearest_water_tile`.
    ///
    /// **`None` is not "there is no water".** A world attached from a snapshot
    /// (`crates/core`'s `attach_world`) fetches no tiles at all, so every
    /// answer here is `None` for it. That is the right failure — a plant sited
    /// against terrain nobody has read would be sited by guesswork — but it is
    /// a failure, not an observation.
    /// Does `area` cover at least one tile that still holds `item`?
    ///
    /// The question a mining drill's site has to answer, and the one
    /// [`PlanState::is_area_clear_of`] never asks: that one says only whether
    /// something is in the way (ore is not), while this one *requires* ore for
    /// the drill. A drill placed one tile off the patch places 100 % and
    /// mines nothing, which is the same silent failure as a drill facing the
    /// wrong way.
    ///
    /// Tiles, not the box, for the same reason [`PlanState::delivers_into`]
    /// works in tiles: a resource occupies a whole tile and a collision box
    /// covers 1.398 of every 2 tiles it stands on.
    ///
    /// **A drill neither claims nor consumes what it stands on.**
    /// `Effect::ConsumeResource` is emitted by mining and by nothing else, so
    /// this reads what is left after the *plan's own hand-mining* and says
    /// nothing about the patch being drained by the machine. That is a
    /// modelling gap this stage does not close.
    pub fn covers_resource(&self, area: &Rect, item: &str) -> bool {
        tiles_under(area)
            .iter()
            .any(|tile| self.resource_available(&Position::from(tile), item) > 0)
    }

    /// Does `area` cover any resource tile at all?
    ///
    /// **A preference, never a refusal.** Ore blocks no placement -- see
    /// [`occupant_of`](Self::occupant_of) -- so this is not asked by anything
    /// that decides whether a build is legal. It is asked by
    /// [`crate::method::util::free_area_near_where`], which searches the rings
    /// twice: once for a site that covers no ore, and only then for any site
    /// that fits. So a furnace still lands beside a patch rather than on it
    /// whenever there is ground beside the patch, and lands on it rather than
    /// nowhere when there is not.
    ///
    /// That split is what the old rule could not express. Refusing ore
    /// outright was the same answer to "prefer not to" and "may not", and the
    /// second answer is the one the game never gives: the cost was `NoRoute`,
    /// `NoSiteFound` and a `MinerLine` that could not site at any radius.
    ///
    /// Presence, not quantity, and not claims: a tile whose ore this plan has
    /// drained is still ore in the ground, and whether a *mine* has spoken for
    /// it is the narrower question [`covers_claimed_resource`](Self::covers_claimed_resource)
    /// answers -- that one is a refusal, because burying ore a plan has already
    /// promised a bot is a contradiction rather than a preference.
    pub fn covers_any_resource(&self, area: &Rect) -> bool {
        tiles_under(area)
            .iter()
            .any(|tile| self.base.entity_graph.any_resource_at(tile))
    }

    /// Does `area` cover a resource tile this plan has already committed to a
    /// mining action?
    ///
    /// The mirror of [`PlanState::resource_tile_blocked`]'s first source, and
    /// the other half of the same rule: that one keeps a *mine* off ground the
    /// plan has built on, this one keeps a *placement* off ground the plan has
    /// promised to a miner. Both exist because nothing constrains the order the
    /// two commitments are made in — a `Mine` expanded before the cell is sited
    /// claims its tiles into an empty entity overlay, and a cell sited before
    /// the mine writes its drill into an empty claim ledger — so a single-sided
    /// check would only ever cover whichever order happened to be the one in
    /// front of it.
    ///
    /// Whole-tile, like [`PlanState::is_resource_claimed`] and for the same
    /// reason: a claim says the tile is spoken for, not how much of it is.
    /// Crowding ([`PlanState::is_resource_crowded`]) is deliberately *not*
    /// asked. That rule is about where a bot stands, and a machine does not
    /// stand anywhere; a drill on the tile next to a mined one is legal, and
    /// the run this comes from mined `(-33.5, -24.5)` and `(-34.5, -24.5)`
    /// against a drill at `[-34, -23]` successfully.
    ///
    /// Reads the *claim* ledger, not `resource_available`: a tile a mining
    /// action has committed to still holds its ore, which is exactly why
    /// `covers_resource` above is happy to site a drill on it.
    pub fn covers_claimed_resource(&self, area: &Rect) -> bool {
        tiles_under(area)
            .iter()
            .any(|tile| self.claimed.contains_key(tile))
    }

    /// Where the machine `entity` puts what it makes, in world coordinates.
    ///
    /// **The game's own answer when it has one.** `FactorioEntity` carries a
    /// `drop_position` and the mod fills it in for every entity it serialises,
    /// so a drill a live save already contains reports where it really drops —
    /// including a pumpjack, whose reported value `EntityGraph::add` has to
    /// correct. Only an entity *this plan placed* has none, because a method
    /// builds a `FactorioEntity` from a name, a position and a direction and
    /// nothing else; for those the north-frame table
    /// [`delivery_offset`] is turned into the entity's own facing.
    ///
    /// `None` when neither source can say, which is a refusal to guess: a
    /// wrong delivery point is a cell that places 100 % and produces nothing.
    pub fn delivery_position(&self, entity: &FactorioEntity) -> Option<Position> {
        if let Some(drop) = &entity.drop_position {
            return Some(drop.clone());
        }
        let facing = Direction::from_u8(entity.direction)?;
        Some(entity.position.add(&delivery_offset(&entity.name, facing)?))
    }

    /// Does the machine standing at `from` deliver into the machine standing at
    /// `to`?
    ///
    /// The model behind [`crate::action::Condition::Feeds`], and the whole of
    /// what makes a `Goal::Producing` more than "two machines stand somewhere".
    ///
    /// # Tile containment, not box containment, and the number that decides it
    ///
    /// A burner drill at an integer position facing north drops at
    /// `(-0.35, -1.3)` from its own centre. The stone furnace two tiles north
    /// of it — the vanilla starter pair, the thing this is here to model — has
    /// a collision box of `+/-0.69921875`, so its near edge is at `-1.30078125`
    /// and the drop point misses it by **0.00078125 of a tile**, one part in
    /// 1280. Box containment would therefore reject the one layout the whole of
    /// stage 1 is built on, and any layout that satisfied it would have been
    /// derived from the wrong rule.
    ///
    /// A drop point resolves to the *tile* it lands in, so the question is
    /// whether that tile is one of the tiles the target covers — the same
    /// `tiles_under` this state already uses to ask which resource tiles a
    /// footprint would cover.
    /// The furnace covers both of its tiles, the drop lands in one of them, and
    /// the 1/1280 never comes up.
    ///
    /// Both ends must be machines this state can *see* and *size*: an unknown
    /// prototype, an entity that has been removed, or a source with no delivery
    /// point all answer `false`.
    pub fn delivers_into(&self, from: &Position, to: &Position) -> bool {
        let Some(source) = self.entity_at(from) else {
            return false;
        };
        let Some(target) = self.entity_at(to) else {
            return false;
        };
        // A push: the source has a drop point and the target covers the tile
        // it lands in. A drill, and an inserter's drop side.
        if let Some(drop) = self.delivery_position(&source)
            && self.covers_tile(&target, &drop)
        {
            return true;
        }
        // A pull: the *target* is an inserter whose pickup tile is one the
        // source covers. A furnace does not push into an inserter and an
        // inserter is not "delivered into" in the drill's sense — but items do
        // move from the one to the other, which is what this predicate is
        // named for. `pickup_position` answers `None` for everything that is
        // not an inserter, so this disjunct widens nothing else.
        if let Some(pickup) = self.pickup_position(&target)
            && self.covers_tile(&source, &pickup)
        {
            return true;
        }
        false
    }

    /// Where the inserter standing as `entity` picks up from, or `None` if it
    /// is not an inserter.
    pub fn pickup_position(&self, entity: &FactorioEntity) -> Option<Position> {
        let facing = Direction::from_u8(entity.direction)?;
        Some(entity.position.add(&pickup_offset(&entity.name, facing)?))
    }

    /// Is `point` inside one of the tiles `entity` stands on?
    ///
    /// Tiles, not the box, for the 1/1280 reason [`delivers_into`] states at
    /// length: a burner drill's drop point misses a stone furnace's collision
    /// box by 0.00078125 of a tile, and box containment would reject the one
    /// layout stage 1 exists to build.
    fn covers_tile(&self, entity: &FactorioEntity, point: &Position) -> bool {
        let Some(facing) = Direction::from_u8(entity.direction) else {
            return false;
        };
        let Some(area) = self.collision_area_facing(&entity.name, &entity.position, facing) else {
            return false;
        };
        tiles_under(&area).contains(&Pos::from(point))
    }

    pub fn nearest_water_tile(&self, from: &Position, max_radius: f64) -> Option<FactorioTile> {
        self.base.entity_graph.nearest_water_tile(from, max_radius)
    }

    /// Every water tile inside `bounds`, in `(x, y)` order.
    ///
    /// One bounded query, so a shoreline search asks the quad tree once
    /// instead of once per candidate tile. Since `fa8dabf3` a fully charted
    /// map carries ~410,000 water tiles, and the per-tile shape of this
    /// question is what would make that expensive.
    pub fn water_tiles_within(&self, bounds: &Rect) -> Vec<FactorioTile> {
        self.base
            .entity_graph
            .tiles_within(bounds)
            .into_iter()
            .filter(FactorioTile::is_water)
            .collect()
    }

    /// The ground an entity already in the plan covers.
    ///
    /// `create_entity` fills the entity's `bounding_box` from its prototype, so
    /// this is normally that box. The fallback — the single tile the entity
    /// stands on — is for entities put into the state directly with neither a
    /// bounding box nor a known prototype, and it under-reserves; it is the
    /// least this can claim without inventing a size.
    ///
    /// Public since 2026-09-06 so `method::power` can draw the ground a block
    /// occupies from the entities it is about to place. See
    /// [`Excluded::Ground`], whose doc says why footprints and not positions.
    pub fn footprint_of(&self, entity: &FactorioEntity) -> Rect {
        if entity.bounding_box.width() > 0. && entity.bounding_box.height() > 0. {
            return entity.bounding_box.clone();
        }
        Direction::from_u8(entity.direction)
            .and_then(|facing| self.collision_area_facing(&entity.name, &entity.position, facing))
            .unwrap_or_else(|| tile_area(&Pos::from(&entity.position)))
    }

    /// Whether an entity of `name` could be built with its centre at `position`.
    ///
    /// The question `is_position_free` could not ask. A stone furnace is 1.398
    /// tiles across, so two of them one tile apart overlap while each one's
    /// *tile* is free — which is how the red-science plan came to site furnaces
    /// at `[-34, -1]` and `[-34, 0]` and have the game refuse the second.
    ///
    /// Entities are compared box against box rather than tile against tile, so
    /// a placement is refused when it would actually collide and not merely
    /// when it shares a tile.
    pub fn is_area_free(&self, name: &str, position: &Position) -> bool {
        self.is_area_free_facing(name, position, Direction::North)
    }

    /// [`is_area_free`](Self::is_area_free) for an entity that stands facing
    /// `direction`.
    ///
    /// Two things change with the direction, and both of them matter:
    ///
    /// * the footprint rotates (see
    ///   [`collision_area_facing`](Self::collision_area_facing));
    /// * nothing else. **Water tolerance is a property of the entity, not of
    ///   its facing** — see [`collides_with_water`](Self::collides_with_water)
    ///   — and it applies to `is_area_free` too, which is why the plain
    ///   function is this one facing north rather than a stricter sibling. A
    ///   pump refused for standing in the lake it pumps from would be refused
    ///   at every angle equally.
    pub fn is_area_free_facing(
        &self,
        name: &str,
        position: &Position,
        direction: Direction,
    ) -> bool {
        match self.collision_area_facing(name, position, direction) {
            Some(area) => self.is_area_clear_of(&area, self.collides_with_water(name)),
            None => false,
        }
    }

    /// How far a placement's own footprint pushes the acting character's
    /// stand-point away from the entity's centre: half the diagonal of the
    /// entity's collision box, plus half the diagonal of the character's.
    ///
    /// This is the annulus's inner radius, not a margin on top of one — at
    /// exactly this distance the two boxes can, in the worst-case orientation,
    /// touch at a corner (which `boxes_overlap`'s own `TOUCH_SLACK` already
    /// treats as clear); any closer and they are guaranteed to overlap.
    ///
    /// The guarantee: no point of a box is farther from that box's own centre
    /// than the box's half-diagonal (that is what a half-diagonal *is* — the
    /// distance from centre to corner). So if a point `p` is common to both
    /// boxes, the triangle inequality gives `|entity_centre - character_centre|
    /// <= |entity_centre - p| + |p - character_centre| <= entity_half_diag +
    /// character_half_diag`. Contrapositive: centres farther apart than that
    /// sum cannot share a point. This is the same reasoning
    /// [`PlanState::is_area_clear`] already uses to widen its own search
    /// radius, applied here to bound a minimum instead of a maximum.
    ///
    /// `None` when the world has no prototype for `name` — mirrors
    /// [`PlanState::collision_area`]: an unknown size is not something this
    /// can bound, not a guessed zero. In practice this is moot for any `Place`
    /// action that could ever run: its own [`crate::action::Condition::AreaFree`]
    /// needs the same prototype and refuses the action first.
    ///
    /// The character's own box falls back to
    /// [`VANILLA_CHARACTER_COLLISION_HALF_SIDE`] when the world carries no
    /// `character` prototype; see that constant's doc for where it comes from.
    pub fn placement_clearance(&self, name: &str) -> Option<f64> {
        let entity = self.base.entity_prototypes.get(name)?;
        let entity_half_diag = {
            let b = &entity.collision_box;
            (b.width() / 2.).hypot(b.height() / 2.)
        };
        let character = character_half_box(&self.base);
        let character_half_diag = character.0.hypot(character.1);
        Some(entity_half_diag + character_half_diag)
    }

    /// Whether anything the plan can see occupies `area`.
    ///
    /// Six sources, because no single one of them sees everything: entities
    /// this plan has placed, entities the base world already had, the terrain
    /// nobody built, the characters standing on it, ore, and the footprints
    /// the game itself has already refused. The middle three
    /// are the odd ones — `EntityGraph::add` only ever inserts a whitelist of
    /// *factory* entity types into the entity tree, so trees, cliffs, small
    /// rocks, units and water tiles have to come out of
    /// `blocking_boxes_within`; characters are not in any tree at all and come
    /// from `base.players` via
    /// [`characters`](PlanState#structfield.characters); and
    /// `add` routes resource entities into `resources`/`resource_tree` only,
    /// so ore has to be asked for by tile.
    ///
    /// Leaving the terrain source out is what made the planner site a stone
    /// furnace on a forest tile and the game answer `can_place_entity said
    /// 'no'` (run `run-1788309767-54739`, first dispatched action). The data
    /// was never missing — `EntityGraph` is fed `surface.find_entities(area)`,
    /// which is ~79% trees — it was only in a tree this function did not read.
    /// Leaving the character source out produced the *same message* from the
    /// same call for an entirely different reason two runs later, and again
    /// three runs after that when the character source was there but excluded
    /// the roster; see the `characters` field doc for both. The sixth source
    /// is the admission that this list will keep being incomplete: it is not
    /// a model of anything, it is the game's own refusals played back, and it
    /// is what stops the next unmodelled obstacle costing a whole milestone
    /// instead of one placement. See
    /// [`refused`](PlanState#structfield.refused).
    fn is_area_clear(&self, area: &Rect) -> bool {
        self.is_area_clear_of(area, true)
    }

    /// [`is_area_clear`](Self::is_area_clear), with water optionally not
    /// counted as an obstacle.
    ///
    /// `water_blocks` is the caller's answer to "is the thing being sited here
    /// subject to this source of occupancy", and it is consulted by exactly
    /// one of the five sources. Everything else keeps blocking either way: an
    /// entity, a character or a refused footprint standing on a lake still
    /// blocks, whatever the pump's mask says about tiles.
    ///
    /// The water is identified by name through `EntityGraph::is_water_at`, not
    /// by the blocking box's own payload, which carries a bare `is_minable`
    /// flag and no name at all. A box's centre is the tile it came from — a
    /// tile's blocking box is exactly that tile's 1x1 square — which is the
    /// same key the `removed` lookup one line above already uses on the very
    /// same rectangle.
    ///
    /// **There is no `resource_blocks` twin, because ore is not an obstacle.**
    /// See [`occupant_of`](Self::occupant_of).
    fn is_area_clear_of(&self, area: &Rect, water_blocks: bool) -> bool {
        self.occupant_of(area, water_blocks, true).is_none()
    }

    /// [`is_area_clear_of`](Self::is_area_clear_of), but SAYING WHAT IS THERE.
    ///
    /// The same five sources in the same order, because it is the same
    /// function -- `is_area_clear_of` is now `occupant_of(..).is_none()`, so
    /// the two cannot drift into disagreeing about whether ground is clear
    /// while disagreeing about why.
    ///
    /// It exists because a bare `false` is a bad refusal. `BuildBlock`
    /// (`method::blueprint`) places a block at a *fixed* anchor and has no
    /// siting story at all, so occupancy is its most likely refusal -- and
    /// until this existed the fact reached the caller as
    /// `PlannerError::ChainOwnerInfeasible`, which blames an internal
    /// scheduling decision for a fact about the ground. Four runs across
    /// three anchors were spent distinguishing hypotheses that a named tile
    /// would have settled in one line.
    ///
    /// **Ore is not one of the sources.** In Factorio a resource entity's
    /// entire collision mask is the single `resource` layer, and no buildable
    /// prototype carries that layer: of the 579 prototypes in the live 2.1.17
    /// snapshot (`crates/core/tests/live-2.1.17-world-snapshot.json`) exactly
    /// 12 name it, and all 12 *are* resources; the 1.x fixture agrees across
    /// the `resource-layer` → `resource` rename. So the game lets a belt, a
    /// pole, a furnace or an assembler be built on an ore patch, and a live
    /// `can_place_entity` at an ore tile confirms it. Until
    /// `ore-does-not-block` this function refused ore to everything but a
    /// mining drill, which was **a policy stated as a collision rule** — "do not bury the
    /// patch you are about to mine" — and it failed in the safe direction, so
    /// nothing caught it: it refused legal ground as `NoRoute` and
    /// `NoSiteFound` and never built anywhere illegal. What survives of the
    /// policy is the part that was always the real requirement and is checked
    /// where it belongs: [`covers_claimed_resource`](Self::covers_claimed_resource)
    /// keeps a plan from building over ore that same plan has promised a bot
    /// will hand-mine, and `method::blueprint`'s `drills_are_fed` keeps a
    /// drill on ore it can extract.
    ///
    /// `characters_block` is the fourth of the five sources' own toggle, and
    /// it exists for exactly one caller: [`PlanState::siting_occupant`],
    /// used only while *choosing* an anchor (`method::blueprint::search_site`
    /// via `first_obstruction`). A character is not durable ground -- it
    /// walks away on its own, with no action and no plan commitment -- so a
    /// bystander (or one of this plan's own bots) standing in a candidate
    /// ring on one expansion and gone on the next must not change which ring
    /// wins; see `search_site`'s own doc for why the search has to answer the
    /// same way twice. Every other caller ([`PlanState::placement_occupant`]
    /// included) passes `true`: once a block is actually being built at a
    /// fixed anchor, a character standing on the footprint is exactly the
    /// fact the caller needs told, cleared by walking rather than by moving
    /// the block.
    ///
    /// **Ghosts are not one of the five sources either.** Measured live
    /// against Factorio 2.1.17 before `ActionKind::StampGhosts` existed:
    /// `only_ghosts = true` on `rcon_place_blueprint` validates no clearance
    /// at all, and a real placement consumes the ghost beneath it cleanly
    /// rather than being refused by it. A ghost the entity loops below found
    /// by bounding box alone would report `Occupant::Entity("entity-ghost")`
    /// -- blocking the exact `Place` the marker exists to make possible,
    /// which is worse than not stamping at all. Skipped by name in both
    /// entity loops, unconditionally: no caller of `occupant_of` should ever
    /// want a ghost to collide.
    fn occupant_of(
        &self,
        area: &Rect,
        water_blocks: bool,
        characters_block: bool,
    ) -> Option<Occupant> {
        for entity in self.added.values() {
            if entity.name == GHOST_ENTITY_NAME {
                continue;
            }
            if boxes_overlap(&self.footprint_of(entity), area) {
                return Some(Occupant::Entity(entity.name.clone()));
            }
        }
        // `find_entities_in_radius` keeps only entities whose *centre* point
        // falls within `radius` of `search_center` (see
        // `EntityGraph::find_entities_in_radius`,
        // `core/src/graph/entity_graph.rs:132`) — it does not know the
        // entity's own footprint, so a neighbour whose box overlaps `area`
        // while its centre sits outside `radius` would silently be skipped.
        // By the triangle inequality for the Euclidean norm, an entity whose
        // box overlaps `area` has its centre no further from `area`'s centre
        // than `area`'s own half-diagonal plus that entity's half-diagonal.
        // Widening the radius by the largest half-diagonal among the world's
        // known prototypes therefore cannot miss a genuine overlap, as long
        // as no unknown prototype is wider than every known one. The exact
        // test is the `boxes_overlap` below; this only narrows the search.
        let area_half_diagonal = (area.width() / 2.).hypot(area.height() / 2.);
        let radius = area_half_diagonal + self.max_prototype_half_diagonal + TOUCH_SLACK;
        for entity in
            self.base
                .entity_graph
                .find_entities_in_radius(area.center(), radius, None, None)
        {
            if self.removed.contains(&Pos::from(&entity.position)) {
                continue;
            }
            if entity.name == GHOST_ENTITY_NAME {
                continue;
            }
            if boxes_overlap(&entity.bounding_box, area) {
                return Some(Occupant::Entity(entity.name.clone()));
            }
        }
        // Characters. Nothing above can see one: they are in no tree, and
        // `removed` cannot free them either — the plan has no action that
        // makes a character move out of the way, and believing one will is the
        // same wrong answer as not seeing it at all. Roster bots included:
        // being on the roster is not a promise that this plan will move you.
        //
        // `characters_block` is this source's own toggle -- see this
        // function's doc for the one caller (siting) that turns it off,
        // because a character is the one source among these six that is
        // known to move with no plan action at all.
        if characters_block {
            for (player, character) in &self.characters {
                if boxes_overlap(character, area) {
                    // Whether it is one of the bots this plan is FOR is the
                    // whole difference between "someone is standing there"
                    // and "the thing you asked to build is under your own
                    // feet", which is what a researcher building a block
                    // near their roster hits.
                    return Some(Occupant::Character {
                        player: *player,
                        on_roster: self.bots.contains_key(&BotId(*player)),
                    });
                }
            }
        }
        // Footprints the game has already refused a build at. Not a model of
        // an obstacle -- a verdict about one. `removed` cannot clear these
        // either: nothing the plan does is known to change the answer.
        //
        // ASKED BEFORE THE BLOCKING BOXES, since 2026-09-06. Both can cover
        // the same ground, and when they do this one knows what it is talking
        // about and the other does not: a refusal carries the game's own list
        // of what stood in the box, while a blocking box is an anonymous
        // rectangle. Asking the anonymous source first is what produced
        // "occupied by a tree, cliff, rock or unit" for a tile the game had
        // just reported as holding nothing but ore. See
        // `docs/superpowers/notes/2026-09-06-a-failed-placement-blames-a-tree.md`.
        for refused in &self.refused {
            if boxes_overlap(&refused.area, area) {
                return Some(Occupant::Refused {
                    entity: refused.entity.clone(),
                    blockers: refused.blockers.clone(),
                    tile: refused.tile.clone(),
                });
            }
        }
        // Everything the entity tree structurally cannot hold: trees, cliffs,
        // small rocks, units, and `player_collidable` tiles (water). These
        // arrive as bare rectangles — `blocked_tree` keeps only an
        // `is_minable` flag, no name and no position — so a plan that removed
        // a base entity is matched by the tile its box is centred on, which is
        // the same key `remove_entity` stores and the same one the entity loop
        // above compares. A minable obstacle is *not* treated as clear: no
        // method emits an action to mine one out of the way, so believing a
        // tree will move is the same wrong answer as not seeing it at all.
        //
        // LAST of the six, because it is the only one that cannot say what it
        // found. `is_minable` is the single bit the tree stores and the whole
        // of what `Occupant::Terrain` may claim.
        for (blocked, minable) in self
            .base
            .entity_graph
            .blocking_boxes_within_minable(area)
            .into_iter()
        {
            if self.removed.contains(&Pos::from(&blocked.center())) {
                continue;
            }
            if !water_blocks && self.base.entity_graph.is_water_at(&blocked.center()) {
                continue;
            }
            if boxes_overlap(&blocked, area) {
                return Some(if self.base.entity_graph.is_water_at(&blocked.center()) {
                    Occupant::Water
                } else {
                    Occupant::Terrain { minable }
                });
            }
        }
        // Ore is deliberately NOT a sixth source: a resource collides on the
        // `resource` layer alone and nothing buildable carries it, so the game
        // builds straight over a patch. See this function's doc.
        None
    }

    /// What occupies the ground an entity of `name` facing `direction` would
    /// stand on, if anything does.
    ///
    /// The naming twin of [`is_area_free_facing`](Self::is_area_free_facing),
    /// and it answers with the same tolerances: water blocks only what
    /// collides with water, and ore blocks nothing at all.
    ///
    /// `Some(Occupant::Unknown)` -- not `None` -- when the world carries no
    /// prototype for `name`: `collision_area_facing` cannot size the
    /// footprint, `is_area_free_facing` answers `false` for exactly that
    /// case, and answering "clear" here would be the one wrong answer.
    pub fn placement_occupant(
        &self,
        name: &str,
        position: &Position,
        direction: Direction,
    ) -> Option<Occupant> {
        match self.collision_area_facing(name, position, direction) {
            Some(area) => self.occupant_of(&area, self.collides_with_water(name), true),
            None => Some(Occupant::Unknown),
        }
    }

    /// [`placement_occupant`](Self::placement_occupant), for choosing a site
    /// rather than building at one already chosen -- the one caller that
    /// needs to know what is on the ground and NOT know who happens to be
    /// standing on it.
    ///
    /// A character is not durable ground: nothing else among the five sources
    /// `occupant_of` checks can move with no plan action behind it, which is
    /// exactly why `method::blueprint::search_site`'s stability argument
    /// depends on this and not on `placement_occupant`. A bystander (or one
    /// of this plan's own bots) standing in a candidate ring on one
    /// expansion and gone -- or arrived -- on the next must not change which
    /// ring the search picks; that is the identical two-half-factories
    /// failure `search_site`'s own doc describes for a moving *seed*,
    /// arriving instead through a moving *obstacle*. See `search_site`'s doc
    /// for the full argument.
    ///
    /// `expand`'s own footprint pre-check (`BuildBlock::expand`, which builds
    /// at a fixed, already-chosen anchor) still calls `placement_occupant`,
    /// not this: once a block is actually going down, a character standing
    /// on the footprint is precisely the fact the caller needs told, and
    /// `Occupant::Character`'s `on_roster` flag exists so that fact can say
    /// "cleared by walking, not by moving the block."
    pub(crate) fn siting_occupant(
        &self,
        name: &str,
        position: &Position,
        direction: Direction,
    ) -> Option<Occupant> {
        match self.collision_area_facing(name, position, direction) {
            Some(area) => self.occupant_of(&area, self.collides_with_water(name), false),
            None => Some(Occupant::Unknown),
        }
    }

    /// Every obstacle a walking character could not pass through, inside
    /// `window` -- the three sources [`crate::enclosure`] uses, none of them
    /// the six [`PlanState::is_area_clear_of`] does. See that module's own
    /// doc for why characters, refusals and resource tiles are all left out
    /// here on purpose.
    ///
    /// **A belt is not a wall.** Every source here is a *buildability* index
    /// -- a belt in the way genuinely refuses a furnace -- and a character
    /// walks straight over one. `factorio_bot_core::graph::enclosure::
    /// blocks_character` is the predicate and carries the account; the
    /// planner has to apply it as well as the executor, because prevention
    /// and detection reading different walls is exactly the disagreement
    /// `crate::enclosure`'s module doc exists to forbid.
    ///
    /// The plan's own tentative entities and the base world's entities are
    /// filtered by name. `blocking_boxes_within` carries no name -- its
    /// payload is a bare `is_minable` flag -- so the walkable boxes gathered
    /// from the first two sources are subtracted from it instead.
    fn walkable_obstacles_within(&self, window: &Rect) -> Vec<Rect> {
        let prototypes = self.base.entity_graph.entity_prototypes();
        let blocks =
            |name: &str| factorio_bot_core::graph::enclosure::blocks_character(&prototypes, name);
        let mut obstacles = Vec::new();
        let mut walkable = Vec::new();
        for entity in self.added.values() {
            let footprint = self.footprint_of(entity);
            if blocks(&entity.name) {
                obstacles.push(footprint);
            } else {
                walkable.push(footprint);
            }
        }
        let area_half_diagonal = (window.width() / 2.).hypot(window.height() / 2.);
        let radius = area_half_diagonal + self.max_prototype_half_diagonal + TOUCH_SLACK;
        for entity in
            self.base
                .entity_graph
                .find_entities_in_radius(window.center(), radius, None, None)
        {
            if self.removed.contains(&Pos::from(&entity.position)) {
                continue;
            }
            if blocks(&entity.name) {
                obstacles.push(entity.bounding_box.clone());
            } else {
                walkable.push(entity.bounding_box.clone());
            }
        }
        let mut blocked = Vec::new();
        for area in self.base.entity_graph.blocking_boxes_within(window) {
            if self.removed.contains(&Pos::from(&area.center())) {
                continue;
            }
            blocked.push(area);
        }
        obstacles.extend(factorio_bot_core::graph::enclosure::drop_walkable(
            blocked, &walkable,
        ));
        obstacles
    }

    /// The blocked-cell grid [`crate::enclosure`]'s fills read, for the
    /// `enclosure::SEARCH_RADIUS`-tile window around `from` -- or `None` when
    /// that window is not entirely inside the region `blocked_tree` covers,
    /// in which case a query for it would come back short and the missing
    /// part would read as open ground. See
    /// `crates/core::graph::enclosure::EscapeUnknown::OutsideModel` for the
    /// same check made once already, against the real world rather than a
    /// plan.
    fn enclosure_grid(&self, from: &Position) -> Option<(Vec<bool>, (f64, f64))> {
        let (window, origin) = crate::enclosure::window(from);
        let tree = self.base.entity_graph.blocked_tree();
        let bounds = tree.bounding_box();
        let modelled = (
            bounds.origin.x as f64,
            bounds.origin.y as f64,
            (bounds.origin.x + bounds.size.width) as f64,
            (bounds.origin.y + bounds.size.height) as f64,
        );
        drop(tree);
        if window.left_top.x() < modelled.0
            || window.left_top.y() < modelled.1
            || window.right_bottom.x() > modelled.2
            || window.right_bottom.y() > modelled.3
        {
            return None;
        }
        let half_box = character_half_box(&self.base);
        let obstacles = self.walkable_obstacles_within(&window);
        Some((
            crate::enclosure::rasterize(obstacles.into_iter(), origin, half_box),
            origin,
        ))
    }

    /// Can a character standing at `from` reach ground outside a
    /// `enclosure::SEARCH_RADIUS`-tile window, given what this state alone
    /// believes is there?
    ///
    /// Used twice for one judgement: once against the state before a
    /// candidate placement exists and once against a fork that already
    /// carries it (see [`crate::enclosure::check`]). Neither call mutates
    /// anything and both read the same three obstacle sources, so the two
    /// answers differ only by whatever the fork actually added.
    pub(crate) fn escape_from(&self, from: &Position) -> crate::enclosure::Escape {
        match self.enclosure_grid(from) {
            None => crate::enclosure::Escape::Unknown,
            Some((blocked, _origin)) => crate::enclosure::fill_from_center(&blocked),
        }
    }

    /// The nearest tile a character at `from` could walk to under `self`'s
    /// own occupancy that stays connected to open ground under `after`'s --
    /// typically a fork of `self` that already carries a candidate placement.
    ///
    /// Two fills over the same window rather than one fill per tile
    /// considered: `self`'s grid gives every reachable cell in nearest-first
    /// order, `after`'s grid gives the set still connected to the window's
    /// own edge, and the first cell in the first list that is also in the
    /// second is the answer. `None` when no such cell exists in the window --
    /// the honest answer when a placement's only escape route runs through
    /// its own footprint, or when either grid could not be built at all (see
    /// [`Self::enclosure_grid`]).
    pub(crate) fn nearest_safe_escape(
        &self,
        after: &PlanState,
        from: &Position,
    ) -> Option<Position> {
        let (before_blocked, origin) = self.enclosure_grid(from)?;
        let (after_blocked, _) = after.enclosure_grid(from)?;
        let safe = crate::enclosure::reachable_from_boundary(&after_blocked);
        for cell in crate::enclosure::bfs_order_from_center(&before_blocked) {
            if safe[crate::enclosure::cell_index(cell.0, cell.1)] {
                return Some(crate::enclosure::cell_to_position(origin, cell));
            }
        }
        None
    }

    /// Every roster or non-roster bot within `radius` of `point`, as
    /// `(id, position)` -- the candidate pool [`crate::enclosure::check`]
    /// tests against a footprint. Ordered by player id, since `characters` is
    /// a `BTreeMap`.
    pub(crate) fn characters_near(
        &self,
        point: &Position,
        radius: f64,
    ) -> Vec<(PlayerId, Position)> {
        self.characters
            .iter()
            .filter_map(|(id, rect)| {
                let center = rect.center();
                (calculate_distance(point, &center) <= radius).then_some((*id, center))
            })
            .collect()
    }

    /// The footprints [`is_area_clear`](PlanState::is_area_clear) refuses
    /// because the game refused them first, in the deterministic order
    /// [`PlanState::from_world`] sorted them into.
    ///
    /// Exposed so a caller can say *which* sites the planner is avoiding --
    /// a plan that quietly prefers distant tiles and cannot say why is the
    /// failure mode this memory would otherwise introduce.
    pub fn refused_footprints(&self) -> &[RefusedFootprint] {
        &self.refused
    }

    /// Whether the game has already refused this exact placement.
    ///
    /// Narrower than [`PlanState::is_area_free`], which answers "is anything
    /// in the way" from five sources at once. This asks only about the one
    /// source that is a verdict rather than a model, so a caller can tell
    /// "the plan no longer fits the world" from "the game said no to this",
    /// and treat the second as durable. `crates/executor`'s `recover` uses it
    /// to refuse a tier-1 retry of a placement that is guaranteed to be
    /// refused again.
    ///
    /// `false` for an entity the world has no prototype for: an unknown size
    /// is not a footprint that can be compared, and the refusals themselves
    /// fall back to a unit box in that case, which is not a shape to test
    /// somebody else's placement against.
    pub fn is_site_refused(&self, name: &str, position: &Position) -> bool {
        let Some(area) = self.collision_area(name, position) else {
            return false;
        };
        self.refused
            .iter()
            .any(|refused| boxes_overlap(&refused.area, &area))
    }

    /// Whether the game has already told this bot there is no route from where
    /// it is standing to `to`.
    ///
    /// The walking counterpart of [`PlanState::is_site_refused`], and like it
    /// a *verdict* rather than a model: nothing in this crate can tell whether
    /// two points are connected — `PlanState` knows what occupies ground, not
    /// what a pathfinder can traverse — so the only way to know is to have
    /// been told, and the only way to still know on the next plan is to have
    /// kept it.
    ///
    /// # Both arguments matter, and `from` is the one that keeps this honest
    ///
    /// `failed to path find` means *unreachable from here*, not unreachable.
    /// A bot that has since moved is asking a question this ledger has no
    /// answer to, and gets none — see [`WalkRefusal::applies_to`], which owns
    /// the comparison so that this crate and the executor that writes the
    /// refusals cannot drift about what "the same question" means.
    ///
    /// `from` is the position the *plan* has the bot at when it would set off,
    /// which for its first step is the world's own reading and afterwards is
    /// the plan's simulated arrival. Matching a simulated position against an
    /// observed one is sound in the only direction that matters: it says "the
    /// plan expects the bot to be about where it was when the game refused
    /// it", which is exactly when the refusal still applies.
    pub fn is_walk_refused(&self, bot: BotId, from: &Position, to: &Position) -> bool {
        self.refused_walks
            .iter()
            .any(|refusal| refusal.applies_to(bot.0, from, to))
    }

    /// Every walk the game refused, in the deterministic order
    /// [`PlanState::from_world`] sorted them into.
    ///
    /// Exposed for the same reason [`PlanState::refused_footprints`] is: a
    /// plan that quietly sends a different bot and cannot say why is the
    /// failure mode this memory would otherwise introduce.
    pub fn refused_walks(&self) -> &[WalkRefusal] {
        &self.refused_walks
    }

    /// Whether this bot is sealed into a pocket where it is standing — see the
    /// [`PlanState::walled_in`] field.
    pub fn is_walled_in(&self, bot: BotId) -> bool {
        self.walled_in.contains_key(&bot)
    }

    /// Every walled-in bot with the size of its pocket, in square tiles.
    ///
    /// Exposed for the same reason [`PlanState::refused_walks`] is: a plan
    /// that quietly sizes a gathering goal for two bots out of four and cannot
    /// say why is the failure mode this exclusion would otherwise introduce.
    /// `crates/scripting_lua`'s planning entry point narrates it.
    pub fn walled_in(&self) -> &BTreeMap<BotId, f64> {
        &self.walled_in
    }

    /// Whether the game has benched this bot where it stands -- see the
    /// [`PlanState::benched`] field.
    pub fn is_benched(&self, bot: BotId) -> bool {
        self.benched.contains_key(&bot)
    }

    /// Every benched bot with the position its bench was earned at.
    ///
    /// Exposed for the reason [`PlanState::walled_in`] is: a plan that
    /// quietly gives a bot nothing and cannot say why is the failure mode
    /// this exclusion would otherwise introduce.
    pub fn benched(&self) -> &BTreeMap<BotId, Position> {
        &self.benched
    }

    /// Whether this bot may be handed work no other bot could take over --
    /// a gathering share, or the chain actor's role. `false` for a bot that
    /// is walled in or benched; the two readers of this,
    /// [`crate::method::have::even_shares`] and
    /// [`crate::method::pick_chain_actor`], ask exactly this one question.
    pub fn may_own_work(&self, bot: BotId) -> bool {
        !self.is_walled_in(bot) && !self.is_benched(bot)
    }

    /// Whether any bot is walled in or benched -- the cheap test the two
    /// readers above make first, so a healthy run provably takes the path
    /// it always did.
    pub fn any_sidelined(&self) -> bool {
        !self.walled_in.is_empty() || !self.benched.is_empty()
    }

    /// Whether this tile is clear.
    ///
    /// A tile-granularity question, kept because that is what
    /// `Condition::PositionFree` asks. It says nothing about whether a
    /// *particular* entity fits — a placement wants [`is_area_free`], which
    /// knows how big the thing being placed is.
    ///
    /// **Ore is not occupancy.** A resource collides on the `resource` layer
    /// alone and nothing buildable carries it, so an ore tile is clear ground
    /// as far as any placement is concerned; whether the plan has *promised*
    /// that ore to a mining action is a separate question with a separate
    /// answer, [`covers_claimed_resource`](Self::covers_claimed_resource).
    pub fn is_position_free(&self, position: &Position) -> bool {
        self.is_area_clear(&tile_area(&Pos::from(position)))
    }

    /// Every entity the plan can *name* within `radius` of `centre`, newest
    /// first: the ones this plan has placed, then the ones the base world
    /// already had, with anything this plan removed left out.
    ///
    /// Ordered by `(x, y, name)` with `total_cmp`, because
    /// `EntityGraph::find_entities_in_radius` walks a quad tree whose query
    /// order is not defined and this crate's output has to be byte-identical
    /// across runs.
    ///
    /// # What "can name" excludes, and why it matters here
    ///
    /// `EntityGraph::add` only inserts a **whitelist** of entity types into
    /// its entity tree (`crates/core/src/graph/entity_graph.rs`): trees,
    /// cliffs, small rocks and units are not on it and never reach this at
    /// all, though they do reach `blocked_tree` and so still refuse
    /// placements. Entities this plan places itself go through
    /// [`PlanState::create_entity`] and are visible immediately.
    ///
    /// `electric-pole` and `generator` **used** to be missing from that
    /// whitelist too, which made [`PlanState::electric_supply_kw`] score every
    /// hand-built power plant 0 kW. They were added on 2026-09-02; this doc
    /// still said otherwise on 2026-09-04 and sent a diagnosis of exactly that
    /// failure looking in the wrong crate. The real cause was
    /// [`EntityGraph::find_entities_in_radius`]'s radius filter, which was
    /// Manhattan while the `self.added` loop below is Euclidean — so one
    /// function returned a disc for the entities this plan had placed and an
    /// L1 diamond for the ones the world already had, and a pole 196 tiles
    /// away on the diagonal fell outside the diamond of radius 256. Both
    /// halves are Euclidean now.
    ///
    /// [`EntityGraph::find_entities_in_radius`]: factorio_bot_core::graph::entity_graph::EntityGraph::find_entities_in_radius
    pub fn entities_within(&self, centre: &Position, radius: f64) -> Vec<FactorioEntity> {
        let mut out: Vec<FactorioEntity> = Vec::new();
        let mut seen: BTreeSet<Pos> = BTreeSet::new();
        for entity in self.added.values() {
            if calculate_distance(&entity.position, centre) <= radius {
                seen.insert(Pos::from(&entity.position));
                out.push(entity.clone());
            }
        }
        for entity in
            self.base
                .entity_graph
                .find_entities_in_radius(centre.clone(), radius, None, None)
        {
            let key = Pos::from(&entity.position);
            if self.removed.contains(&key) || !seen.insert(key) {
                continue;
            }
            out.push(entity);
        }
        out.sort_by(|a, b| {
            a.position
                .x
                .total_cmp(&b.position.x)
                .then(a.position.y.total_cmp(&b.position.y))
                .then(a.name.cmp(&b.name))
        });
        out
    }

    /// Every entity of this name in the overlay and the base world.
    ///
    /// Used by block siting to run `already_stands` backwards
    /// (`method::blueprint::recover_anchor`): a block's own entities can
    /// stand anywhere the plan has building history, and the caller has no
    /// position to search around yet -- finding one IS the question this
    /// method answers, which is why it is not built on
    /// [`PlanState::entities_within`]: that one needs a centre, and there
    /// isn't one yet.
    ///
    /// The base half reads `EntityGraph`'s own quad tree directly
    /// (`inner_tree().iter()`) rather than `find_entities_in_radius` with an
    /// invented "big enough" radius -- there is no radius that is honestly
    /// "the whole map" from this crate, which knows nothing of the quad
    /// tree's bounds and must not guess at them.
    ///
    /// Returns them in a deterministic order -- the planner is pure and an
    /// iteration order that varies would make a plan vary. Sorted by
    /// `Pos::from(&e.position)`, same as [`PlanState::entities_within`].
    pub fn entities_named(&self, name: &str) -> Vec<FactorioEntity> {
        let mut out: Vec<FactorioEntity> = Vec::new();
        let mut seen: BTreeSet<Pos> = BTreeSet::new();
        for entity in self.added.values() {
            if entity.name == name {
                seen.insert(Pos::from(&entity.position));
                out.push(entity.clone());
            }
        }
        let tree = self.base.entity_graph.inner_tree();
        for (entity, _rect) in tree.iter().map(|(_, v)| v) {
            if entity.name != name {
                continue;
            }
            let key = Pos::from(&entity.position);
            if self.removed.contains(&key) || !seen.insert(key) {
                continue;
            }
            out.push(entity.clone());
        }
        out.sort_by_key(|e| Pos::from(&e.position));
        out
    }

    /// [`entities_named`](Self::entities_named) for many names at once, in
    /// ONE traversal of the overlay and ONE of the base world's quad tree --
    /// bucketed by name, so a caller that used to ask once per name (or,
    /// worse, once per blueprint ENTITY, as `method::blueprint::recover_anchor`
    /// did) asks once total instead.
    ///
    /// A 179-entity, 7-distinct-name blueprint drove `recover_anchor` to 179
    /// whole-world scans -- once per entity, not even once per distinct name
    /// -- which is what made a fixed-anchor `Site::At` a thousand tiles from
    /// anything cost ~95s: the scan is over `inner_tree()`, the WHOLE base
    /// world, regardless of how much of it is relevant. This is the fix: one
    /// pass, testing membership in `names` (a `BTreeSet`, `O(log n)` per
    /// entity) rather than equality with one string, and every distinct name
    /// falls out of the same pass.
    ///
    /// Same dedup and ordering contract as `entities_named`, applied
    /// per-name: an overlay entity wins over a base entity at the same tile,
    /// `removed` hides a base entity outright, and each name's bucket is
    /// sorted by `Pos`. A name absent from the result had nothing standing
    /// under it anywhere -- the map holds no empty buckets, so
    /// `by_name.is_empty()` after calling this is exactly "none of these
    /// names has anything built yet".
    pub fn entities_named_any(
        &self,
        names: &BTreeSet<String>,
    ) -> BTreeMap<String, Vec<FactorioEntity>> {
        let mut out: BTreeMap<String, Vec<FactorioEntity>> = BTreeMap::new();
        let mut seen: BTreeMap<String, BTreeSet<Pos>> = BTreeMap::new();
        for entity in self.added.values() {
            if names.contains(&entity.name) {
                seen.entry(entity.name.clone())
                    .or_default()
                    .insert(Pos::from(&entity.position));
                out.entry(entity.name.clone())
                    .or_default()
                    .push(entity.clone());
            }
        }
        let tree = self.base.entity_graph.inner_tree();
        for (entity, _rect) in tree.iter().map(|(_, v)| v) {
            if !names.contains(&entity.name) {
                continue;
            }
            let key = Pos::from(&entity.position);
            if self.removed.contains(&key) {
                continue;
            }
            if !seen.entry(entity.name.clone()).or_default().insert(key) {
                continue;
            }
            out.entry(entity.name.clone())
                .or_default()
                .push(entity.clone());
        }
        for bucket in out.values_mut() {
            bucket.sort_by_key(|e| Pos::from(&e.position));
        }
        out
    }

    /// [`entities_named_any`](Self::entities_named_any)'s counterpart for
    /// GHOSTS, bucketed by [`FactorioEntity::ghost_name`] rather than
    /// [`FactorioEntity::name`].
    ///
    /// **A ghost's own `name` is always the literal `"entity-ghost"`, never
    /// the entity it will become** -- verified live against Factorio 2.1.17:
    /// a ghost reports `name = "entity-ghost"` with the real name in
    /// `ghost_name`, a separation load-bearing enough that
    /// `method::blueprint::already_stands` depends on it to avoid ever
    /// reading a ghost as a built entity. Calling
    /// [`entities_named_any`](Self::entities_named_any) with real entity
    /// names (`"stone-furnace"`, say) therefore finds no ghost, ever, no
    /// matter how many stand -- this is the method that looks at the field
    /// that actually carries the design intent.
    ///
    /// Same one-pass-over-many-names shape as its sibling, and the same
    /// dedup/order contract: an overlay ghost wins over a base one at the
    /// same tile, `removed` hides a base one outright, and each name's
    /// bucket is sorted by `Pos`.
    pub fn ghosts_named_any(
        &self,
        names: &BTreeSet<String>,
    ) -> BTreeMap<String, Vec<FactorioEntity>> {
        let mut out: BTreeMap<String, Vec<FactorioEntity>> = BTreeMap::new();
        let mut seen: BTreeMap<String, BTreeSet<Pos>> = BTreeMap::new();
        for entity in self.added.values() {
            if entity.name != GHOST_ENTITY_NAME {
                continue;
            }
            let Some(ghost_name) = entity.ghost_name.as_ref() else {
                continue;
            };
            if !names.contains(ghost_name) {
                continue;
            }
            seen.entry(ghost_name.clone())
                .or_default()
                .insert(Pos::from(&entity.position));
            out.entry(ghost_name.clone())
                .or_default()
                .push(entity.clone());
        }
        let tree = self.base.entity_graph.inner_tree();
        for (entity, _rect) in tree.iter().map(|(_, v)| v) {
            if entity.name != GHOST_ENTITY_NAME {
                continue;
            }
            let Some(ghost_name) = entity.ghost_name.as_ref() else {
                continue;
            };
            if !names.contains(ghost_name) {
                continue;
            }
            let key = Pos::from(&entity.position);
            if self.removed.contains(&key) {
                continue;
            }
            if !seen.entry(ghost_name.clone()).or_default().insert(key) {
                continue;
            }
            out.entry(ghost_name.clone())
                .or_default()
                .push(entity.clone());
        }
        for bucket in out.values_mut() {
            bucket.sort_by_key(|e| Pos::from(&e.position));
        }
        out
    }

    /// The nearest pole to `from`, within `radius`, whose own supply area has
    /// at least `kw` of generation **left uncommitted** — i.e. somewhere a
    /// consumer could be built and actually run.
    ///
    /// The anchor a method searches around when it needs a *powered* site.
    /// Searching around the bot instead would only ever find power the bot
    /// happens to be standing in, and `free_area_near`'s rings reach 12 tiles.
    ///
    /// Ordered by `(distance, x, y)` with `total_cmp`, so two poles equally far
    /// away resolve the same way on every run.
    ///
    /// Subject to exactly the blindness [`PlanState::entities_within`]
    /// describes: a pole a live world already contains is not visible here.
    pub fn nearest_supply_anchor(&self, from: &Position, radius: f64, kw: f64) -> Option<Position> {
        let mut candidates: Vec<(f64, Position, Rect)> = self
            .entities_within(from, radius)
            .into_iter()
            .filter_map(|entity| {
                let supply = pole_supply_half_extent(&self.base.entity_prototypes, &entity.name)?;
                let box_ = Rect::new(
                    &Position::new(entity.position.x() - supply, entity.position.y() - supply),
                    &Position::new(entity.position.x() + supply, entity.position.y() + supply),
                );
                Some((
                    calculate_distance(&entity.position, from),
                    entity.position.clone(),
                    box_,
                ))
            })
            .collect();
        candidates.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.x.total_cmp(&b.1.x))
                .then(a.1.y.total_cmp(&b.1.y))
        });
        candidates
            .into_iter()
            // Headroom, not nameplate, and for the same reason
            // `Condition::Powered` uses it: a pole whose network is already
            // spoken for is not somewhere a consumer "could be built and
            // actually run". Nothing is excluded from the demand here because
            // the consumer this is siting does not exist yet.
            .find(|(_, _, box_)| {
                (self.electric_supply_kw(box_) - self.electric_demand_kw(box_, None))
                    .total_cmp(&kw)
                    .is_ge()
            })
            .map(|(_, position, _)| position)
    }

    /// Would a pole of `name` standing at `position` supply `area`?
    ///
    /// The coverage half of [`electric_supply_kw`](Self::electric_supply_kw),
    /// asked on its own, because siting a pole is a different question from
    /// reading one: `electric_supply_kw` answers "what reaches this ground",
    /// and a plant choosing where to put its pole needs "would this pole reach
    /// the engine". Sharing [`pole_supply_half_extent`] is the point — a
    /// second table of supply areas is exactly the drift this avoids.
    ///
    /// Overlap, not containment, because that is the game's rule: an entity is
    /// supplied when its bounding box meets the supply area.
    ///
    /// `false` for a pole this crate does not know the supply area of, which
    /// refuses rather than over-credits.
    pub fn pole_would_supply(&self, name: &str, position: &Position, area: &Rect) -> bool {
        let Some(supply) = pole_supply_half_extent(&self.base.entity_prototypes, name) else {
            return false;
        };
        let box_ = Rect::new(
            &Position::new(position.x() - supply, position.y() - supply),
            &Position::new(position.x() + supply, position.y() + supply),
        );
        boxes_overlap(&box_, area)
    }

    /// How much generation, in kW, is wired to whatever occupies `area`.
    ///
    /// The question `Condition::Powered` asks, and the reason it is asked at
    /// all: **power coverage is not power capacity.** A lab can sit inside a
    /// pole's supply area, be fully connected, and do nothing whatever,
    /// because nothing on that network generates. Run 30 read
    /// `generated_kw = 0.0` in all 541 of its force samples while the plan
    /// treated `Researched(automation)` as satisfied by crafting a lab.
    ///
    /// Three steps, in order, and each one is load-bearing:
    ///
    /// 1. **Coverage.** Which poles' supply areas overlap `area` at all.
    ///    Overlap, not containment, because that is the game's own rule: an
    ///    entity is supplied when its bounding box meets the supply area.
    /// 2. **Connectivity.** Which poles those poles reach, transitively, by
    ///    copper wire. Two poles are wired when they are within the *smaller*
    ///    of their two maximum wire distances, which is what the game does.
    /// 3. **Capacity.** The generators whose own footprint is covered by a
    ///    pole in that same component, summed. A generator on another network
    ///    contributes nothing, which is the failure `is_powered`-style
    ///    coverage checks miss.
    ///
    /// # What this deliberately does not count
    ///
    /// * **Solar panels**, still — but no longer because the number is
    ///   unknowable. [`Self::solar_average_kw`] derives a panel's daily
    ///   average from the surface's own curve, which is deterministic; what is
    ///   missing is any check that the accumulators to carry the night are
    ///   standing. Crediting an average without that check trades a false
    ///   refusal for a base that dies at midnight. See
    ///   [`DETERMINISTIC_GENERATOR_TYPES`].
    /// * **Accumulators**, for the same reason once removed: they store what
    ///   solar generated, and [`Self::accumulators_per_panel`] says how many
    ///   are needed rather than what a standing one is worth.
    /// * **Whether the generator is actually running.** A steam engine with no
    ///   steam produces nothing, and nothing in `FactorioSurface` says whether
    ///   it has any. This counts nameplate capacity, so a boiler that is out
    ///   of fuel reads as powered. Naming it here because it is the residual
    ///   this function does *not* close.
    /// * **Anything further than [`POWER_SEARCH_RADIUS`] from `area`.** A
    ///   bounded search, because the entity graph offers no "every entity"
    ///   query and an unbounded one would be a full scan on every condition
    ///   check. A power plant beyond that radius reads as absent.
    pub fn electric_supply_kw(&self, area: &Rect) -> f64 {
        let Some(net) = self.electric_network(area) else {
            return 0.;
        };
        // 3. Capacity on those components.
        let mut total = 0.;
        for entity in &net.nearby {
            let Some(kw) = generation_kw(&self.base.entity_prototypes, &entity.name) else {
                continue;
            };
            if net.carries(&self.footprint_of(entity)) {
                total += kw;
            }
        }
        total
    }

    /// **Uncommitted** capacity, in kW, on the network that reaches `area`:
    /// [`electric_supply_kw`](Self::electric_supply_kw) less
    /// [`electric_demand_kw_excluding`](Self::electric_demand_kw_excluding).
    ///
    /// The arithmetic `Condition::Powered` and `Condition::BlockPowered` are
    /// both made of, in one place, so the two cannot drift — and the number
    /// `method::power` reports when it refuses, so a refusal quotes the
    /// figure the refusal was actually decided by rather than a second
    /// computation of it.
    ///
    /// Negative is a real answer: a network can already be committed past its
    /// generation, and rounding that up to zero would hide it.
    pub fn electric_headroom_kw(&self, area: &Rect, except: Excluded<'_>) -> f64 {
        self.electric_supply_kw(area) - self.electric_demand_kw_excluding(area, except)
    }

    /// What one machine of `name` draws, in kW, as the demand ledger charges it.
    ///
    /// The same [`consumer_kw`] table [`electric_demand_kw`](Self::electric_demand_kw)
    /// sums over, exposed so a method stating a
    /// [`Condition::Powered`](crate::action::Condition::Powered) can state the
    /// number the budget will actually charge it rather than a second copy of
    /// it. A cell that budgeted 75 kW for a machine the ledger charges 150 for
    /// would pass its own check and brown out the network, which is the
    /// *coverage is not capacity* failure with the two halves swapped.
    ///
    /// `None` for a name the table does not carry, and callers must treat that
    /// as "not something this planner may put on a network" rather than as
    /// zero — that table is the one in this file whose unknown name errs
    /// towards permitting, and its own doc says so.
    pub fn consumer_draw_kw(&self, name: &str) -> Option<f64> {
        consumer_kw(&self.base.entity_prototypes, name)
    }

    /// What one generator of `name` contributes, in kW, as
    /// [`electric_supply_kw`](Self::electric_supply_kw) credits it.
    ///
    /// The [`generation_kw`] half of the pair
    /// [`consumer_draw_kw`](Self::consumer_draw_kw) exposes, and exposed for
    /// the same reason: a method that *sizes* generation against a demand has
    /// to size it in the units the supply ledger will credit. `power.rs`
    /// hardcoding a steam engine's 900 kW to decide how many engines a plant
    /// needs would be a second copy of this table, and a plant sized against
    /// the wrong copy passes its own arithmetic and browns out the network —
    /// the same failure `consumer_draw_kw` exists to prevent, with the halves
    /// swapped.
    ///
    /// `None` for a name the table does not carry. Unlike `consumer_kw`, that
    /// is the **safe** direction: an unknown generator makes nothing, so a
    /// caller sizing against it refuses rather than promising.
    pub fn generator_output_kw(&self, name: &str) -> Option<f64> {
        generation_kw(&self.base.entity_prototypes, name)
    }

    /// How far a pole of `name` can throw a wire, in tiles.
    ///
    /// The third of the pair [`consumer_draw_kw`](Self::consumer_draw_kw) and
    /// [`generator_output_kw`](Self::generator_output_kw) exposes, for exactly
    /// their reason: a method deciding whether a blueprint's poles are
    /// *connected* has to ask in the units [`pole_wire_reach`] answers in, and
    /// a second copy of that number elsewhere is a copy that can disagree.
    ///
    /// # It is a per-pole fact, and the pairwise rule is the trap
    ///
    /// `method::power`'s `POLE_WIRE_REACH_TILES` is **7.5 — a small pole's**
    /// reach, as its own doc says — and it was being applied to every pole in
    /// a blueprint. Medium is 9, substation 18, and `big-electric-pole` is the
    /// **32** that the table behind this read as 30 until 2026-09-07. So the
    /// universal is
    /// wrong for three of the four types, always in the direction that
    /// *under*-reaches: a block whose poles really are wired reports
    /// `disconnected_poles` and is refused for a distribution fault it does
    /// not have.
    ///
    /// **And the game's rule is the SMALLER of the two poles' distances**, so
    /// even a correct per-pole table has to be applied *pairwise* rather than
    /// per pole. An accessor that hands back one pole's reach is the input to
    /// that minimum, never the answer on its own.
    ///
    /// Latent rather than live only because every pole in every fixture today
    /// is a small pole (FurnaceLine 13, MinerLine 3, ElectricSmelter 3,
    /// StarterSteamEngineBoiler 2). It stops being latent the moment a real
    /// Factorio blueprint is imported, which routinely mixes pole types.
    ///
    /// `None` for a name this world does not describe as a pole —
    /// **unknown, never zero**. A caller must not read that as "cannot reach
    /// anything"; see [`pole_would_supply`](Self::pole_would_supply), whose
    /// gate on `entity_type` exists so an unsizable pole is not silently
    /// not-a-pole. A pole whose prototype declares a reach of **0** answers
    /// `Some(0.0)`, which is the game's own statement and is a different fact
    /// from silence — [`pole_wire_reach`] believes it rather than falling back.
    pub fn pole_wire_reach_tiles(&self, name: &str) -> Option<f64> {
        pole_wire_reach(&self.base.entity_prototypes, name)
    }

    /// What one solar panel of `name` contributes **averaged over a day**, in
    /// kW, on this world's surface.
    ///
    /// # Why this is not [`generator_output_kw`](Self::generator_output_kw)
    ///
    /// The supply ledger credits nameplate capacity, which for every
    /// deterministic generator is also what it makes at every hour. A solar
    /// panel's nameplate is its **noon** figure and it makes that for a
    /// fraction of the day, so nameplate and average are different numbers and
    /// the difference is the whole trap: a base sized on 60 kW per panel is
    /// dead every night, and this repo already records twice that an
    /// under-supplied network reads as completely dead rather than as slow.
    /// The owner's ruling is that solar is planned at average output.
    ///
    /// # Every term is derived, and none of them is here
    ///
    /// The two endpoints come from the panel's own prototype
    /// ([`FactorioEntityPrototype::solar_panel_performance_at_day`] and
    /// `..._at_night`) and everything else from
    /// [`factorio_bot_core::factorio::world::FactorioSurface::daylight`] — the four day-phase boundaries,
    /// `ticks_per_day` and `solar_power_multiplier`. There is no table here
    /// and no fallback, deliberately: `0.7` is a *result* of vanilla Nauvis
    /// arithmetic, and writing it down would be the mod-compatibility defect
    /// the standing rule names, in the one place where a mod is most likely to
    /// differ. See [`factorio_bot_core::types::SurfaceDaylight::average_solar_fraction`].
    ///
    /// # `None` means unknown, and it is why no baseline moved
    ///
    /// A world whose sender predates the daylight channel — which is every
    /// dump this project has archived — answers `None`, exactly as it did
    /// before this existed, and a caller must read that as "this planner may
    /// not put solar on a network" rather than as zero. That is the same
    /// refusal `electric_supply_kw` already makes; see its doc for why solar
    /// is still not *credited* there.
    pub fn solar_average_kw(&self, name: &str) -> Option<f64> {
        let daylight = self.base.daylight()?;
        let prototype = self.base.entity_prototypes.get(name)?;
        let noon_kw = prototype.max_energy_production_kw().filter(|kw| *kw > 0.)?;
        // The two endpoints gate this as much as they scale it: they carry
        // `subclasses: ["SolarPanel"]`, so their presence is what says the
        // prototype is a panel at all. A name that is not one answers `None`
        // here rather than being credited a curve it does not follow.
        let fraction = daylight.average_solar_fraction(
            prototype.solar_panel_performance_at_day?,
            prototype.solar_panel_performance_at_night?,
        )?;
        Some(noon_kw * fraction)
    }

    /// How many accumulators of `accumulator` one panel of `panel` needs for
    /// the array to carry its own average load through the night.
    ///
    /// # The derivation, and the number it lands on
    ///
    /// An array sized at [`solar_average_kw`](Self::solar_average_kw) carries
    /// a flat load equal to its average. It makes more than that around noon
    /// and less around midnight, and the *less* is what accumulators cover.
    /// [`factorio_bot_core::types::SurfaceDaylight::night_deficit_fraction`] integrates that shortfall
    /// exactly — the area between the flat load line and the curve wherever
    /// the curve is beneath it — and multiplying by the panel's own
    /// joules-per-tick and the surface's `ticks_per_day` turns it into joules.
    /// Dividing by the accumulator's
    /// [`FactorioEntityPrototype::electric_buffer_capacity`] gives this.
    ///
    /// On Nauvis it is **0.8467** — the owner's 25 panels to 21 accumulators
    /// (0.84) scaled by the day length the running game reports, 25,200 ticks
    /// against the 25,000 every reference repeats. Reached without either
    /// number being written down, and the 0.8% gap is fully explained rather
    /// than tuned away.
    ///
    /// # 0.84 is the accumulator ratio and 0.7 is the average — they are not
    /// the same statement
    ///
    /// Worth saying because the two are easy to conflate, and conflating them
    /// would size an array 20% short. `25:21` says nothing about average
    /// output; it says how much *storage* a day's shortfall needs. The average
    /// is 0.7 of nameplate, from the same curve, by a different integral. Both
    /// fall out of this one channel, which is the corroboration: two
    /// independently-known vanilla ratios from one derivation.
    ///
    /// # What it is not
    ///
    /// Not a discharge-rate check. An accumulator's
    /// `max_energy_production` is its 300 kW discharge *limit*, and a bank
    /// with enough joules can still be unable to deliver them fast enough.
    /// This sizes energy only; the rate question is a separate one and is not
    /// answered anywhere yet.
    ///
    /// `None` on a world that has not reported daylight, or for a `panel`
    /// that is not a solar panel or an `accumulator` with no buffer — unknown,
    /// never zero.
    pub fn accumulators_per_panel(&self, panel: &str, accumulator: &str) -> Option<f64> {
        let daylight = self.base.daylight()?;
        let ticks_per_day = f64::from(daylight.ticks_per_day?);
        let panel = self.base.entity_prototypes.get(panel)?;
        let deficit_fraction = daylight.night_deficit_fraction(
            panel.solar_panel_performance_at_day?,
            panel.solar_panel_performance_at_night?,
        )?;
        // `max_energy_production` is joules per tick, so a day of it at full
        // output is that times the day's ticks, and the deficit is a fraction
        // of exactly that.
        let deficit_joules =
            panel.max_energy_production.filter(|j| *j > 0.)? * ticks_per_day * deficit_fraction;
        let buffer_joules = self
            .base
            .entity_prototypes
            .get(accumulator)?
            .electric_buffer_capacity
            .filter(|j| *j > 0.)?;
        Some(deficit_joules / buffer_joules)
    }

    /// How much draw, in kW, is already committed on the network that reaches
    /// `area` — the *other* half of the question
    /// [`electric_supply_kw`](Self::electric_supply_kw) answers.
    ///
    /// **`Condition::Powered` is a per-consumer test without this, and that is
    /// not the same as a network budget.** Each consumer asks "does 900 kW
    /// reach this ground" and each is told yes, so twelve assembling machines,
    /// four drills, a lab and thirty inserters — 1,560 kW — all pass
    /// individually on one 900 kW engine. For a single lab nobody could have
    /// hit it; for a factory it is *coverage is not capacity* one level up,
    /// and it fails quietly: Factorio degrades an under-supplied network
    /// proportionally, so everything runs at 58 % and every lag edge in the
    /// plan is wrong by 1.7× with no error anywhere.
    ///
    /// `except` names a consumer **not** to charge — the one being asked
    /// about. Excluding it is not an optimisation: a machine this plan has
    /// already placed would otherwise be counted against its own budget, so
    /// the second check of an identical plan would refuse what the first
    /// accepted. A non-idempotent predicate is an oscillation in a supervisor
    /// loop. The match is by tile, through [`Pos`], because that is how
    /// [`create_entity`](Self::create_entity) keys the overlay and two
    /// consumers cannot stand on one tile anyway.
    ///
    /// **One position is not enough for a caller siting a whole block**, whose
    /// own consumers are all in the overlay and all already counted in the
    /// `kw` it asks for. [`electric_demand_kw_excluding`](Self::electric_demand_kw_excluding)
    /// takes an [`Excluded`] instead; this is its one-consumer wrapper.
    ///
    /// The walk is [`electric_supply_kw`](Self::electric_supply_kw)'s own —
    /// literally the same [`ElectricNetwork`], built by the same three steps —
    /// so there is one notion of "the same network" and not two. A budget
    /// subtracted from a supply computed over a *different* network would be
    /// worse than no budget at all.
    ///
    /// # A crafting machine with no recipe draws nothing
    ///
    /// An assembling machine's `energy_usage` is what it draws **while
    /// crafting**; with no recipe set it can never craft, and what it draws
    /// is its drain -- 2.5 kW for an `assembling-machine-1`, a thirtieth of
    /// the figure in [`consumer_kw`]. So a machine whose `recipe` is `None`
    /// is not charged here.
    ///
    /// This matters because a replan is what sets recipes. A plan cut short
    /// after its machines went down and before its `set recipe` leaves dead
    /// machines on the network, and charging each of them 75 kW is how
    /// `run-1788608648-56109`'s fourth plan came to read 145 kW of headroom
    /// on a 900 kW engine with two labs and *no running machine at all* on
    /// it -- five recipe-less machines from two abandoned cells were 375 of
    /// the missing 755 -- and sited a third power plant for a cell wanting
    /// 189. The plan that finishes those machines sets their recipes in its
    /// overlay as it does so, and from that moment they are charged.
    ///
    /// Not a 2.5 kW drain entry instead of zero: the drain is not in
    /// [`consumer_kw`], and a second per-machine table for a thirtieth of the
    /// first is the drift that table's own doc warns against. The 2.5 kW is
    /// inside the slack nameplate accounting already has.
    ///
    /// # What it cannot see
    ///
    /// A consumer [`consumer_kw`] does not name draws nothing here, which
    /// over-states headroom. That is the one table in this file whose unknown
    /// name errs towards permitting rather than refusing, and its own doc
    /// comment says so.
    pub fn electric_demand_kw(&self, area: &Rect, except: Option<&Position>) -> f64 {
        self.electric_demand_kw_excluding(
            area,
            except.map_or(Excluded::Nothing, Excluded::Consumer),
        )
    }

    /// [`electric_demand_kw`](Self::electric_demand_kw) with the general
    /// exclusion: the same walk, the same ledger, one notion of "the same
    /// network", and [`Excluded`] instead of a single tile.
    ///
    /// The one-consumer form is the wrapper above rather than a second copy,
    /// because two demand walks that could disagree would be worse than no
    /// budget at all — the argument this function's own doc already makes
    /// about supply and demand walking different networks, one level down.
    pub fn electric_demand_kw_excluding(&self, area: &Rect, except: Excluded<'_>) -> f64 {
        let Some(net) = self.electric_network(area) else {
            return 0.;
        };
        let mut total = 0.;
        for entity in &net.nearby {
            if except.covers(&entity.position) {
                continue;
            }
            let Some(kw) = consumer_kw(&self.base.entity_prototypes, &entity.name) else {
                continue;
            };
            if takes_a_recipe(&entity.name) && entity.recipe.is_none() {
                continue;
            }
            if net.carries(&self.footprint_of(entity)) {
                total += kw;
            }
        }
        total
    }

    /// The poles and generators through which `area` reads its supply: every
    /// pole of a wired component whose supply area meets `area`, and every
    /// generator one of those poles covers. Name and position of each, in
    /// the fixed order [`entities_within`](Self::entities_within) returns.
    ///
    /// **What a `Condition::Powered` is made of, so a method can order an
    /// action after the placements that make it true.** `Powered` is a
    /// statement about the state and no `Effect` satisfies it, so
    /// `ActionNetwork::infer_edges` draws no edge to it; the scheduler
    /// checks it against a state that holds every placement already
    /// *chosen*, whatever tick that placement was given. Measured on
    /// `run-1788617269-96746` (eight character bots, 5x): the research was
    /// scheduled at 42,905 and the one pole joining the labs' poles to the
    /// steam engine, `[38.5, -7.5]`, at 49,411 — the pole had been chosen
    /// earlier, on a bot that was busy, so `Powered` held in the sim and
    /// nothing said the research had to wait for it. The labs stood
    /// `no_power` with all 85 packs inside for 12,300 ticks, and the two
    /// researches ran 13,000 ticks over their planned durations. A method
    /// that turns this list into `Condition::EntityAt` preconditions gets
    /// the edges by inference, from exactly the placements that create
    /// these entities, and none for entities the world already carries.
    ///
    /// Empty when nothing reaches `area`, which is `Powered`'s "no".
    pub fn powering_entities(&self, area: &Rect) -> Vec<(Position, String)> {
        let Some(net) = self.electric_network(area) else {
            return Vec::new();
        };
        let mut out: Vec<(Position, String)> = Vec::new();
        // `net.poles` and `net.parent` are indexed in the order the poles
        // were filtered out of `net.nearby`, so the same filter, applied in
        // the same order, recovers each pole's index.
        let mut pole_index = 0;
        for entity in &net.nearby {
            let is_pole = pole_supply_half_extent(&self.base.entity_prototypes, &entity.name)
                .is_some()
                && pole_wire_reach(&self.base.entity_prototypes, &entity.name).is_some();
            if is_pole {
                if net.supplying.contains(&net.root(pole_index)) {
                    out.push((entity.position.clone(), entity.name.clone()));
                }
                pole_index += 1;
                continue;
            }
            if generation_kw(&self.base.entity_prototypes, &entity.name).is_some()
                && net.carries(&self.footprint_of(entity))
            {
                out.push((entity.position.clone(), entity.name.clone()));
            }
        }
        out
    }

    /// Every entity that could be on `centre`'s electric network, found by
    /// **following the wire** rather than by drawing a bigger circle.
    ///
    /// Seeds with one [`POWER_SEARCH_RADIUS`] disc around `centre`, then
    /// repeatedly expands from each pole it has found by that pole's own wire
    /// reach, until a pass adds no new pole. A generator at the far end of a
    /// ten-pole run is therefore found, and one standing alone beyond the last
    /// pole is not — which is exactly the physical rule.
    ///
    /// # There is no hop limit, deliberately
    ///
    /// A cap was suggested, with the sound reasoning that if the search ever
    /// stopped on its own bound that should be *visible* rather than looking
    /// like "no supply" — otherwise a constant-shaped false refusal is merely
    /// replaced by a traversal-shaped one, which is harder to spot.
    ///
    /// The better answer to "make the bound observable" is to have no bound.
    /// **Termination is guaranteed by finiteness, not by a cap**: every pass
    /// expands only from poles not expanded before, the world holds finitely
    /// many, so the frontier empties. Nothing here can stop early, so there is
    /// no early stop to report, and no constant that some future map makes
    /// wrong.
    ///
    /// The cost is bounded by **the size of the connected pole network**, which
    /// is the thing that actually carries the power, rather than by a guess
    /// about map scale. The common case — a lab beside its plant, no pole
    /// chain — finds no pole to expand from beyond the first disc and
    /// terminates in one pass, so putting this on every condition check costs
    /// what the old single disc cost.
    ///
    /// Order is deterministic: entities are accumulated into a [`BTreeMap`]
    /// keyed by tile, so the result does not depend on the order poles were
    /// discovered in. This crate's determinism rule is not negotiable, and a
    /// traversal is exactly where insertion order would leak in.
    fn electric_entities(&self, centre: &Position) -> Vec<FactorioEntity> {
        let mut found: BTreeMap<Pos, FactorioEntity> = BTreeMap::new();
        let mut expanded: BTreeSet<Pos> = BTreeSet::new();

        for entity in self.entities_within(centre, POWER_SEARCH_RADIUS) {
            found.insert(Pos::from(&entity.position), entity);
        }

        loop {
            // The poles found so far that have not yet been expanded from.
            // Collected before expanding so the borrow ends, and sorted by
            // tile because `found` is a BTreeMap.
            let frontier: Vec<(Pos, Position, f64)> = found
                .iter()
                .filter_map(|(key, entity)| {
                    if expanded.contains(key) {
                        return None;
                    }
                    let reach = pole_wire_reach(&self.base.entity_prototypes, &entity.name)?;
                    Some((key.clone(), entity.position.clone(), reach))
                })
                .collect();
            if frontier.is_empty() {
                return found.into_values().collect();
            }
            for (key, position, reach) in frontier {
                expanded.insert(key);
                for entity in self.entities_within(&position, reach) {
                    found.entry(Pos::from(&entity.position)).or_insert(entity);
                }
            }
        }
    }

    /// Steps 1 and 2 of [`electric_supply_kw`](Self::electric_supply_kw):
    /// which poles are near `area`, which of them are wired together, and
    /// which of those components reach `area` at all.
    ///
    /// `None` when no pole reaches the ground, which is both callers' zero.
    fn electric_network(&self, area: &Rect) -> Option<ElectricNetwork> {
        let centre = Position::new(
            (area.left_top.x() + area.right_bottom.x()) / 2.,
            (area.left_top.y() + area.right_bottom.y()) / 2.,
        );
        let nearby = self.electric_entities(&centre);

        // 1. Poles, with the supply box and wire reach the vanilla prototypes
        //    give them.
        let poles: Vec<(Position, Rect, f64)> = nearby
            .iter()
            .filter_map(|entity| {
                let supply = pole_supply_half_extent(&self.base.entity_prototypes, &entity.name)?;
                let wire = pole_wire_reach(&self.base.entity_prototypes, &entity.name)?;
                let box_ = Rect::new(
                    &Position::new(entity.position.x() - supply, entity.position.y() - supply),
                    &Position::new(entity.position.x() + supply, entity.position.y() + supply),
                );
                Some((entity.position.clone(), box_, wire))
            })
            .collect();
        if poles.is_empty() {
            return None;
        }

        // 2. Connectivity, as a union-find over pole indices. `poles` is
        //    already in the deterministic order `entities_within` fixed, so
        //    the components come out the same on every run.
        let mut parent: Vec<usize> = (0..poles.len()).collect();
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for a in 0..poles.len() {
            for b in (a + 1)..poles.len() {
                let reach = poles[a].2.min(poles[b].2);
                if calculate_distance(&poles[a].0, &poles[b].0) <= reach {
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                    if ra != rb {
                        parent[ra] = rb;
                    }
                }
            }
        }

        // The components that reach `area` at all. A `BTreeSet` of roots, so
        // two poles of one network are one entry however many cover the site.
        let covering: Vec<usize> = poles
            .iter()
            .enumerate()
            .filter(|(_, (_, supply, _))| boxes_overlap(supply, area))
            .map(|(index, _)| index)
            .collect();
        let mut supplying: BTreeSet<usize> = BTreeSet::new();
        for index in covering {
            let root = find(&mut parent, index);
            supplying.insert(root);
        }
        if supplying.is_empty() {
            return None;
        }
        Some(ElectricNetwork {
            nearby,
            poles: poles.into_iter().map(|(_, supply, _)| supply).collect(),
            parent,
            supplying,
        })
    }

    /// Records a placed entity, giving it the footprint its prototype says it
    /// has if it arrived without one.
    ///
    /// The fill-in happens here, once, rather than at each of the places that
    /// ask about occupancy: methods build a `FactorioEntity` from a name and a
    /// position and leave `bounding_box` at its zero default, and a zero box
    /// collides with nothing.
    pub fn create_entity(&mut self, mut entity: FactorioEntity) {
        let key = Pos::from(&entity.position);
        if (entity.bounding_box.width() == 0. || entity.bounding_box.height() == 0.)
            && let Some(area) = Direction::from_u8(entity.direction).and_then(|facing| {
                self.collision_area_facing(&entity.name, &entity.position, facing)
            })
        {
            entity.bounding_box = area;
        }
        self.removed.remove(&key);
        self.added.insert(key, entity);
    }

    /// Put `recipe` on the crafting machine standing at `position`.
    ///
    /// The overlay half of [`crate::action::Effect::SetRecipe`]. It reads the
    /// machine through [`Self::entity_at`], so a machine this plan placed and
    /// one the world already had are treated the same, and writes the modified
    /// copy back under the tile that was asked about -- the same key
    /// [`Self::create_entity`] and [`Self::remove_entity`] use, and the same
    /// one `entity_at` will read it back from.
    ///
    /// **Assignment, not accumulation.** A machine has exactly one recipe, so
    /// setting the same one twice lands on the same state and setting a
    /// different one replaces rather than adds. That is what makes the
    /// matching `ActionKind::SetRecipe` the one dispatch in this planner that
    /// is safe to re-run under `recover.rs`'s tier 1.
    ///
    /// # Empty ground is an error, not a no-op
    ///
    /// Nothing here checks whether the acting force may *use* the recipe:
    /// that is [`crate::method::util::recipe_gate`]'s question and it is about
    /// the force, not about this tile. What is checked is that there is a
    /// machine to set it on at all, because a method that emits this without
    /// ordering it after the placement has written a plan whose later
    /// `Condition::RecipeSet` would otherwise pass against a machine nobody
    /// built.
    pub fn set_recipe(&mut self, position: &Position, recipe: &str) -> Result<(), PlannerError> {
        let Some(mut entity) = self.entity_at(position) else {
            return Err(PlannerError::NoMachineForRecipe {
                position: position.to_string(),
                recipe: recipe.to_string(),
            });
        };
        entity.recipe = Some(recipe.to_string());
        let key = Pos::from(position);
        self.removed.remove(&key);
        self.added.insert(key, entity);
        Ok(())
    }

    pub fn remove_entity(&mut self, position: &Position) {
        let key = Pos::from(position);
        self.added.remove(&key);
        self.removed.insert(key);
    }

    /// Ore remaining at a tile: the tile's capacity less what this plan has taken.
    ///
    /// Presence comes from `resource_contains`, not `entity_at`: `EntityGraph::add`
    /// routes resource entities into `resources`/`resource_tree` only, so they are
    /// absent from the entity tree that `entity_at` queries.
    pub fn resource_available(&self, position: &Position, item: &str) -> u32 {
        let key = Pos::from(position);
        if !self.base.entity_graph.resource_contains(item, key.clone()) {
            return 0;
        }
        // What the game said, and only if it said nothing,
        // `DEFAULT_RESOURCE_PER_TILE`. The substitution happens exactly here,
        // once, so every selector that reads this ledger -- and the four in
        // `method::util` all do, through `resource_unclaimed` -- sees the same
        // capacity for a tile. A second substitution site is how the seats
        // count and the tile walk would start disagreeing about how many bots
        // a patch can hold.
        let capacity = self
            .base
            .entity_graph
            .resource_amount(item, &key)
            .unwrap_or(DEFAULT_RESOURCE_PER_TILE);
        capacity.saturating_sub(self.consumed.get(&key).copied().unwrap_or(0))
    }

    /// Has this plan already committed `position` to a mining action?
    ///
    /// See the [`claimed`](PlanState#structfield.claimed) field for why a
    /// commitment is whole-tile rather than by amount.
    pub fn is_resource_claimed(&self, position: &Position) -> bool {
        self.claimed.contains_key(&Pos::from(position))
    }

    /// How far apart the tiles of two *different* mining actions must be.
    ///
    /// # Why exclusivity alone was not enough
    ///
    /// Whole-tile claims stop two bots being sent to the same ore. They say
    /// nothing about where a bot *stands* to mine, and a bot has to stand
    /// somewhere: run `run-1788313837-06402` spread four bots across four
    /// adjacent tiles and then lost six of thirteen mines to
    /// `could not start mining for 301 ticks: another character is standing on
    /// the iron-ore`. Before reservation the four raced for one tile and the
    /// losers failed cleanly; after it they were neatly spaced one tile apart
    /// and blocked each other physically.
    ///
    /// # The derivation, term by term
    ///
    /// * **`resource_reach_distance`** — the radius of the disc a miner may
    ///   rest in. This is not an estimate of where the walk lands: it is the
    ///   bound `FactorioRcon::player_mine_timed` *enforces*
    ///   (`within_resource_reach`, `crates/core/src/factorio/rcon.rs`), and
    ///   the same bound the mod re-checks before setting `mining_state`. A bot
    ///   further out than this does not mine at all, so every bot that does
    ///   mine tile `T` is somewhere in `disc(T, reach)`. The executor aims
    ///   *inside* it — at `reach` less the pathfinder's slack and the arrival
    ///   margin (`approach_aim`) — so this is the worst case rather than the
    ///   expected one, which is the direction a separation has to err in. The roster's largest plausible reach is
    ///   used; see [`MAX_PLAUSIBLE_RESOURCE_REACH`].
    /// * **`(0.5 + character_x).hypot(0.5 + character_y)`** — the furthest a
    ///   character's *centre* can be from a tile's centre while its collision
    ///   box still overlaps that tile. Two axis-aligned boxes overlap only
    ///   while both axis gaps are under the sum of their half-sides, so the
    ///   extreme is the corner-to-corner case, which is that hypotenuse.
    ///   [`TILE_HALF_SIDE`] is the tile's half-side; the character's comes
    ///   from the world's own `character` prototype (`character_half_box`).
    ///
    /// Their sum is the triangle inequality applied once: if
    /// `d(A, B) >= reach + overlap`, then a bot anywhere within `reach` of `B`
    /// is more than `overlap` from `A`, and so cannot be standing on `A`.
    /// With live numbers (2.7, ±0.19921875) that is **3.690** tiles.
    ///
    /// # What it deliberately is not
    ///
    /// It is not the mod's step-aside distance and does not try to be. The
    /// step-aside (`mine_step_aside_waypoint`) is the recovery for a bot that
    /// is *already* blocked; this is what stops the plan creating the block in
    /// the first place. Both are wanted — a re-plan cannot know where the
    /// previous plan's bots are still standing, so the step-aside remains the
    /// backstop across plans — but only one of them belongs in the planner.
    pub fn mining_tile_separation(&self) -> f64 {
        self.mining_tile_separation
    }

    /// Would a character standing at `stand` be standing on the resource tile
    /// whose ore sits at `tile`?
    ///
    /// The character's collision box against the tile's own square. Both are
    /// axis-aligned, so they share ground exactly while the gap on both axes
    /// is under the sum of their half-sides. `tile` is a tile *centre* — a
    /// real resource sits at `(-40.5, -48.5)` — so [`TILE_HALF_SIDE`] reaches
    /// its edges without any offset being restored first.
    ///
    /// This is the thing the mod reports as `another character is standing on
    /// the <ore>`, stated in geometry the planner can check, and
    /// [`PlanState::mining_tile_separation`] is derived from it.
    pub fn character_stands_on_tile(&self, stand: &Position, tile: &Position) -> bool {
        let (half_x, half_y) = character_half_box(&self.base);
        (stand.x() - tile.x()).abs() < TILE_HALF_SIDE + half_x
            && (stand.y() - tile.y()).abs() < TILE_HALF_SIDE + half_y
    }

    /// Is `position` too close to a tile some *other* mining action has
    /// already claimed?
    ///
    /// Distances are measured between tile *centres*, taken from the claim's
    /// stored `Position` rather than rebuilt from its flooring `Pos` key —
    /// see the [`claimed`](PlanState#structfield.claimed) field. Rebuilding
    /// would shift every claim by half a tile on each axis and make a
    /// separation that reads exact wrong by up to 0.71 tiles.
    ///
    /// The tile's own claim never counts: "claimed" and "crowded" are separate
    /// questions and a caller can ask either. [`PlanState::resource_unclaimed`]
    /// asks both.
    ///
    /// Asked on behalf of [`PlanState::claim_runner`] — the bot whose timeline
    /// the *next* claim would sit on. See
    /// [`PlanState::is_resource_crowded_for`], which this is the driver-facing
    /// spelling of.
    pub fn is_resource_crowded(&self, position: &Position) -> bool {
        self.is_resource_crowded_for(position, self.claim_runner)
    }

    /// [`PlanState::is_resource_crowded`], asked on behalf of a stated runner
    /// rather than the current one.
    ///
    /// A claim held by `runner` does not crowd `runner`: one bot runs one
    /// action at a time, so its own claims are disjoint in time and need no
    /// separation from each other. See [`MiningClaim`] for the whole argument
    /// and for why `None` on either side is never a match.
    ///
    /// Spelled out as a parameter as well as read off the state because the
    /// two questions are genuinely different and both are asked.
    /// [`crate::method::util::resource_seats`] counts how many *distinct* bots
    /// can mine an item at once and so must ask with `None`, whatever chain it
    /// happens to be called from; every selector that is choosing a tile for
    /// the chain in hand asks with that chain's runner. Passing it explicitly
    /// is what keeps a seat count from silently answering the tile-selection
    /// question.
    pub fn is_resource_crowded_for(
        &self,
        position: &Position,
        runner: Option<ClaimRunner>,
    ) -> bool {
        let key = Pos::from(position);
        self.claimed.iter().any(|(claimed_key, claim)| {
            *claimed_key != key
                && !same_runner(claim.runner, runner)
                && calculate_distance(position, &claim.centre) < self.mining_tile_separation
        })
    }

    /// Whose timeline a claim made now sits on. See
    /// [`claim_runner`](PlanState#structfield.claim_runner).
    pub fn claim_runner(&self) -> Option<ClaimRunner> {
        self.claim_runner
    }

    /// Bot-ticks this expansion has already committed `bot` to.
    ///
    /// The one *load* figure this crate can honestly report while a plan is
    /// still being built. A method choosing between bots wants "who is least
    /// busy", and there is no schedule yet to ask -- but every action carries
    /// its nominal duration, and the driver emits each one under a chain whose
    /// owner is known ([`ClaimRunner::Bot`]) or not. Summing the durations
    /// emitted under `bot`'s own chains counts the work this expansion has
    /// *already* handed that bot, in every verb it has.
    ///
    /// # Why every verb, and not the mining alone
    ///
    /// This used to be `planned_mining`: the raw units on the tiles
    /// `bot`'s claims had stamped, on the argument that mining was 65.6% of
    /// a rung-1 run's action time (`run-1788465258-49050`). Since `7e330a2c`
    /// a bot's stone and coal come off *rocks* -- one `Chop` of 360 ticks for
    /// 24 to 50 units -- and a swing stamps no tile, so a bot whose load was
    /// rock swings, furnace crafts and placements read as idle to the one
    /// method that asks (`method::have::furnace_suppliers`). Measured on the
    /// reference dump's `researched:automation`, in the rehearsal at the
    /// first smelt with a slot to hand away: bot 2 read 53 units on the old
    /// key while 2,670 ticks had been emitted under its chains -- 360 of
    /// them mining, 660 swings, 1,380 crafts.
    ///
    /// # What it deliberately does not count
    ///
    /// * An action in an **unowned** chain ([`ClaimRunner::Chain`]) or
    ///   outside a chain, because nothing yet says which bot will run it.
    ///   Counting it against a guess would make the load figure disagree with
    ///   the schedule, and the conservative reading -- "not this bot's, as far
    ///   as anyone can prove" -- is the same one crowding takes.
    /// * Walks and waits. The scheduler emits walks and finds waits; neither
    ///   exists during expansion, so no expansion-time ledger can hold them.
    /// * Machine time. A furnace's smelting is a lag edge, not any bot's
    ///   action.
    ///
    /// So this is a *ranking* key, not a cost model: it is used to break ties
    /// between bots that are otherwise interchangeable, never to price a plan.
    ///
    /// Deterministic by construction -- one `BTreeMap` read, integer ticks.
    pub fn planned_ticks(&self, bot: BotId) -> Ticks {
        self.planned_ticks.get(&bot).copied().unwrap_or(0)
    }

    /// Add `duration` to the ledger [`Self::planned_ticks`] reads, against the
    /// bot whose timeline the current chain provably sits on -- and against
    /// nobody when there is none, for the reason that method gives.
    ///
    /// Called by the driver for every `Step::Act` it simulates, so the ledger
    /// keeps pace with the network. Reads `claim_runner` rather than taking a
    /// bot, so it cannot disagree with the claims about whose work this is.
    pub fn note_planned_ticks(&mut self, duration: Ticks) {
        if let Some(ClaimRunner::Bot(bot)) = self.claim_runner {
            let entry = self.planned_ticks.entry(bot).or_insert(0);
            *entry = entry.saturating_add(duration);
        }
    }

    /// Has this plan already committed a batch of work to the machine at
    /// `position`? See
    /// [`committed_machines`](PlanState#structfield.committed_machines).
    ///
    /// Asked by *selection* — which standing furnace a smelt may adopt — and
    /// by nothing physical. `Condition::EntityAt` still reports what stands,
    /// because a commitment is a fact about this plan, not about the world the
    /// executor will meet, the same boundary
    /// [`PlanState::resource_unclaimed`] draws.
    pub fn machine_committed(&self, position: &Position) -> bool {
        self.committed_machines.contains(&Pos::from(position))
    }

    /// Is any machine this plan has committed to standing within `radius` of
    /// `position`?
    ///
    /// The tile-exact [`PlanState::machine_committed`] asks about adoption;
    /// this asks about *ground*, and exists because a commitment is made
    /// before the placement that realises it is emitted. `smelt_steps` sites a
    /// whole furnace bank into a fork, commits each slot, and only then emits
    /// the placements -- so between those two moments the tiles are spoken for
    /// and [`PlanState::is_area_free`] cannot see it. A method siting anything
    /// else in that window (a stockpile's chest is the case that found this)
    /// picks a tile the bank is about to take, and the collision surfaces at
    /// schedule time as an `AreaFree` precondition that "does not hold" for the
    /// bot that owns the chain -- a message that says nothing about the cause.
    ///
    /// Deterministic: an ordered walk of a `BTreeSet` and one distance test.
    pub fn machine_committed_near(&self, position: &Position, radius: f64) -> bool {
        self.committed_machines.iter().any(|tile| {
            calculate_distance(
                &Position::new(tile.0 as f64 + 0.5, tile.1 as f64 + 0.5),
                position,
            ) <= radius
        })
    }

    /// Commit the machine at `position` to this plan, so no later method
    /// adopts it as idle. Committing one twice is a no-op, not an error: a
    /// method that sites a bank re-commits its own members on a replan.
    pub fn commit_machine(&mut self, position: &Position) {
        self.committed_machines.insert(Pos::from(position));
    }

    /// What a later batch would have to wait for to use the machine at
    /// `position`, or `None` if nothing in this plan may queue behind it.
    ///
    /// `None` covers two different situations on purpose, because a caller
    /// treats them alike: a machine this plan never committed (ask
    /// [`PlanState::machine_committed`] to tell them apart) and one it
    /// committed without being able to prove the batch drains it. Both mean
    /// "do not queue here".
    pub fn machine_queue(&self, position: &Position) -> Option<&MachineQueue> {
        self.machine_queue.get(&Pos::from(position))
    }

    /// Record that the batch this plan just put into the machine at `position`
    /// smelts `item`, is finished and removed by `release`, and adds `ticks` of
    /// machine time to whatever was queued there already.
    ///
    /// Additive in `queued` and replacing in `release` and `item`: the release
    /// of the *newest* batch is the one a further batch has to wait for, while
    /// the machine time is the whole queue. Committing the machine is a
    /// separate call ([`PlanState::commit_machine`]) and must still be made —
    /// this map says a batch *may* queue, never that the machine is free.
    pub fn queue_machine(
        &mut self,
        position: &Position,
        item: &str,
        release: ActionId,
        taker: Option<BotId>,
        ticks: Ticks,
    ) {
        let key = Pos::from(position);
        let queued = self
            .machine_load
            .get(&key)
            .map_or(0, |q| *q)
            .saturating_add(ticks);
        self.machine_load.insert(key.clone(), queued);
        self.machine_queue.insert(
            key,
            MachineQueue {
                item: item.to_owned(),
                release,
                taker,
                queued,
            },
        );
    }

    /// Forget that anything may queue behind the machine at `position`.
    ///
    /// Called where a batch is emitted that cannot be proved to leave the
    /// machine empty. It does **not** un-commit the machine, and that asymmetry
    /// is the point: the plan is still using it, and now nothing else may.
    ///
    /// Nor does it forget the machine time already queued: the load a machine
    /// carries is a fact about every batch this plan put into it, and a smelt
    /// that unqueues a furnace while it emits and queues it again behind its
    /// own take must find the total where it left it. See `machine_load`.
    pub fn unqueue_machine(&mut self, position: &Position) {
        self.machine_queue.remove(&Pos::from(position));
    }

    /// Bind claims made from here on to `runner`'s timeline, and answer
    /// crowding questions for it. Returns the previous binding, which the
    /// driver restores on every exit path exactly as it restores
    /// `ExpansionCtx::chain`.
    pub fn set_claim_runner(&mut self, runner: Option<ClaimRunner>) -> Option<ClaimRunner> {
        std::mem::replace(&mut self.claim_runner, runner)
    }

    /// Ore at a tile that is still *available to plan against*: what
    /// [`PlanState::resource_available`] reports, or zero once the tile has
    /// been committed to a mining action, or covered by something else.
    ///
    /// This — not `resource_available` — is what tile *selection* must ask.
    /// The physical reading answers "will the ore be there when the bot
    /// swings", which is what `Condition::ResourceAvailable` needs and which a
    /// claim must not distort; this one answers "may I send another bot here",
    /// and the answer is no — whether because the tile itself is spoken for,
    /// because a bot mining a nearby claim will be standing on it, because a
    /// character is standing on it *now*
    /// ([`Self::resource_tile_occupied`]), or because something else occupies
    /// it ([`Self::resource_tile_blocked`]). All four, in the order they are
    /// cheap to test.
    ///
    /// Two of the four are asked for the *current* claim runner
    /// ([`PlanState::claim_runner`]): crowding, and — since 2026-09-06 —
    /// whether the tile's own claim yields ([`PlanState::claim_yields_to`],
    /// which relaxes only for the runner that made the claim, and only where
    /// the world stated the tile's amount). Occupied and built over stay
    /// runner-blind, because a tile somebody is standing on or has built over
    /// is that way for everybody. [`PlanState::resource_unclaimed_for`] is
    /// the same question asked on behalf of a stated runner.
    pub fn resource_unclaimed(&self, position: &Position, item: &str) -> u32 {
        self.resource_unclaimed_for(position, item, self.claim_runner)
    }

    /// [`PlanState::resource_unclaimed`], asked on behalf of a stated runner.
    ///
    /// `None` is the strictest answer — it treats every claim in the plan as a
    /// conflict — and is what [`crate::method::util::resource_seats`] asks
    /// with, because seats are about *distinct bots at once*.
    pub fn resource_unclaimed_for(
        &self,
        position: &Position,
        item: &str,
        runner: Option<ClaimRunner>,
    ) -> u32 {
        if self.is_resource_claimed(position) && !self.claim_yields_to(position, item, runner) {
            return 0;
        }
        if self.is_resource_crowded_for(position, runner) {
            return 0;
        }
        if self.resource_tile_occupied(position) {
            return 0;
        }
        if self.resource_tile_blocked(&Pos::from(position)) {
            return 0;
        }
        self.resource_available(position, item)
    }

    /// May the runner that already holds this tile's claim draw from it
    /// **again**?
    ///
    /// Two conditions, and both are needed:
    ///
    /// * **The claim is on the asking runner's own serial timeline.** That is
    ///   the argument [`MiningClaim`] already makes for crowding — a bot runs
    ///   one action at a time, an owned chain is offered to one bot and an
    ///   unowned one binds to a single bot the moment its first action is
    ///   placed — so two draws by the same runner are provably disjoint in
    ///   time and cannot collide on the ground. `None` on either side is
    ///   *unknown* and never matches, exactly as in
    ///   [`is_resource_crowded_for`](Self::is_resource_crowded_for).
    /// * **The world said how much this tile holds.** This is the whole of
    ///   what whole-tile exclusivity was ever protecting. Its stated reason
    ///   was that "the planner cannot know what a tile really holds" — with
    ///   [`DEFAULT_RESOURCE_PER_TILE`] standing in for a reading nobody took,
    ///   a second draw from the same tile spends a number that was invented.
    ///   Where `EntityGraph::resource_amount` carries the game's own
    ///   reading, that reason is simply absent: `consumed` subtracts what the
    ///   plan has taken (see [`Self::resource_available`]) and the tile
    ///   reports zero of its own accord once it is drained. A tile the world
    ///   said nothing about keeps the old rule and the old reason.
    ///
    /// # Why this is not a tuning knob
    ///
    /// A claim spends a whole tile, and a tile is up to hundreds of ore. On
    /// the seed-31337 t=0 dump the charted iron field is 940 tiles holding
    /// 522,467 ore; planning `have:pumpjack:1` for three bots claimed 324 of
    /// those tiles — still holding 134,734 ore between them — and the
    /// separation rule crowded the remaining 616 out, so a share of **6** ore
    /// found nothing and the whole plan was refused with
    /// `NoApplicableMethod`. Two bots planned the same goal on the same map.
    /// The cliff is in the arithmetic and not in the ground: shares are
    /// per-bot, so a roster of `n` burns `n` tiles per shortfall where a
    /// roster of two burns two, and a deep goal has hundreds of shortfalls.
    /// Adding a bot made a plan into a refusal, and four bots is the default
    /// roster.
    fn claim_yields_to(
        &self,
        position: &Position,
        item: &str,
        runner: Option<ClaimRunner>,
    ) -> bool {
        let key = Pos::from(position);
        let Some(claim) = self.claimed.get(&key) else {
            return false;
        };
        same_runner(claim.runner, runner)
            && self.base.entity_graph.resource_amount(item, &key).is_some()
    }

    /// Is a character standing on the resource tile whose ore sits at
    /// `position`, right now, in the world this plan was built from?
    ///
    /// The crowding rule ([`PlanState::is_resource_crowded`]) keeps two tiles
    /// of *one plan* far enough apart that neither bot can stand on the
    /// other's. It says nothing about a character that is already there and
    /// that this plan is not moving, because a claim is per-`PlanState` and
    /// every plan starts with an empty one. Run
    /// `workspace/runs/run-1788329146-40305` is what that costs: rungs 1-3
    /// left all four bots parked where their rung-2 copper mine had put them,
    /// rung 4 then scheduled all of its steps onto bot 1 and none onto bots 2,
    /// 3 and 4, and the tiles it sent bot 1 to were the tiles the other three
    /// were still standing on. Bots 2, 3 and 4 did not move once between tick
    /// 6000 and the end of the run (`samples.jsonl`); bot 4 sat at
    /// `(23.305, 53.77)` and bot 1 was sent to `(23.5, 53.5)`, bot 2 sat at
    /// `(22.203, 48.785)` and bot 1 was sent to `(22.5, 48.5)`. Both mines
    /// died on `could not start mining for 301 ticks: another character is
    /// standing on the copper-ore`, and the milestone re-planned the same
    /// tiles eight times before reporting `stuck`.
    ///
    /// # Every character, the eventual miner included
    ///
    /// A bot standing on the tile it is itself about to mine is fine — the mod
    /// only refuses when `player.selected` resolves to a character that is not
    /// the miner — so an exemption for "the assignee" would be the tighter
    /// rule if it could be stated. It cannot, for two independent reasons.
    ///
    /// * **Three of the four selectors have no bot to exempt.**
    ///   [`crate::method::util::resource_supply_at_least`] and
    ///   [`crate::method::util::resource_seats`] are reached from
    ///   `Method::applicable` and `Method::concurrency`, which are handed a
    ///   `&PlanState` and nothing else; `resource_seats` is what
    ///   `SplitAcrossBots` sizes a split from. An exemption available only to
    ///   [`crate::method::util::resource_tiles_for`] would make seats promise
    ///   bots that selection then refuses to place, which is precisely the
    ///   agreement this ledger exists to keep.
    /// * **`Mine::expand`'s `ctx.chain_actor` is not the assignee.** Methods
    ///   emit `Actor::Role` with `pinned: None`; the runner is decided by
    ///   `schedule`, and a chain opened because its method `converges` gets no
    ///   owner at all ("who runs it stays the scheduler's decision", see
    ///   `expand_goal_body`). A tile chosen while exempting the bot expansion
    ///   had in hand is a tile the scheduler may hand to a different bot,
    ///   which reproduces this exact failure in a narrower form. That is the
    ///   same argument the `characters` field records for placement, and the
    ///   same one `run-1788322836-81715` settled there.
    ///
    /// So the occupant's own presence is not an exception — and it does not
    /// need to be one. The cost is that a bot parked on ore is sent to the
    /// next tile of the same patch instead of the one under its feet: one step
    /// of `resource_tiles_for`'s already-sorted walk, on a patch of thousands.
    /// The cost of the other direction is the milestone. Only the conservative
    /// direction is self-correcting.
    ///
    /// The predicate is [`PlanState::character_stands_on_tile`] — the mod's
    /// `another character is standing on the <ore>` stated as geometry, and the
    /// same one [`PlanState::mining_tile_separation`] is derived from — so the
    /// two rules cannot disagree about what "standing on" means. `position` is
    /// a tile *centre*, which is what `EntityGraph::resource_patches` hands
    /// out, so no half-tile offset has to be restored first; the character's
    /// centre is recovered from the box
    /// [`characters`](PlanState#structfield.characters) stores, which was
    /// built symmetrically around it.
    fn resource_tile_occupied(&self, position: &Position) -> bool {
        self.characters
            .values()
            .any(|character| self.character_stands_on_tile(&character.center(), position))
    }

    /// Does anything the game would select *instead of the ore* stand over the
    /// resource tile `tile`?
    ///
    /// Two sources, and neither of them sees the other's.
    ///
    /// * **Entities this plan has placed.** Nothing ever placed anything on
    ///   ore during a gathering goal until `11fabe43` added `PlaceDrill`, and
    ///   `is_area_clear_of` lets anything stand on a patch, because the game
    ///   does (see [`PlanState::occupant_of`]) — so the plan acquired the
    ///   ability to bury the ore it was about to mine, and this ledger did not
    ///   learn about it. That makes this ledger, not a placement rule, the
    ///   thing standing between a plan and its own ore. Run `run-1788455754-92581` is what
    ///   that cost: one plan carried `place burner-mining-drill at [-34, -23]`
    ///   and `mine 4 iron-ore` at `(-33.5, -22.5)`, a tile inside that drill's
    ///   own footprint, and the mine died on `expected iron-ore at
    ///   (-33.5/-22.5), found burner-mining-drill`. Both came out of a single
    ///   `PlaceDrill` expansion — the cell is written into the state before
    ///   its own bill is expanded, so the ore that paid for the drill was
    ///   selected from under it.
    /// * **Blocking boxes** — debris, a tree, a rock, water, *and every
    ///   machine the base world already holds*: the counterpart
    ///   of the fourth source in [`PlanState::is_area_clear`], aimed at the
    ///   opposite mistake. There, the base world can hold an obstacle that
    ///   reads as open ground because `entity_tree` never sees it; here, the
    ///   base world can hold *ore* that reads as minable when the entity the
    ///   game will actually select at that position is something else.
    ///   `EntityGraph::add` routes crash-site wreckage — a `simple-entity`,
    ///   exactly like a small rock — into `blocked_tree` only, never
    ///   `entity_tree` (see that function's whitelist), and the mod's
    ///   `player.update_selected_entity` does not filter by name: whichever
    ///   entity is selectable at the position wins. When that is the wreck
    ///   instead of the ore, mining stalls with `expected coal ..., found
    ///   crash-site-spaceship-wreck-...` until the mod's own timeout fires
    ///   (`workspace/runs/run-1788317597-64759`, rung 4).
    ///
    ///   This source is wider than its name suggests, and it is what keeps
    ///   the *previous* plan's drill out of selection: `EntityGraph::add`
    ///   files every entity with a non-zero collision box into `blocked_tree`
    ///   except resources and rails, machines included, so a cell that really
    ///   got built is out of selection for the next milestone -- **for as
    ///   long as the graph keeps the box.** It did not always: until
    ///   `EntityGraph::remove` learned which box is an entity's own, a drill
    ///   emptying one of the tiles under itself made the mod report that
    ///   tile deleted, and the removal swept the drill's box out with the
    ///   ore's. `run-1788559688-08406` lost six of its seven plan-1 drills
    ///   from this tree that way and plan 2 hand-mined under two of them.
    ///   The rule stays here and reads the graph rather than growing a third
    ///   source over `entity_tree`: the graph is the one place that knows
    ///   both what stands and where, and a second reading here would only
    ///   have hidden the graph's forgetting. Both halves have tests.
    ///
    /// The ore itself is not gone — `add` never removes a resource entity
    /// because something else was placed over it, and this does not touch
    /// [`PlanState::resource_available`], which still reports the tile's full
    /// physical count. Only *new selection* is refused: a tile this returns
    /// `true` for must be skipped in favour of the next one, not reported as
    /// exhausted.
    ///
    /// The mirror of this — a *placement* refusing ground some mining action
    /// has already spoken for — is [`PlanState::covers_claimed_resource`].
    /// Both are needed because expansion order decides which of the two
    /// commitments is made first, and nothing constrains that order.
    fn resource_tile_blocked(&self, tile: &Pos) -> bool {
        let area = tile_area(tile);
        if self
            .added
            .values()
            .any(|entity| boxes_overlap(&self.footprint_of(entity), &area))
        {
            return true;
        }
        self.base
            .entity_graph
            .blocking_boxes_within(&area)
            .into_iter()
            .filter(|blocked| !self.removed.contains(&Pos::from(&blocked.center())))
            .any(|blocked| boxes_overlap(&blocked, &area))
    }

    /// Commit `position` to a mining action without taking anything from it.
    ///
    /// Separate from [`PlanState::consume_resource`] so a caller can say which
    /// of the two it means; `consume_resource` calls this, because taking ore
    /// out of a tile in a plan is also the plan committing to that tile.
    ///
    /// The claim is stamped with [`PlanState::claim_runner`] — whose timeline
    /// it sits on — which is why the driver has to have bound that before the
    /// effect is applied. See [`MiningClaim`].
    pub fn claim_resource(&mut self, position: &Position) {
        self.claimed.insert(
            Pos::from(position),
            MiningClaim {
                centre: position.clone(),
                runner: self.claim_runner,
            },
        );
    }

    /// Take `count` of `item` out of a tile, and commit the tile to the action
    /// that took it.
    ///
    /// The bound checked is the *physical* one, so a tile can still be drawn
    /// from twice by anything that deliberately does so (the schedule's replay
    /// re-applies each effect once, and the executor's recovery re-plans from
    /// a fresh state). Selection is what claims exclude, via
    /// [`PlanState::resource_unclaimed`].
    pub fn consume_resource(
        &mut self,
        position: &Position,
        item: &str,
        count: u32,
    ) -> Result<(), PlannerError> {
        let available = self.resource_available(position, item);
        if available < count {
            return Err(PlannerError::InsufficientItems {
                bot: BotId(0),
                item: item.to_string(),
                required: count,
                available,
            });
        }
        *self.consumed.entry(Pos::from(position)).or_insert(0) += count;
        self.claim_resource(position);
        Ok(())
    }

    /// Has the acting force finished `tech`, either already or under this plan?
    ///
    /// The union of the plan's overlay and what the acting force reports. The
    /// two cannot contradict each other on the *time* axis, because research
    /// is monotone: nothing in the game or in this planner un-researches a
    /// technology, so the overlay can only add. They could once contradict
    /// each other on the *force* axis, which is what `force` above fixes —
    /// this asks the same force `technology` reads costs from, and no other.
    pub fn is_researched(&self, tech: &str) -> bool {
        if self.researched.contains(tech) {
            return true;
        }
        matches!(self.technology(tech), Some(t) if t.researched)
    }

    /// Had the acting force finished `tech` *before this plan started*?
    ///
    /// [`Self::is_researched`]'s world half, without the overlay. The two
    /// answer different questions and the difference is load-bearing: a
    /// technology the overlay knows about is one some action in this same plan
    /// still has to run, so anything depending on it needs an ordering edge to
    /// that action. A technology the *world* reports is already done, so
    /// nothing orders against it and nothing has to.
    ///
    /// Collapsing the two is what `run-1788338409-63794` cost: the first share
    /// of a split goal researched `automation-science-pack` and applied
    /// `Effect::Researched` to the expansion state, so every later share read
    /// the recipe as plainly open, emitted no `Condition::Researched`, got no
    /// inferred edge, and was dispatched at tick zero. Three of four bots were
    /// told to craft a recipe the force had not unlocked yet.
    pub fn is_world_researched(&self, tech: &str) -> bool {
        matches!(self.technology(tech), Some(t) if t.researched)
    }

    pub fn set_researched(&mut self, tech: &str) {
        self.researched.insert(tech.to_string());
    }

    /// The base world's resource patches, with each patch's tiles in a stable
    /// order.
    ///
    /// `EntityGraph::resource_patches` builds `elements` by iterating a
    /// `HashMap` (`core/src/graph/entity_graph.rs:164-170`), so tile order
    /// varies with the randomised hash seed. Planning is required to be
    /// deterministic — the same world and the same goal must give the same
    /// schedule — and a method picking "the first tile of the patch" would
    /// otherwise pick a different one on every run. Sorting here keeps the fix
    /// inside the planner rather than perturbing `core`.
    ///
    /// **This makes tile order deterministic, not the patches themselves.** The
    /// flood fill at `entity_graph.rs:152-162` marks `pos` where it means
    /// `other`, so a tile pushed onto the stack is only ever labelled if it
    /// still has an unlabelled neighbour when it is popped; one that does not
    /// stays unlabelled and seeds a fresh patch. The fixture's contiguous
    /// 11x11 ore square therefore comes back as two or three patches, and which
    /// tiles are orphaned changes between calls *within one process*. Sorting
    /// cannot repair a partition that is wrong before it arrives. Fixing it is
    /// a one-word change in `core` and belongs with a test there.
    pub fn resource_patches(&self, item: &str) -> Vec<ResourcePatch> {
        let mut patches = self.base.entity_graph.resource_patches(item);
        for patch in &mut patches {
            patch
                .elements
                .sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        }
        patches
    }

    /// Is `item` something this world's ground yields at all?
    ///
    /// `!self.resource_patches(item).is_empty()`, without the flood fill and
    /// without `EntityGraph::resource_patches`'s miss warning -- see that
    /// method for why a predicate needs its own spelling. Asked of the base
    /// world only, exactly as `resource_patches` is: a plan neither creates
    /// nor exhausts a patch, it only claims and drains tiles of one.
    pub fn has_resource_patches(&self, item: &str) -> bool {
        self.base.entity_graph.has_resource_patches(item)
    }

    /// Why a character could not mine `resource` by hand, or `None` when
    /// nothing the prototypes say forbids it.
    ///
    /// The rule is [`FactorioEntityPrototype::hand_mining_obstacle`]'s, and
    /// this supplies its two inputs from the base world: the character's own
    /// `resource_categories`, falling back to the vanilla default when the
    /// prototype table has no `character` or one captured before the field
    /// existed (the same situation `character_mining_speed` covers), and the
    /// item table, which cannot say anything when it is empty and is then
    /// trusted rather than read as "nothing is an item".
    ///
    /// `None` also for a name with no prototype: a question about iron plate
    /// is not a question about mining, and this method has nothing to say.
    ///
    /// [`FactorioEntityPrototype::hand_mining_obstacle`]:
    /// factorio_bot_core::types::FactorioEntityPrototype::hand_mining_obstacle
    pub fn hand_mining_obstacle(&self, resource: &str) -> Option<HandMiningObstacle> {
        let prototypes = &self.base.entity_prototypes;
        let character_categories: Vec<String> = prototypes
            .get("character")
            .and_then(|character| character.resource_categories.clone())
            // An empty list is a character that mines nothing, which no game
            // ships; it is what a `{}` on the wire deserialises to, and it is
            // read as "not said" rather than as a ban on every ore.
            .filter(|categories| !categories.is_empty())
            .unwrap_or_else(|| {
                VANILLA_CHARACTER_RESOURCE_CATEGORIES
                    .iter()
                    .map(|category| (*category).to_string())
                    .collect()
            });
        let items = &self.base.item_prototypes;
        let is_item = |name: &str| items.is_empty() || items.contains_key(name);
        let proto = prototypes.get(resource)?;
        proto.hand_mining_obstacle(&character_categories, &is_item)
    }

    /// The resource entity whose mining yields `item`, **whether or not any of
    /// it is charted**.
    ///
    /// Read off the prototype table, which the game sends whole, rather than
    /// off the resource map, which holds only charted tiles. That difference
    /// is the whole point: it is what lets a refusal tell "this item does not
    /// come out of the ground" from "it does, and no ground the plan can see
    /// has any".
    ///
    /// The prototype named like the item wins when it qualifies -- every
    /// vanilla ore is named for what it yields -- and otherwise the smallest
    /// qualifying name, so the answer does not depend on `DashMap`'s
    /// iteration order. `None` for anything no `resource` prototype yields.
    pub fn resource_yielding(&self, item: &str) -> Option<String> {
        let yields = |proto: &factorio_bot_core::types::FactorioEntityPrototype| {
            proto.entity_type == "resource"
                && proto
                    .mine_result
                    .as_ref()
                    .is_some_and(|products| products.contains_key(item))
        };
        let prototypes = &self.base.entity_prototypes;
        if prototypes.get(item).is_some_and(|proto| yields(&proto)) {
            return Some(item.to_string());
        }
        prototypes
            .iter()
            .filter(|proto| yields(proto))
            .map(|proto| proto.name.clone())
            .min()
    }

    /// Asks the model whether it has terrain at the edge and the middle of the
    /// disc of `radius` around `origin`.
    ///
    /// Seventeen points -- the origin, then the eight compass directions at
    /// half the radius and at the full radius -- each answered by a one-tile
    /// box query against the tile tree. Deliberately *not* a count of every
    /// charted tile: a fully charted map carries ~410,000 water tiles alone,
    /// and reading them to find a bounding box would cost more than a whole
    /// map score for an answer no better than seventeen probes give.
    ///
    /// A probe finds a tile iff the mod wrote that chunk's tiles out, which it
    /// does once per chunk for every tile in it (`writeout_tiles`,
    /// `control.lua`) -- so a hit means the chunk is charted, not merely that
    /// something interesting stands there. A world attached from a snapshot
    /// fetches no tiles at all and answers blind everywhere.
    ///
    /// This used to live in `crate::score` and reach past `PlanState` into
    /// the entity graph; it is here so a refusal can ask the same question a
    /// map score does.
    #[must_use]
    pub fn charting(&self, origin: &Position, radius: f64) -> ChartingScore {
        let probes = charting_probes(origin, radius);
        let blind: Vec<Position> = probes
            .into_iter()
            .filter(|(_, _, point)| {
                self.base
                    .entity_graph
                    .tiles_within(&tile_box(point))
                    .is_empty()
            })
            .map(|(_, _, point)| point)
            .collect();
        ChartingScore {
            covered: CHARTING_PROBES - blind.len(),
            probes: CHARTING_PROBES,
            blind,
        }
    }

    /// Whether the model has ground for the tile under `point`.
    ///
    /// One probe of [`Self::charting`], on its own: the same one-tile query
    /// against the tile tree, with the same meaning. A hit means the mod
    /// wrote that chunk's tiles out (`writeout_tiles`), so the chunk is
    /// charted -- **not** that anything interesting stands there, and a miss
    /// means the chunk was never written out rather than that the ground is
    /// empty.
    ///
    /// Exists so [`crate::method::scout`] can ask cell by cell over its
    /// lattice, where `charting`'s fixed seventeen probes are the wrong
    /// shape.
    #[must_use]
    pub fn is_charted(&self, point: &Position) -> bool {
        !self
            .base
            .entity_graph
            .tiles_within(&tile_box(point))
            .is_empty()
    }

    /// The nearest charted enemy structure to `from` -- its name, its
    /// position and how far away it is -- or `None` when the model holds
    /// none.
    ///
    /// A read of [`factorio_bot_core::graph::entity_graph::EntityGraph::nearest_threat`],
    /// lifted onto `PlanState` so a method can ask it the way it asks
    /// [`Self::charting`], rather than reaching past the state into the graph.
    ///
    /// # `None` is *unknown*, never *safe*
    ///
    /// This is the same asymmetry [`Self::charting`] has and it matters more
    /// here, because the caller is [`crate::method::scout::Scout`] -- which
    /// asks precisely while sending a bot towards ground **nothing has
    /// charted**, so `None` is the expected answer at the frontier and is the
    /// one place it must not be read as an all-clear. The stand-off this
    /// feeds can only ever avoid nests somebody has already seen; the ring
    /// beyond the frontier is unknown by construction, which is the argument
    /// for exploring one ring at a time and re-planning, and not for
    /// exploring further on the strength of an empty answer.
    ///
    /// Distances are Euclidean (`calculate_distance`), matching
    /// `EntityGraph`'s own measurement -- deliberately not
    /// `Position::distance`, which is Manhattan despite the name.
    #[must_use]
    pub fn nearest_threat(&self, from: &Position) -> Option<(String, Position, f64)> {
        self.base.entity_graph.nearest_threat(from)
    }

    /// [`Self::charting`] with the frontier and the resource census attached:
    /// the sentence a `NotCharted` refusal carries.
    ///
    /// The frontier is the nearest blind probe, ties broken by probe order
    /// (the compass, east first, clockwise) so two runs on one dump name the
    /// same direction. The census is the fingerprint's, so it costs one walk
    /// of the charted resource tiles -- affordable on a refusal path, and the
    /// same number `score-map` prints.
    #[must_use]
    pub fn charting_summary(&self, origin: &Position, radius: f64) -> ChartingSummary {
        let score = self.charting(origin, radius);
        let frontier = charting_probes(origin, radius)
            .into_iter()
            .filter(|(_, _, point)| {
                self.base
                    .entity_graph
                    .tiles_within(&tile_box(point))
                    .is_empty()
            })
            .map(|(direction, distance, position)| Frontier {
                direction,
                position,
                distance,
            })
            // The probe list is ordered by distance already (origin, half,
            // full), so the first blind probe is the nearest; stated as a
            // `min_by` anyway so the claim does not depend on the list's
            // construction.
            .min_by(|a, b| a.distance.total_cmp(&b.distance));
        let seen = self
            .base
            .entity_graph
            .resource_fingerprint()
            .map(|print| print.tiles)
            .unwrap_or_default();
        ChartingSummary {
            origin: origin.clone(),
            radius,
            score,
            frontier,
            seen,
        }
    }

    /// Every standing tree or rock that yields `item`, as
    /// `(entity name, position, what one of them yields)`.
    ///
    /// The renewable half of "where do items come from". `resource_patches`
    /// answers for ore, which is a tile with an amount; this answers for an
    /// entity that yields a fixed bill once and is then gone. Wood is the item
    /// that made it necessary: it has no recipe and no ore, so before this the
    /// planner's only wood was whatever the bots happened to start the run
    /// holding -- four items, for ever, across a whole game.
    ///
    /// Ordered by entity name and then by tile, both from `BTreeMap`s in
    /// `core`, so the order is the data's. Callers still sort by distance
    /// themselves; this only guarantees the *input* to that sort does not move
    /// between runs.
    ///
    /// Trees this plan has already chopped are gone from the answer:
    /// `Effect::RemoveEntity` puts them in `removed`, which is what stops two
    /// actions of one plan swinging at the same tree.
    pub fn minable_sources(&self, item: &str) -> Vec<(String, Position, u32)> {
        let mut out: Vec<(String, Position, u32)> = Vec::new();
        for (name, yields) in self.base.entity_graph.minables_yielding(item) {
            for position in self.base.entity_graph.minable_positions(&name) {
                if self.removed.contains(&Pos::from(&position)) {
                    continue;
                }
                out.push((name.clone(), position, yields));
            }
        }
        out
    }

    /// Whether [`PlanState::minable_sources`] would answer with anything.
    ///
    /// Short-circuits, so the common answer costs one map lookup rather than a
    /// walk of every tree on the map.
    pub fn has_minable_source(&self, item: &str) -> bool {
        self.base
            .entity_graph
            .minables_yielding(item)
            .iter()
            .any(|(name, _)| {
                self.base
                    .entity_graph
                    .minable_positions(name)
                    .iter()
                    .any(|position| !self.removed.contains(&Pos::from(position)))
            })
    }
}

/// How many points [`PlanState::charting`] probes: the origin and the compass
/// at two ranges.
const CHARTING_PROBES: usize = 1 + 2 * COMPASS.len();

/// The probe points of [`PlanState::charting`], in probe order, each with the
/// compass name of its direction and its distance from `origin`.
fn charting_probes(origin: &Position, radius: f64) -> Vec<(&'static str, f64, Position)> {
    let mut points = Vec::with_capacity(CHARTING_PROBES);
    points.push(("here", 0., origin.clone()));
    for scale in [0.5, 1.0] {
        for (name, dx, dy) in COMPASS {
            points.push((
                name,
                radius * scale,
                Position::new(
                    origin.x() + dx * radius * scale,
                    origin.y() + dy * radius * scale,
                ),
            ));
        }
    }
    points
}

/// The one-tile box around `point` a charting probe asks the tile tree for.
fn tile_box(point: &Position) -> Rect {
    Rect::new(
        &Position::new(point.x() - TILE_HALF_SIDE, point.y() - TILE_HALF_SIDE),
        &Position::new(point.x() + TILE_HALF_SIDE, point.y() + TILE_HALF_SIDE),
    )
}

#[cfg(test)]
mod tests {
    /// **The load a machine carries survives a smelt adopting it.**
    ///
    /// `smelt_steps` unqueues a furnace while it emits and queues it again
    /// behind its own take, and `queued` used to be reset to the newest
    /// batch by that cycle -- so "least-loaded first" compared each
    /// furnace's last batch instead of its queue, and green piled
    /// twenty-six batches onto one furnace. The total lives in
    /// `machine_load` now, which only `queue_machine` writes.
    #[test]
    fn the_queued_total_survives_unqueueing() {
        let mut state = PlanState::from_world(
            Arc::new(factorio_bot_core::test_utils::fixture_world()),
            &[BotId(1), BotId(2)],
        );
        let pos = Position::new(-38.0, -16.0);
        state.commit_machine(&pos);
        state.queue_machine(&pos, "iron-plate", ActionId(7), Some(BotId(1)), 768);
        assert_eq!(state.machine_queue(&pos).map(|q| q.queued), Some(768));
        state.unqueue_machine(&pos);
        assert!(
            state.machine_queue(&pos).is_none(),
            "nothing may queue while a smelt emits"
        );
        state.queue_machine(&pos, "iron-plate", ActionId(19), Some(BotId(2)), 192);
        let entry = state
            .machine_queue(&pos)
            .expect("queued again behind the newest take");
        assert_eq!(entry.queued, 960, "both batches, not just the newest");
        assert_eq!(entry.release, ActionId(19));
        assert_eq!(entry.taker, Some(BotId(2)));
    }

    use super::*;
    use factorio_bot_core::test_utils::{fixture_entity_prototypes, fixture_world};
    use factorio_bot_core::types::{FactorioEntity, Position};

    /// The fixture, with every iron tile re-delivered carrying the amount the
    /// game would have reported for it — the same re-delivery
    /// `tests/tile_capacity.rs` uses, and for the same reason: it is how a
    /// real reading arrives, and the *only* difference from `fixture_world()`
    /// is that the world now says what a tile holds.
    fn state_with_iron_holding(bots: &[BotId], amount: u32) -> PlanState {
        let world = fixture_world();
        let ore: Vec<FactorioEntity> = world
            .entity_graph
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .map(|tile| {
                let mut entity = FactorioEntity::new_resource(
                    &tile,
                    factorio_bot_core::types::Direction::North,
                    "iron-ore",
                );
                entity.amount = Some(amount);
                entity
            })
            .collect();
        assert!(!ore.is_empty(), "the fixture carries an iron field");
        world
            .update_chunk_entities(ore)
            .expect("re-delivering ore with amounts");
        PlanState::from_world(Arc::new(world), bots)
    }

    /// One tile of the fixture's iron field, chosen the same way every time —
    /// the lowest `(x, y)`, as `tests/tile_capacity.rs` chooses it, so the
    /// tile is a property of the field rather than a literal that a fixture
    /// change could quietly move off the ore.
    fn a_stated_iron_tile(state: &PlanState) -> Position {
        let mut tiles: Vec<Position> = state
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|patch| patch.elements)
            .collect();
        tiles.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        tiles.into_iter().next().expect("the field has tiles")
    }

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)])
    }

    fn entity_at_pos(name: &str, x: f64, y: f64) -> FactorioEntity {
        FactorioEntity {
            name: name.into(),
            entity_type: "furnace".into(),
            position: Position::new(x, y),
            ..Default::default()
        }
    }

    // ---- slot capacity -----------------------------------------------------

    /// The §2 table of
    /// `docs/superpowers/specs/2026-09-04-world-model-divergence-design.md`,
    /// as measured on a live 2.1.17 instance, restated against the fixture's
    /// own stack sizes (iron-plate 100, iron-ore 50, electronic-circuit 200).
    ///
    /// The point of the assertions on `electronic-circuit` is that the answer
    /// **tracks the item, not the machine**: an assembler holds 100 gears and
    /// 200 circuits in the same output slot, which is what makes this a read
    /// of `stack_size` rather than a table of machines.
    #[test]
    fn slot_capacity_is_a_stack_out_and_a_stack_plus_the_overload_in() {
        let s = state();
        for (slot, item, want) in [
            (InventorySlot::FurnaceResult, "iron-plate", 100),
            (InventorySlot::AssemblerOutput, "iron-gear-wheel", 100),
            (InventorySlot::AssemblerOutput, "electronic-circuit", 200),
            (InventorySlot::Fuel, "coal", 50),
            (InventorySlot::LabInput, "automation-science-pack", 200),
            (InventorySlot::FurnaceSource, "iron-ore", 70),
            (InventorySlot::AssemblerInput, "iron-plate", 120),
            (InventorySlot::AssemblerInput, "electronic-circuit", 220),
        ] {
            assert_eq!(
                s.slot_capacity(slot, item),
                Some(want),
                "{slot:?} of {item}"
            );
        }
    }

    /// The input margin is **additive**, not proportional: 50 -> 70,
    /// 100 -> 120, 200 -> 220. Proportional would have made the stack of 200
    /// answer 240, and this is the assertion that would catch someone
    /// "simplifying" it into a multiplier.
    #[test]
    fn the_input_overload_is_additive_across_every_stack_size() {
        let s = state();
        for item in ["iron-ore", "iron-plate", "electronic-circuit"] {
            let stack = s.stack_size(item).expect("the fixture has this prototype");
            assert_eq!(
                s.slot_capacity(InventorySlot::AssemblerInput, item),
                Some(stack + INPUT_OVERLOAD),
                "{item} stacks to {stack}"
            );
        }
    }

    /// **Unknown must mean unbounded, never a guessed number.** Several
    /// fixtures in this crate build worlds with no item prototypes at all and
    /// pin their plans byte-for-byte; a fallback constant would silently
    /// re-size every transfer in them. A chest answers the same way for a
    /// different reason -- `slots x stack_size` needs an inventory size the
    /// entity prototypes do not carry.
    #[test]
    fn an_unknown_item_and_a_chest_are_both_unbounded() {
        let s = state();
        assert_eq!(
            s.stack_size("no-such-item"),
            None,
            "no prototype, no stack size"
        );
        for slot in InventorySlot::ALL {
            assert_eq!(
                s.slot_capacity(slot, "no-such-item"),
                None,
                "{slot:?} of an item the world does not describe"
            );
        }
        assert_eq!(
            s.slot_capacity(InventorySlot::Chest, "iron-plate"),
            None,
            "a chest's slot count is not in the prototypes"
        );
    }

    // ---- electric supply ---------------------------------------------------

    /// Two big poles 31 tiles apart are wired, because the game's reach is 32.
    ///
    /// **This table read 30.0 until 2026-09-07**, a Factorio 1.x number that
    /// nobody had ever checked against the game's own `entities.lua`, so a
    /// legal big-pole span read as a broken network and everything past it
    /// lost its power. 31 is the one-tile window that tells the two apart.
    #[test]
    fn two_big_poles_are_wired_at_thirty_one_tiles() {
        let mut s = state();
        for x in [0., 31.] {
            s.create_entity(FactorioEntity {
                name: "big-electric-pole".into(),
                position: Position::new(x, 0.),
                ..Default::default()
            });
        }
        s.create_entity(FactorioEntity {
            name: "steam-engine".into(),
            position: Position::new(31., 1.),
            ..Default::default()
        });

        let kw = s.electric_supply_kw(&lab_area(&s, Position::new(0., 0.)));
        assert_eq!(
            kw, 900.,
            "a big pole reaches 32, so a 31-tile span carries the engine's power"
        );
    }

    // ---- supply area, from the prototype -----------------------------------

    /// A `PlanState` whose `name` prototype declares `supply_area_distance`,
    /// or, with `None`, has it explicitly cleared.
    ///
    /// The fixture ships every vanilla pole with the field **absent**, which
    /// is what a pre-2026-09-06 dump looks like, so overwriting it in place is
    /// what lets a test say "the reach came from the world" rather than "the
    /// reach happens to equal the table".
    fn state_with_supply_area(name: &str, distance: Option<f64>) -> PlanState {
        let world = fixture_world();
        let mut prototype = world
            .entity_prototypes
            .get(name)
            .expect("the fixture ships this prototype")
            .clone();
        prototype.supply_area_distance = distance;
        world.entity_prototypes.insert(name.into(), prototype);
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// A pole's reach is read off its own prototype, not off the vanilla
    /// table.
    ///
    /// **The whole point of the change.** A modded `small-electric-pole` that
    /// supplies 10x10 must supply 10x10 here, and the only way to see that is
    /// to state a number the table does not contain and watch coverage follow
    /// it. 5.0 reaches ground 4.5 tiles out and vanilla's 2.5 does not.
    #[test]
    fn a_poles_supply_area_comes_from_its_own_prototype() {
        let far = Rect::new(&Position::new(4.4, -0.1), &Position::new(4.6, 0.1));

        let retuned = state_with_supply_area("small-electric-pole", Some(5.0));
        assert!(
            retuned.pole_would_supply("small-electric-pole", &Position::new(0., 0.), &far),
            "the prototype says 5.0, so 4.5 tiles away is inside the supply area"
        );

        let vanilla = state_with_supply_area("small-electric-pole", Some(2.5));
        assert!(
            !vanilla.pole_would_supply("small-electric-pole", &Position::new(0., 0.), &far),
            "at 2.5 the same ground is outside it, so the number is doing the work"
        );
    }

    /// A pole prototype with no `supply_area_distance` — every world dumped
    /// before the field existed — falls back to the vanilla number rather than
    /// reading as a pole that supplies nothing.
    ///
    /// This is what keeps `workspace/scripts/map.json` and every archived run
    /// planning at all. It is a compatibility shim and is documented as one on
    /// [`vanilla_pole_supply_half_extent`].
    #[test]
    fn a_pole_prototype_without_the_field_falls_back_to_vanilla() {
        let s = state_with_supply_area("small-electric-pole", None);
        let inside = Rect::new(&Position::new(2.3, -0.1), &Position::new(2.4, 0.1));
        let outside = Rect::new(&Position::new(2.6, -0.1), &Position::new(2.7, 0.1));
        assert!(
            s.pole_would_supply("small-electric-pole", &Position::new(0., 0.), &inside),
            "vanilla's 2.5 must still be reachable when the sender said nothing"
        );
        assert!(
            !s.pole_would_supply("small-electric-pole", &Position::new(0., 0.), &outside),
            "and it must still end at 2.5, not become unbounded"
        );
    }

    // ---- wire reach, from the prototype ------------------------------------

    /// A `PlanState` whose `name` prototype declares `maximum_wire_distance`,
    /// or, with `None`, has it explicitly cleared.
    ///
    /// The fixture ships every vanilla pole with the field **absent**, which
    /// is what every world dumped before 2026-09-07 looks like, so setting it
    /// in place is what lets a test say "the reach came from the world" rather
    /// than "the reach happens to equal the table".
    fn state_with_wire_reach(name: &str, distance: Option<f64>) -> PlanState {
        let world = fixture_world();
        let mut prototype = world
            .entity_prototypes
            .get(name)
            .expect("the fixture ships this prototype")
            .clone();
        prototype.maximum_wire_distance = distance;
        world.entity_prototypes.insert(name.into(), prototype);
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// A pole's wire reach is read off its own prototype, not off the vanilla
    /// table.
    ///
    /// **The whole point of the change.** A modded `small-electric-pole` that
    /// throws 40 tiles must throw 40 here, and the only way to see that is to
    /// state a number the table does not contain and watch connectivity follow
    /// it. Two small poles 31 tiles apart are wired at 40 and are not at
    /// vanilla's 7.5, so the number is doing the work.
    #[test]
    fn a_poles_wire_reach_comes_from_its_own_prototype() {
        let wired = |reach: Option<f64>| {
            let mut s = state_with_wire_reach("small-electric-pole", reach);
            for x in [0., 31.] {
                s.create_entity(FactorioEntity {
                    name: "small-electric-pole".into(),
                    position: Position::new(x, 0.),
                    ..Default::default()
                });
            }
            s.create_entity(FactorioEntity {
                name: "steam-engine".into(),
                position: Position::new(31., 1.),
                ..Default::default()
            });
            s.electric_supply_kw(&lab_area(&s, Position::new(0., 0.)))
        };

        assert_eq!(
            wired(Some(40.)),
            900.,
            "the prototype says 40, so a 31-tile span carries the engine"
        );
        assert_eq!(
            wired(Some(7.5)),
            0.,
            "at vanilla's 7.5 the same two poles are not wired at all"
        );
    }

    /// **A declared zero is believed, and that is what keeps it different from
    /// silence.** `get_max_wire_distance()` answers 0 for an entity with no
    /// wires and the mod sends that zero, so `Some(0.0)` is the game speaking
    /// and `None` is the sender saying nothing. Only the second falls back.
    ///
    /// Merging them would either blind the planner on every archived world or
    /// silently re-credit a wireless prototype with a vanilla pole's span.
    ///
    /// The absent half is also covered end-to-end, through connectivity rather
    /// than through this accessor, by
    /// `two_big_poles_are_wired_at_thirty_one_tiles`: the fixture ships
    /// every pole with the field absent, so that test *is* the shim's live
    /// case, and a separate one written here for it was deleted as a
    /// duplicate of it — a falsification run found the two dying to the same
    /// mutation.
    #[test]
    fn a_declared_zero_wire_reach_is_not_the_same_as_an_absent_one() {
        assert_eq!(
            state_with_wire_reach("small-electric-pole", Some(0.))
                .pole_wire_reach_tiles("small-electric-pole"),
            Some(0.),
            "the world says zero, so zero it is -- the table must not fire"
        );
        assert_eq!(
            state_with_wire_reach("small-electric-pole", None)
                .pole_wire_reach_tiles("small-electric-pole"),
            Some(7.5),
            "and silence is the case the table exists for"
        );
    }

    /// Only an `electric-pole` gets a reach out of this, however loudly its
    /// prototype declares one.
    ///
    /// `get_max_wire_distance()` is the maximum over *every* wire kind, so a
    /// machine reports its **circuit** wire distance. Measured over 1,028 live
    /// prototypes: **4 poles and 94 other entities report a positive number**
    /// — `stone-furnace` and `wooden-chest` both 9, `power-switch` 10,
    /// `agricultural-tower` 30. Without this gate a plan would wire its
    /// network through an assembling machine. The gate is the same one
    /// [`pole_supply_half_extent`] uses, from the same side.
    #[test]
    fn only_an_electric_pole_gets_a_wire_reach() {
        let s = state_with_wire_reach("stone-furnace", Some(10.));
        assert_eq!(
            s.pole_wire_reach_tiles("stone-furnace"),
            None,
            "a furnace that reports a wire distance is still not a pole"
        );
    }

    // ---- electrical draw and output, from the prototype --------------------

    /// A `PlanState` whose `name` prototype declares `electric_energy_usage`
    /// and `max_energy_production`, **in joules per tick** as the game reports
    /// them, or has either explicitly cleared with `None`.
    ///
    /// The fixture ships all 528 prototypes with both fields **absent**, which
    /// is what every world dumped before 2026-09-07 looks like, so setting one
    /// in place is what lets a test say "the number came from the world"
    /// rather than "the number happens to equal the table".
    fn state_with_energy(
        name: &str,
        usage_joules_per_tick: Option<f64>,
        production_joules_per_tick: Option<f64>,
    ) -> PlanState {
        let world = fixture_world();
        let mut prototype = world
            .entity_prototypes
            .get(name)
            .expect("the fixture ships this prototype")
            .clone();
        prototype.electric_energy_usage = usage_joules_per_tick;
        prototype.max_energy_production = production_joules_per_tick;
        world.entity_prototypes.insert(name.into(), prototype);
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// A consumer's draw is read off its own prototype, not off the vanilla
    /// table.
    ///
    /// **The whole point of the change**, and stated in the game's own unit:
    /// 4,000 joules per tick is 240 kW, a number the table does not contain
    /// for any name, so a table read cannot produce it by accident. It also
    /// pins the x60/1000 conversion at a value where a missing or doubled
    /// factor of 60 is unmistakable.
    #[test]
    fn a_consumers_draw_comes_from_its_own_prototype() {
        let s = state_with_energy("electric-furnace", Some(4000.), None);
        assert_eq!(
            s.consumer_draw_kw("electric-furnace"),
            Some(240.),
            "4000 J/tick is 240 kW, and vanilla's 180 must not win"
        );
    }

    /// A consumer prototype with no `electric_energy_usage` — every world
    /// dumped before the field existed, `workspace/scripts/map.json` included
    /// — falls back to the vanilla number rather than reading as a machine
    /// that draws nothing.
    ///
    /// A compatibility shim, documented as one on [`vanilla_consumer_kw`].
    /// Deleting the equivalent pole shim made all three offline goals refuse
    /// to expand at all, blaming the water.
    #[test]
    fn a_consumer_prototype_without_the_field_falls_back_to_vanilla() {
        let s = state_with_energy("electric-furnace", None, None);
        assert_eq!(
            s.consumer_draw_kw("electric-furnace"),
            Some(180.),
            "the sender said nothing, so vanilla's 180 kW still stands"
        );
    }

    /// **An inserter keeps its duty cycle whether the prototype says nothing
    /// or says zero**, and the live game says nothing.
    ///
    /// Measured on all 1,028 prototypes of a 2.1.17 game: 28 carry
    /// `energy_usage` and none reports 0, `inserter` included — it has an
    /// electric energy source, so it passes the mod's gate, but the attribute
    /// is optional and absent on it. The absent case is therefore the real
    /// one; the zero case is a guard against a modded prototype that states a
    /// standing draw of nothing, and both are asserted because a reader who
    /// saw only the guard would conclude the wrong thing about vanilla.
    ///
    /// Silent if got wrong, and expensively so: a 48-inserter block would read
    /// as 0 kW of demand, pass its own headroom check and brown out — the
    /// *coverage is not capacity* failure this file exists to prevent.
    #[test]
    fn an_inserter_keeps_its_duty_cycle_whether_absent_or_zero() {
        for stated in [None, Some(0.)] {
            let s = state_with_energy("inserter", stated, None);
            assert_eq!(
                s.consumer_draw_kw("inserter"),
                Some(INSERTER_DUTY_KW),
                "{stated:?} is 'the prototype does not answer', not 'draws nothing'"
            );
        }
    }

    /// **A machine the hand-kept table never named now costs what it costs.**
    ///
    /// The old table's unknown name drew *nothing*, which is the one direction
    /// this file's tables err towards permitting: an unmodelled machine on the
    /// network is headroom that is not there. A live 2.1.17 game names 17
    /// electric consumers the table did not carry, and `small-lamp` is not
    /// hypothetical — the `FurnaceLine` fixture stands three of them and was
    /// budgeting zero for all three.
    ///
    /// Falsifiable in one line: delete the prototype read and this is `None`.
    #[test]
    fn a_consumer_the_table_never_named_is_charged_from_its_prototype() {
        assert_eq!(
            vanilla_consumer_kw("small-lamp"),
            None,
            "the hand-kept table never heard of a lamp"
        );
        let s = state_with_energy("small-lamp", Some(5000. / 60.), None);
        assert_eq!(
            s.consumer_draw_kw("small-lamp"),
            Some(5.),
            "and the game says 5 kW, which is now what the ledger charges"
        );
    }

    /// A generator's output is read off its own prototype.
    ///
    /// 20,000 J/tick is 1,200 kW — not 900, not 5,800, and not any multiple of
    /// either, so nothing but the prototype can produce it.
    #[test]
    fn a_generators_output_comes_from_its_own_prototype() {
        let s = state_with_energy("steam-engine", None, Some(20000.));
        assert_eq!(
            s.generator_output_kw("steam-engine"),
            Some(1200.),
            "20000 J/tick is 1200 kW, and vanilla's 900 must not win"
        );
    }

    /// The generation half of the compatibility shim.
    #[test]
    fn a_generator_prototype_without_the_field_falls_back_to_vanilla() {
        let s = state_with_energy("steam-engine", None, None);
        assert_eq!(
            s.generator_output_kw("steam-engine"),
            Some(900.),
            "every archived world predates the field and must still see engines"
        );
    }

    /// **A solar panel answers `get_max_energy_production()` and must still
    /// generate nothing here.**
    ///
    /// The number it answers with is its *noon* output, so crediting it makes
    /// the same plan feasible or not according to what time of day the run
    /// started. That is the trap CLAUDE.md names, and reading the prototype
    /// without [`DETERMINISTIC_GENERATOR_TYPES`] would have walked straight
    /// into it — `solar-panel` was absent from the old table by omission, and
    /// omission is not a gate.
    #[test]
    fn a_solar_panel_declaring_production_generates_nothing() {
        let s = state_with_energy("solar-panel", None, Some(1000.));
        assert_eq!(
            s.generator_output_kw("solar-panel"),
            None,
            "noon output is not a deterministic number and is not credited"
        );
        let accumulators = state_with_energy("accumulator", None, Some(5000.));
        assert_eq!(
            accumulators.generator_output_kw("accumulator"),
            None,
            "an accumulator's discharge limit is not generation either"
        );
    }

    // ---- solar, which needs the surface and not just the prototype ---------

    /// Vanilla Nauvis' daylight curve, as `LuaSurface` reports it.
    fn nauvis_daylight() -> factorio_bot_core::types::SurfaceDaylight {
        factorio_bot_core::types::SurfaceDaylight {
            surface: Some(factorio_bot_core::types::SurfaceId::nauvis()),
            // The live 2.1.17 figure. See
            // `crates/core/tests/live-2.1.17-daylight.json`.
            ticks_per_day: Some(25_200),
            dawn: Some(0.75),
            dusk: Some(0.25),
            evening: Some(0.45),
            morning: Some(0.55),
            daytime: Some(0.0),
            solar_power_multiplier: Some(1.0),
            always_day: Some(false),
            freeze_daytime: Some(false),
        }
    }

    /// A `PlanState` carrying a vanilla `solar-panel` and `accumulator`, and
    /// optionally a daylight curve.
    ///
    /// The fixture ships all 528 prototypes with none of the solar fields, so
    /// they are set in place here for the same reason `state_with_energy`
    /// does: a number that came from the world is distinguishable from a
    /// number that happens to match a table.
    fn state_with_solar(daylight: Option<factorio_bot_core::types::SurfaceDaylight>) -> PlanState {
        let world = fixture_world();
        let mut panel = world
            .entity_prototypes
            .get("solar-panel")
            .expect("the fixture ships a solar panel")
            .clone();
        // 1,000 J/tick is the 60 kW on a vanilla panel's tooltip: its NOON
        // output, which is the whole reason this pair of accessors exists.
        panel.max_energy_production = Some(1000.);
        panel.solar_panel_performance_at_day = Some(1.0);
        panel.solar_panel_performance_at_night = Some(0.0);
        world.entity_prototypes.insert("solar-panel".into(), panel);

        let mut accumulator = world
            .entity_prototypes
            .get("accumulator")
            .expect("the fixture ships an accumulator")
            .clone();
        accumulator.max_energy_production = Some(5000.);
        accumulator.electric_buffer_capacity = Some(5_000_000.);
        world
            .entity_prototypes
            .insert("accumulator".into(), accumulator);

        if let Some(daylight) = daylight {
            world.update_daylight(daylight);
        }
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// **The owner's ruling, derived rather than typed: solar is planned at
    /// average output, and a vanilla panel averages 42 kW of its 60.**
    ///
    /// Every term comes from somewhere: the 60 kW from the panel's own
    /// `max_energy_production`, the two endpoints from its
    /// `solar_panel_performance_at_*`, and the 0.7 from the surface's four
    /// day-phase boundaries. Nothing in the tree contains 42, or 0.7, and
    /// that is the point — a rate copied from a table is a mod-compatibility
    /// defect by the standing rule, and solar is where a mod is likeliest to
    /// differ.
    #[test]
    fn a_solar_panel_is_worth_its_daily_average_and_not_its_noon_figure() {
        let s = state_with_solar(Some(nauvis_daylight()));
        assert_eq!(
            s.generator_output_kw("solar-panel"),
            None,
            "the nameplate ledger still refuses it, and must",
        );
        let average = s
            .solar_average_kw("solar-panel")
            .expect("a vanilla panel on a vanilla surface");
        assert!(
            (average - 42.).abs() < 1e-9,
            "60 kW at noon x 0.7 of a day, got {average}",
        );
    }

    /// **`None` means unknown, and that is why no offline baseline moved.**
    ///
    /// Every world this project has archived predates the daylight channel, so
    /// every one of them answers exactly as it did before these accessors
    /// existed. A zero here would be a claim that the surface is dark, which
    /// nobody established.
    #[test]
    fn a_world_that_never_reported_daylight_answers_unknown_not_zero() {
        let s = state_with_solar(None);
        assert_eq!(s.solar_average_kw("solar-panel"), None);
        assert_eq!(
            s.accumulators_per_panel("solar-panel", "accumulator"),
            None,
            "and the accumulator half refuses on the same terms",
        );
    }

    /// A prototype that is not a solar panel has no curve to follow, and the
    /// endpoints' absence is what says so — they carry
    /// `subclasses: ["SolarPanel"]`, so the live game omits them on everything
    /// else. A steam engine credited a daylight average would be a silent
    /// 30% under-count of a generator that runs all night.
    #[test]
    fn a_machine_that_is_not_a_panel_has_no_daily_average() {
        let s = state_with_solar(Some(nauvis_daylight()));
        assert_eq!(s.solar_average_kw("steam-engine"), None);
        assert_eq!(
            s.solar_average_kw("accumulator"),
            None,
            "an accumulator declares production and is not a panel",
        );
    }

    /// **The second vanilla ratio out of the same channel: 25 panels to 21
    /// accumulators, scaled by the day the running game actually reports.**
    ///
    /// Independent of the 0.7 average — a different integral of the same curve
    /// — which is why landing both is corroboration rather than one number
    /// checked twice. The energy path is explicit: 0.168 of a day's full
    /// output is 4.234 MJ per panel on a 25,200-tick day, against an
    /// accumulator's 5 MJ — 0.8467, which is the familiar 0.84 times
    /// 25,200/25,000 and nothing else.
    #[test]
    fn the_accumulator_ratio_is_derived_and_lands_on_the_vanilla_twenty_five_to_twenty_one() {
        let s = state_with_solar(Some(nauvis_daylight()));
        let ratio = s
            .accumulators_per_panel("solar-panel", "accumulator")
            .expect("a vanilla pair on a vanilla surface");
        assert!(
            (ratio - 0.84 * 25_200. / 25_000.).abs() < 1e-9,
            "4.234 MJ of shortfall against a 5 MJ buffer, got {ratio}",
        );
        assert!(
            (ratio * 25. - 21.).abs() < 0.2,
            "which is 25 panels to 21 accumulators to within the day-length \
             correction, got {} per 25 panels",
            ratio * 25.,
        );
    }

    /// The buffer is the accumulator's real number and it is **not** its
    /// `max_energy_production`: that field carries the 300 kW discharge
    /// *limit*, a rate rather than a store. Sizing against it would ask for
    /// 14,000 accumulators per panel.
    #[test]
    fn an_accumulator_with_no_buffer_declared_cannot_be_sized_against() {
        let world = fixture_world();
        let mut panel = world
            .entity_prototypes
            .get("solar-panel")
            .expect("the fixture ships a solar panel")
            .clone();
        panel.max_energy_production = Some(1000.);
        panel.solar_panel_performance_at_day = Some(1.0);
        panel.solar_panel_performance_at_night = Some(0.0);
        world.entity_prototypes.insert("solar-panel".into(), panel);

        let mut accumulator = world
            .entity_prototypes
            .get("accumulator")
            .expect("the fixture ships an accumulator")
            .clone();
        // The discharge limit is declared; the store is not.
        accumulator.max_energy_production = Some(5000.);
        accumulator.electric_buffer_capacity = None;
        world
            .entity_prototypes
            .insert("accumulator".into(), accumulator);
        world.update_daylight(nauvis_daylight());
        let s = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        assert_eq!(
            s.accumulators_per_panel("solar-panel", "accumulator"),
            None,
            "a discharge rate is not a store and must not be read as one",
        );
    }

    /// The surface's own multiplier reaches the kW answer, not just the
    /// fraction. It is exactly the knob that makes an array a different size
    /// on a different planet, and it is 1 on Nauvis, so a version that dropped
    /// it would pass every other test in this file.
    #[test]
    fn the_surfaces_solar_multiplier_reaches_the_kilowatt_answer() {
        let mut daylight = nauvis_daylight();
        daylight.solar_power_multiplier = Some(0.5);
        let s = state_with_solar(Some(daylight));
        let average = s.solar_average_kw("solar-panel").expect("a dimmer surface");
        assert!((average - 21.).abs() < 1e-9, "half of 42 kW, got {average}",);
    }

    /// **A beacon is not a pole, and the same field means something else on
    /// it.**
    ///
    /// `supply_area_distance` rides on both `ElectricPole` and `Beacon`, so a
    /// reader that trusts the number without checking `entity_type` reports a
    /// beacon as supplying 6x6 of electricity it was never wired to carry.
    /// Vanilla's beacon declares 3, which is the largest of the four pole
    /// numbers but the one least entitled to be believed here.
    #[test]
    fn a_beacon_declaring_a_supply_area_supplies_no_power() {
        let s = state_with_supply_area("beacon", Some(3.0));
        let beside = Rect::new(&Position::new(1.4, -0.1), &Position::new(1.6, 0.1));
        assert!(
            !s.pole_would_supply("beacon", &Position::new(0., 0.), &beside),
            "a beacon's 3 is distance beyond its footprint, and it is not a power source"
        );
    }

    /// A name no prototype describes contributes no coverage, which
    /// under-credits rather than over-credits.
    #[test]
    fn a_pole_the_world_has_never_heard_of_supplies_nothing() {
        let s = state();
        let anywhere = Rect::new(&Position::new(-1., -1.), &Position::new(1., 1.));
        assert!(
            !s.pole_would_supply("mod-mega-pole", &Position::new(0., 0.), &anywhere),
            "an unknown name is unknown reach, never a default one"
        );
    }

    /// A `PlanState` with a pole at `pole` and, optionally, a steam engine at
    /// `engine`.
    ///
    /// Built through `create_entity` — the overlay — rather than through the
    /// world, because `EntityGraph::add`'s whitelist does not admit
    /// `electric-pole` or `generator` and an entity added to the world would
    /// be unreadable by name. See [`PlanState::entities_within`].
    fn powered(pole: Option<Position>, engine: Option<Position>) -> PlanState {
        let mut s = state();
        for (name, position) in [("small-electric-pole", pole), ("steam-engine", engine)] {
            if let Some(position) = position {
                s.create_entity(FactorioEntity {
                    name: name.into(),
                    position,
                    ..Default::default()
                });
            }
        }
        s
    }

    /// The box a lab centred at `pos` would cover.
    fn lab_area(s: &PlanState, pos: Position) -> Rect {
        s.collision_area("lab", &pos)
            .expect("the fixture has a lab")
    }

    /// A chain of poles carries power however long it is.
    ///
    /// **This is the whole point of the change and the case the old code got
    /// wrong.** A pole run past `POWER_SEARCH_RADIUS` carried power that
    /// `Condition::Powered` could not see, and the scheduler checks the same
    /// condition, so it was a real ceiling. Seed 31337's crude oil is 256–384
    /// tiles from spawn, which is four to six times the old bound.
    ///
    /// Poles are spaced 7 tiles, inside a small pole's 7.5 wire reach. 60
    /// poles reach ~420 tiles, well past the old 64.
    fn chained(poles: usize, spacing: f64) -> (PlanState, Position) {
        let mut s = state();
        for i in 0..poles {
            s.create_entity(FactorioEntity {
                name: "small-electric-pole".into(),
                position: Position::new(spacing * i as f64, 0.),
                ..Default::default()
            });
        }
        // The engine sits at the far end, beside the last pole.
        let far = spacing * (poles - 1) as f64;
        s.create_entity(FactorioEntity {
            name: "steam-engine".into(),
            position: Position::new(far, 2.),
            ..Default::default()
        });
        (s, Position::new(0., 0.))
    }

    #[test]
    fn a_generator_at_the_far_end_of_a_long_pole_run_is_found() {
        let (s, consumer) = chained(60, 7.);
        let distance = 7. * 59.;
        assert!(
            distance > 64. * 4.,
            "the fixture must reach well past the old bound: {distance}"
        );

        let kw = s.electric_supply_kw(&lab_area(&s, consumer));
        assert_eq!(
            kw, 900.,
            "a steam engine {distance} tiles away, wired the whole way, must count"
        );
    }

    /// The other half, and the one that stops this being "count everything".
    ///
    /// A generator beyond the last pole is **not** on the network, however
    /// close the poles get to it. Without this the traversal would be a
    /// licence to count any generator on the map.
    #[test]
    fn a_generator_past_the_last_pole_is_not_found() {
        let (mut s, consumer) = chained(60, 7.);
        // A second engine far beyond the end of the wire, unreachable.
        s.create_entity(FactorioEntity {
            name: "steam-engine".into(),
            position: Position::new(7. * 200., 0.),
            ..Default::default()
        });

        let kw = s.electric_supply_kw(&lab_area(&s, consumer));
        assert_eq!(
            kw, 900.,
            "only the wired engine counts; the isolated one must not"
        );
    }

    /// A gap wider than the wire reach breaks the run, and everything past the
    /// break stops counting. The physical rule, asserted rather than assumed.
    #[test]
    fn a_gap_wider_than_the_wire_reach_breaks_the_chain() {
        let mut s = state();
        // Two poles by the consumer, then a gap of 20 (over a small pole's
        // 7.5), then poles leading to the engine.
        for x in [0., 7.] {
            s.create_entity(FactorioEntity {
                name: "small-electric-pole".into(),
                position: Position::new(x, 0.),
                ..Default::default()
            });
        }
        for x in [27., 34., 41.] {
            s.create_entity(FactorioEntity {
                name: "small-electric-pole".into(),
                position: Position::new(x, 0.),
                ..Default::default()
            });
        }
        s.create_entity(FactorioEntity {
            name: "steam-engine".into(),
            position: Position::new(41., 2.),
            ..Default::default()
        });

        let kw = s.electric_supply_kw(&lab_area(&s, Position::new(0., 0.)));
        assert_eq!(
            kw, 0.,
            "the engine is past a 20-tile gap, wider than a small pole's 7.5 reach"
        );
    }

    /// The common case must stay one pass: a plant beside its consumer finds
    /// no pole to expand from beyond the first disc.
    ///
    /// Asserted because the traversal is on every `Powered` check, and a
    /// change that made the cheap case walk the map would be a real
    /// regression that no correctness test would notice.
    #[test]
    fn a_plant_beside_its_consumer_needs_no_traversal() {
        let s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        let near = s.electric_entities(&Position::new(10.5, 10.5));
        let one_disc = s.entities_within(&Position::new(10.5, 10.5), POWER_SEARCH_RADIUS);
        assert_eq!(
            near.len(),
            one_disc.len(),
            "with every pole already inside the seed disc, the traversal must add nothing"
        );
    }

    /// The end-to-end one: a power plant the **world** already carries, not
    /// one this plan placed.
    ///
    /// Every other test in this section builds its plant through
    /// `create_entity`, the expansion overlay, because until 2026-09-02
    /// `EntityGraph::add` dropped `electric-pole` and `generator` before they
    /// reached the entity tree and nothing could read their name. That is the
    /// gap that made every live world score 0 kW and every research refuse.
    /// This test goes through `FactorioSurface` instead, so it fails if that
    /// whitelist ever narrows again.
    #[test]
    fn a_power_plant_the_world_already_carries_reads_as_supply() {
        let world = fixture_world();
        world
            .update_chunk_entities(vec![
                world_entity(
                    "small-electric-pole",
                    "electric-pole",
                    10.5,
                    10.5,
                    0.296_875,
                    0.296_875,
                ),
                world_entity("steam-engine", "generator", 12.5, 10.5, 2.5, 4.695_312_5),
            ])
            .expect("adding the plant must not fail");
        let s = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert_eq!(
            s.electric_supply_kw(&lab_area(&s, Position::new(8.5, 8.5))),
            900.0,
            "a hand-built plant standing in the world is supply the plan can see"
        );
    }

    /// A solar panel the world carries is *readable* and still not credited.
    ///
    /// The two halves are separate claims and this is the one that could
    /// regress quietly: widening the whitelist makes the panel visible, and
    /// visible is one short step from counted. Its output is a function of the
    /// in-game clock, so counting it would make the same plan feasible or not
    /// according to when the run started.
    #[test]
    fn a_solar_panel_the_world_carries_is_visible_and_still_not_power() {
        let world = fixture_world();
        world
            .update_chunk_entities(vec![
                world_entity(
                    "small-electric-pole",
                    "electric-pole",
                    10.5,
                    10.5,
                    0.296_875,
                    0.296_875,
                ),
                world_entity(
                    "solar-panel",
                    "solar-panel",
                    13.5,
                    10.5,
                    2.796_875,
                    2.796_875,
                ),
            ])
            .expect("adding the panel must not fail");
        let s = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert!(
            s.entities_within(&Position::new(10.5, 10.5), 8.)
                .iter()
                .any(|entity| entity.name == "solar-panel"),
            "the panel has to be readable by name, or this asserts nothing"
        );
        assert_eq!(
            s.electric_supply_kw(&lab_area(&s, Position::new(8.5, 8.5))),
            0.0,
            "a solar panel is not deterministic generation"
        );
    }

    /// A `FactorioEntity` shaped the way the game reports one: with the
    /// bounding box it actually occupies. `EntityGraph::add` skips a
    /// zero-width box outright, so a default-constructed entity would never
    /// reach the tree and the test would pass for the wrong reason.
    fn world_entity(
        name: &str,
        entity_type: &str,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> FactorioEntity {
        FactorioEntity {
            name: name.into(),
            entity_type: entity_type.into(),
            position: Position::new(x, y),
            bounding_box: Rect::new(
                &Position::new(x - w / 2., y - h / 2.),
                &Position::new(x + w / 2., y + h / 2.),
            ),
            ..Default::default()
        }
    }

    /// A pole covering the site with a generator wired to it is power.
    #[test]
    fn a_covered_site_with_a_generator_on_its_network_has_supply() {
        let s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        assert_eq!(
            s.electric_supply_kw(&lab_area(&s, Position::new(8.5, 8.5))),
            900.0,
            "one steam engine, covered by the same pole as the site"
        );
    }

    /// **Coverage is not capacity.** The same pole, the same site, no
    /// generator: zero.
    ///
    /// This is the check that would have passed on the base run 30 actually
    /// had — `generated_kw = 0.0` in all 541 force samples — if it stopped at
    /// "a pole reaches it".
    #[test]
    fn a_pole_with_no_generator_supplies_nothing() {
        let s = powered(Some(Position::new(10.5, 10.5)), None);
        assert_eq!(
            s.electric_supply_kw(&lab_area(&s, Position::new(8.5, 8.5))),
            0.0
        );
    }

    /// And capacity is not coverage: a generator with no pole reaching the
    /// site is not power either.
    #[test]
    fn a_generator_with_no_pole_supplies_nothing() {
        let s = powered(None, Some(Position::new(12.5, 10.5)));
        assert_eq!(
            s.electric_supply_kw(&lab_area(&s, Position::new(8.5, 8.5))),
            0.0
        );
    }

    /// A generator on a *different* network contributes nothing.
    ///
    /// Two poles 20 tiles apart — well past a small pole's 7.5-tile wire
    /// reach — so they are two networks. The engine sits in the far pole's
    /// supply area, the site in the near one's. Both halves of the naive check
    /// pass (there is a pole here, there is a generator somewhere) and the
    /// answer is still nothing.
    #[test]
    fn a_generator_on_another_network_does_not_supply_the_site() {
        let mut s = powered(Some(Position::new(10.5, 10.5)), None);
        s.create_entity(FactorioEntity {
            name: "small-electric-pole".into(),
            position: Position::new(30.5, 10.5),
            ..Default::default()
        });
        s.create_entity(FactorioEntity {
            name: "steam-engine".into(),
            position: Position::new(32.5, 10.5),
            ..Default::default()
        });
        assert_eq!(
            s.electric_supply_kw(&lab_area(&s, Position::new(8.5, 8.5))),
            0.0,
            "20 tiles apart is two networks, not one"
        );

        // Bridge them with a pole in wire range of both, and the same engine
        // now counts — so the zero above is about connectivity and not about
        // the engine being unreadable.
        s.create_entity(FactorioEntity {
            name: "small-electric-pole".into(),
            position: Position::new(17.5, 10.5),
            ..Default::default()
        });
        s.create_entity(FactorioEntity {
            name: "small-electric-pole".into(),
            position: Position::new(24.5, 10.5),
            ..Default::default()
        });
        assert_eq!(
            s.electric_supply_kw(&lab_area(&s, Position::new(8.5, 8.5))),
            900.0,
            "two hops of 7 tiles each, inside a small pole's 7.5-tile reach"
        );
    }

    /// Solar is deliberately not credited: its output depends on the in-game
    /// time of day, and a planner whose output must be identical for identical
    /// inputs cannot make a feasibility decision on a number that is not.
    ///
    /// The steam engine in the same position *is* credited, so this is about
    /// the panel and not about the geometry.
    #[test]
    fn a_solar_panel_is_not_counted_as_generation() {
        let mut s = powered(Some(Position::new(10.5, 10.5)), None);
        s.create_entity(FactorioEntity {
            name: "solar-panel".into(),
            position: Position::new(12.5, 10.5),
            ..Default::default()
        });
        assert_eq!(
            s.electric_supply_kw(&lab_area(&s, Position::new(8.5, 8.5))),
            0.0,
            "a solar panel's output is a function of the clock"
        );

        let steam = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        assert_eq!(
            steam.electric_supply_kw(&lab_area(&steam, Position::new(8.5, 8.5))),
            900.0,
            "the same geometry with a steam engine does count"
        );
    }

    // ---- electric demand ---------------------------------------------------

    /// The demand walk answers over the same network the supply walk does.
    #[test]
    fn demand_counts_the_consumers_the_same_poles_reach() {
        let mut s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        let site = lab_area(&s, Position::new(8.5, 8.5));
        assert_eq!(s.electric_supply_kw(&site), 900.0);
        assert_eq!(
            s.electric_demand_kw(&site, None),
            0.0,
            "an engine and a pole are not consumers"
        );

        // A lab inside the pole's 5x5 supply box.
        s.create_entity(FactorioEntity {
            name: "lab".into(),
            position: Position::new(8.5, 8.5),
            ..Default::default()
        });
        assert_eq!(s.electric_demand_kw(&site, None), 60.0);

        // And an assembling machine beside it -- one with a recipe, since a
        // machine with none can never craft and is charged nothing.
        s.create_entity(FactorioEntity {
            name: "assembling-machine-1".into(),
            position: Position::new(11.5, 8.5),
            recipe: Some("iron-gear-wheel".into()),
            ..Default::default()
        });
        assert_eq!(s.electric_demand_kw(&site, None), 135.0, "60 + 75");
    }

    /// A crafting machine with no recipe set draws nothing but its drain, and
    /// is charged nothing.
    ///
    /// `run-1788608648-56109`, plan 4: five recipe-less machines from two
    /// cells earlier plans never finished were charged 375 kW of a 900 kW
    /// engine, the cell to build wanted 189, and the plan sited a third
    /// power plant. The lab beside them keeps its 60: it has no recipe to
    /// set and draws whenever it has packs.
    #[test]
    fn a_crafting_machine_with_no_recipe_is_not_charged() {
        let mut s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        let site = lab_area(&s, Position::new(8.5, 8.5));
        s.create_entity(FactorioEntity {
            name: "lab".into(),
            position: Position::new(8.5, 8.5),
            ..Default::default()
        });
        s.create_entity(FactorioEntity {
            name: "assembling-machine-1".into(),
            entity_type: "assembling-machine".into(),
            position: Position::new(11.5, 8.5),
            ..Default::default()
        });
        assert_eq!(
            s.electric_demand_kw(&site, None),
            60.0,
            "the lab, and not the machine nothing has set a recipe on"
        );
        s.set_recipe(&Position::new(11.5, 8.5), "iron-gear-wheel")
            .expect("the machine stands");
        assert_eq!(
            s.electric_demand_kw(&site, None),
            135.0,
            "and the moment a recipe goes on it, it is charged"
        );
    }

    /// `except` names a tile, and it excludes exactly that one.
    #[test]
    fn the_consumer_asked_about_is_the_one_left_out_of_its_own_budget() {
        let mut s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        for (name, pos) in [
            ("lab", Position::new(8.5, 8.5)),
            ("assembling-machine-1", Position::new(11.5, 8.5)),
        ] {
            s.create_entity(FactorioEntity {
                name: name.into(),
                position: pos,
                // Only the assembler reads it; a machine with none is not
                // charged.
                recipe: Some("iron-gear-wheel".into()),
                ..Default::default()
            });
        }
        let site = lab_area(&s, Position::new(8.5, 8.5));
        assert_eq!(s.electric_demand_kw(&site, None), 135.0);
        assert_eq!(
            s.electric_demand_kw(&site, Some(&Position::new(8.5, 8.5))),
            75.0,
            "the lab is left out and the assembler is not"
        );
        assert_eq!(
            s.electric_demand_kw(&site, Some(&Position::new(11.5, 8.5))),
            60.0,
            "and the other way round"
        );
        assert_eq!(
            s.electric_demand_kw(&site, Some(&Position::new(30.5, 30.5))),
            135.0,
            "a tile no consumer stands on excludes nothing"
        );
    }

    /// A consumer on another network is not this network's problem, exactly as
    /// a generator on another network is not this network's supply.
    #[test]
    fn demand_on_another_network_is_not_counted() {
        let mut s = powered(Some(Position::new(10.5, 10.5)), None);
        // Twenty tiles away, past a small pole's 7.5-tile wire reach.
        s.create_entity(FactorioEntity {
            name: "small-electric-pole".into(),
            position: Position::new(30.5, 10.5),
            ..Default::default()
        });
        s.create_entity(FactorioEntity {
            name: "lab".into(),
            position: Position::new(30.5, 8.5),
            ..Default::default()
        });
        let site = lab_area(&s, Position::new(8.5, 8.5));
        assert_eq!(
            s.electric_demand_kw(&site, None),
            0.0,
            "the far lab is on an island of its own"
        );

        // Bridge the two networks and the same lab now counts, so the zero
        // above is about connectivity and not about the lab being unreadable.
        for x in [17.5, 24.5] {
            s.create_entity(FactorioEntity {
                name: "small-electric-pole".into(),
                position: Position::new(x, 10.5),
                ..Default::default()
            });
        }
        assert_eq!(s.electric_demand_kw(&site, None), 60.0);
    }

    /// Burner machines draw coal, not kilowatts, and are absent from
    /// `consumer_kw` rather than zero-valued.
    #[test]
    fn a_burner_machine_spends_none_of_the_electric_budget() {
        let mut s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        // **Distinct tiles.** `create_entity` keys the overlay by floored
        // `Pos`, so three machines at one position are one machine — the first
        // draft of this test put all three at `(9.5, 11.5)` and passed a
        // mutation that gives `stone-furnace` a 90 kW electric draw, because
        // the furnace was never in the state at all.
        for (name, x, y) in [
            ("stone-furnace", 9.5, 11.5),
            ("burner-mining-drill", 11.5, 11.5),
            ("burner-inserter", 12.5, 8.5),
        ] {
            s.create_entity(FactorioEntity {
                name: name.into(),
                position: Position::new(x, y),
                ..Default::default()
            });
        }
        let site = lab_area(&s, Position::new(8.5, 8.5));
        // Each one really is standing, on the pole's own network, or the zero
        // below would be about absence rather than about units.
        let mut seen: Vec<String> = s
            .entities_within(&Position::new(10.5, 10.5), 8.)
            .into_iter()
            .filter(|e| e.name.starts_with("stone-") || e.name.starts_with("burner-"))
            .map(|e| e.name)
            .collect();
        seen.sort();
        assert_eq!(
            seen,
            vec!["burner-inserter", "burner-mining-drill", "stone-furnace"]
        );
        // And a consumer among them would be counted, so the walk reaches this
        // ground.
        let mut check = s.clone();
        check.create_entity(FactorioEntity {
            name: "lab".into(),
            position: Position::new(9.5, 11.5),
            ..Default::default()
        });
        assert_eq!(check.electric_demand_kw(&site, None), 60.0);

        assert_eq!(
            s.electric_demand_kw(&site, None),
            0.0,
            "a stone furnace's 90 kW is coal, and an entry for it here would \
             be a number in the wrong units"
        );
    }

    /// The anchor will not site a consumer on a network whose generation is
    /// already spoken for.
    #[test]
    fn the_supply_anchor_refuses_a_network_that_is_already_committed() {
        let mut s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        assert_eq!(
            s.nearest_supply_anchor(&Position::new(0., 0.), 64., 900.),
            Some(Position::new(10.5, 10.5)),
            "an idle engine has all 900 kW to give"
        );
        // One assembling machine of the largest kind, with a recipe on it:
        // 375 kW committed.
        s.create_entity(FactorioEntity {
            name: "assembling-machine-3".into(),
            position: Position::new(9.5, 8.5),
            recipe: Some("iron-gear-wheel".into()),
            ..Default::default()
        });
        assert_eq!(
            s.nearest_supply_anchor(&Position::new(0., 0.), 64., 900.),
            None,
            "525 kW is what is left, and 900 was asked for"
        );
        assert_eq!(
            s.nearest_supply_anchor(&Position::new(0., 0.), 64., 525.),
            Some(Position::new(10.5, 10.5)),
            "and 525 is exactly what is left"
        );
    }

    /// Two reads of the same state give the same number, bit for bit.
    ///
    /// A sum of `f64`s is order-dependent, so this is a claim about
    /// `entities_within`'s ordering reaching all the way through the demand
    /// walk, not about arithmetic.
    #[test]
    fn the_same_state_gives_the_same_demand_twice() {
        let mut s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        for (name, x, y) in [
            ("lab", 8.5, 8.5),
            ("assembling-machine-1", 11.5, 8.5),
            ("inserter", 9.5, 12.5),
            ("electric-mining-drill", 12.5, 8.5),
        ] {
            s.create_entity(FactorioEntity {
                name: name.into(),
                position: Position::new(x, y),
                // Only the assembler reads it; a machine with none is not
                // charged.
                recipe: Some("iron-gear-wheel".into()),
                ..Default::default()
            });
        }
        let site = lab_area(&s, Position::new(8.5, 8.5));
        let first = s.electric_demand_kw(&site, None);
        assert_eq!(first.to_bits(), s.electric_demand_kw(&site, None).to_bits());
        assert_eq!(first, 60.0 + 75.0 + 13.0 + 90.0);
    }

    /// The anchor a placement search uses: the nearest pole with generation on
    /// its own network, and `None` when there is none.
    #[test]
    fn the_supply_anchor_is_a_pole_with_generation_behind_it() {
        let s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        assert_eq!(
            s.nearest_supply_anchor(&Position::new(0., 0.), 64., 60.),
            Some(Position::new(10.5, 10.5))
        );
        // Out of range of the search, not out of range of the pole.
        assert_eq!(
            s.nearest_supply_anchor(&Position::new(0., 0.), 5., 60.),
            None
        );
        // And a demand the network cannot meet is refused rather than rounded.
        assert_eq!(
            s.nearest_supply_anchor(&Position::new(0., 0.), 64., 1_000.),
            None
        );
    }

    /// `entities_within` orders its answer, because a quad-tree query does
    /// not and this crate's output has to be byte-identical across runs.
    #[test]
    fn entities_within_comes_back_in_a_fixed_order() {
        let s = powered(
            Some(Position::new(10.5, 10.5)),
            Some(Position::new(12.5, 10.5)),
        );
        let names: Vec<String> = s
            .entities_within(&Position::new(0., 0.), 64.)
            .into_iter()
            .map(|e| format!("{} {}", e.name, e.position))
            .collect();
        for _ in 0..20 {
            let again: Vec<String> = s
                .entities_within(&Position::new(0., 0.), 64.)
                .into_iter()
                .map(|e| format!("{} {}", e.name, e.position))
                .collect();
            assert_eq!(names, again);
        }
        assert!(
            names.iter().any(|n| n.starts_with("small-electric-pole")),
            "the overlay's own entities are in it: {names:?}"
        );
    }

    #[test]
    fn a_second_furnace_one_tile_from_the_first_does_not_fit() {
        // The defect, in miniature. The neighbouring tile is empty by the tile
        // test -- the first furnace's *centre* is not on it -- but a stone
        // furnace is 1.398 tiles across, so the boxes overlap and the game
        // refuses the second placement. This is what sited the red-science
        // furnaces at [-34, -1] and [-34, 0].
        let mut s = state();
        let first = Position::new(0., -1.);
        assert!(s.is_area_free("stone-furnace", &first), "open ground");
        s.create_entity(entity_at_pos("stone-furnace", first.x, first.y));

        assert!(
            !s.is_area_free("stone-furnace", &Position::new(0., 0.)),
            "a furnace one tile from another overlaps it"
        );
        // Two tiles apart it does fit -- otherwise this test would also pass on
        // an `is_area_free` that simply always says no.
        assert!(
            s.is_area_free("stone-furnace", &Position::new(0., 1.)),
            "two tiles of clearance is enough for a 1.398-wide entity"
        );
    }

    #[test]
    fn a_wide_base_world_neighbour_outside_the_old_radius_still_blocks() {
        // Reviewer's case. `find_entities_in_radius`
        // (`core/src/graph/entity_graph.rs:132`) filters candidates on each
        // entity's *centre* point, not its footprint. A real `rock-huge` is
        // 3 tiles across (half-extent 1.5, per its prototype's collision
        // box), so a copy of it centred 1.75 tiles from the query centre
        // geometrically overlaps a stone furnace placed there -- but the old
        // radius, half the query box's own span plus a flat `1.`, topped out
        // at ~1.699 and so never asked about a neighbour that far out. This
        // builds the neighbour with its *real* collision box rather than
        // going through `spawn_rocks`/`new_rock`, which hard-codes every rock
        // to 1.2 wide regardless of name and would not reproduce the defect.
        let world = fixture_world();
        let neighbour_pos = Position::new(1.75, 0.);
        let collision_box = fixture_entity_prototypes()
            .get("rock-huge")
            .expect("fixture has rock-huge")
            .collision_box
            .clone();
        world
            .update_chunk_entities(vec![FactorioEntity {
                name: "rock-huge".into(),
                entity_type: "simple-entity".into(),
                position: neighbour_pos.clone(),
                bounding_box: add_to_rect(&collision_box, &neighbour_pos),
                ..Default::default()
            }])
            .unwrap();
        let s = PlanState::from_world(Arc::new(world), &[]);

        assert!(
            !s.is_area_free("stone-furnace", &Position::new(0., 0.)),
            "a stone furnace at the origin overlaps the rock-huge's real \
             footprint, even though the rock's centre is 1.75 away and the \
             old query radius topped out at ~1.70"
        );
    }

    #[test]
    fn how_much_room_is_needed_comes_from_the_prototype() {
        // Identical geometry, two tiles apart, and the answer differs by
        // prototype: 1.398 across fits, 2.398 across does not. Neither number
        // appears here, and a fixed size -- whatever it was -- would have to
        // give these two the same answer.
        let mut small = state();
        small.create_entity(entity_at_pos("stone-furnace", 0., -1.));
        assert!(
            small.is_area_free("stone-furnace", &Position::new(0., 1.)),
            "a stone furnace fits two tiles from another"
        );

        let mut large = state();
        large.create_entity(entity_at_pos("assembling-machine-1", 0., -1.));
        assert!(
            !large.is_area_free("assembling-machine-1", &Position::new(0., 1.)),
            "an assembling machine does not fit in the same gap"
        );
    }

    /// The defect that stuck run `run-1788309767-54739` on its first action.
    ///
    /// `fixture_world` spawns 100 trees around `(-20, -20)` and a 4x4 patch of
    /// `player_collidable` water around `(40, 40)` — the same two obstacle
    /// classes a real map is full of, and the same two `EntityGraph::add`
    /// keeps out of `entity_tree`. Before `is_area_clear` read
    /// `blocking_boxes_within`, both of these read as open ground and the
    /// planner sited furnaces on them; the game answered
    /// `can_place_entity said 'no'` and the run had no way forward.
    #[test]
    fn a_furnace_does_not_fit_on_a_tree_or_in_water() {
        let s = state();
        assert!(
            !s.is_area_free("stone-furnace", &Position::new(-20., -20.)),
            "the fixture's trees are centred here; a furnace cannot go on them"
        );
        assert!(
            !s.is_area_free("stone-furnace", &Position::new(40., 40.)),
            "the fixture's water is here; a furnace cannot go on it"
        );
        // Open ground a few tiles from either, so this cannot pass on an
        // `is_area_free` that simply always says no.
        assert!(
            s.is_area_free("stone-furnace", &Position::new(0., -1.)),
            "open ground still takes a furnace"
        );
    }

    /// The fixture's iron field, dead centre. Every tile a
    /// `burner-mining-drill`'s 1.398-wide box touches here is ore.
    fn on_the_iron() -> Position {
        Position::new(-40., 40.)
    }

    /// **A mining drill must be able to stand on the ore it mines.**
    ///
    /// `is_area_clear_of` counted resource tiles as occupancy for every
    /// entity, so the one machine whose whole purpose is to stand on ore could
    /// not be sited anywhere on any map — not "rarely", not "on a crowded
    /// patch": a drill is *defined* by standing on a resource, so the refusal
    /// was total. Nothing had hit it because the only thing this planner had
    /// ever placed was a stone furnace, which wants to be near ore and never
    /// on it.
    ///
    /// Since `ore-does-not-block` this is no longer an exemption but the
    /// general answer: ore occupies nothing, for a drill or for anything else.
    /// See the sibling test below.
    #[test]
    fn a_mining_drill_may_stand_on_the_ore_it_mines() {
        let s = state();
        assert!(
            s.is_area_free("burner-mining-drill", &on_the_iron()),
            "a burner drill on iron ore is the whole of stage 1"
        );
        assert!(
            s.is_area_free("electric-mining-drill", &on_the_iron()),
            "and the electric one, whose box is 2.797 across, is still all ore"
        );
    }

    /// **Ore blocks nothing, because nothing collides with it.**
    ///
    /// A resource entity's whole collision mask is the `resource` layer and no
    /// buildable prototype carries that layer — 12 of 1,028 live 2.1.17
    /// prototypes name it and all 12 are resources, asserted in
    /// `crates/core/tests/live_2_1_payloads.rs`'s
    /// `nothing_buildable_collides_with_the_resource_layer` — so the game
    /// builds a belt, a pole, a furnace or an assembler straight over a patch,
    /// which a live `can_place_entity` confirms.
    ///
    /// This planner refused all of that until `ore-does-not-block`, as a
    /// *policy* ("do not bury the patch you are about to mine") stated as if it
    /// were a collision rule. It failed in the safe direction and so was never
    /// caught: legal ground came back as `NoRoute` and `NoSiteFound`, a belt
    /// route could not cross a patch, and `MinerLine` could not site at any
    /// radius because its own belt-and-pole corridor ran over the ore its
    /// drills need. What survives of the policy is
    /// [`PlanState::covers_claimed_resource`], which is the part that was
    /// always the real requirement: do not build over ore *this plan has
    /// promised a bot will mine*.
    #[test]
    fn ore_blocks_nothing_because_nothing_collides_with_it() {
        let s = state();
        assert!(
            s.is_area_free("stone-furnace", &on_the_iron()),
            "the game allows a furnace on ore, so the planner must"
        );
        assert!(
            s.is_area_free("assembling-machine-1", &on_the_iron()),
            "and an assembler"
        );
        assert!(
            s.is_area_free("transport-belt", &on_the_iron()),
            "a belt crossing a patch is the routing case this cost us"
        );
        assert!(
            s.is_area_free("small-electric-pole", &on_the_iron()),
            "and a pole beside it is the other half of a MinerLine corridor"
        );
        assert!(
            s.is_position_free(&on_the_iron()),
            "the tile-granularity question names no entity, and now needs to \
             name none: ore occupies no tile"
        );
        assert_eq!(
            s.placement_occupant("stone-furnace", &on_the_iron(), Direction::North),
            None,
            "and nothing is named as being in the way, because nothing is"
        );
    }

    /// Ore stopping being an obstacle does not make anything else stop being
    /// one.
    ///
    /// Without this the change could be "a footprint on ore fits whatever else
    /// is there", which would pass the test above and put machines in lakes and
    /// inside each other.
    #[test]
    fn dropping_ore_drops_only_ore() {
        let mut s = state();
        assert!(
            !s.is_area_free("burner-mining-drill", &Position::new(-20., -20.)),
            "the fixture's trees still stop a drill"
        );
        assert!(
            !s.is_area_free("burner-mining-drill", &Position::new(40., 40.)),
            "and so does the fixture's water"
        );
        s.create_entity(entity_at_pos("stone-furnace", -40., 40.));
        assert!(
            !s.is_area_free("burner-mining-drill", &on_the_iron()),
            "and a machine already standing on the ore is still in the way — \
             ore is not an obstacle, but everything standing on it still is"
        );
    }

    /// `stands_on_resources` answers a question about the MACHINE — does it
    /// need ore underfoot to work — and an entity the world has no prototype
    /// for answers `false`.
    ///
    /// An unknown name is not something this can vouch for, and its callers
    /// (`method::blueprint`'s `drills_are_fed` and `nearest_ore_seed`) read a
    /// `true` as licence to site *at* ore. It decides no placement: since
    /// `ore-does-not-block` nothing in `occupant_of` consults it.
    #[test]
    fn stands_on_resources_names_the_machines_that_need_ore_underfoot() {
        let s = state();
        assert!(s.stands_on_resources("burner-mining-drill"));
        assert!(s.stands_on_resources("electric-mining-drill"));
        assert!(
            s.stands_on_resources("pumpjack"),
            "a pumpjack is a mining drill that stands on crude oil, and crude \
             oil is a resource like any other"
        );
        assert!(!s.stands_on_resources("stone-furnace"));
        assert!(
            !s.stands_on_resources("not-a-real-entity"),
            "an unknown name is not vouched for"
        );
    }

    /// A tree the plan has chopped is no longer a source of wood.
    ///
    /// This is the whole of what stops two chops in one plan swinging at the
    /// same tree: `Effect::RemoveEntity` puts the tile in `removed`, and this
    /// is where `removed` is honoured. Without it a plan needing eight wood
    /// would emit two actions at one position, and the second would fail at
    /// the game with "no entity to mine" -- about a tree the first action had
    /// just taken.
    #[test]
    fn a_chopped_tree_stops_being_a_source() {
        let at = Position::new(5.5, 5.5);
        let world = factorio_bot_core::test_utils::fixture_world();
        world
            .update_chunk_entities(vec![FactorioEntity {
                name: "tree-01".into(),
                entity_type: "tree".into(),
                bounding_box: factorio_bot_core::factorio::util::add_to_rect(
                    &factorio_bot_core::types::Rect::from_wh(0.8, 0.8),
                    &at,
                ),
                position: at.clone(),
                ..Default::default()
            }])
            .expect("a fixture world accepts a tree");
        let mut s = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        assert_eq!(
            s.minable_sources("wood"),
            vec![("tree-01".to_string(), at.clone(), 4)]
        );
        assert!(s.has_minable_source("wood"));

        s.remove_entity(&at);
        assert!(
            s.minable_sources("wood").is_empty(),
            "the overlay hides it even though the base world still holds it"
        );
        assert!(!s.has_minable_source("wood"));
    }

    /// A tree the plan has already mined out of the way must stop blocking.
    ///
    /// `blocked_tree` carries no name and no position, only a rectangle, so
    /// the `removed` ledger is matched against the tile the rectangle is
    /// centred on. Getting that key wrong would leave every removal invisible
    /// on this path, which is silent: the placement would simply never be
    /// planned.
    #[test]
    fn removing_a_tree_frees_the_ground_under_it() {
        let mut s = state();
        let tree = Position::new(-20., -20.);
        assert!(!s.is_area_free("stone-furnace", &tree));
        // The fixture's trees are one per tile and 0.8 across, so a tree
        // centred a whole tile away still reaches into a furnace's 1.398-wide
        // box: the 3x3 around the site, not the 2x2 under it, is what has to
        // go.
        for x in -21..=-19 {
            for y in -21..=-19 {
                s.remove_entity(&Position::new(x as f64, y as f64));
            }
        }
        assert!(
            s.is_area_free("stone-furnace", &tree),
            "with the trees mined the ground is buildable again"
        );
    }

    #[test]
    fn an_entity_with_no_prototype_is_refused_rather_than_guessed_at() {
        let s = state();
        assert!(
            s.collision_area("not-a-real-entity", &Position::new(0., 0.))
                .is_none()
        );
        assert!(
            !s.is_area_free("not-a-real-entity", &Position::new(0., 0.)),
            "an unknown size must fail to plan, not be assumed small"
        );
    }

    /// Drives the same `boxes_overlap` the real box-against-box check uses,
    /// with an actual character-sized box standing at the computed distance.
    ///
    /// Not a ghost check: ghosts do not collide (`only_ghosts = true`
    /// validates nothing, per `CLAUDE.md`'s note on the trap), so a test that
    /// only asked `is_area_free`/a ghost placement to succeed would prove
    /// nothing about whether the clearance is actually big enough. This asks
    /// the geometry question directly, against the real fixture collision
    /// boxes.
    ///
    /// The diagonal, not an axis, because two axis-aligned squares first
    /// touch along their diagonal at exactly the sum of their half-diagonals —
    /// on an axis the true threshold is the (smaller) sum of half-*widths*,
    /// so an axis-aligned probe would pass even for a clearance far too small
    /// to be safe in the worst-case orientation.
    #[test]
    fn placement_clearance_keeps_the_characters_own_box_off_the_footprint() {
        let s = state();
        let clearance = s
            .placement_clearance("stone-furnace")
            .expect("fixture has a stone-furnace prototype");

        let furnace_box = s
            .collision_area("stone-furnace", &Position::new(0., 0.))
            .expect("fixture has a stone-furnace prototype");
        let character_box = fixture_entity_prototypes()
            .get("character")
            .expect("fixture has a character prototype")
            .collision_box
            .clone();

        let diag = std::f64::consts::FRAC_1_SQRT_2;
        let stand_at_clearance = Position::new(clearance * diag, clearance * diag);
        assert!(
            !boxes_overlap(
                &furnace_box,
                &add_to_rect(&character_box, &stand_at_clearance)
            ),
            "a character standing at the computed clearance, on the diagonal, \
             must clear the furnace's own footprint"
        );

        // A few centimetres inside it, same direction, must overlap -- or the
        // clearance is generous enough to pass regardless of what it actually
        // computed.
        let too_close = clearance - 0.05;
        let stand_too_close = Position::new(too_close * diag, too_close * diag);
        assert!(
            boxes_overlap(&furnace_box, &add_to_rect(&character_box, &stand_too_close)),
            "0.05 tiles inside the computed clearance must still collide on \
             the diagonal, or this test proves nothing"
        );
    }

    #[test]
    fn placement_clearance_grows_with_the_entity_and_is_none_for_an_unknown_one() {
        let s = state();
        let furnace = s
            .placement_clearance("stone-furnace")
            .expect("fixture has a stone-furnace prototype");
        let assembler = s
            .placement_clearance("assembling-machine-1")
            .expect("fixture has an assembling-machine-1 prototype");
        assert!(
            assembler > furnace,
            "a bigger entity needs more clearance: furnace {furnace}, assembler {assembler}"
        );
        assert!(
            s.placement_clearance("not-a-real-entity").is_none(),
            "an unknown size must not be guessed at, same as `collision_area`"
        );
    }

    #[test]
    fn a_placed_entity_carries_the_footprint_its_prototype_gives_it() {
        // Methods build a `FactorioEntity` from a name and a position and leave
        // `bounding_box` at zero; a zero box collides with nothing, so the fill
        // in `create_entity` is what makes the overlap test above possible.
        let mut s = state();
        assert_eq!(
            entity_at_pos("stone-furnace", 0., -1.).bounding_box.width(),
            0.
        );
        s.create_entity(entity_at_pos("stone-furnace", 0., -1.));
        let stored = s.entity_at(&Position::new(0., -1.)).expect("placed");
        assert!(
            stored.bounding_box.width() > 1.,
            "expected the prototype's 1.398, got {}",
            stored.bounding_box.width()
        );
    }

    #[test]
    fn fork_shares_the_base_world() {
        let a = state();
        let b = a.fork();
        assert!(Arc::ptr_eq(a.base(), b.base()));
    }

    #[test]
    fn fork_isolates_inventory_changes() {
        let a = state();
        let mut b = a.fork();
        b.gain(BotId(1), "iron-ore", 5);
        assert_eq!(b.inventory_count(BotId(1), "iron-ore"), 5);
        assert_eq!(a.inventory_count(BotId(1), "iron-ore"), 0);
    }

    /// `item_totals` is the roster's inventory, not one bot's.
    ///
    /// It exists so the driver can diff a chain's produce, and a diff that saw
    /// only the bot it happened to start from would under-count every chain
    /// whose actor is anyone else — silently, since a smaller diff just
    /// reserves less.
    #[test]
    fn item_totals_reports_every_bots_holdings() {
        let mut a = state();
        a.gain(BotId(1), "iron-plate", 3);
        a.gain(BotId(2), "iron-plate", 4);
        a.gain(BotId(2), "coal", 1);
        let totals = a.item_totals();
        assert_eq!(totals.get("iron-plate"), Some(&7));
        assert_eq!(totals.get("coal"), Some(&1));
        assert_eq!(totals.get("wood"), None, "nothing is invented");
    }

    #[test]
    fn total_count_sums_across_bots() {
        let mut a = state();
        a.gain(BotId(1), "iron-plate", 3);
        a.gain(BotId(2), "iron-plate", 4);
        assert_eq!(a.total_count("iron-plate"), 7);
    }

    #[test]
    fn lose_more_than_held_is_an_error() {
        let mut a = state();
        a.gain(BotId(1), "coal", 2);
        assert!(a.lose(BotId(1), "coal", 3).is_err());
        assert_eq!(a.inventory_count(BotId(1), "coal"), 2);
    }

    #[test]
    fn a_bot_takes_its_position_inventory_and_reach_from_its_player() {
        // The only path a real world takes into the planner: `fixture_world()`
        // has no players, so every other test exercises the `None` arm and the
        // `Some` arm's field reads run nowhere else.
        use factorio_bot_core::types::FactorioPlayer;

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                position: Position::new(12.5, -7.5),
                main_inventory: BTreeMap::from([("iron-plate".to_string(), 42u32)]),
                build_distance: 12,
                reach_distance: 8,
                resource_reach_distance: 4.0,
                ..Default::default()
            },
        );

        let s = PlanState::from_world(Arc::new(world), &[BotId(1), BotId(2)]);
        let bot = s.bot(BotId(1)).expect("bot 1 exists");
        assert_eq!(bot.position, Position::new(12.5, -7.5));
        assert_eq!(s.inventory_count(BotId(1), "iron-plate"), 42);
        assert_eq!(bot.build_distance, 12.0);
        assert_eq!(bot.reach_distance, 8.0);
        assert_eq!(bot.resource_reach_distance, 4.0);

        // Bot 2 has no player and still falls back to the defaults.
        let other = s.bot(BotId(2)).expect("bot 2 exists");
        assert_eq!(other.position, Position::new(0., 0.));
        assert_eq!(other.build_distance, 10.0);

        // The fallback is recorded: bot 2 got invented data, bot 1 did not.
        assert_eq!(s.unknown_bots(), &BTreeSet::from([BotId(2)]));
    }

    #[test]
    fn a_bot_with_a_real_player_is_not_unknown() {
        // Guards the flag in isolation from the fallback-data test above: a
        // `PlanState` built entirely from real players must report no unknown
        // bots at all, not merely "fewer than requested".
        use factorio_bot_core::types::FactorioPlayer;

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                ..Default::default()
            },
        );

        let s = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert!(
            s.unknown_bots().is_empty(),
            "bot 1 has a real player and must not be flagged unknown"
        );
    }

    #[test]
    fn missing_bots_get_default_reach_distances() {
        let a = state();
        let bot = a.bot(BotId(1)).expect("bot 1 exists");
        assert_eq!(bot.build_distance, 10.0);
        assert_eq!(bot.reach_distance, 10.0);
        assert_eq!(bot.resource_reach_distance, 3.0);
    }

    fn iron_ore_tile(state: &PlanState) -> Position {
        state
            .resource_patches("iron-ore")
            .first()
            .expect("fixture_world has an iron-ore patch")
            .elements
            .first()
            .expect("patch has tiles")
            .clone()
    }

    #[test]
    fn patch_tiles_come_back_sorted() {
        let a = state();
        for patch in a.resource_patches("iron-ore") {
            let mut sorted = patch.elements.clone();
            sorted.sort_by(|l, r| l.x.total_cmp(&r.x).then(l.y.total_cmp(&r.y)));
            assert_eq!(
                patch.elements, sorted,
                "every patch's tiles must come back sorted by (x, y)"
            );
        }
        // Deliberately not asserted: that two calls return the same patches.
        // They do not — see `resource_patches` for why, and why the planner
        // cannot fix it.
    }

    #[test]
    fn consuming_ore_reduces_what_is_available() {
        let mut a = state();
        let pos = iron_ore_tile(&a);
        let before = a.resource_available(&pos, "iron-ore");
        assert!(before > 0, "fixture ore tile should hold ore");
        a.consume_resource(&pos, "iron-ore", 1).unwrap();
        assert_eq!(a.resource_available(&pos, "iron-ore"), before - 1);
    }

    #[test]
    fn consuming_more_ore_than_present_is_an_error() {
        let mut a = state();
        let pos = iron_ore_tile(&a);
        let all = a.resource_available(&pos, "iron-ore");
        assert!(a.consume_resource(&pos, "iron-ore", all + 1).is_err());
        assert_eq!(a.resource_available(&pos, "iron-ore"), all);
    }

    #[test]
    fn consuming_ore_does_not_affect_the_fork_parent() {
        let a = state();
        let pos = iron_ore_tile(&a);
        let before = a.resource_available(&pos, "iron-ore");
        let mut b = a.fork();
        b.consume_resource(&pos, "iron-ore", 1).unwrap();
        assert_eq!(a.resource_available(&pos, "iron-ore"), before);
    }

    #[test]
    fn created_entities_occupy_their_position() {
        let mut a = state();
        let pos = Position::new(3., 3.);
        assert!(a.is_position_free(&pos));
        let furnace = FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: pos.clone(),
            ..Default::default()
        };
        a.create_entity(furnace);
        assert!(!a.is_position_free(&pos));
        assert_eq!(a.entity_at(&pos).unwrap().name, "stone-furnace");
    }

    #[test]
    fn removed_entities_free_their_position() {
        let mut a = state();
        let pos = Position::new(3., 3.);
        let furnace = FactorioEntity {
            name: "stone-furnace".into(),
            position: pos.clone(),
            ..Default::default()
        };
        a.create_entity(furnace);
        a.remove_entity(&pos);
        assert!(a.is_position_free(&pos));
        assert!(a.entity_at(&pos).is_none());
    }

    /// A tile holding ore is free ground, and the ore is still there.
    ///
    /// Both halves matter. The planner counted an ore tile as occupied until
    /// `ore-does-not-block`, which is what refused a belt route across a patch
    /// -- `Condition::PositionFree` is the belt lane's own check
    /// (`method::assemble`) -- while the game builds over ore happily. And the
    /// ore does not stop existing because nothing blocks on it: the ledger that
    /// says how much is there is untouched, which is what a drill sited here
    /// will ask.
    #[test]
    fn a_tile_holding_ore_is_free_ground() {
        let a = state();
        let ore = a
            .resource_patches("iron-ore")
            .into_iter()
            .flat_map(|p| p.elements)
            .next()
            .expect("fixture has iron ore");
        assert!(a.resource_available(&ore, "iron-ore") > 0);
        // Resources never reach the entity tree, so `entity_at` is blind to
        // them -- and occupancy is too, on purpose.
        assert!(a.entity_at(&ore).is_none());
        assert!(
            a.is_position_free(&ore),
            "ore tile {:?} was reported occupied",
            ore
        );
    }

    #[test]
    fn research_defaults_to_unresearched_and_can_be_set() {
        let mut a = state();
        assert!(!a.is_researched("automation"));
        a.set_researched("automation");
        assert!(a.is_researched("automation"));
    }

    /// A reservation is a claim on stock, not a withdrawal of it. Both halves
    /// are asserted: `available` must fall, and the inventory the emitted plan
    /// is checked against must not move at all — a reservation that debited
    /// the inventory would make `Effect::LoseItem` fail on items the bot
    /// really holds.
    #[test]
    fn a_reservation_hides_stock_from_available_without_spending_it() {
        let mut a = state();
        a.gain(BotId(1), "iron-plate", 10);
        let whose = Holder::Share(BotId(1));

        assert_eq!(a.available(&whose, "iron-plate"), 10);
        a.reserve(&whose, "iron-plate", 4);
        assert_eq!(a.available(&whose, "iron-plate"), 6, "four are spoken for");
        assert_eq!(
            a.inventory_count(BotId(1), "iron-plate"),
            10,
            "but none have been spent"
        );
        a.lose(BotId(1), "iron-plate", 10)
            .expect("a reservation must not block spending what is really held");

        a.release(&whose, "iron-plate", 4);
        a.gain(BotId(1), "iron-plate", 10);
        assert_eq!(a.available(&whose, "iron-plate"), 10, "and released again");
    }

    /// Reservations add up rather than overwrite: two claims on the same item
    /// hide both, which is the whole point when one recipe's ingredients each
    /// reduce to the same intermediate.
    #[test]
    fn two_reservations_on_one_item_hide_both() {
        let mut a = state();
        a.gain(BotId(1), "iron-gear-wheel", 10);
        let whose = Holder::Share(BotId(1));
        a.reserve(&whose, "iron-gear-wheel", 6);
        a.reserve(&whose, "iron-gear-wheel", 3);
        assert_eq!(a.available(&whose, "iron-gear-wheel"), 1);
        a.release(&whose, "iron-gear-wheel", 6);
        assert_eq!(a.available(&whose, "iron-gear-wheel"), 7);
    }

    /// The two ledgers, and the deliberate asymmetry between them. A bot's
    /// reservation names a bot, so it lowers the roster total too. An
    /// `Anyone` reservation names none, so it lowers the total and nobody's
    /// individual figure.
    #[test]
    fn a_bot_reservation_lowers_the_roster_total_but_an_anyone_reservation_lowers_no_bot() {
        let mut a = state();
        a.gain(BotId(1), "coal", 4);
        a.gain(BotId(2), "coal", 6);
        assert_eq!(a.available(&Holder::Anyone, "coal"), 10);

        a.reserve(&Holder::Share(BotId(1)), "coal", 3);
        assert_eq!(a.available(&Holder::Share(BotId(1)), "coal"), 1);
        assert_eq!(
            a.available(&Holder::Share(BotId(2)), "coal"),
            6,
            "one bot's claim says nothing about another's stock"
        );
        assert_eq!(
            a.available(&Holder::Anyone, "coal"),
            7,
            "but it is spoken for as far as the roster is concerned"
        );

        a.reserve(&Holder::Anyone, "coal", 2);
        assert_eq!(a.available(&Holder::Anyone, "coal"), 5);
        assert_eq!(
            a.available(&Holder::Share(BotId(2)), "coal"),
            6,
            "an Anyone claim names no bot, so it cannot be charged to one"
        );
    }

    /// Releasing more than was reserved clamps at zero rather than wrapping,
    /// which would otherwise hand a caller `u32::MAX` items of headroom.
    #[test]
    fn releasing_more_than_was_reserved_clamps_at_nothing_reserved() {
        let mut a = state();
        a.gain(BotId(1), "stone", 5);
        let whose = Holder::Share(BotId(1));
        a.reserve(&whose, "stone", 2);
        a.release(&whose, "stone", 9);
        assert_eq!(a.available(&whose, "stone"), 5);
    }

    /// The separation is a *derived* number, not a tuned one: it is the reach
    /// the game enforces plus the radius at which a character stops standing
    /// on a tile. Both halves are pinned here so a change to either shows up
    /// as a change to this, rather than silently.
    #[test]
    fn the_mining_separation_is_reach_plus_the_occupancy_radius() {
        let a = state();
        let reach = a.bot(BotId(1)).expect("bot 1").resource_reach_distance;
        let occupancy = a.mining_tile_separation() - reach;
        // The fixture world's `character` box, or the vanilla fallback: either
        // way, half a tile plus half a character on each axis, corner to
        // corner.
        let (half_x, half_y) = character_half_box(a.base());
        assert!(
            (occupancy - (TILE_HALF_SIDE + half_x).hypot(TILE_HALF_SIDE + half_y)).abs() < 1e-12,
            "separation {} is not reach {} plus the occupancy radius",
            a.mining_tile_separation(),
            reach
        );
    }

    /// The occupancy radius has to be the *supremum* of
    /// `character_stands_on_tile`, or the separation built on it is either
    /// unsafe (too small) or wasteful (too large). Walked in a fine ring
    /// rather than argued.
    #[test]
    fn the_occupancy_radius_is_exactly_where_a_character_stops_standing_on_a_tile() {
        let a = state();
        let tile = Position::new(-40.5, 40.5);
        let radius =
            a.mining_tile_separation() - a.bot(BotId(1)).expect("bot 1").resource_reach_distance;
        for step in 0..720 {
            let angle = std::f64::consts::TAU * f64::from(step) / 720.;
            let (sin, cos) = angle.sin_cos();
            let outside = Position::new(
                tile.x() + cos * (radius + 1e-6),
                tile.y() + sin * (radius + 1e-6),
            );
            assert!(
                !a.character_stands_on_tile(&outside, &tile),
                "a character {radius} out at {angle} rad still stands on the tile"
            );
        }
        // And it is not slack: the corner direction still overlaps just inside.
        let diagonal = std::f64::consts::FRAC_1_SQRT_2 * (radius - 1e-6);
        assert!(a.character_stands_on_tile(
            &Position::new(tile.x() + diagonal, tile.y() + diagonal),
            &tile
        ));
    }

    /// A player with no character reports `f64::MAX` here. Propagating that
    /// would crowd every tile on the map out of every plan.
    #[test]
    fn an_unbounded_reach_does_not_become_an_unbounded_separation() {
        use factorio_bot_core::types::FactorioPlayer;

        let world = fixture_world();
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                resource_reach_distance: f64::MAX,
                ..Default::default()
            },
        );
        let a = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert!(
            a.mining_tile_separation() < 5.,
            "separation ran away: {}",
            a.mining_tile_separation()
        );
    }

    /// The whole point of storing the claim's `Position` beside its `Pos` key:
    /// a tile centre must not be rounded down before a distance is measured
    /// to it.
    #[test]
    fn crowding_is_measured_from_tile_centres_not_from_floored_keys() {
        let mut a = state();
        let tile = Position::new(-40.5, 40.5);
        a.claim_resource(&tile);

        let separation = a.mining_tile_separation();
        // A tile just outside the separation measured from the *centre*, and
        // just inside it if the claim had been floored to (-41, 40).
        let outside = Position::new(-40.5 + separation + 0.01, 40.5);
        assert!(
            !a.is_resource_crowded(&outside),
            "measured from the floored key, this would read as crowded"
        );
        let inside = Position::new(-40.5 + separation - 0.01, 40.5);
        assert!(a.is_resource_crowded(&inside));
    }

    /// Claimed and crowded are separate questions, and `resource_unclaimed`
    /// asks both. A tile is never crowded by its own claim.
    #[test]
    fn a_tile_next_to_a_claim_is_crowded_without_being_claimed() {
        let mut a = state();
        let tile = Position::new(-40.5, 40.5);
        let neighbour = Position::new(-39.5, 40.5);
        a.claim_resource(&tile);

        assert!(a.is_resource_claimed(&tile));
        assert!(
            !a.is_resource_crowded(&tile),
            "a tile must not crowd itself, or the two predicates cannot be read apart"
        );
        assert!(!a.is_resource_claimed(&neighbour));
        assert!(a.is_resource_crowded(&neighbour));

        assert_eq!(a.resource_unclaimed(&neighbour, "iron-ore"), 0);
        assert_eq!(
            a.resource_available(&neighbour, "iron-ore"),
            DEFAULT_RESOURCE_PER_TILE,
            "crowding is a fact about the plan, not about the ground"
        );
    }

    /// The ceiling this lifts, stated at the smallest scale it exists at.
    ///
    /// One bot mining two neighbouring tiles cannot be standing on the second
    /// while it mines the first: `schedule` runs a bot's actions one at a
    /// time. So a claim on a bot's own timeline must not crowd that bot out of
    /// the tile beside it, however close it is.
    #[test]
    fn a_runner_is_not_crowded_by_its_own_claim() {
        let mut a = state();
        a.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));
        let tile = Position::new(-40.5, 40.5);
        let neighbour = Position::new(-39.5, 40.5);
        a.claim_resource(&tile);

        assert!(
            calculate_distance(&tile, &neighbour) < a.mining_tile_separation(),
            "the fixture must put these two inside the separation, or this \
             test asserts nothing"
        );
        assert!(
            !a.is_resource_crowded(&neighbour),
            "a bot cannot stand on its own next tile while mining this one"
        );
        assert_eq!(
            a.resource_unclaimed(&neighbour, "iron-ore"),
            DEFAULT_RESOURCE_PER_TILE
        );
    }

    /// The negative control for `a_runner_is_not_crowded_by_its_own_claim`:
    /// two *different* bots really can be there at once, so the separation
    /// stands. Without this the relaxation could be "nothing crowds anything"
    /// and both tests would pass.
    #[test]
    fn a_claim_on_another_bots_timeline_still_crowds() {
        let mut a = state();
        a.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));
        let tile = Position::new(-40.5, 40.5);
        let neighbour = Position::new(-39.5, 40.5);
        a.claim_resource(&tile);

        a.set_claim_runner(Some(ClaimRunner::Bot(BotId(2))));
        assert!(a.is_resource_crowded(&neighbour));
        assert_eq!(a.resource_unclaimed(&neighbour, "iron-ore"), 0);
    }

    /// An unowned chain is still **one** chain, and `schedule`'s
    /// `chain_binding` puts a whole chain on one bot. So its own claims are
    /// serial with each other — and with nobody else's, because which bot it
    /// landed on is not known here.
    #[test]
    fn an_unowned_chain_is_one_timeline_and_only_its_own() {
        let mut a = state();
        let tile = Position::new(-40.5, 40.5);
        let neighbour = Position::new(-39.5, 40.5);

        a.set_claim_runner(Some(ClaimRunner::Chain(ChainId(7))));
        a.claim_resource(&tile);
        assert!(
            !a.is_resource_crowded(&neighbour),
            "one chain is one runner, whoever it turns out to be"
        );

        a.set_claim_runner(Some(ClaimRunner::Chain(ChainId(8))));
        assert!(
            a.is_resource_crowded(&neighbour),
            "two chains may be two bots at once"
        );
        a.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));
        assert!(
            a.is_resource_crowded(&neighbour),
            "an unowned chain may be bot 1's, and may not; unknown is not a match"
        );
    }

    /// `None` is *unknown*, not a wildcard — and two unknowns are not each
    /// other. An action in no chain at all stays individually assignable, so
    /// two of them may run on two bots at the same moment.
    ///
    /// This is also what keeps every pre-existing caller of `claim_resource`
    /// — the tests in this crate, and `PlanState::consume_resource` reached
    /// outside an expansion — behaving exactly as it did.
    #[test]
    fn an_unknown_runner_matches_nothing_including_another_unknown() {
        let mut a = state();
        let tile = Position::new(-40.5, 40.5);
        let neighbour = Position::new(-39.5, 40.5);
        a.claim_resource(&tile);

        assert_eq!(a.claim_runner(), None);
        assert!(a.is_resource_crowded(&neighbour));
        assert!(a.is_resource_crowded_for(&neighbour, Some(ClaimRunner::Bot(BotId(1)))));
    }

    /// A tile the world never stated an amount for keeps the old rule: the
    /// planner would be spending [`DEFAULT_RESOURCE_PER_TILE`], a number
    /// nobody read, so a bot is still refused its own claimed tile. The
    /// fixture's ore carries no `amount` (`FactorioEntity::new_resource`
    /// leaves it `None`), which is exactly that case.
    #[test]
    fn a_runner_does_not_get_a_guessed_tile_back() {
        let mut a = state();
        a.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));
        let tile = Position::new(-40.5, 40.5);
        a.claim_resource(&tile);

        assert!(a.is_resource_claimed(&tile));
        assert_eq!(a.resource_unclaimed(&tile, "iron-ore"), 0);
    }

    /// The world stated what this tile holds, so the runner that claimed it
    /// draws from it again — for what is *left*, not for the whole tile — and
    /// every other runner is still refused.
    ///
    /// This is the arithmetic behind the roster cliff: a claim used to spend a
    /// whole tile, so a plan's `n`th share found a field that was claimed and
    /// crowded to the last tile while hundreds of thousands of ore sat in it.
    /// See [`PlanState::claim_yields_to`].
    #[test]
    fn a_runner_gets_a_stated_tile_back_for_what_is_left() {
        let mut a = state_with_iron_holding(&[BotId(1), BotId(2)], 40);
        let tile = a_stated_iron_tile(&a);
        assert_eq!(
            a.resource_unclaimed(&tile, "iron-ore"),
            40,
            "the tile is free and holds what the world said before anything claims it"
        );
        a.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));
        a.consume_resource(&tile, "iron-ore", 6)
            .expect("the tile holds forty");

        assert!(a.is_resource_claimed(&tile));
        assert_eq!(
            a.resource_unclaimed(&tile, "iron-ore"),
            34,
            "its own claim costs the runner what it took, not the tile"
        );
        assert_eq!(
            a.resource_unclaimed_for(&tile, "iron-ore", Some(ClaimRunner::Bot(BotId(2)))),
            0,
            "the claim is still exclusive against everybody else"
        );
        assert_eq!(
            a.resource_unclaimed_for(&tile, "iron-ore", None),
            0,
            "an unknown runner matches nothing, here as everywhere"
        );
    }

    /// The other half of the ledger: a tile drawn dry reports nothing, so a
    /// reusable claim cannot become an infinite one.
    #[test]
    fn a_stated_tile_still_runs_out() {
        let mut a = state_with_iron_holding(&[BotId(1)], 10);
        let tile = a_stated_iron_tile(&a);
        a.set_claim_runner(Some(ClaimRunner::Bot(BotId(1))));
        a.consume_resource(&tile, "iron-ore", 10).expect("all ten");

        assert_eq!(a.resource_unclaimed(&tile, "iron-ore"), 0);
        assert!(a.consume_resource(&tile, "iron-ore", 1).is_err());
    }

    /// The binding is a stack, not a setting: `set_claim_runner` hands back
    /// what it replaced so the driver can restore it on every exit path, the
    /// way it restores `ExpansionCtx::chain`.
    #[test]
    fn setting_a_claim_runner_returns_the_one_it_replaced() {
        let mut a = state();
        assert_eq!(a.set_claim_runner(Some(ClaimRunner::Bot(BotId(1)))), None);
        assert_eq!(a.set_claim_runner(None), Some(ClaimRunner::Bot(BotId(1))));
        assert_eq!(a.claim_runner(), None);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod inserter_geometry_tests {
    use super::*;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{FactorioEntity, Position};

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    fn put(s: &mut PlanState, name: &str, x: f64, y: f64, direction: u8) {
        s.create_entity(FactorioEntity {
            name: name.into(),
            position: Position::new(x, y),
            direction,
            ..Default::default()
        });
    }

    /// **CLAUDE.md's first measured case**, taken from a real game rather than
    /// from this table: chest / burner-inserter / chest, and
    /// `direction = 12` ("west") is what moves items *west to east*.
    ///
    /// The rule under test is that an inserter's `direction` names the side it
    /// **picks up** from. Getting it backwards produces a layout that places
    /// 100 %, passes every geometry check, and does absolutely nothing.
    #[test]
    fn an_inserter_facing_west_moves_items_west_to_east() {
        let mut s = state();
        put(&mut s, "iron-chest", 9.5, 10.5, 0);
        put(&mut s, "burner-inserter", 10.5, 10.5, Direction::West as u8);
        put(&mut s, "iron-chest", 11.5, 10.5, 0);

        let west = Position::new(9.5, 10.5);
        let inserter = Position::new(10.5, 10.5);
        let east = Position::new(11.5, 10.5);

        assert!(
            s.delivers_into(&west, &inserter),
            "direction 12 picks up from the west chest"
        );
        assert!(
            s.delivers_into(&inserter, &east),
            "and drops into the east one"
        );
        assert!(
            !s.delivers_into(&east, &inserter),
            "it does not pick up from the side it drops into"
        );
        assert!(
            !s.delivers_into(&inserter, &west),
            "and it does not drop into the side it picks up from"
        );
    }

    /// **CLAUDE.md's second measured case**: for a row fed from a belt to its
    /// north, input and output inserters are both `direction = 0`.
    ///
    /// An input inserter takes from the belt north of it and puts into the
    /// machine south of it. Both cases fall out of the same two offsets, which
    /// is the point: one rule, two independent measurements.
    #[test]
    fn a_row_fed_from_the_north_takes_from_the_north_and_drops_to_the_south() {
        let mut s = state();
        put(&mut s, "iron-chest", 10.5, 9.5, 0);
        put(&mut s, "inserter", 10.5, 10.5, Direction::North as u8);
        put(&mut s, "assembling-machine-1", 10.5, 12.5, 0);

        let belt = Position::new(10.5, 9.5);
        let inserter = Position::new(10.5, 10.5);
        let machine = Position::new(10.5, 12.5);

        assert!(
            s.delivers_into(&belt, &inserter),
            "direction 0 picks up one tile north"
        );
        assert!(
            s.delivers_into(&inserter, &machine),
            "and drops one tile south, which a 3x3 machine centred two tiles \
             south covers"
        );
    }

    /// The trap itself, stated as a test: the same three machines with the
    /// inserter turned round move nothing.
    #[test]
    fn an_inserter_turned_round_feeds_nothing_although_it_places_perfectly() {
        let mut s = state();
        put(&mut s, "iron-chest", 9.5, 10.5, 0);
        put(&mut s, "burner-inserter", 10.5, 10.5, Direction::East as u8);
        put(&mut s, "iron-chest", 11.5, 10.5, 0);

        let west = Position::new(9.5, 10.5);
        let inserter = Position::new(10.5, 10.5);
        let east = Position::new(11.5, 10.5);

        // It places: the ground is clear and every box is disjoint.
        assert!(s.entity_at(&inserter).is_some());
        // And it runs backwards.
        assert!(!s.delivers_into(&west, &inserter));
        assert!(!s.delivers_into(&inserter, &east));
        assert!(s.delivers_into(&east, &inserter));
        assert!(s.delivers_into(&inserter, &west));
    }

    /// A long-handed inserter reaches two tiles, not one, on both sides.
    #[test]
    fn a_long_handed_inserter_reaches_two_tiles_on_each_side() {
        let mut s = state();
        put(&mut s, "iron-chest", 10.5, 8.5, 0);
        put(&mut s, "long-handed-inserter", 10.5, 10.5, 0);
        put(&mut s, "iron-chest", 10.5, 12.5, 0);
        let far_north = Position::new(10.5, 8.5);
        let inserter = Position::new(10.5, 10.5);
        let far_south = Position::new(10.5, 12.5);
        assert!(s.delivers_into(&far_north, &inserter));
        assert!(s.delivers_into(&inserter, &far_south));

        // And a plain inserter in the same place reaches neither.
        let mut short = state();
        put(&mut short, "iron-chest", 10.5, 8.5, 0);
        put(&mut short, "inserter", 10.5, 10.5, 0);
        put(&mut short, "iron-chest", 10.5, 12.5, 0);
        assert!(!short.delivers_into(&far_north, &inserter));
        assert!(!short.delivers_into(&inserter, &far_south));
    }

    /// The pickup half is an **inserter's** claim, not a general one.
    ///
    /// **The geometry here is chosen so the test can fail.** The first draft
    /// used a 2x2 stone furnace, and a 2x2 machine's own hypothetical pickup
    /// tile lands *inside itself*, so the assertion held whatever
    /// `inserter_reach` said — it survived the mutation that gives every
    /// entity an inserter's reach. A 1x1 entity one tile from the chest is the
    /// arrangement in which "not an inserter" is the only thing standing
    /// between the two.
    #[test]
    fn only_an_inserter_takes_from_the_machine_beside_it() {
        let mut s = state();
        let chest = Position::new(10.5, 9.5);
        let pole = Position::new(10.5, 10.5);
        put(&mut s, "iron-chest", chest.x(), chest.y(), 0);
        put(&mut s, "small-electric-pole", pole.x(), pole.y(), 0);
        // An *inserter* in the pole's place would take from the chest, which
        // is what makes the refusal below about the entity kind and not about
        // the distance.
        let mut with_inserter = state();
        put(&mut with_inserter, "iron-chest", chest.x(), chest.y(), 0);
        put(&mut with_inserter, "inserter", pole.x(), pole.y(), 0);
        assert!(with_inserter.delivers_into(&chest, &pole));

        assert!(
            !s.delivers_into(&chest, &pole),
            "a pole beside a chest is two entities standing near each other"
        );
    }

    /// Stage 1's drill-into-furnace claim is untouched by the pickup half.
    ///
    /// The disjunction added for inserters could have widened `Feeds` for
    /// everything; this pins that it did not.
    #[test]
    fn a_furnace_does_not_reach_back_into_a_drill_that_faces_away() {
        let mut s = state();
        put(
            &mut s,
            "burner-mining-drill",
            10.0,
            10.0,
            Direction::South as u8,
        );
        put(&mut s, "stone-furnace", 10.0, 8.0, 0);
        let drill = Position::new(10.0, 10.0);
        let furnace = Position::new(10.0, 8.0);
        assert!(
            !s.delivers_into(&drill, &furnace),
            "a drill facing south drops to the south"
        );
        assert!(
            !s.delivers_into(&furnace, &drill),
            "and a furnace takes from nothing"
        );
    }
}

#[cfg(test)]
mod occupant_naming_tests {
    use super::*;
    use factorio_bot_core::factorio::world::PlacementRefusal;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{Direction, FactorioEntity, Position};

    /// Open ground in the fixture, so anything found here was put here by the
    /// test.
    ///
    /// **Not the run's own (-16, -14).** That tile has a tree on it *in this
    /// fixture* -- a different map from the run's, and a coincidence, but one
    /// that made `the_site_starts_clear` fail on the first attempt and would
    /// have made every assertion below about a world with two obstacles in
    /// it. The control is what caught it.
    const SITE: (f64, f64) = (500., 500.);

    fn site() -> Position {
        Position::new(SITE.0, SITE.1)
    }

    /// The game's own refusal, carrying what the mod found in the box it
    /// judged -- exactly the shape `note_placement_refusal` builds.
    fn refusal() -> PlacementRefusal {
        PlacementRefusal::at_dispatch(
            Some(6198),
            "burner-mining-drill",
            site(),
            0,
            vec!["iron-ore".to_string()],
            Some("dirt-4".to_string()),
        )
    }

    /// The control. Every assertion below is about ground that starts empty,
    /// so a fixture that happened to have something here would make them all
    /// vacuous.
    #[test]
    fn the_site_starts_clear() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        assert_eq!(
            state.placement_occupant("burner-mining-drill", &site(), Direction::North),
            None,
            "nothing stands at {SITE:?} in the fixture"
        );
    }

    /// **The defect.** A refusal and an anonymous blocking box over the same
    /// ground, and the refusal is what gets said.
    ///
    /// The fixture is deliberately hostile: a tree is put at the refused site
    /// as well, so `blocking_boxes_within` genuinely answers here and the
    /// old order genuinely reached it first. Without the tree this test would
    /// pass under either order and prove nothing about which source is asked
    /// first.
    ///
    /// The old answer was `Occupant::Terrain`, whose message recited
    /// "a tree, cliff, rock or unit" -- four things it had not read -- over a
    /// source that carried the game's own list of what it found. See
    /// `docs/superpowers/notes/2026-09-06-a-failed-placement-blames-a-tree.md`.
    #[test]
    fn a_refusal_is_named_over_an_anonymous_box_on_the_same_ground() {
        let world = fixture_world();
        world
            .entity_graph
            .add(vec![FactorioEntity::new_tree(&site())], None)
            .expect("the tree loads");
        world.record_placement_refusal(refusal());
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        // The hostile half, asserted rather than assumed: the anonymous
        // source really does cover this ground.
        assert_eq!(
            state
                .base()
                .entity_graph
                .blocking_boxes_within(&Rect::new(
                    &Position::new(SITE.0 - 0.5, SITE.1 - 0.5),
                    &Position::new(SITE.0 + 0.5, SITE.1 + 0.5),
                ))
                .len(),
            1,
            "the tree must be a blocking box here, or this test cannot tell \
             the two sources apart"
        );

        let occupant = state
            .placement_occupant("burner-mining-drill", &site(), Direction::North)
            .expect("the ground is not clear");
        assert_eq!(
            occupant,
            Occupant::Refused {
                entity: "burner-mining-drill".to_string(),
                blockers: vec!["iron-ore".to_string()],
                tile: Some("dirt-4".to_string()),
            },
            "the source that knows what it found is the one that answers"
        );
        let said = occupant.to_string();
        assert_eq!(
            said,
            "a footprint the game already refused a burner-mining-drill at, \
             where it found iron-ore on tile dirt-4",
            "the message repeats the game's own evidence, verbatim"
        );
    }

    /// And an anonymous box, when it is the only thing there, says exactly
    /// what it knows and no more.
    ///
    /// Absolute strings, not "contains": the whole defect was a message that
    /// said more than it had read.
    #[test]
    fn an_unnamed_obstacle_does_not_call_itself_a_tree() {
        let world = fixture_world();
        let mut cliff = FactorioEntity::new_stone_furnace(&site(), Direction::North);
        cliff.name = "cliff".into();
        cliff.entity_type = "cliff".into();
        world
            .entity_graph
            .add(vec![cliff], None)
            .expect("the cliff loads");
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        let occupant = state
            .placement_occupant("burner-mining-drill", &site(), Direction::North)
            .expect("the ground is not clear");
        assert_eq!(occupant, Occupant::Terrain { minable: false });
        assert_eq!(
            occupant.to_string(),
            "occupied by something with a collision box that this model \
             cannot name -- a cliff, a unit, or anything else the entity tree \
             does not hold"
        );
    }

    /// The other half of the same bit: a tree may still be called a tree,
    /// because `is_minable` is read rather than guessed.
    #[test]
    fn a_minable_obstacle_is_still_named_a_tree_or_rock() {
        let world = fixture_world();
        world
            .entity_graph
            .add(vec![FactorioEntity::new_tree(&site())], None)
            .expect("the tree loads");
        let state = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        let occupant = state
            .placement_occupant("burner-mining-drill", &site(), Direction::North)
            .expect("the ground is not clear");
        assert_eq!(occupant, Occupant::Terrain { minable: true });
        assert_eq!(occupant.to_string(), "occupied by a tree or rock");
    }
}
