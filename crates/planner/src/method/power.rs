//! Building the power a lab needs.
//!
//! An offshore pump on a shoreline, three pipes, a boiler, a steam engine and
//! one small electric pole: 900 kW, at the water, for about 45 iron plates.
//! This is the subsystem `2026-09-02-research-needs-power.md` named as stage 2
//! and `2026-09-02-building-power.md` designed but could not build, because
//! the planner could not see water. It can since `9ca7229a`.
//!
//! # Water moves, and the premise that said otherwise was wrong twice
//!
//! This module used to open with a paragraph that justified siting the plant
//! on the shoreline and nowhere else:
//!
//! > *"Water is the one input that cannot be moved. Coal is five items in an
//! > inventory. Siting the plant at the coal and running pipe to the lake
//! > costs `pipe-to-ground` at 15 iron plates per 10 tiles — a 60-tile
//! > separation roughly doubles rung 7's whole iron bill, which is about 98
//! > plates. Siting it at the water costs one walk."*
//!
//! **Both halves are wrong**, and they are stated here rather than quietly
//! deleted because a *design constraint* was justified by them: shoreline
//! adjacency was treated as a physical law when it is a cost comparison, and
//! everything built on top of that inherits the error.
//!
//! 1. **Water moves. Through pipes. That is what pipes are for.** A fluid
//!    carried in a pipe run is as movable as coal in an inventory; the
//!    difference is that the carrier is built once and then costs nothing,
//!    where an item in an inventory costs a walk every time.
//! 2. **The price quoted was the wrong item's, and it was the expensive
//!    one.** Read off `base/prototypes/recipe.lua` and
//!    `base/prototypes/entity/entities.lua`, both present in this repo under
//!    `workspace/server/data`:
//!
//!    | recipe | ingredients | yields | tiles it spans | iron plates per tile |
//!    |---|---|---|---|---|
//!    | `pipe` | 1 iron-plate | **1** pipe | 1 | **1.00** |
//!    | `pipe-to-ground` | 10 pipe + 5 iron-plate | **2** pieces | 12 | 1.25 |
//!
//!    A `pipe` is one iron plate and covers one tile, so plain pipe is
//!    **1 plate per tile** — see [`PIPE_PLATES_PER_TILE`]. A `pipe-to-ground`
//!    *pair* costs 15 plates (10 pipe at a plate each, plus 5) and its
//!    prototype states `max_underground_distance = 10`, so the two ends stand
//!    12 tiles apart end to end: 1.25 plates per tile, and it exists to
//!    **cross an obstacle**, not to save material. See
//!    [`PIPE_TO_GROUND_PLATES_PER_PAIR`].
//!
//!    So the old paragraph quoted the dearer of the two items as if it were
//!    the price of piping, and *even that* number was misread: 15 plates buys
//!    12 tiles, not 10. The honest figure for the route it was ruling out is
//!    **1 plate a tile**, and a 60-tile pipe run is 60 plates, not 90.
//!
//! The owner's judgement, which is now this module's premise: *"piping water
//! is not expensive! its usually a better option than restricting ourselves
//! to places with water."*
//!
//! # Pricing the two routes honestly
//!
//! There are two ways to join a lake to a consumer, and neither is a
//! constraint — they are alternatives with prices. Both need the same pump,
//! boiler, engine and pole, so the plant's own ~45 plates cancel and only the
//! **span** differs:
//!
//! * **pipe the water in.** The plant stands at the consumer; a pipe run of
//!   `N` tiles reaches the lake. Cost: `N` iron plates
//!   ([`pipe_run_plates`]).
//! * **wire the power out.** The plant stands at the lake; a pole run of `N`
//!   tiles carries the electricity. A `small-electric-pole` is
//!   1 wood + 2 copper-cable for **two** poles, and copper-cable is
//!   1 copper-plate for two cable — so one craft is **1 wood + 1 copper-plate
//!   for 2 poles**. `maximum_wire_distance` is 7.5, so poles stand about
//!   every 7 tiles: `ceil(N / 7)` poles, `ceil(poles / 2)` crafts
//!   ([`pole_run_items`]).
//!
//! **There is no material crossover: the pole run is cheaper per tile at
//! every distance.** 1.00 iron plate a tile against 1/14 wood + 1/14
//! copper-plate — about 0.14 items a tile, a factor of seven. Any claim that
//! piping is ruled out on price is false in the other direction too.
//!
//! **What binds is supply, not price.** Wood is the item this planner cannot
//! make: a four-bot run starts with four (see [`PLANT_ADOPT_RADIUS`], which
//! spends one on a pole and says so), and no method in this crate mines a
//! tree. Four wood is eight poles is about **56 tiles of wire, ever** — and
//! poles are wanted elsewhere. Iron plate is the item a run mines and smelts
//! by the hundred, so the pipe route has no ceiling at all: the 355-tile
//! separation that [`supply_for`]'s world-anchored fallback exists for is 355
//! plates of pipe, expensive but *buildable*, against 51 poles that a
//! four-wood run cannot craft.
//!
//! So the crossover is a **supply** crossover at roughly 56 tiles, and it is
//! an artefact of this planner rather than of the game: teach it to mine a
//! tree and the pole route wins everywhere on materials. Until then, wire
//! short runs and pipe long ones. Neither is a law about where a plant may
//! stand.
//!
//! # Three transports, and the rule that falls out of their prices
//!
//! Water and power are two of three things a plant has to join up. The third
//! is **coal**, and once it is priced the objective changes shape. All three
//! from `recipe.lua`, and all three asserted in
//! `the_three_transports_are_the_recipes_own`:
//!
//! | move | recipe | per tile |
//! |---|---|---|
//! | power, by `small-electric-pole` | 1 wood + 2 copper-cable → **2** poles, every ~7 tiles | ~0.07 wood + ~0.07 copper plate |
//! | water, by `pipe` | 1 iron-plate → 1 pipe, 1 tile | **1.0 iron plate** |
//! | coal, by `transport-belt` | 1 iron-plate + 1 iron-gear-wheel (2 plates) → **2** belts | **1.5 iron plates** |
//!
//! **Belt is the dearest of the three and power is nearly free.** So the
//! objective is not "put the plant at the water" and not "put the plant at the
//! consumer" — it is **minimise the belted distance**. Consumer proximity
//! barely enters it, because wire is an order of magnitude cheaper per tile
//! than either other route.
//!
//! The rule, in one line: **wire the power, pipe the water, do not move the
//! coal.**
//!
//! ## The caveat that decides which regime applies
//!
//! **In today's bootstrap the coal is not belted — a bot carries it**, which
//! is exactly what the retracted premise meant by *"coal is five items in an
//! inventory"* ([`PLANT_COAL`] is a single load, placed once). While that
//! holds, moving coal costs no material at all, the belt row of the table does
//! not apply, and siting at the water is free and correct. **Today's behaviour
//! is therefore not wrong.**
//!
//! The rule above becomes decisive the moment coal delivery is *automated*,
//! which is where the self-feeding cell work is heading. So the siting
//! decision wants to know **whether this plant's coal is belted or carried**
//! and pick accordingly — not to hard-code either answer. That conditional is
//! designed, with the weighted objective over all three transports, in
//! `docs/superpowers/notes/2026-09-06-piping-water-is-cheap.md`. It is
//! deliberately **not built here**: it needs a pipe router and a belt-aware
//! fuel model that this crate does not have, and half of it would silently
//! assume one regime.
//!
//! **The plant is still built at the water today**, and [`supply_for`] wires
//! the power out rather than piping the water in — which is legitimate,
//! because that is the cheaper route in materials and because
//! `PlanState::electric_supply_kw` follows the wire and can therefore *see*
//! it. What is no longer claimed is that this is forced. Siting the plant
//! away from the shore and piping to it is designed and costed in
//! `docs/superpowers/notes/2026-09-06-piping-water-is-cheap.md`; it is not
//! built, because it needs an obstacle-aware pipe router this crate does not
//! have, and a half-built one is worse than none.
//!
//! The older arithmetic in
//! `docs/superpowers/notes/2026-09-02-building-power.md` §5 rests on the
//! retracted premise and should be read with this section beside it.
//!
//! The lab follows the plant rather than the other way round: it has to stand
//! inside the pole's supply area anyway, and `Researched::lab_site` already
//! searches around the supplying pole rather than around the bot.
//!
//! # The plant is sized against the demand it is asked for
//!
//! Until 2026-09-06 [`plan_plant`] took no kilowatts at all: one pump, three
//! pipes, one boiler, one steam engine, one pole, 900 kW, handed back for any
//! demand whatever. Three of [`supply_for`]'s four tiers checked the demand;
//! the fourth — the one that *builds* — did not, so a plan wanting 2,000 kW
//! got a plant short by 1,100 and an `Ok`. See
//! `docs/superpowers/notes/2026-09-06-power-as-capacity-over-time.md`.
//!
//! [`engines_for`] now sizes the engine row from the demand, and
//! [`MAX_ENGINES_PER_BOILER`] bounds it at the ratio the prototypes state:
//! `PlannerError::PowerPlantTooSmall` above that, rather than an undersized
//! plant reported as success.
//!
//! **The check is over the end state of the plan, and that is deliberate
//! rather than a simplification.** Demand here is monotone non-decreasing —
//! no method removes a consumer and none unsets a recipe — and so is supply,
//! so the peak demand over any schedule *is* the final demand and a static
//! sum is exactly the worst case a time-phased ledger would find. What
//! phasing does matter for is generation arriving after the load that needs
//! it, and that is a sequencing problem already solved by
//! `PlanState::powering_entities` turning a `Condition::Powered`'s poles and
//! generators into `Condition::EntityAt` preconditions. The note above records
//! the monotonicity assumption so a future `Effect` that removes an entity
//! knows to come back here.
//!
//! # Solar is excluded because it is not deterministic
//!
//! This module used to say solar was excluded *structurally*: `solar-energy`
//! needs `logistic-science-pack`, which needs the lab the plant is powering,
//! so research needs power and solar power needs research. That circle is
//! real, and it is a good reason not to plan a **first** plant out of solar —
//! but it is not a reason to refuse to count a panel a run already has, and a
//! solar panel is entirely pre-oil (5 steel plate, 15 electronic circuit, 5
//! copper plate), so the technology is reachable long before this argument
//! suggests. The claim was overstated.
//!
//! The reason that actually binds is `crate::state`'s: **a panel's output is a
//! function of the map clock.** Vanilla states `production = "60kW"`, which is
//! peak; the Nauvis daily average is about 42 kW and the value at night is
//! zero. This crate is pure and deterministic and has no in-game time of day
//! among its inputs, so crediting 60 plans a base that is dead for a third of
//! every day and crediting 42 plans one that browns out every night.
//! `generation_kw` credits zero, which under-credits — the direction every
//! other table in that file chooses for an unknown. The honest way in later is
//! an accumulator-backed figure, which is a model rather than a table entry.

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::ActionId;
use crate::method::have::PLACE_TICKS;
use crate::method::util::free_area_near_where;
use crate::method::{ExpansionCtx, Step};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::{FromPrimitive, ToPrimitive};
use factorio_bot_core::types::{Direction, FactorioEntity, Pos, Position, Rect};
use std::collections::BTreeSet;

/// The entities the plant is made of.
pub const PUMP: &str = "offshore-pump";
pub const PIPE: &str = "pipe";
pub const BOILER: &str = "boiler";
pub const ENGINE: &str = "steam-engine";
pub const POLE: &str = "small-electric-pole";

/// How many pipes one plant lays. Derived by [`layout`], asserted by a test —
/// this is the bill, not the design.
///
/// Independent of the engine count: engines chain directly off each other's
/// steam connection, so a second engine adds no pipe. See [`layout`].
pub const PIPE_COUNT: u32 = 3;

// ---------------------------------------------------------------------------
// What a span costs, either way round
//
// The module doc's "Pricing the two routes honestly" is these numbers. They
// are here, next to the bill, so that a reader deciding where to put a plant
// finds the comparison in the code rather than in a note.
// ---------------------------------------------------------------------------

/// Iron plates one tile of plain `pipe` costs.
///
/// `recipe.lua`: `pipe` is `{iron-plate, 1}` for one pipe, and one pipe covers
/// one tile. **One plate a tile**, which is the number the retracted premise
/// in this module's doc should have quoted.
pub const PIPE_PLATES_PER_TILE: u32 = 1;

/// Iron plates one `pipe-to-ground` **pair** costs.
///
/// `recipe.lua`: `10 pipe + 5 iron-plate` yields **2** pieces, and a pipe is a
/// plate, so a pair is 15 plates. Dearer per tile than plain pipe — it exists
/// to cross an obstacle, not to save material.
pub const PIPE_TO_GROUND_PLATES_PER_PAIR: u32 = 15;

/// Tiles one `pipe-to-ground` pair spans, end to end.
///
/// `entities.lua` states `max_underground_distance = 10` on the underground
/// connection, so the two ends may stand with ten tiles between them: 10 + the
/// two ends themselves = 12. 15 plates over 12 tiles is 1.25 a tile, against
/// [`PIPE_PLATES_PER_TILE`]'s 1.
pub const PIPE_TO_GROUND_PAIR_SPAN_TILES: u32 = 12;

/// A `small-electric-pole`'s `maximum_wire_distance`, in tiles.
///
/// `entities.lua`. This is the wire, not the supply area (2.5) — carrying
/// power along a run is a wire question.
pub const POLE_WIRE_REACH_TILES: f64 = 7.5;

/// How far apart poles are assumed to stand along a run, in tiles.
///
/// [`POLE_WIRE_REACH_TILES`] rounded down to a whole tile, which is what a bot
/// placing on the tile grid can actually achieve without measuring diagonals.
pub const POLE_SPACING_TILES: f64 = 7.;

/// Poles one craft of `small-electric-pole` yields.
///
/// `recipe.lua`: `1 wood + 2 copper-cable` yields **2**. The doubling is the
/// easiest thing to miss here and it halves the bill.
pub const POLES_PER_CRAFT: u32 = 2;

/// Wood one craft of `small-electric-pole` costs.
///
/// **The binding constraint on the pole route**, and the reason the module doc
/// calls the crossover a supply crossover rather than a price one: no method
/// in this crate makes wood.
pub const POLE_CRAFT_WOOD: u32 = 1;

/// Copper plates one craft of `small-electric-pole` costs.
///
/// The recipe names 2 `copper-cable`, and `copper-cable` is 1 copper-plate for
/// **two** cable — so one craft is one plate, not two.
pub const POLE_CRAFT_COPPER_PLATES: u32 = 1;

/// Iron plates one tile of `transport-belt` costs, times two.
///
/// `recipe.lua`: `1 iron-plate + 1 iron-gear-wheel` yields **2** belts, and an
/// `iron-gear-wheel` is 2 iron-plates — so 3 plates buy 2 belts. Stored
/// doubled because it is 1.5, and this crate keeps its bills in whole items:
/// [`belt_run_plates`] halves it.
///
/// **The dearest of the three transports**, which is the whole of the owner's
/// argument in the module doc: belt 1.5 a tile, pipe 1.0, pole ~0.14.
pub const BELT_HALF_PLATES_PER_TILE: u32 = 3;

/// Iron plates to belt an item `tiles` tiles.
///
/// Nothing in this crate belts coal to a boiler yet — [`PLANT_COAL`] is a
/// single load a bot carries. This is here so the comparison the module doc
/// makes is arithmetic rather than assertion, and so the day somebody does
/// belt it, the number is already derived from the recipe.
#[must_use]
pub fn belt_run_plates(tiles: f64) -> u32 {
    if tiles <= 0. {
        return 0;
    }
    let halves = (tiles.ceil() as u32).saturating_mul(BELT_HALF_PLATES_PER_TILE);
    halves.div_ceil(2)
}

/// Iron plates to pipe water `tiles` tiles.
///
/// Plain pipe, at [`PIPE_PLATES_PER_TILE`] a tile, rounded up: a fractional
/// tile still costs a whole pipe. The plant's own [`PIPE_COUNT`] pipes are
/// **not** included — this is the span, and the plant is billed either way
/// round.
///
/// Nothing in this crate builds such a run yet; it is here so the comparison
/// [`pole_run_items`] is half of can be made, and asserted, rather than
/// asserted in prose. See the module doc.
#[must_use]
pub fn pipe_run_plates(tiles: f64) -> u32 {
    if tiles <= 0. {
        return 0;
    }
    (tiles.ceil() as u32).saturating_mul(PIPE_PLATES_PER_TILE)
}

/// Poles to carry power `tiles` tiles, and the `(wood, copper plates)` to
/// craft them.
///
/// The plant's own pole is the anchor of the run and is already in the plant's
/// bill, so it is not counted: a run of `tiles` needs `ceil(tiles / 7)` more
/// poles, and poles come two to a craft.
///
/// Returned as three numbers rather than one so the caller sees **which**
/// material it is spending. That is the whole point of the comparison: a
/// 355-tile pipe run is 355 iron plates, which a run mines; the same distance
/// in poles is 51 poles and 26 wood, which a four-bot run cannot obtain at
/// all.
#[must_use]
pub fn pole_run_items(tiles: f64) -> (u32, u32, u32) {
    if tiles <= 0. {
        return (0, 0, 0);
    }
    let poles = (tiles / POLE_SPACING_TILES).ceil() as u32;
    let crafts = poles.div_ceil(POLES_PER_CRAFT);
    (
        poles,
        crafts.saturating_mul(POLE_CRAFT_WOOD),
        crafts.saturating_mul(POLE_CRAFT_COPPER_PLATES),
    )
}

/// How many steam engines one boiler drives.
///
/// **Derived from two vanilla prototype numbers, not chosen**, which is why it
/// is stated with them:
///
/// * `boiler` states `energy_consumption = "1.8MW"`
///   (`base/prototypes/entity/entities.lua`, readable in this repo's
///   `workspace/data`), and a boiler turns all of that into steam;
/// * `steam-engine` states `fluid_usage_per_tick = 0.5`, `effectivity = 1` and
///   `maximum_temperature = 165`. Steam carries 0.2 kJ per unit per degree, so
///   165 °C against the 15 °C default is 30 kJ a unit, and
///   `0.5 units/tick × 60 ticks/s × 30 kJ = 900 kW` — which is exactly the
///   figure `crate::state`'s `generation_kw` credits an engine, arrived at
///   from the other end.
///
/// 1.8 MW over 900 kW is two. A third engine needs a second boiler, which is a
/// second water tap on the pipe run: real shoreline geometry, and out of scope
/// for the change that introduced this constant. Past it [`plan_plant_for`]
/// raises [`PlannerError::PowerPlantTooSmall`] rather than returning a plant
/// that cannot carry what it was asked for.
///
/// # The 1.8 MW ceiling is this planner's, and the water is nowhere near it
///
/// Read alone, "one boiler drives two engines" invites 1.8 MW to be heard as a
/// fact about Factorio. It is not, and the gap is two orders of magnitude
/// wide. Verified against `workspace/server/data/base/prototypes/entity/
/// entities.lua` on 2026-09-06: `offshore-pump` states `pumping_speed = 20`,
/// which is fluid units per **tick** -- 1,200 water/s -- and a boiler burning
/// 1.8 MW to lift water 150 °C at 0.2 kJ/unit/°C consumes 60 water/s. So one
/// offshore pump feeds **20 boilers and 40 engines, about 36 MW**.
///
/// What refuses at 1.8 MW is the *layout*, not the water: [`Plant`] carries a
/// single `boiler` position, [`plant_steps`] fuels it once, and `layout` is
/// a rigid pump-pipes-boiler-engines row rotated as one body about the pump's
/// tile centre. Growing it is designed in
/// `docs/superpowers/notes/2026-09-06-one-place-that-decides-power.md` and
/// deliberately not built there: it changes `Plant`'s shape, the coal bill,
/// the shore-fitting search and `method::assemble`'s `fuel_for`, which is more
/// than a constant's worth of change.
///
/// An owner-supplied ratio of "1 pump : 200 boilers : 400 engines" is ten
/// times the measured one; the arithmetic is written out above so the next
/// reader can check it rather than pick between two numbers.
pub const MAX_ENGINES_PER_BOILER: u32 = 2;

/// How many engines a plant carrying `kw` needs, or why it cannot be built.
///
/// Ceiling division, floor one: a plant with no engine generates nothing and
/// is not a plant, so `kw = 0` — which is what the [`plan_plant`] compatibility
/// wrapper passes — still asks for the single engine that has always been
/// built.
///
/// The engine's output is read through
/// [`PlanState::generator_output_kw`](crate::state::PlanState::generator_output_kw)
/// rather than written down here, so the number a plant is *sized* against is
/// the number `electric_supply_kw` will *credit* it. A second copy would be
/// the same drift `PlanState::consumer_draw_kw` exists to prevent, with the
/// halves swapped: a plant sized against 1,000 kW an engine would pass its own
/// arithmetic and brown out the network.
///
/// A world whose prototypes this crate cannot price an engine from refuses,
/// which is the direction an unknown generator errs in everywhere else.
pub fn engines_for(state: &PlanState, kw: f64) -> Result<u32, PlannerError> {
    let each = state
        .generator_output_kw(ENGINE)
        .ok_or(PlannerError::PowerPlantTooSmall {
            needed_kw: kw,
            plant_kw: 0.,
        })?;
    let wanted = (kw / each).ceil().max(1.);
    // `total_cmp` rather than `>`: this crate orders every float that way.
    if wanted.total_cmp(&f64::from(MAX_ENGINES_PER_BOILER)).is_gt() {
        return Err(PlannerError::PowerPlantTooSmall {
            needed_kw: kw,
            plant_kw: each * f64::from(MAX_ENGINES_PER_BOILER),
        });
    }
    // In range by the test above, and `wanted >= 1`.
    Ok(wanted as u32)
}

/// How much coal goes into the boiler, in items.
///
/// **A flat number, and deliberately not derived from the plan's duration.**
/// The research itself costs `60 kW x 100 s = 6 MJ`, which is 1.5 coal at 4 MJ
/// each — so a plan that inserts *one* coal stalls at about two thirds. But
/// sizing from the research duration is the wrong model anyway: the boiler is
/// lit when it is fuelled, and the plan still has to mine, smelt, craft and
/// carry ten science packs before the research starts. In every archived run
/// that is tens of thousands of ticks of a powered-but-idle lab drawing its
/// standby ~2 kW. The planner has no wall clock and its makespan is a schedule
/// rather than an observation, so it cannot honestly derive the fuel bill from
/// its own duration.
///
/// Five coal is 20 MJ: about 3.3x the research, enough for the idle window and
/// one retry, and cheap beside the ~40 ore the science packs already cost.
///
/// **If it runs dry, nothing detects it.** `PlanState::electric_supply_kw`
/// counts nameplate capacity, so a boiler with an empty fuel slot still reads
/// as 900 kW, and `crates/executor` waits on the game's own
/// `on_research_finished` with no modelled duration — it would sit for ever at
/// whatever percentage the research reached. That is the residual this
/// constant does not close; closing it wants a fuel monitor, not a bigger
/// number.
pub const PLANT_COAL: u32 = 5;

/// How far [`plan_plant`] looks for water before paying for a wider read, in
/// tiles.
///
/// **A scan bound, not a policy bound.** Water found beyond it is still built
/// against ([`PLANT_WATER_WIDE_SCAN_RADIUS`]); all this number decides is how
/// much terrain is read on the common path.
///
/// # It used to refuse, and the refusal was wrong
///
/// This constant was called `PLANT_SITE_RADIUS`, and its own doc said it was
/// "the same bound `PlanState::electric_supply_kw` and `Researched`'s lab
/// search already use". Those two are about a **pole's supply area**, which is
/// a physical constant of the game. This one guarded a **walk**, which is a
/// cost. One number stood for two unrelated quantities, and the walk half of
/// it was never derived from anything.
///
/// Run `run-1788379071-00467` is what that cost. It satisfied rungs 1-6 and
/// refused rung 7 with *"the nearest water is 67.8 tiles away, and a power
/// plant may not be sited more than 64 tiles from the bot that has to carry
/// it there"* -- short by 3.8 tiles. Off that same run's `samples.jsonl`,
/// every bot in it had already been further from spawn than the water was:
/// 68.8, 69.9, 72.5 and 71.5 tiles. The bound refused a journey the run was
/// making routinely, in the run it refused.
///
/// # Why no cap replaced it
///
/// The walk is **already priced**. Every part of the plant carries a
/// `Condition::AtPosition` at the plant site, so [`crate::schedule`] emits a
/// walk for it and charges `distance / WALK_TILES_PER_TICK` -- 0.14 tiles per
/// tick, so 67.8 tiles is 485 ticks. Against run 32's own clock (40,775 ticks
/// to reach rung 7), a plant at 64 tiles costs 2.1% of the run in walking, at
/// 128 tiles 4.2%, at 256 tiles 8.4%. None of those is "the bot spends the
/// run walking", and no measurement says where that line is -- so the planner
/// does not pretend to know one. A distant plant is a *worse plan*, and a
/// worse plan is what a makespan is for.
///
/// The number itself is unchanged at 64 because nothing about the cheap scan
/// changed; raising it would only move work from the second tier into the
/// first.
pub const PLANT_WATER_SCAN_RADIUS: f64 = 64.;

/// How far [`plan_plant`] looks when the cheap scan found nothing, in tiles.
///
/// This tier used to exist only to write a better epitaph -- it measured a
/// distance the planner was about to refuse anyway. It now *finds the water
/// the plant is built against*, which makes it the only remaining bound on
/// where a plant may go, so it has to carry its own justification rather than
/// inherit the one the refusal used to have.
///
/// **It is a read-cost bound.** `nearest_water_tile` is linear in the tiles
/// the quad tree holds inside a `2R`-by-`2R` box, and a fully charted map
/// carries ~410,000 water tiles since `fa8dabf3`. At 128 that box is 65,536
/// tiles, read once, and only on the path that would otherwise have nothing to
/// offer; at 256 it is 262,144, four times the cost for a benefit that is
/// speculative -- water further out is water the game may not have charted at
/// all, since the planner only ever sees the chunks it has been sent. The cost
/// grows as `R^2`; the chance of a lake appearing in the new ring does not.
///
/// So the refusal that survives is [`PlannerError::PowerPlantNeedsWater`], and
/// it names this radius. "The plan can see no water within 128 tiles" is a
/// statement about what was looked at -- true, and actionable -- rather than a
/// statement about what is allowed, which is what the old bound claimed and
/// could not support.
pub const PLANT_WATER_WIDE_SCAN_RADIUS: f64 = 128.;

/// How far around the nearest water tile a shoreline is looked for, in tiles.
///
/// The nearest water tile is very unlikely to be a *buildable* shoreline: it
/// may be a one-tile inlet, or the plant may not fit behind it. This is how
/// much of that lake's edge gets tried before the whole lake is given up on.
const SHORE_SEARCH_RADIUS: i32 = 10;

/// How far a plant that **already stands** is looked for before one is built,
/// in tiles.
///
/// # The run this exists for
///
/// `run-1788408407-02764` is the furthest this project has got: rung 1
/// satisfied, a working plant standing at the lake — offshore pump at
/// `[-5.5, -57.5]`, boiler, engine at `[-11.5, -54.5]`, pole at
/// `[-13.5, -56.5]` — and a lab that had actually run a research on it. Its
/// second replan of rung 2, at tick 150,645, then planned **a whole second
/// plant**: `place offshore-pump at [9.5, -45.5]`, a boiler, an engine and
/// three pipes, on the other side of the map. Bot 1 was at `[-51.25, 20.77]`
/// at that moment, 86 tiles from the standing plant's pole, and both callers
/// of [`plan_plant`] asked for supply within 64 tiles **of the bot** and, on
/// being told there was none there, built one. The run died two replans later
/// with [`PlannerError::PowerPlantNeedsShore`], because by then the only
/// shoreline it could reach was the one the first plant was standing on.
///
/// # Why the bound is this large
///
/// It is a *read-cost* bound, like [`PLANT_WATER_WIDE_SCAN_RADIUS`], and not a
/// policy bound — nothing here says a plant 200 tiles away is a good idea, only
/// that a plant that exists is worth walking to. The comparison is not close:
///
/// * adopting costs **one walk**, which [`crate::schedule`] already prices at
///   `distance / WALK_TILES_PER_TICK` — 0.14 tiles a tick, so even 256 tiles is
///   about 1,830 ticks;
/// * building costs an offshore pump, three pipes, a boiler and a steam engine
///   — about 45 iron plates, which have to be mined and smelted first, and in
///   every archived run that is *tens of thousands* of ticks — plus five coal
///   and **one wood**, of which a four-bot run has exactly four and can make no
///   more (`crate::method::assemble`'s `POLE_OFFSET`).
///
/// So adoption wins by two orders of magnitude at any distance this planner
/// can see, and the only question left is how much of the entity graph to
/// read. [`PlanState::entities_within`] is a quad-tree query over *entities* —
/// hundreds in a starter base, against the ~410,000 water **tiles** that make
/// [`PLANT_WATER_WIDE_SCAN_RADIUS`] a cost worth minding — and it only runs at
/// all when the cheap search around the bot has already failed.
///
/// 256 also **strictly exceeds the furthest a plant this planner could build**,
/// which is the property that stops it preferring construction to a walk: the
/// water may be up to 128 tiles away, the shoreline up to
/// [`SHORE_SEARCH_RADIUS`] (14.2 tiles diagonally) from that water, and the
/// pole up to the plant's own extent plus `free_area_near_where`'s 12-ring
/// search from the engine — under 30 tiles in total. 128 + 14 + 30 is 172.
///
/// **And it is deliberately not tight.** The refusal this closes was reported
/// with the bots 60 tiles from the pole, where the *existing* 64-tile search
/// should already have found it: that run's own final keyframe reports the
/// pole, the engine and the boiler in the model with **zero** divergence from
/// the game, and replaying `nearest_supply_anchor` against those entities and
/// those bot positions answers `Some([-13.5, -56.5])` at 64 tiles. So the
/// origin that expansion was actually asked from is not one this record
/// carries, and a bound with margin adopts the plant whether that origin is
/// the bot's real position or a stale one.
pub const PLANT_ADOPT_RADIUS: f64 = 256.;

// ---------------------------------------------------------------------------
// The fluid connections, and why they are written down rather than read
// ---------------------------------------------------------------------------

/// Where a fluidbox connecting to an **offshore pump** must sit, relative to
/// the pump's position, with the pump facing north.
///
/// # These tables are not read from `fluidbox_prototypes`, and that is a
/// # finding, not a shortcut
///
/// `FactorioEntityPrototype::fluidbox_prototypes` carries a `positions` array
/// of four entries, one per cardinal direction, and
/// `2026-09-02-building-power.md` §5 recommended reading the geometry straight
/// out of it. Checked rather than trusted, that does not work, for two
/// independent reasons:
///
/// 1. **The two captures in this repo disagree about what `positions` means.**
///    `crates/core/tests/entity-prototype-fixtures.json` — the world every
///    test in this crate plans against — reports the boiler's water connection
///    at `(-2, 0.5)` and the steam engine's at `(0, 3)`. The live 2.1.17
///    capture (`live-2.1.17-world-snapshot.json`) reports `(-1, 0.5)` and
///    `(0, 2)` for the same two connections. They differ by exactly one tile
///    along each connection's own direction, because the fixture holds
///    Factorio 1.x's reading (the *target* tile, one step out) and the live
///    game holds 2.x's (`PipeConnectionDefinition::position`, "position
///    relative to entity's center where pipes can connect", which is *inside*
///    the entity). Code that reads `positions` as a target is one tile wrong
///    against a real game; code that reads it as a point is one tile wrong
///    against every test in this crate.
/// 2. **The direction is not sent at all.** Recovering the target from the
///    point needs `PipeConnectionDefinition::direction`, and
///    `FactorioEntityPrototype` has no field for it. It cannot be inferred
///    geometrically either: the pump's connection point is its own centre, so
///    all four cardinals leave the collision box, and the boiler's is equally
///    far from the west edge and the south edge.
///
/// So the north-frame geometry is written down here, from the vanilla
/// prototype definitions (`base/prototypes/entity/entities.lua`, readable in
/// this repo's `workspace/data`), the same discipline as `crate::state`'s pole
/// tables and `COAL_BURN_TICKS`. What *is* read from the game is the rotation:
/// [`Position::turn`] turns a north-frame offset into any cardinal, and
/// `the_connection_table_matches_the_prototype_the_tests_plan_against` checks
/// every entry of these tables against the fixture's own `positions` array so
/// the two cannot drift apart silently.
///
/// The unit is the tile the connecting fluidbox occupies. Two entities are
/// joined when a pipe stands on a tile that **both** of them name here: a
/// pipe's own connection point is its centre, so an entity's target tile is
/// exactly where a pipe has to go to reach it.
const PUMP_OUTPUT: (f64, f64) = (0., 1.);

/// The boiler's two water connections, north-facing: one tile beyond each end
/// of its southern row. Vanilla `position = {-1, 0.5}` facing west and
/// `{1, 0.5}` facing east.
const BOILER_WATER: [(f64, f64); 2] = [(-2., 0.5), (2., 0.5)];

/// The boiler's steam connection, north-facing: one tile beyond the middle of
/// its northern row. Vanilla `position = {0, -0.5}` facing north.
///
/// **This is what makes the boiler face away from the water.** A north-facing
/// boiler sends its steam north; the plant turns it to face the opposite way
/// from the pump so the engine ends up inland rather than in the lake.
const BOILER_STEAM: (f64, f64) = (0., -1.5);

/// The steam engine's two connections, north-facing: one tile beyond each end
/// of its five-tile length. Vanilla `position = {0, 2}` facing south and
/// `{0, -2}` facing north.
const ENGINE_STEAM: [(f64, f64); 2] = [(0., 3.), (0., -3.)];

/// The tiles that must be water for an offshore pump facing north to stand on
/// the tile at the origin, as tile offsets.
///
/// # Read off the pump's own buildability rules, and deliberately conservative
///
/// Vanilla's `offshore-pump` states two `tile_buildability_rules`:
///
/// ```text
/// {area = {{-0.4, -0.4}, {0.4, 0.4}}, required_tiles = ground, colliding_tiles = water}
/// {area = {{-1, -2},     {1, -1}},    required_tiles = water}
/// ```
///
/// The pump's `tile_width`/`tile_height` are stated as `1`, so its centre is a
/// tile *centre*, and the first rule is then exactly "the tile under the pump
/// is ground" — which is why this list does not contain the origin and why
/// [`shoreline_faces_water`] tests it separately. The second rule's box spans
/// three tile columns and two tile rows, and this list is every tile it
/// touches.
///
/// **Every tile it touches, not every tile it covers.** The box only half
/// covers the outer columns, and whether the engine tests overlap or coverage
/// is not something this repo can settle without building one. Requiring all
/// six is the strict reading: it refuses shorelines the game might accept and
/// accepts none it would refuse, which is the safe direction for a planner
/// whose alternative is committing a bot to a walk and a placement that fails.
///
/// The 1.x rule that `FactorioRcon::find_offshore_pump_placement_options`
/// implements — "a **water** tile whose neighbour ahead is not water" — is
/// **wrong for 2.x** and was deliberately not ported: the 2.0 pump stands on
/// land with the water in front of it, not in the water with land in front.
/// That function has no callers anywhere and asks only for `"water"`, never
/// `"deepwater"`.
const SHORE_WATER_TILES: [(i32, i32); 6] = [(-1, -2), (0, -2), (1, -2), (-1, -1), (0, -1), (1, -1)];

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// One building of the plant, with the direction it stands in.
#[derive(Clone, Debug, PartialEq)]
pub struct PlantPart {
    pub name: &'static str,
    pub position: Position,
    pub direction: Direction,
}

/// A whole plant, sited and checked, ready to be turned into steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Plant {
    /// The parts still to be placed, in build order: pump, pipe, pipe, boiler,
    /// pipe, engine, pole -- less whatever [`standing`](Self::standing) holds.
    ///
    /// A plant [`plan_plant`] sites has all seven here. A plant
    /// [`complete_plant`] finishes has only the missing ones, and the bill
    /// [`plant_steps`] emits is read off this list, so a pump that already
    /// stands is neither crafted nor placed again.
    pub parts: Vec<PlantPart>,
    /// The parts of this plant the world already holds, by name and tile.
    ///
    /// Empty for a plant sited from scratch. Carried rather than dropped so a
    /// caller can say *what* was reused, and so a test can assert that a
    /// partial site was completed rather than started over.
    pub standing: Vec<PlantPart>,
    /// Where the coal goes.
    pub boiler: Position,
    /// The first generator whose 900 kW the pole carries.
    ///
    /// Kept as a single position because every caller and test wants "the
    /// engine" of the one-engine plant that has always been built. The whole
    /// row, which is what the pole has to cover, is [`engines`](Self::engines);
    /// this is its first entry.
    pub engine: Position,
    /// Every generator in the row, in build order, nearest the boiler first.
    ///
    /// One entry for a plant sized against a demand under 900 kW, which is
    /// every demand this planner asks for today. See [`engines_for`].
    ///
    /// **The pole is sited to cover all of them**, not just
    /// [`engine`](Self::engine): a second engine the pole does not reach is
    /// 900 kW standing on the ground and reported as 1,800, which is the
    /// coverage-is-not-capacity failure this whole change exists to close.
    pub engines: Vec<Position>,
    /// The pole that carries it, and the anchor the lab is then sited around.
    ///
    /// **The lab follows the plant, not the bot.** `Researched::lab_site`
    /// searches for supply within 64 tiles of whatever origin it is given, and
    /// the plant itself may be up to 64 tiles from the bot — so re-asking from
    /// the bot's position could put a plant just built out of the lab's reach.
    /// Asking from the pole finds it at distance zero.
    pub pole: Position,
    /// Bystanders `fit` found would be sealed into a pocket by this plant's
    /// own footprint, and where each must walk first. Empty in the ordinary
    /// case -- see [`crate::enclosure::check`].
    pub evacuate: Vec<crate::enclosure::Evacuation>,
}

/// Turn a north-frame offset into `direction`.
fn turned(offset: (f64, f64), direction: Direction) -> Option<Position> {
    Position::new(offset.0, offset.1).turn(direction)
}

/// `direction` turned a further `by`.
///
/// A quarter turn is **four** on Factorio 2.x's sixteen-value scale, and a
/// half turn is eight; the plant only ever composes cardinals, so the sum is
/// always another cardinal.
fn compose(direction: Direction, by: Direction) -> Option<Direction> {
    let sum = (Direction::to_u8(&direction)? + Direction::to_u8(&by)?) % 16;
    Direction::from_u8(sum)
}

/// Is `tile` a piece of shoreline an offshore pump could stand on, facing
/// `facing`?
///
/// `facing` points at the water: at direction north the pump's body extends
/// north into the lake and its output pipe comes out to the south. See
/// [`SHORE_WATER_TILES`] for where that comes from and why the rule is
/// stricter than the game's.
fn shoreline_faces_water(tile: &Pos, facing: Direction, water: &BTreeSet<Pos>) -> bool {
    if water.contains(tile) {
        return false;
    }
    SHORE_WATER_TILES.iter().all(|(dx, dy)| {
        match turned((f64::from(*dx), f64::from(*dy)), facing) {
            Some(offset) => water.contains(&Pos(
                tile.0 + offset.x().round() as i32,
                tile.1 + offset.y().round() as i32,
            )),
            None => false,
        }
    })
}

/// The six buildings of a plant whose pump stands at `pump` facing `facing`.
///
/// Everything below the pump is *derived* from the connection tables above
/// rather than stated, so a wrong number in one of those tables moves the
/// layout and fails a test rather than sitting there being decorative.
///
/// Two choices are the layout's own and are not derived:
///
/// * **the lateral step.** The pump's output comes out directly behind it and
///   the boiler's water connections are on its long sides, so the pipe run
///   takes one step along the shore before turning into the boiler. One step
///   is the smallest that works.
/// * **which connection of each pair.** `BOILER_WATER[1]` and
///   `ENGINE_STEAM[1]` put the boiler and the engine on the far side of their
///   joints, i.e. further inland; the other index of each pair would put them
///   in the lake.
///
/// The whole thing is a rigid body rotated about the pump's tile centre, which
/// is what keeps every building on its own build grid at all four facings: a
/// quarter turn about a tile centre takes tile centres to tile centres and
/// tile corners to tile corners, and each building's direction turns with it.
/// `every_facing_puts_every_building_on_its_own_grid` is that claim as a test.
/// `engines` is how many steam engines stand in the row, at least one and at
/// most [`MAX_ENGINES_PER_BOILER`]. **The extra engines cost no extra pipe**:
/// a steam engine's two `ENGINE_STEAM` connections are one tile beyond each
/// end of its own five-tile length, so consecutive engines whose centres are
/// five apart along the row have each one's far connection landing inside the
/// next one's body — which is how the game joins two engines placed end to
/// end, and why [`PIPE_COUNT`] is independent of the engine count.
///
/// The row extends **away from the boiler**, i.e. further inland, along the
/// same axis `ENGINE_STEAM[1]` already chose for the first engine. Extending
/// the other way would put the second engine on top of the boiler.
fn layout(pump: &Position, facing: Direction, engines: u32) -> Option<Vec<PlantPart>> {
    let lateral = turned((1., 0.), facing)?;

    let joint_pump = pump.add(&turned(PUMP_OUTPUT, facing)?);
    let joint_boiler = joint_pump.add(&lateral);

    // Steam away from the water: the boiler faces opposite the pump.
    let boiler_facing = compose(Direction::South, facing)?;
    let boiler = subtract(&joint_boiler, &turned(BOILER_WATER[1], boiler_facing)?);

    let joint_engine = boiler.add(&turned(BOILER_STEAM, boiler_facing)?);
    let engine_facing = facing;
    let engine = subtract(&joint_engine, &turned(ENGINE_STEAM[1], engine_facing)?);

    let mut parts = vec![
        PlantPart {
            name: PUMP,
            position: pump.clone(),
            direction: facing,
        },
        PlantPart {
            name: PIPE,
            position: joint_pump,
            direction: Direction::North,
        },
        PlantPart {
            name: PIPE,
            position: joint_boiler,
            direction: Direction::North,
        },
        PlantPart {
            name: BOILER,
            position: boiler,
            direction: boiler_facing,
        },
        PlantPart {
            name: PIPE,
            position: joint_engine,
            direction: Direction::North,
        },
    ];
    // The step from one engine to the next, which is the engine's own length,
    // derived from `ENGINE_STEAM` rather than written as a bare 5 so that a
    // wrong number there moves the row and fails a test.
    //
    // Both entries are *target* tiles, one beyond each end of the body, so
    // their centres are `length + 1` tiles apart and the body is the tiles
    // strictly between them: 3 - (-3) - 1 = 5.
    let length = (ENGINE_STEAM[0].1 - ENGINE_STEAM[1].1).abs() - 1.;
    // Away from the boiler. `ENGINE_STEAM[1]` is the connection the boiler's
    // pipe reaches (it is what `engine` was placed against, just above), so
    // `ENGINE_STEAM[0]`'s sign is the inland one.
    let pitch = turned((0., length * ENGINE_STEAM[0].1.signum()), engine_facing)?;
    for index in 0..engines {
        let step = f64::from(index);
        parts.push(PlantPart {
            name: ENGINE,
            position: Position::new(engine.x() + pitch.x() * step, engine.y() + pitch.y() * step),
            direction: engine_facing,
        });
    }
    Some(parts)
}

fn subtract(a: &Position, b: &Position) -> Position {
    Position::new(a.x() - b.x(), a.y() - b.y())
}

/// The tile centre of the tile at `pos`.
fn tile_centre(pos: &Pos) -> Position {
    Position::new(f64::from(pos.0) + 0.5, f64::from(pos.1) + 0.5)
}

// ---------------------------------------------------------------------------
// Siting
// ---------------------------------------------------------------------------

/// Where a consumer this plan is about to site will get its power.
#[derive(Clone, Debug, PartialEq)]
pub enum Supply {
    /// A network that already stands, named by the pole to site against.
    ///
    /// Nothing is emitted for it: the entities are in the world already.
    Standing(Position),
    /// Nothing that stands can carry it; this is the plant to build.
    ///
    /// Either a whole plant on a fresh shoreline, or -- when a pump already
    /// stands whose plant was never finished -- the *rest* of that plant: see
    /// [`complete_plant`], and [`Plant::standing`] for which is which. The
    /// caller does not need to tell them apart: [`plant_steps`] places and
    /// bills only what is in [`Plant::parts`].
    Build(Plant),
}

/// The supply a consumer needing `kw` should hang off: one that already
/// stands, or a plant to build.
///
/// # The three tiers, and why they are in this order
///
/// 1. **A network within `near_radius` of `from`.** The common case, and the
///    cheap one — this is the search both callers already did on their own.
/// 2. **A network within [`PLANT_ADOPT_RADIUS`].** The tier this function was
///    added for: a plant that already stands is worth walking almost any
///    distance to, because building a second one costs about 45 iron plates
///    and one of a run's four irreplaceable wood. See [`PLANT_ADOPT_RADIUS`]
///    for the run that was killed by not doing this.
/// 3. **The rest of a plant somebody started.** A pump on a shoreline with
///    its boiler beside it and no engine is most of a plant already paid for,
///    and a replan that walks past it to a fresh shoreline pays for a whole
///    second one -- and then a third, until the shore is full of its own
///    half-built sites and the next plan is refused for want of a shoreline.
///    That is exactly what `run-1788608648-56109` did: four plans, three
///    pumps, two boilers, one engine, and a fifth plan halted with *"no
///    shoreline within 10 tiles of it has room for a pump, a boiler, a steam
///    engine and the pipes between them"*. See [`complete_plant`].
/// 4. **A plant.** Only when the planner can see no working supply at all
///    and nothing standing it could finish.
/// 5. **All four again, from [`plant_world_anchor`].** Every tier above is
///    anchored on `from`, which is the acting bot or the machine site --
///    neither of which is a fact about the map. A roster that walked away
///    from spawn made the planner unable to see a lake that had not moved,
///    and refused every power-needing goal on a well-charted dump. See
///    [`plant_world_anchor`] for the measurement and
///    `retry_from_world_anchor` for which refusals are retried.
///
///    **The fallback is where the caller's pole run stops being free.** A
///    plant at spawn and a consumer far away is a plan only because
///    `PlanState::electric_supply_kw` follows the wire; the wire itself costs
///    what [`pole_run_items`] says, and the module doc's "wire the power, pipe
///    the water" section is the comparison that governs it.
///
/// Tiers 1 and 2 are one question asked twice with a wider bound, and that is
/// a pure cost split rather than a policy: [`PlanState::nearest_supply_anchor`]
/// orders its candidates by distance and returns the first that qualifies, so
/// a single call at [`PLANT_ADOPT_RADIUS`] would give exactly the same answer
/// whenever tier 1 has one. Tier 1 exists so the usual plan does not read a
/// 512-by-512 box of entities to be told what a 128-by-128 box already said —
/// the same two-tier shape, and the same justification, as [`plan_plant`]'s
/// own water scan.
///
/// # What "already stands" is checked to mean
///
/// [`PlanState::nearest_supply_anchor`] with a **positive** `kw` — which both
/// callers pass, since a lab is 60 kW and a cell is 189 kW — verifies, for the
/// pole it returns:
///
/// * the pole is in the world (or in this expansion's overlay) and this crate
///   knows its supply area;
/// * a **generator** — a steam engine, by
///   [`crate::state::PlanState::electric_supply_kw`]'s table — has its own
///   footprint covered by a pole in the *same* wire-connected component, found
///   by union-find over the poles' maximum wire distances. Coverage alone
///   would not do: `kw > 0` cannot be met by a network with no generation on
///   it, so this tier can never adopt a pole that merely exists;
/// * and that the generation left after every consumer already on that network
///   is at least `kw`, through the same demand ledger a `Condition::Powered`
///   is charged against.
///
/// # What it assumes
///
/// **That the plant is running.** Nothing in `FactorioWorld` says whether a
/// steam engine has steam, whether the boiler has water, or whether its fuel
/// slot is empty, so this counts nameplate capacity — the residual
/// `electric_supply_kw` names in its own doc and `PLANT_COAL` names again.
/// Adoption inherits it exactly: a plant whose boiler ran dry is adopted as
/// 900 kW. It is not made worse by adopting rather than building, because a
/// plant this planner *builds* is credited the same 900 kW the moment its
/// `Place` is emitted and long before any coal reaches it; and the cell that
/// adopts tops the boiler up (`crate::method::assemble`'s `fuel_for`), which a
/// second plant on the other side of the map would not have done for this one.
///
/// It also assumes solar and accumulators are absent rather than ignored, and
/// that a consumer `consumer_kw` does not name draws nothing — both inherited,
/// and both already named where they live.
pub fn supply_for(
    state: &PlanState,
    from: &Position,
    near_radius: f64,
    kw: f64,
) -> Result<Supply, PlannerError> {
    for radius in [near_radius, PLANT_ADOPT_RADIUS] {
        if let Some(anchor) = state.nearest_supply_anchor(from, radius, kw) {
            return Ok(Supply::Standing(anchor));
        }
    }
    if let Some(plant) = complete_plant(state, from, kw) {
        return Ok(Supply::Build(plant));
    }
    // The tier that used to ignore `kw` entirely, and the whole of roadmap
    // item 3's defect: three tiers checked the demand and the one that builds
    // did not.
    let roster_anchored = match plan_plant_for(state, from, kw) {
        Ok(plant) => return Ok(Supply::Build(plant)),
        Err(err) => err,
    };
    // Every tier above this line is anchored on `from`, which is the bot or
    // the machine site -- and neither is a fact about the map. See
    // `plant_world_anchor`.
    retry_from_world_anchor(state, from, kw, roster_anchored)
}

/// The spawn tile, and the one anchor for a plant search that is a property of
/// the **world** rather than of the roster's walk history.
///
/// # The defect this closes
///
/// Every tier of [`supply_for`] searches from `from`, and both callers pass
/// something that moves: `method::have` passes the acting bot's position and
/// `method::extract` passes the machine site it just chose. A bot's position
/// is where its last walk left it.
///
/// Measured on 2026-09-06, same seed, same binary, two dumps of the same map:
///
/// | dump | bots at | water from there | result |
/// |---|---|---|---|
/// | `map.json` (t=0) | `(0.5, -0.5)` | 48 tiles | plans, 176 actions |
/// | `map-31337-explored.json` | `(255, 249)` | 355 tiles | **refuses** |
///
/// `score-map` reports water at **48.1 tiles on both dumps**: the lake did not
/// move, the bots did. They had finished an exploration ring and parked, and
/// every power-needing goal on the better-charted map then refused with *"a
/// power plant needs water, and the plan can see none within 128 tiles"* — a
/// true statement about what was looked at, and a false impression of the map.
/// The radii were not the fault and neither was the map; the anchor was.
///
/// # Why the origin, and why it is honest
///
/// Spawn is where freeplay puts the starting resources, where every run
/// begins, and where a t=0 dump's bots stand — so a plant sited from here is
/// the plant the planner would have found on the first tick, which is exactly
/// the stability property a replan wants. It is also the only anchor available
/// without inventing one: this crate is pure, has no map-gen settings, and the
/// alternatives (a roster centroid, the goal's own site) are the same class of
/// walk-history artefact this fixes.
///
/// **It is a fallback, not a preference.** The local search runs first and
/// wins whenever it finds anything, so a plan whose bots are near water still
/// builds beside them; this only replaces a refusal.
///
/// A function rather than a `const` only because `Position::new` is not
/// `const fn`; the value is fixed.
#[must_use]
pub fn plant_world_anchor() -> Position {
    Position::new(0., 0.)
}

/// [`supply_for`]'s last resort: ask the same questions again from
/// [`plant_world_anchor`].
///
/// `roster_anchored` is the refusal the caller-anchored search produced. It is
/// returned unchanged when the retry is not applicable or also fails, because
/// two "no water within 128 tiles" errors are indistinguishable and the first
/// is the one the caller's context explains.
///
/// # Which refusals are retried
///
/// Only the two that are statements about *where the search was standing* —
/// [`PlannerError::PowerPlantNeedsWater`] and
/// [`PlannerError::PowerPlantNeedsShore`]. [`PlannerError::PowerPlantTooSmall`]
/// is a statement about the **demand**, true from every anchor on every map,
/// and retrying it would read a quarter of a million terrain tiles to arrive
/// at the identical error.
///
/// # And the power has to get back
///
/// A plant at spawn and a consumer 355 tiles away are only a plan because
/// `PlanState::electric_supply_kw` **follows the wire** out from the consumer
/// rather than searching one 64-tile disc, so a pole run of any length carries
/// power the planner can see. Before that fix this fallback would have sited a
/// plant the consumer could not be shown to draw from. The pole run itself is
/// the caller's — nothing here emits it — and its price, against piping the
/// water the other way, is [`pole_run_items`] and [`pipe_run_plates`].
fn retry_from_world_anchor(
    state: &PlanState,
    from: &Position,
    kw: f64,
    roster_anchored: PlannerError,
) -> Result<Supply, PlannerError> {
    if !matches!(
        roster_anchored,
        PlannerError::PowerPlantNeedsWater { .. } | PlannerError::PowerPlantNeedsShore { .. }
    ) {
        return Err(roster_anchored);
    }
    // Already effectively anchored there: the retry would read the same
    // terrain to reach the same refusal. `PLANT_WATER_SCAN_RADIUS` is the
    // cheap tier's own bound, so inside it the two searches see the same
    // lakes.
    let anchor = plant_world_anchor();
    if calculate_distance(&anchor, from) < f64::EPSILON {
        return Err(roster_anchored);
    }
    if let Some(pole) = state.nearest_supply_anchor(&anchor, PLANT_ADOPT_RADIUS, kw) {
        return Ok(Supply::Standing(pole));
    }
    if let Some(plant) = complete_plant(state, &anchor, kw) {
        return Ok(Supply::Build(plant));
    }
    plan_plant_for(state, &anchor, kw)
        .map(Supply::Build)
        .map_err(|_| roster_anchored)
}

/// The rest of a plant that already has its pump down, if one stands within
/// [`PLANT_ADOPT_RADIUS`] of `from` and finishing it would leave `kw` of
/// headroom.
///
/// # What "a plant somebody started" is taken to mean
///
/// An offshore pump is the one part of a plant that names the whole layout:
/// [`layout`] is a rigid body hung off the pump's tile and facing, so a pump
/// standing at `p` facing `d` says exactly where its pipes, boiler and engine
/// belong. Every such pump is a candidate, nearest first, and for each one
/// every part of the layout is checked against the world:
///
/// * an entity **of the part's own name on the part's own tile** is the part,
///   already standing, and is neither billed nor placed again;
/// * **empty ground** is a part to place;
/// * **anything else** -- a different entity on the tile, a rock, water where
///   the engine goes -- means this pump's plant cannot be finished as
///   designed, and the pump is passed over. Nothing here plans a plant
///   *around* an obstacle; that is [`plan_plant`]'s job, on ground it chooses.
///
/// The pole is sited exactly as [`fit`] sites it -- but a pole that already
/// stands and reaches the engine is taken first, because the engine may have
/// been the one part a cut-short plan never placed while its pole went down
/// early (bots deal a plant's parts out and place them in whatever order
/// their walks allow; `run-1788608648-56109`'s first plan had all four poles
/// and the pump down before its boiler, and never reached the engine).
///
/// # Why it is asked after the standing-network tiers and before the shoreline
///
/// A network that already generates and has headroom costs nothing; this
/// costs the missing parts; a fresh plant costs all of them. A pump whose
/// plant *is* complete and wired is found by tier 2 when it has headroom, and
/// comes here with nothing missing when it has none -- in which case it is
/// skipped, and the next pump along may be the half-built one whose engine
/// would double the supply. That is the fourth plan of the run above: it had
/// a working 900 kW plant with 145 kW left, a cell to build wanting 189, and
/// a pump-and-boiler 20 tiles away wanting only an engine.
///
/// # Headroom is checked on the finished plant, not assumed
///
/// The completed engine's pole may join a network that is already
/// over-committed, in which case 900 kW more is not `kw` of headroom. So the
/// candidate is finished on a fork and asked the same question
/// [`PlanState::nearest_supply_anchor`] asks of a standing network, at its own
/// pole. A pump whose finished plant would still not carry the consumer is
/// passed over like an unusable one.
///
/// Deterministic: pumps are ordered by `(distance, x, y)` with `total_cmp`,
/// the layout is a fixed sequence, and the pole search is
/// [`free_area_near_where`]'s.
pub fn complete_plant(state: &PlanState, from: &Position, kw: f64) -> Option<Plant> {
    let mut pumps: Vec<(f64, FactorioEntity)> = state
        .entities_within(from, PLANT_ADOPT_RADIUS)
        .into_iter()
        .filter(|entity| entity.name == PUMP)
        .map(|entity| (calculate_distance(&entity.position, from), entity))
        .collect();
    pumps.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.position.x.total_cmp(&b.1.position.x))
            .then(a.1.position.y.total_cmp(&b.1.position.y))
    });
    pumps
        .into_iter()
        .find_map(|(_, pump)| finish(state, &pump, kw))
}

/// [`complete_plant`] for one pump: the plant its layout describes, with the
/// standing parts split from the missing ones, or `None` when it cannot be
/// finished or would not carry `kw`.
fn finish(state: &PlanState, pump: &FactorioEntity, kw: f64) -> Option<Plant> {
    let facing = Direction::from_u8(pump.direction)?;
    if !Direction::orthogonal().contains(&facing) {
        return None;
    }
    // Sized against the demand, exactly as a fresh plant is. A standing
    // one-engine plant asked for 1,200 kW is checked against a two-engine
    // layout, so the second engine's tile is either free -- and the plant is
    // finished by adding it -- or occupied, and this pump is passed over.
    // `engines_for`'s refusal is a `None` here rather than an error: this
    // function's whole contract is "or the next pump along", and
    // `plan_plant_for` raises the named error a moment later.
    let engines = engines_for(state, kw).ok()?;
    let parts = layout(&pump.position, facing, engines)?;
    let mut standing: Vec<PlantPart> = Vec::new();
    let mut missing: Vec<PlantPart> = Vec::new();
    for part in parts {
        match state.entity_at(&part.position) {
            // The same name **centred on the same tile**: `entity_at` answers
            // for any entity covering the point, and a boiler one tile off
            // the layout is not this plant's boiler.
            Some(entity)
                if entity.name == part.name
                    && Pos::from(&entity.position) == Pos::from(&part.position) =>
            {
                standing.push(part);
            }
            Some(_) => return None,
            None => {
                if !state.is_area_free_facing(part.name, &part.position, part.direction) {
                    return None;
                }
                missing.push(part);
            }
        }
    }
    let boiler = standing
        .iter()
        .chain(missing.iter())
        .find(|part| part.name == BOILER)?
        .position
        .clone();
    // Both lists, because the row may be part standing and part missing: a
    // one-engine plant being grown to two has its first engine in `standing`
    // and its second in `missing`. The build order `layout` fixed is what
    // orders them, so `engine` stays the one nearest the boiler however the
    // row is split.
    let engine_parts: Vec<&PlantPart> = layout(&pump.position, facing, engines)?
        .iter()
        .filter(|part| part.name == ENGINE)
        .map(|part| {
            standing
                .iter()
                .chain(missing.iter())
                .find(|other| other.name == ENGINE && other.position == part.position)
        })
        .collect::<Option<Vec<&PlantPart>>>()?;
    let engine = engine_parts.first()?.position.clone();
    let engine_row: Vec<Position> = engine_parts
        .iter()
        .map(|part| part.position.clone())
        .collect();

    let mut trial = state.fork();
    for part in &missing {
        trial.create_entity(entity_for(&trial, part));
    }
    let engine_areas = engine_areas(&trial, &engine_parts)?;
    let pole_origin = engine_row_centre(&engine_row);

    // A pole that already reaches **every** engine, nearest first in
    // `entities_within`'s fixed order; only otherwise one to place, sited
    // exactly as `fit` sites it. All of them, not the first: a standing pole
    // that covers one engine of two leaves the other generating into nothing.
    let pole_standing = trial
        .entities_within(
            &pole_origin,
            f64::from(crate::method::util::FREE_TILE_SEARCH_RADIUS) + 4.,
        )
        .into_iter()
        .find(|entity| {
            engine_areas
                .iter()
                .all(|area| trial.pole_would_supply(&entity.name, &entity.position, area))
        });
    let pole = match pole_standing {
        Some(entity) => {
            let position = entity.position.clone();
            standing.push(PlantPart {
                name: POLE,
                position: position.clone(),
                direction: Direction::North,
            });
            position
        }
        None => {
            let position = pole_site(&trial, &engine_areas, &pole_origin)?;
            let part = PlantPart {
                name: POLE,
                position: position.clone(),
                direction: Direction::North,
            };
            trial.create_entity(entity_for(&trial, &part));
            missing.push(part);
            position
        }
    };
    // A plant with nothing missing is not something to finish: either tier 2
    // already found it, or its network has no headroom and finishing it
    // changes nothing. Either way the next pump along is the one to ask.
    if missing.is_empty() {
        return None;
    }
    // Finished, would it carry the consumer? The same question a standing
    // network is asked, at this plant's own pole.
    trial.nearest_supply_anchor(&pole, 0.5, kw)?;

    let mut plant = Plant {
        parts: missing,
        standing,
        boiler,
        engine,
        engines: engine_row,
        pole,
        evacuate: Vec::new(),
    };
    match crate::enclosure::check(state, &trial, &pump.position) {
        crate::enclosure::EnclosurePrevention::Clear => {}
        crate::enclosure::EnclosurePrevention::Evacuate(evacuations) => {
            plant.evacuate = evacuations;
        }
        crate::enclosure::EnclosurePrevention::Refuse => return None,
    }
    Some(plant)
}

/// Find somewhere to build a plant within reach of `from`, or say why not.
///
/// Deterministic by construction: the water tile is the one
/// `EntityGraph::nearest_water_tile` orders first, the shoreline candidates
/// come out of a ring search in a fixed order, and the four facings are tried
/// north, east, south, west. Nothing here reads a quad tree's own order.
pub fn plan_plant(state: &PlanState, from: &Position) -> Result<Plant, PlannerError> {
    // `engines_for(0.)` is one, so this is exactly the plant this function has
    // always sited. Kept so that every caller and test that does not care
    // about sizing carries on not caring.
    plan_plant_for(state, from, 0.)
}

/// [`plan_plant`], sized against a demand.
///
/// **The tier that was not checking its kilowatts.** [`supply_for`]'s first
/// three tiers all ask whether the supply they found leaves `kw` of headroom;
/// the fourth built a fixed 900 kW plant and returned it for any demand
/// whatever, so a plan wanting 2,000 kW got a plant short by 1,100 and an
/// `Ok`. That shortfall then surfaced as a `Condition::Powered` that would not
/// hold — the symptom, several layers from the decision that caused it. See
/// `docs/superpowers/notes/2026-09-06-power-as-capacity-over-time.md`.
///
/// The engine count comes from [`engines_for`], which refuses with
/// [`PlannerError::PowerPlantTooSmall`] above [`MAX_ENGINES_PER_BOILER`]
/// rather than returning an undersized plant. The refusal happens **before**
/// any terrain is read, so a demand no plant can carry costs nothing to
/// discover.
pub fn plan_plant_for(state: &PlanState, from: &Position, kw: f64) -> Result<Plant, PlannerError> {
    let engines = engines_for(state, kw)?;
    // The narrow search first, and the wide one **only** when it fails.
    // `nearest_water_tile` is linear in the tiles inside its radius, and since
    // `fa8dabf3` a fully charted map carries ~410,000 water tiles: reading a
    // 256-by-256 box on every expansion would charge every successful plan for
    // a case that almost never arises.
    //
    // The second tier *builds against* what it finds. It used to only measure
    // it and then refuse -- see `PLANT_WATER_SCAN_RADIUS` for the run that
    // cost, and for why a longer walk is a worse plan rather than an
    // impossible one. Both tiers order candidates the same way, so a lake at
    // 60 tiles is preferred over one at 100 by the first tier ever seeing it,
    // not by any comparison here.
    let water = match state.nearest_water_tile(from, PLANT_WATER_SCAN_RADIUS) {
        Some(near) => near,
        None => state
            .nearest_water_tile(from, PLANT_WATER_WIDE_SCAN_RADIUS)
            .ok_or(PlannerError::PowerPlantNeedsWater {
                radius: PLANT_WATER_WIDE_SCAN_RADIUS,
            })?,
    };
    let anchor = Pos::from(&water.position);
    let distance = calculate_distance(&tile_centre(&anchor), from);

    // One bounded read of the terrain rather than one per candidate tile. The
    // margin is the two tiles a shoreline test reaches beyond its own tile.
    let reach = f64::from(SHORE_SEARCH_RADIUS) + 2.;
    let centre = tile_centre(&anchor);
    let bounds = Rect::new(
        &Position::new(centre.x() - reach, centre.y() - reach),
        &Position::new(centre.x() + reach, centre.y() + reach),
    );
    let water_tiles: BTreeSet<Pos> = state
        .water_tiles_within(&bounds)
        .iter()
        .map(|tile| Pos::from(&tile.position))
        .collect();

    for radius in 0..=SHORE_SEARCH_RADIUS {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                // Only the ring at exactly this radius; inner ones were done.
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let tile = Pos(anchor.0 + dx, anchor.1 + dy);
                for facing in Direction::orthogonal() {
                    if !shoreline_faces_water(&tile, facing, &water_tiles) {
                        continue;
                    }
                    if let Some(plant) = fit(state, &tile_centre(&tile), facing, engines) {
                        return Ok(plant);
                    }
                }
            }
        }
    }
    Err(PlannerError::PowerPlantNeedsShore {
        distance,
        radius: f64::from(SHORE_SEARCH_RADIUS),
    })
}

/// Does a whole plant fit with its pump at `pump` facing `facing`, pole and
/// all?
///
/// The buildings are checked against `state` rather than against each other:
/// [`layout`] is a rigid body whose parts provably do not overlap
/// (`the_plants_own_buildings_never_overlap_each_other`), so the only question
/// left is whether the world is in the way.
///
/// The pole is different, because *where* it goes depends on where the
/// buildings ended up. It is sited on a fork carrying them, so its ring search
/// cannot pick a tile the engine is standing on.
fn fit(state: &PlanState, pump: &Position, facing: Direction, engines: u32) -> Option<Plant> {
    let parts = layout(pump, facing, engines)?;
    for part in &parts {
        if !state.is_area_free_facing(part.name, &part.position, part.direction) {
            return None;
        }
    }
    let boiler = parts
        .iter()
        .find(|part| part.name == BOILER)?
        .position
        .clone();
    let engine_parts: Vec<&PlantPart> = parts.iter().filter(|part| part.name == ENGINE).collect();
    let engine = engine_parts.first()?.position.clone();
    let engine_row: Vec<Position> = engine_parts
        .iter()
        .map(|part| part.position.clone())
        .collect();

    let mut trial = state.fork();
    for part in &parts {
        trial.create_entity(entity_for(&trial, part));
    }
    let engine_areas = engine_areas(&trial, &engine_parts)?;
    let pole = pole_site(&trial, &engine_areas, &engine_row_centre(&engine_row))?;

    let mut parts = parts;
    let pole_part = PlantPart {
        name: POLE,
        position: pole.clone(),
        direction: Direction::North,
    };
    // Into `trial` too, not just `parts` -- the enclosure check below reads
    // `trial`'s own `added` map, and a plant checked without its pole would
    // miss the one part sited last.
    trial.create_entity(entity_for(&trial, &pole_part));
    parts.push(pole_part);
    let mut plant = Plant {
        parts,
        standing: Vec::new(),
        boiler,
        engine,
        engines: engine_row,
        pole,
        evacuate: Vec::new(),
    };
    // Last, for the same reason `method::assemble::fit` runs it last: every
    // cheaper check above already had the chance to reject this candidate
    // for free.
    match crate::enclosure::check(state, &trial, pump) {
        crate::enclosure::EnclosurePrevention::Clear => {}
        crate::enclosure::EnclosurePrevention::Evacuate(evacuations) => {
            plant.evacuate = evacuations;
        }
        crate::enclosure::EnclosurePrevention::Refuse => return None,
    }
    Some(plant)
}

/// The collision area of every engine in the row, or `None` if the world
/// cannot size one.
///
/// `None` rather than a shorter list: a plant whose second engine has no
/// footprint is one whose pole cannot be checked against it, and an unsized
/// entity is not one this planner commits to — the same answer `is_area_free`
/// gives for an unknown prototype.
fn engine_areas(state: &PlanState, engine_parts: &[&PlantPart]) -> Option<Vec<Rect>> {
    engine_parts
        .iter()
        .map(|part| state.collision_area_facing(ENGINE, &part.position, part.direction))
        .collect()
}

/// Where to put the plant's pole: a free tile whose supply area reaches
/// **every** engine in the row.
///
/// # One implementation, because a second one would be the failure
///
/// Both [`fit`] and [`finish`] site a pole, and both used to ask their own
/// question about their own engine. With a one-engine row those two questions
/// are the same; with two they are not, and a pole that reaches the first
/// engine and not the second is 900 kW standing on the ground while the plan
/// reads 1,800 — placement and function are separate concerns, and this is
/// the class of failure that places 100 % and does nothing.
///
/// So the rule lives here once. `areas` is every engine's collision box, and
/// `all` over it is the whole of the rule: a candidate that misses one engine
/// is not a site, however close it is.
///
/// The search starts at [`engine_row_centre`] rather than at any one engine.
/// With one engine that is the engine, which is where this search has always
/// started; with two it is the joint between them, which is the only stretch
/// of ground a five-by-five supply area can overlap both from.
///
/// `None` when no tile in [`free_area_near_where`]'s ring search qualifies —
/// which refuses the whole plant candidate, rather than building a row half
/// of which generates into nothing.
fn pole_site(state: &PlanState, areas: &[Rect], origin: &Position) -> Option<Position> {
    free_area_near_where(state, origin, POLE, |candidate| {
        areas
            .iter()
            .all(|area| state.pole_would_supply(POLE, candidate, area))
    })
}

/// The point the pole search starts from: the centroid of the engine row.
///
/// **The single-engine path is unchanged by construction, not by luck** — the
/// centroid of one position is that position, which is the origin
/// [`free_area_near_where`] was always given. With two engines it is the joint
/// between them, which is the only place a five-by-five supply area can
/// overlap both.
///
/// The caller has already checked the row is non-empty ([`engines_for`] never
/// answers zero); an empty row would divide by zero, so it answers the origin
/// instead, which no plant is ever sited at.
fn engine_row_centre(engines: &[Position]) -> Position {
    if engines.is_empty() {
        return Position::new(0., 0.);
    }
    let count = engines.len() as f64;
    Position::new(
        engines.iter().map(Position::x).sum::<f64>() / count,
        engines.iter().map(Position::y).sum::<f64>() / count,
    )
}

/// The `FactorioEntity` a part places.
///
/// `entity_type` is read from the prototype rather than guessed: a steam
/// engine's type is `generator` and a small electric pole's is `electric-pole`,
/// neither of which is its name, and `EntityGraph::add`'s whitelist is keyed on
/// the pair.
pub(crate) fn entity_for(state: &PlanState, part: &PlantPart) -> FactorioEntity {
    let entity_type = state
        .base()
        .entity_prototypes
        .get(part.name)
        .map(|proto| proto.entity_type.clone())
        .unwrap_or_else(|| part.name.to_string());
    FactorioEntity {
        name: part.name.to_string(),
        entity_type,
        position: part.position.clone(),
        direction: Direction::to_u8(&part.direction).unwrap_or(0),
        ..Default::default()
    }
}

/// Is `part.position` on the build grid its own prototype gives it?
///
/// Only a test asks, but it asks of all four facings, which is the claim that
/// rotating the whole plant keeps it legal.
#[cfg(test)]
fn on_its_grid(state: &PlanState, part: &PlantPart) -> bool {
    let (offset_x, offset_y) =
        crate::method::util::tile_alignment_facing(state, part.name, part.direction);
    let fract = |v: f64, offset: f64| (v - offset).fract().abs() < 1. / 512.;
    fract(part.position.x(), offset_x) && fract(part.position.y(), offset_y)
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// What the plant needs in the acting bot's hands, in emission order.
///
/// `Holder::Share`, not `Holder::Anyone`, for the same reason the lab and the
/// science packs use it: one bot places these, so one bot has to be holding
/// them, and `Anyone` sizes its shortfall against the sum across the roster.
///
/// **Read off [`Plant::parts`], not stated.** A whole plant bills a pump,
/// three pipes, a boiler, an engine and a pole; a plant being finished bills
/// only what is missing, which is the whole point of finishing it. The coal
/// is billed either way: a boiler that stands may have burnt through what it
/// was given.
fn bill(plant: &Plant) -> Vec<(&'static str, u32)> {
    let mut out: Vec<(&'static str, u32)> = Vec::new();
    for part in &plant.parts {
        match out.iter_mut().find(|(name, _)| *name == part.name) {
            Some((_, count)) => *count += 1,
            None => out.push((part.name, 1)),
        }
    }
    // Per engine, because the boiler burns fuel in proportion to the steam its
    // engines draw: a two-engine plant was sized for roughly twice the load
    // and burns roughly twice the coal over the same window. A one-engine
    // plant -- which is every plant this planner builds today -- bills the
    // unchanged `PLANT_COAL`.
    //
    // The row is read off `parts` and `standing` together rather than off
    // `parts` alone: the coal is for the whole plant, and a finished plant
    // whose first engine already stands still has two engines drinking steam.
    let engines = plant
        .parts
        .iter()
        .chain(plant.standing.iter())
        .filter(|part| part.name == ENGINE)
        .count()
        .max(1) as u32;
    out.push(("coal", PLANT_COAL * engines));
    out
}

/// The steps that build `plant`, and the ids the caller must order its
/// research after.
///
/// **The research is not ordered after these by inference.** No `Effect`
/// satisfies `Condition::Powered` — it is a statement about the world, checked
/// against the state, and `ActionNetwork::infer_edges` can draw no edge to it.
/// So every id comes back and the caller states the edges, exactly as it
/// already states the pack-insert-before-research edges.
///
/// **Every** id, not just the generator's. `Condition::Powered` counts
/// nameplate capacity, so as far as the *model* is concerned an engine and a
/// pole are enough — but an engine with no steam produces nothing, and a
/// boiler with no water or no coal produces no steam. Ordering the research
/// after the pipes and the pump too is the difference between a plan that is
/// right about the model and one that is right about the game. The scheduler
/// would probably get there anyway, by rejecting the research until `Powered`
/// holds; "probably" is not an ordering.
///
/// Every building is also **reserved in `ctx.state` as its `Place` is
/// emitted**, the same discipline the lab placement uses: `expand` returns its
/// whole step list before `run_steps` executes any of it, so a site left
/// unreserved would be chosen twice by two subtrees of the same plan.
pub fn plant_steps(ctx: &mut ExpansionCtx, plant: &Plant) -> (Vec<Step>, Vec<ActionId>) {
    let mut steps: Vec<Step> = Vec::new();
    let mut order_research_after: Vec<ActionId> = Vec::new();

    for (item, count) in bill(plant) {
        steps.push(Step::Subgoal(Goal::Have {
            item: item.into(),
            count,
            whose: Holder::Share(ctx.chain_actor),
        }));
    }

    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);
    let reach = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.reach_distance)
        .unwrap_or(10.0);

    // Every bystander `fit` found would be sealed in by this plant walks
    // clear before any of its parts go down -- see `crate::enclosure::check`.
    // Not folded into `order_research_after`: that list gates power-dependent
    // actions, which already wait on every one of the plant's own placements,
    // and those placements are what the `Step::Link`s below make wait on the
    // evacuation in turn.
    let evacuation_ids: Vec<ActionId> = plant
        .evacuate
        .iter()
        .map(|evacuation| {
            let (step, id) =
                crate::method::util::evacuation_step(ctx, evacuation, "the power plant");
            steps.push(step);
            id
        })
        .collect();
    let mut part_ids: Vec<ActionId> = Vec::new();

    for part in &plant.parts {
        let entity = entity_for(&ctx.state, part);
        let direction = entity.direction;
        // The annulus's inner bound, exactly as the furnace and the lab use
        // it: standing *on* the tile a building is going for satisfies a plain
        // disc and then has the game refuse the build with
        // `player_blocks_placement`.
        let min_radius = ctx.state.placement_clearance(part.name).unwrap_or(0.0);
        let id = ctx.ids.next();
        order_research_after.push(id);
        part_ids.push(id);
        steps.push(Step::Act(Box::new(Action {
            id,
            kind: ActionKind::Place {
                entity: Box::new(entity.clone()),
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: part.position.clone(),
                    radius: build,
                    min_radius,
                },
                Condition::AreaFree {
                    pos: part.position.clone(),
                    entity: part.name.into(),
                    direction,
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: part.name.into(),
                    count: 1,
                },
            ],
            eff: vec![
                Effect::LoseItem {
                    who: Actor::Role,
                    item: part.name.into(),
                    count: 1,
                },
                Effect::CreateEntity(Box::new(entity.clone())),
            ],
            duration: PLACE_TICKS,
            pinned: None,
            label: format!("place {} at {}", part.name, part.position),
        })));
        ctx.state.create_entity(entity);
    }
    for evacuation_id in &evacuation_ids {
        for part_id in &part_ids {
            steps.push(Step::Link {
                from: *evacuation_id,
                to: *part_id,
                lag: 0,
            });
        }
    }

    let fuel = ctx.ids.next();
    order_research_after.push(fuel);
    steps.push(Step::Act(Box::new(Action {
        id: fuel,
        kind: ActionKind::Insert {
            pos: plant.boiler.clone(),
            entity: BOILER.into(),
            slot: InventorySlot::Fuel,
            item: "coal".into(),
            count: PLANT_COAL,
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: plant.boiler.clone(),
                radius: reach,
                min_radius: 0.0,
            },
            Condition::EntityAt {
                pos: plant.boiler.clone(),
                name: BOILER.into(),
            },
            Condition::HasItem {
                who: Actor::Role,
                item: "coal".into(),
                count: PLANT_COAL,
            },
        ],
        eff: vec![Effect::LoseItem {
            who: Actor::Role,
            item: "coal".into(),
            count: PLANT_COAL,
        }],
        duration: crate::method::have::TRANSFER_TICKS,
        pinned: None,
        label: format!("fuel the boiler with {} coal", PLANT_COAL),
    })));

    (steps, order_research_after)
}

// ---------------------------------------------------------------------------
// One place that decides power
// ---------------------------------------------------------------------------

/// A small electric pole's copper-wire reach, in tiles, from vanilla 2.1.
///
/// **A second copy of a number [`crate::state::PlanState`]'s `pole_wire_reach`
/// already holds** -- and, since this section moved here from
/// `method::extract` on 2026-09-06, a third one sits eight hundred lines above
/// it as [`POLE_WIRE_REACH_TILES`]. That is worth saying out loud rather than
/// hiding. The state table is private and this module is not allowed to widen
/// it, so the duplicate is pinned *behaviourally* instead of by inspection --
/// `method::extract`'s `two_poles_a_wire_reach_apart_are_one_network` builds
/// two poles at exactly this distance and at half a tile more, and asks
/// [`crate::state::PlanState`] itself which pairs share a network. A test that
/// merely compared this constant to a copy of itself would agree with the code
/// that wrote it, which is the failure
/// `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md` is
/// about. [`POLE_WIRE_REACH_TILES`] is deliberately *not* folded into this:
/// they hold the same number today, but one is the reach a wire spans and the
/// other is what [`pole_run_items`]' bill arithmetic is written against, and
/// merging them would make a future change to either silently change both.
///
/// Nothing in `pole_run` *depends* on it being right: the run's final check
/// is the game's rule evaluated over a fork. Getting it wrong makes a run
/// refuse, or lay more poles than it needed; it cannot make one that does not
/// carry power read as one that does.
pub const WIRE_REACH: f64 = 7.5;

/// How far apart `pole_run` aims to put consecutive poles, in tiles.
///
/// Strictly less than [`WIRE_REACH`], and the margin is not decorative. A pole
/// is placed on a tile, so a nominal point is snapped to a tile centre, and
/// the ring search may then move it a further ring or two to find free ground.
/// At exactly the reach, either of those pushes the pair out of contact and
/// the link is silently gone -- silently, because a broken wire looks exactly
/// like a pole that is standing. 6.0 leaves 1.5 tiles of slack, which covers
/// the half-tile of snapping plus one ring of search, and costs one extra pole
/// per 30 tiles of run.
pub const POLE_STEP: f64 = 6.;

/// The longest pole run [`ensure_powered`] will lay, in poles.
///
/// **A bill bound, not a distance bound.** The walk is already priced by
/// [`crate::schedule`], exactly as [`PLANT_WATER_SCAN_RADIUS`]' doc argues, so
/// distance alone is a worse plan rather than an impossible one. What is *not*
/// already priced is the wood: a small electric pole is half a wood plus a
/// copper cable, a four-bot run starts with four wood, and everything beyond
/// that is trees to be chopped. 64 poles is 32 wood and about 380 tiles of run
/// at [`POLE_STEP`] -- past the 256-to-384 tiles at which seed 31337's first
/// crude oil is charted, and far enough that a longer one wants a big-pole run
/// and a real argument rather than a bigger number here.
///
/// Exceeding it makes [`ensure_powered`] answer `Ok(None)`, which each caller
/// turns into its own named refusal.
pub const MAX_POLE_RUN: usize = 64;

/// What [`ensure_powered`] hands back: the steps that make the site powered,
/// the ids the caller must order its own placement after, and the
/// [`Condition::Powered`] those steps were chosen to satisfy.
///
/// The condition travels with the steps on purpose. It is the *same* value the
/// run was checked against inside `pole_run`, so a caller that puts it on
/// its `Place` as a precondition cannot state a different draw, a different
/// prototype or a different tile from the one the poles were laid for. Two
/// independently constructed copies would be free to drift.
pub struct Powering {
    /// Plant, pole bills and pole placements, in build order.
    pub steps: Vec<Step>,
    /// Every action id the consumer's own placement must be ordered after.
    ///
    /// Every id, not just the generator's: an engine with no steam produces
    /// nothing and a boiler with no water makes no steam, so the consumer
    /// waits for the whole plant. Nothing satisfies [`Condition::Powered`], so
    /// `infer_edges` draws no edge on its own -- the caller holds both ends
    /// and the caller states them.
    pub ids: Vec<ActionId>,
    /// The headroom test the caller should put on its placement.
    pub powered: Condition,
}

/// Somewhere with the capacity to run `kw`: a network that already stands, or
/// a plant built inline.
///
/// The half of the power decision that both [`ensure_powered`] and
/// `method::assemble` need. It returns the **anchor** -- a position on a
/// powered network -- together with the steps that build the plant when one
/// had to be built, and the ids everything downstream must be ordered after.
///
/// A standing network answers with no steps and no ids: there is nothing to
/// wait for.
///
/// It has to be inline rather than a subgoal in both callers, for the same
/// reason: a subgoal is expanded after the method returns, and the site is
/// chosen *from* the anchor, so a caller planned against a state with no plant
/// in it has nowhere to be.
pub fn supply_anchor(
    ctx: &mut ExpansionCtx,
    from: &Position,
    radius: f64,
    kw: f64,
) -> Result<(Position, Vec<Step>, Vec<ActionId>), PlannerError> {
    match supply_for(&ctx.state, from, radius, kw)? {
        Supply::Standing(anchor) => Ok((anchor, Vec::new(), Vec::new())),
        Supply::Build(plant) => {
            let anchor = plant.pole.clone();
            let (steps, ids) = plant_steps(ctx, &plant);
            Ok((anchor, steps, ids))
        }
    }
}

/// **The one place that decides power for a site.** If `consumer` standing at
/// `site` with footprint `area` and draw `kw` is not already powered, find
/// supply within `radius` and lay a pole run to it.
///
/// `occupants` is what the site will hold once the caller places it: they are
/// created in the routing fork *before* the pole search, so a pole cannot be
/// sited on ground the caller's own building is about to take. They are not
/// reserved in `ctx.state` -- that happens with the caller's `Place` -- so a
/// refusal between here and there leaves nothing behind.
///
/// # Three answers, and the middle one is the interesting one
///
/// * `Err` -- supply itself is impossible, and [`supply_for`] says why by
///   name (`PowerPlantNeedsShore`, `PowerPlantTooSmall`, a `Have` shortfall).
/// * `Ok(None)` -- supply exists but **no run of at most [`MAX_POLE_RUN`]
///   poles carries it there**, or the model cannot see the finished run
///   carrying power. The caller names its own refusal, because what an
///   unreachable site means differs: `method::extract` calls it
///   `ExtractionNotModelled`.
/// * `Ok(Some(_))` -- the steps, the ids and the condition. An
///   already-powered site answers this way too, with empty steps and empty
///   ids, so a caller never has to write the "already powered" branch itself.
///
/// # It refuses before it emits
///
/// Nothing is returned unless the [`Condition::Powered`] holds against a fork
/// carrying every pole -- the game's own rule, walked over
/// [`crate::state::PlanState`]'s union-find, not this module's [`POLE_STEP`]
/// arithmetic. A half-built power line is worse than none: the machine stands,
/// draws nothing, and reads as placed. The plant's own steps are emitted
/// before that check, which is the one place this differs from
/// `method::connect`'s all-or-nothing promise -- a plant is worth having on
/// its own, a half-run of poles is not.
///
/// # Why it takes `kw` rather than computing it
///
/// A blueprint's draw is the sum over its decoded entity list
/// (`state.rs`'s `consumer_kw`), a cell's is `cell_demand_kw` times the number
/// of cells, and an extractor's is one `consumer_draw_kw` lookup. Only the
/// caller knows which. What this function guarantees is that whatever number
/// arrives is the number [`supply_for`] sizes the plant against *and* the
/// number the returned [`Condition::Powered`] tests for -- so coverage and
/// capacity cannot disagree.
pub fn ensure_powered(
    ctx: &mut ExpansionCtx,
    consumer: &str,
    site: &Position,
    area: &Rect,
    kw: f64,
    radius: f64,
    occupants: &[FactorioEntity],
) -> Result<Option<Powering>, PlannerError> {
    let powered = Condition::Powered {
        pos: site.clone(),
        entity: consumer.into(),
        kw,
    };
    if powered.holds(&ctx.state, ctx.chain_actor) {
        return Ok(Some(Powering {
            steps: Vec::new(),
            ids: Vec::new(),
            powered,
        }));
    }

    let (anchor, mut steps, mut ids) = supply_anchor(ctx, site, radius, kw)?;

    let mut trial = ctx.state.fork();
    for occupant in occupants {
        trial.create_entity(occupant.clone());
    }
    let Some(run) = pole_run(&mut trial, &anchor, site, area, &powered, ctx.chain_actor)? else {
        return Ok(None);
    };
    for pole in run {
        steps.push(Step::Subgoal(Goal::Have {
            item: POLE.into(),
            count: 1,
            whose: Holder::Share(ctx.chain_actor),
        }));
        let (step, id) = place_step(ctx, POLE, &pole);
        steps.push(step);
        ids.push(id);
    }
    Ok(Some(Powering {
        steps,
        ids,
        powered,
    }))
}

// ---------------------------------------------------------------------------
// Carrying power to the site
// ---------------------------------------------------------------------------

/// The poles that join the network at `anchor` to the extractor standing at
/// `site` with footprint `area`, in build order and **including** the pole
/// that covers the extractor itself.
///
/// `trial` is mutated: every pole chosen is created in it, so the next ring
/// search cannot pick a tile an earlier pole took. The caller passes a fork
/// that already carries the extractor.
///
/// `Ok(None)` means "no run of at most [`MAX_POLE_RUN`] poles carries it",
/// which the caller turns into [`PlannerError::ExtractionNotModelled`].
/// `Err` is only what a `Have` shortfall would raise, which cannot happen
/// here -- the bill is emitted by the caller.
///
/// # It refuses before it emits, and the refusal is the game's own rule
///
/// Nothing is returned unless `powered` -- the extractor's own
/// [`Condition::Powered`], a headroom test -- holds against `trial` with every
/// pole in it. That check walks [`crate::state::PlanState`]'s union-find over
/// the poles' real wire distances and its own supply-area table, so a run that
/// this module's [`POLE_STEP`] arithmetic thought was fine but the game would
/// not wire together is refused rather than built. A half-built power line is
/// worse than none: the machine stands, draws nothing, and reads as placed.
pub(crate) fn pole_run(
    trial: &mut PlanState,
    anchor: &Position,
    site: &Position,
    area: &factorio_bot_core::types::Rect,
    powered: &Condition,
    actor: crate::ids::BotId,
) -> Result<Option<Vec<Position>>, PlannerError> {
    // The pole that covers the machine. Sited first, because it is the one
    // whose position is constrained by something other than the run.
    //
    // A ring search of this module's own rather than `free_area_near_where`'s,
    // for one reason: that helper **refuses any candidate covering ore**, and
    // the tile this pole has to reach is by construction the middle of a
    // resource patch. That rule is right for the buildings it was written for
    // and wrong here -- a pole collides on nothing a resource carries, and the
    // game builds one straight over a patch.
    let head = {
        let snapshot = trial.fork();
        match ring_search(&snapshot, site, |candidate| {
            snapshot.pole_would_supply(POLE, candidate, area)
        }) {
            Some(head) => head,
            None => return Ok(None),
        }
    };
    let Some(path) = route_poles(trial, anchor, &head) else {
        return Ok(None);
    };
    for pole in &path {
        trial.create_entity(entity_for(
            trial,
            &crate::method::power::PlantPart {
                name: POLE,
                position: pole.clone(),
                direction: Direction::North,
            },
        ));
    }

    // The one check that matters, and the only one not made of this module's
    // own arithmetic: the game's rule, over a fork carrying every pole.
    if !powered.holds(trial, actor) {
        return Ok(None);
    }
    Ok(Some(path))
}

/// How far a pole may be looked for around a nominal tile, in tiles.
///
/// The same 12 as `method::util::FREE_TILE_SEARCH_RADIUS`, which is what every
/// other siting search in this crate uses; stated here because this module
/// does its own ring search (see `pole_run` on why) and a search that quietly
/// used a different radius from the rest of the crate would make "no room"
/// mean two things.
const POLE_SEARCH_RADIUS: i32 = 12;

/// The nearest tile centre to `from`, outwards in fixed rings, on which a small
/// electric pole could stand and which `accept` allows.
///
/// A pole is one tile, so its build grid is tile centres and the candidate is
/// `floor + 0.5` -- the same half-tile convention as a resource entity, and
/// the reason this can compare candidates against a well's own position without
/// converting anything.
fn ring_search(
    state: &PlanState,
    from: &Position,
    accept: impl Fn(&Position) -> bool,
) -> Option<Position> {
    let base_x = from.x().floor() as i32;
    let base_y = from.y().floor() as i32;
    for radius in 0..=POLE_SEARCH_RADIUS {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let candidate =
                    Position::new(f64::from(base_x + dx) + 0.5, f64::from(base_y + dy) + 0.5);
                if !state.is_area_free(POLE, &candidate) || state.is_site_refused(POLE, &candidate)
                {
                    continue;
                }
                if accept(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// How many pole sites [`route_poles`] may examine before giving up.
///
/// A budget rather than a bound on the answer: the search is best-first, so on
/// open ground it walks almost straight to the head and spends a few dozen
/// nodes for a run of ten poles. The budget is what stops a run that is walled
/// in -- a lake across the whole approach, say -- from expanding a disc of
/// candidates until somebody's patience runs out. It buys about
/// [`MAX_POLE_RUN`] detours' worth of exploration, which is far more than any
/// run this planner would want to build.
const ROUTE_BUDGET: usize = 4_000;

/// A run of pole sites from `anchor` to `head`, each within [`WIRE_REACH`] of
/// the last, **excluding** the anchor and **including** the head.
///
/// # Why this is a search and not a straight line
///
/// It was a straight line for exactly one measurement. On the seed-31337 dump
/// with a well charted at `[80.5, -40.5]`, the line from the power plant's pole
/// ran into ground it could not stand on at `[54.3, -24.1]` and the whole cell
/// refused -- on a map that is covered in trees, rocks and water, which is
/// every real map. A router that only works on a billiard table is a router
/// that always refuses.
///
/// So: best-first over pole sites, expanding the frontier node nearest the head
/// first. Each node offers sixteen compass directions at three step lengths,
/// which gives the search a way *around* an obstacle rather than only through
/// it, and every candidate is a real free tile checked against the same state
/// the placements will be made in.
///
/// Deterministic by construction: the frontier is ordered `(distance to head,
/// x, y)` with `total_cmp`, the direction and step tables are fixed, and
/// `visited` is a `BTreeSet` of integer tiles. Nothing reads a hash order.
///
/// **It is still not a good router.** It knows nothing about the cost of the
/// wood it is spending, it will happily take a long way round, and it has no
/// notion of sharing a run with another consumer. What it has is the property
/// that matters here: it either returns a run every hop of which stands on
/// ground the plan believes is free, or it returns nothing.
fn route_poles(state: &PlanState, anchor: &Position, head: &Position) -> Option<Vec<Position>> {
    /// The sixteen compass directions, as unit-ish vectors. Sixteen rather
    /// than eight so a detour can leave at a shallow angle instead of turning
    /// 45 degrees.
    fn directions() -> Vec<(f64, f64)> {
        (0..16)
            .map(|i| {
                let theta = std::f64::consts::TAU * f64::from(i) / 16.;
                (theta.cos(), theta.sin())
            })
            .collect()
    }
    // Long steps first: the shorter ones exist to squeeze past an obstacle,
    // not to be preferred. `POLE_STEP` itself is the nominal.
    let steps = [POLE_STEP, POLE_STEP * 0.66, POLE_STEP * 0.4];

    let tile = |p: &Position| (p.x().floor() as i32, p.y().floor() as i32);
    let mut came_from: std::collections::BTreeMap<(i32, i32), Position> =
        std::collections::BTreeMap::new();
    let mut parent: std::collections::BTreeMap<(i32, i32), (i32, i32)> =
        std::collections::BTreeMap::new();
    let mut visited: std::collections::BTreeSet<(i32, i32)> = std::collections::BTreeSet::new();
    let mut frontier: Vec<Position> = vec![anchor.clone()];
    came_from.insert(tile(anchor), anchor.clone());
    visited.insert(tile(anchor));
    let mut examined = 0usize;

    while !frontier.is_empty() {
        examined += 1;
        if examined > ROUTE_BUDGET {
            return None;
        }
        // Nearest the head first, ties by x then y so the answer depends on
        // the geometry and not on insertion order.
        let (index, _) = frontier.iter().enumerate().min_by(|(_, a), (_, b)| {
            calculate_distance(a, head)
                .total_cmp(&calculate_distance(b, head))
                .then(a.x.total_cmp(&b.x))
                .then(a.y.total_cmp(&b.y))
        })?;
        let node = frontier.remove(index);

        if calculate_distance(&node, head) <= WIRE_REACH {
            // Walk the parents back, then reverse: the anchor is dropped (it
            // already stands) and the head is appended.
            let mut run: Vec<Position> = Vec::new();
            let mut at = tile(&node);
            while at != tile(anchor) {
                run.push(came_from.get(&at)?.clone());
                at = *parent.get(&at)?;
            }
            run.reverse();
            if run.len() + 1 > MAX_POLE_RUN {
                return None;
            }
            run.push(head.clone());
            return Some(run);
        }

        for (dx, dy) in directions() {
            for step in steps {
                let nominal = Position::new(node.x() + dx * step, node.y() + dy * step);
                let candidate = Position::new(nominal.x().floor() + 0.5, nominal.y().floor() + 0.5);
                let key = tile(&candidate);
                if visited.contains(&key) {
                    continue;
                }
                if calculate_distance(&candidate, &node) > WIRE_REACH {
                    continue;
                }
                if !state.is_area_free(POLE, &candidate) || state.is_site_refused(POLE, &candidate)
                {
                    continue;
                }
                visited.insert(key);
                came_from.insert(key, candidate.clone());
                parent.insert(key, tile(&node));
                frontier.push(candidate);
            }
        }
    }
    None
}

/// One `Place` for a one-tile building, with the id the caller must order
/// against.
///
/// The same shape `power::plant_steps` emits, minus the plant's bill: the
/// caller states its own `Goal::Have` so a pole run's wood shortfall refuses
/// through the ordinary machinery.
fn place_step(ctx: &mut ExpansionCtx, name: &'static str, position: &Position) -> (Step, ActionId) {
    let entity = entity_for(
        &ctx.state,
        &crate::method::power::PlantPart {
            name,
            position: position.clone(),
            direction: Direction::North,
        },
    );
    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);
    let id = ctx.ids.next();
    let step = Step::Act(Box::new(Action {
        id,
        kind: ActionKind::Place {
            entity: Box::new(entity.clone()),
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: position.clone(),
                radius: build,
                min_radius: ctx.state.placement_clearance(name).unwrap_or(0.0),
            },
            Condition::AreaFree {
                pos: position.clone(),
                entity: name.into(),
                direction: 0,
            },
            Condition::HasItem {
                who: Actor::Role,
                item: name.into(),
                count: 1,
            },
        ],
        eff: vec![
            Effect::LoseItem {
                who: Actor::Role,
                item: name.into(),
                count: 1,
            },
            Effect::CreateEntity(Box::new(entity.clone())),
        ],
        duration: PLACE_TICKS,
        pinned: None,
        label: format!("place {name} at {position}"),
    }));
    ctx.state.create_entity(entity);
    (step, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::method::util::free_area_near;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    /// The plant's fuel load fits in the one slot it goes into.
    ///
    /// This site was already inside the cap and is the only fuel insert in the
    /// crate that never needed splitting -- five coal against a slot of fifty.
    /// It is asserted rather than assumed because [`PLANT_COAL`] is a number
    /// somebody may raise: its own doc argues from "enough for the idle window
    /// and one retry", which is an argument about *duration* and has nothing
    /// in it that stops at a stack. A boiler that quietly takes 50 of a 60-coal
    /// load and leaves 10 in the bot's pocket is the same defect the rest of
    /// this increment is about, arrived at from the other direction.
    #[test]
    fn the_plants_fuel_load_fits_in_the_one_slot_it_goes_into() {
        let s = state();
        let cap = s
            .slot_capacity(crate::action::InventorySlot::Fuel, "coal")
            .expect("the fixture carries coal");
        assert!(
            PLANT_COAL <= cap,
            "a boiler's fuel inventory is one slot holding {cap}, and the \
             plant asks for {PLANT_COAL}"
        );
    }

    #[test]
    fn the_connection_table_matches_the_prototype_the_tests_plan_against() {
        // The fixture reports Factorio 1.x's reading of `positions` -- the
        // *target* tile, one step out along the connection -- which is exactly
        // what the tables here hold. The live 2.1.17 capture reports the 2.x
        // reading and is one tile short on every entry; see `PUMP_OUTPUT`'s
        // doc. Pinning the fixture side is what stops somebody "simplifying"
        // these tables into a read of `fluidbox_prototypes` without noticing
        // that the two halves of the repo disagree.
        let s = state();
        let positions = |name: &str, box_index: usize, connection: usize| -> (f64, f64) {
            let proto = s
                .base()
                .entity_prototypes
                .get(name)
                .expect("the fixture carries the plant's prototypes");
            let fluid = proto
                .fluidbox_prototypes
                .as_ref()
                .expect("a fluid entity has fluidboxes")[box_index]
                .clone();
            let connections = fluid
                .pipe_connections
                .as_ref()
                .clone()
                .expect("a fluidbox has connections");
            let p = &connections[connection].positions[0];
            (p.x(), p.y())
        };

        assert_eq!(positions(PUMP, 0, 0), PUMP_OUTPUT, "the pump's output");
        assert_eq!(positions(BOILER, 0, 0), BOILER_WATER[0], "boiler water 0");
        assert_eq!(positions(BOILER, 0, 1), BOILER_WATER[1], "boiler water 1");
        assert_eq!(positions(BOILER, 1, 0), BOILER_STEAM, "boiler steam");
        assert_eq!(positions(ENGINE, 0, 0), ENGINE_STEAM[0], "engine 0");
        assert_eq!(positions(ENGINE, 0, 1), ENGINE_STEAM[1], "engine 1");
    }

    #[test]
    fn the_four_rotations_of_a_connection_are_the_prototypes_own_four() {
        // `Position::turn`'s sense is not obvious -- `rotate_clockwise` is
        // `(x, y) -> (y, -x)`, which is anticlockwise on a y-down screen -- so
        // it is pinned against the data rather than reasoned about. Each
        // prototype's `positions[k]` is its `positions[0]` turned by the
        // cardinal `4 * k`, and that is what makes `turned()` the right
        // rotation for these tables.
        let s = state();
        for name in [PUMP, BOILER, ENGINE, PIPE] {
            let proto = s.base().entity_prototypes.get(name).expect("prototype");
            for fluid in proto.fluidbox_prototypes.as_ref().expect("fluidboxes") {
                for connection in fluid
                    .pipe_connections
                    .as_ref()
                    .clone()
                    .expect("connections")
                {
                    let base = connection.positions[0].clone();
                    for (k, facing) in Direction::orthogonal().into_iter().enumerate() {
                        let want = &connection.positions[k];
                        let got = base.turn(facing).expect("a cardinal rotation");
                        assert!(
                            (got.x() - want.x()).abs() < 1. / 512.
                                && (got.y() - want.y()).abs() < 1. / 512.,
                            "{name}: positions[{k}] is {want:?} but turning positions[0] \
                             by {facing:?} gives {got:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_pipes_stand_where_both_neighbours_reach() {
        // The whole point of the layout: a pipe on a tile that two fluidboxes
        // both name is what joins them. Asserted at all four facings, because
        // the joints are what a wrong rotation breaks first -- and it breaks
        // *silently*, since every building still places.
        for facing in Direction::orthogonal() {
            let parts = layout(&Position::new(0.5, 0.5), facing, 1).expect("a cardinal layout");
            let at = |name: &str| {
                parts
                    .iter()
                    .find(|p| p.name == name)
                    .expect("the part exists")
                    .clone()
            };
            let pipes: Vec<Position> = parts
                .iter()
                .filter(|p| p.name == PIPE)
                .map(|p| p.position.clone())
                .collect();
            let pump = at(PUMP);
            let boiler = at(BOILER);
            let engine = at(ENGINE);

            let target = |part: &PlantPart, offset: (f64, f64)| {
                part.position
                    .add(&turned(offset, part.direction).expect("cardinal"))
            };
            let reaches = |p: &Position| {
                pipes
                    .iter()
                    .any(|pipe| calculate_distance(pipe, p) < 1. / 512.)
            };

            assert!(
                reaches(&target(&pump, PUMP_OUTPUT)),
                "{facing:?}: the pump's output has no pipe on it"
            );
            assert!(
                reaches(&target(&boiler, BOILER_WATER[1])),
                "{facing:?}: the boiler's water inlet has no pipe on it"
            );
            assert!(
                reaches(&target(&boiler, BOILER_STEAM)),
                "{facing:?}: the boiler's steam outlet has no pipe on it"
            );
            assert!(
                reaches(&target(&engine, ENGINE_STEAM[1])),
                "{facing:?}: the engine's steam inlet has no pipe on it"
            );
            // And the two pipes that carry water are neighbours, or the pump's
            // pipe is an island.
            let water_pipes: Vec<&Position> = pipes
                .iter()
                .filter(|pipe| calculate_distance(pipe, &target(&boiler, BOILER_STEAM)) > 1. / 512.)
                .collect();
            assert_eq!(water_pipes.len(), 2, "{facing:?}: two pipes carry water");
            assert!(
                (calculate_distance(water_pipes[0], water_pipes[1]) - 1.).abs() < 1. / 512.,
                "{facing:?}: the two water pipes must be adjacent, got {:?} and {:?}",
                water_pipes[0],
                water_pipes[1]
            );
        }
    }

    #[test]
    fn every_facing_puts_every_building_on_its_own_grid() {
        // A quarter turn about a tile centre maps tile centres to tile centres
        // and tile corners to tile corners, and each building's direction
        // turns with the body -- so a layout that is legal facing north is
        // legal at all four facings. That is a claim, and this is it.
        let s = state();
        for facing in Direction::orthogonal() {
            for part in layout(&Position::new(10.5, 10.5), facing, 1).expect("cardinal") {
                assert!(
                    on_its_grid(&s, &part),
                    "{facing:?}: {} at {} is off its build grid",
                    part.name,
                    part.position
                );
            }
        }
    }

    #[test]
    fn the_plants_own_buildings_never_overlap_each_other() {
        // `fit` checks each building against the world and not against its
        // siblings, which is only sound because of this.
        let s = state();
        for facing in Direction::orthogonal() {
            let parts = layout(&Position::new(10.5, 10.5), facing, 1).expect("cardinal");
            for (i, a) in parts.iter().enumerate() {
                for b in parts.iter().skip(i + 1) {
                    let box_a = s
                        .collision_area_facing(a.name, &a.position, a.direction)
                        .expect("prototype");
                    let box_b = s
                        .collision_area_facing(b.name, &b.position, b.direction)
                        .expect("prototype");
                    let overlap = box_a.left_top.x() < box_b.right_bottom.x()
                        && box_b.left_top.x() < box_a.right_bottom.x()
                        && box_a.left_top.y() < box_b.right_bottom.y()
                        && box_b.left_top.y() < box_a.right_bottom.y();
                    assert!(
                        !overlap,
                        "{facing:?}: {} at {} overlaps {} at {}",
                        a.name, a.position, b.name, b.position
                    );
                }
            }
        }
    }

    #[test]
    fn a_shoreline_needs_water_ahead_and_ground_underfoot() {
        // A lake four rows deep, so that a tile *inside* it still has the full
        // three-by-two block in front of it. A two-row lake would make the
        // last assertion below true for the wrong reason -- the ground check
        // and the water-ahead check would both refuse, and dropping the
        // ground check would change nothing.
        let mut water = BTreeSet::new();
        for x in 0..3 {
            for y in 0..4 {
                water.insert(Pos(x, y));
            }
        }
        // Standing at (1, 4) facing north: the six tiles above are the block.
        assert!(shoreline_faces_water(&Pos(1, 4), Direction::North, &water));
        // One tile of the block missing is not a shoreline.
        let mut holed = water.clone();
        holed.remove(&Pos(0, 2));
        assert!(!shoreline_faces_water(&Pos(1, 4), Direction::North, &holed));
        // Facing the other way there is no water at all.
        assert!(!shoreline_faces_water(&Pos(1, 4), Direction::South, &water));
        // And a pump may not stand in the lake it pumps from, however much
        // water is in front of it: at (1, 3) the block above is all water and
        // the *only* thing refusing is the ground under the pump.
        assert!(
            shoreline_faces_water(&Pos(1, 4), Direction::North, &water),
            "control: one row further out is a real shoreline"
        );
        assert!(
            !shoreline_faces_water(&Pos(1, 3), Direction::North, &water),
            "a tile with a perfect block of water ahead of it is still no place \
             for a pump when it is itself under water"
        );
    }

    #[test]
    fn a_pole_only_supplies_what_its_supply_area_reaches() {
        // `fit` chooses where the pole goes by this predicate alone, so a
        // predicate that says yes to everything sites the pole anywhere and
        // the plant reads as powered by luck.
        let s = state();
        let engine = s
            .collision_area(ENGINE, &Position::new(0.5, 0.5))
            .expect("the fixture carries a steam-engine prototype");
        assert!(
            s.pole_would_supply(POLE, &Position::new(2.5, 0.5), &engine),
            "a pole two tiles from a 2.5x4.7 engine reaches it"
        );
        assert!(
            !s.pole_would_supply(POLE, &Position::new(20.5, 0.5), &engine),
            "a pole twenty tiles away does not"
        );
        assert!(
            !s.pole_would_supply("stone-furnace", &Position::new(2.5, 0.5), &engine),
            "and a thing that is not a pole supplies nothing at all"
        );
    }

    #[test]
    fn a_plant_is_sited_on_the_fixtures_lake() {
        let s = state();
        let plant = plan_plant(&s, &Position::new(0., 0.)).expect("the fixture has a lake");
        assert_eq!(
            plant.parts.len(),
            7,
            "pump, three pipes, boiler, engine, pole: {:#?}",
            plant.parts
        );
        for part in &plant.parts {
            assert!(
                s.is_area_free_facing(part.name, &part.position, part.direction),
                "{} at {} does not fit",
                part.name,
                part.position
            );
        }
    }

    /// `plant_steps` turns a non-empty `evacuate` into a real
    /// [`ActionKind::Evacuate`], pinned to the bystander, ordered before
    /// every one of the plant's own placements -- the half of the wiring
    /// `tests/enclosure_prevention.rs` (real `run-1788432181-42528` geometry)
    /// cannot reach, because it exercises `crate::enclosure::check` directly
    /// rather than the step emission downstream of it.
    #[test]
    fn plant_steps_walks_a_bystander_clear_before_any_part_goes_down() {
        let s = state();
        let mut plant = plan_plant(&s, &Position::new(0., 0.)).expect("a lake");
        plant.evacuate = vec![crate::enclosure::Evacuation {
            bot: BotId(2),
            to: Position::new(1.5, 1.5),
            pocket_tiles: 4.0,
        }];

        let mut ctx = ExpansionCtx::new(s.clone(), BotId(1));
        let (steps, _needs_power) = plant_steps(&mut ctx, &plant);

        let evacuate_id = steps
            .iter()
            .find_map(|step| match step {
                Step::Act(action) if matches!(action.kind, ActionKind::Evacuate { .. }) => {
                    assert_eq!(
                        action.pinned,
                        Some(BotId(2)),
                        "an evacuation must be pinned to the bystander, never to \
                         whichever bot the scheduler later gives the chain"
                    );
                    Some(action.id)
                }
                _ => None,
            })
            .expect("an evacuate action was emitted for the bystander");

        let place_ids: Vec<ActionId> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Act(action) if matches!(action.kind, ActionKind::Place { .. }) => {
                    Some(action.id)
                }
                _ => None,
            })
            .collect();
        assert_eq!(place_ids.len(), plant.parts.len());

        let linked_from_evacuate: Vec<ActionId> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Link { from, to, .. } if *from == evacuate_id => Some(*to),
                _ => None,
            })
            .collect();
        assert_eq!(
            linked_from_evacuate.len(),
            place_ids.len(),
            "every placement must wait on the evacuation, not just the first"
        );
        for place_id in &place_ids {
            assert!(linked_from_evacuate.contains(place_id));
        }
    }

    /// Box against box, the way `PlanState::is_area_clear_of` tests it.
    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.left_top.x() < b.right_bottom.x()
            && a.right_bottom.x() > b.left_top.x()
            && a.left_top.y() < b.right_bottom.y()
            && a.right_bottom.y() > b.left_top.y()
    }

    /// A pump position at which a whole plant facing `facing` fits the
    /// fixture, found by scanning rather than assumed: `fit` wants no water
    /// under the pump, only room, so any clear patch will do, and the scan
    /// keeps the test honest about the fixture's contents.
    fn a_place_where_a_plant_fits(s: &PlanState, facing: Direction) -> (Position, Plant) {
        for y in -40..40 {
            for x in -40..40 {
                let pump = Position::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                if let Some(plant) = fit(s, &pump, facing, 1) {
                    return (pump, plant);
                }
            }
        }
        panic!("the fixture has nowhere for a plant facing {facing:?}");
    }

    /// A tile centre inside the engine's turned box **and outside its
    /// north-frame box** -- two tiles from the centre along the box's long
    /// axis. The engine is 2.5 by 4.7, so along its long axis that tile is
    /// 0.35 inside the box and along its short axis it would be 0.75 outside
    /// it: the one tile that tells a turned check from an unturned one.
    fn a_tile_only_the_turned_box_covers(s: &PlanState, engine: &PlantPart) -> Position {
        let area = s
            .collision_area_facing(ENGINE, &engine.position, engine.direction)
            .expect("the engine has a prototype");
        let c = area.center();
        let tile = if area.width() > area.height() {
            Position::new(c.x() + 2.0, c.y())
        } else {
            Position::new(c.x(), c.y() + 2.0)
        };
        assert_eq!(tile.x().fract().abs(), 0.5, "{tile} is not a tile centre");
        assert_eq!(tile.y().fract().abs(), 0.5, "{tile} is not a tile centre");
        let pole = s
            .collision_area(POLE, &tile)
            .expect("the pole has a prototype");
        assert!(overlap(&pole, &area), "{tile} is not inside {area:?}");
        let unturned = s
            .collision_area(ENGINE, &engine.position)
            .expect("the engine has a prototype");
        // South is the north frame turned twice: the same shape, so this
        // tile cannot tell them apart and the assertion is only for the two
        // facings that change the shape.
        assert!(
            !overlap(&pole, &unturned)
                || matches!(engine.direction, Direction::North | Direction::South),
            "{tile} must lie outside the north-frame box for the test to see a turn"
        );
        tile
    }

    /// **Order one: the plant is expanded first, a pole from another chain
    /// second.** Once `plant_steps` has emitted the plant, the `added`
    /// overlay carries every part with its direction, so a pole sited
    /// afterwards -- the lab's, the cell's, anyone's -- cannot land on the
    /// engine's turned box, and `free_area_near` walks it off.
    ///
    /// This is the case `run-1788569499-05724` was suspected of, and it
    /// holds: the pole at `[42.5, -5.5]` there came from the *next* plan,
    /// after the engine had been refused for a different reason entirely.
    /// Pinned all the same, in all four facings, because the east and west
    /// ones are the shape a north-frame check would get wrong.
    #[test]
    fn a_pole_sited_after_the_plant_stays_off_the_engines_turned_box() {
        for facing in Direction::orthogonal() {
            let s = state();
            let (_pump, plant) = a_place_where_a_plant_fits(&s, facing);
            let engine = plant
                .parts
                .iter()
                .find(|p| p.name == ENGINE)
                .expect("an engine");
            let inside = a_tile_only_the_turned_box_covers(&s, engine);
            assert!(
                s.is_area_free(POLE, &inside),
                "{facing:?}: before the plant is emitted the tile is open ground"
            );

            let mut ctx = ExpansionCtx::new(s, BotId(1));
            let _ = plant_steps(&mut ctx, &plant);

            assert!(
                !ctx.state.is_area_free(POLE, &inside),
                "{facing:?}: the emitted engine claims {inside}"
            );
            let sited =
                free_area_near(&ctx.state, &inside, POLE).expect("there is room for a pole");
            let pole = ctx
                .state
                .collision_area(POLE, &sited)
                .expect("the pole has a prototype");
            for part in &plant.parts {
                let area = ctx
                    .state
                    .collision_area_facing(part.name, &part.position, part.direction)
                    .expect("every part has a prototype");
                assert!(
                    !overlap(&pole, &area),
                    "{facing:?}: a pole sited at {sited} overlaps {} at {} facing {:?}",
                    part.name,
                    part.position,
                    part.direction
                );
            }
        }
    }

    /// **Order two: the pole is already there, the plant is sited second.**
    /// A pole another chain emitted onto what would be the engine's turned
    /// box refuses that candidate outright -- `fit` asks
    /// `is_area_free_facing` with the engine's own direction -- while the
    /// north-frame question, which is the one a direction-blind check would
    /// ask, still says the ground is free. That difference is the whole test.
    #[test]
    fn a_pole_already_standing_refuses_the_plant_candidate_over_it() {
        for facing in Direction::orthogonal() {
            let s = state();
            let (pump, plant) = a_place_where_a_plant_fits(&s, facing);
            let engine = plant
                .parts
                .iter()
                .find(|p| p.name == ENGINE)
                .expect("an engine");
            let inside = a_tile_only_the_turned_box_covers(&s, engine);

            let mut taken = state();
            taken.create_entity(FactorioEntity {
                name: POLE.to_string(),
                entity_type: "electric-pole".to_string(),
                position: inside.clone(),
                ..Default::default()
            });
            assert!(
                !taken.is_area_free_facing(ENGINE, &engine.position, facing),
                "{facing:?}: the engine's turned box is taken"
            );
            if facing == Direction::East || facing == Direction::West {
                assert!(
                    taken.is_area_free(ENGINE, &engine.position),
                    "{facing:?}: the north-frame box is NOT taken -- which is exactly \
                     the answer a direction-blind check would act on"
                );
            }
            assert!(
                fit(&taken, &pump, facing, 1).is_none(),
                "{facing:?}: the candidate whose engine would stand on the pole is refused"
            );
        }
    }

    #[test]
    fn the_plant_powers_itself() {
        // The point of the whole module. A fork carrying the plant must read
        // as 900 kW at the pole, through the same `electric_supply_kw` the
        // research condition uses -- no separate path, no special case.
        let s = state();
        let plant = plan_plant(&s, &Position::new(0., 0.)).expect("a lake");
        let mut built = s.fork();
        for part in &plant.parts {
            built.create_entity(entity_for(&built, part));
        }
        let pole = plant
            .parts
            .iter()
            .find(|p| p.name == POLE)
            .expect("a pole")
            .position
            .clone();
        let area = built
            .collision_area(POLE, &pole)
            .expect("the pole has a prototype");
        assert_eq!(
            built.electric_supply_kw(&area),
            900.0,
            "the engine has to be wired to the pole the plant places"
        );
    }

    #[test]
    fn a_world_with_no_water_refuses_by_name() {
        let world = fixture_world();
        // Same world, lake removed: `update_chunk_tiles` is additive, so the
        // graph is rebuilt from a world that never had one.
        let dry = factorio_bot_core::factorio::world::FactorioWorld::new();
        dry.update_entity_prototypes(
            world
                .entity_prototypes
                .iter()
                .map(|e| e.value().clone())
                .collect(),
        )
        .expect("prototypes");
        let s = PlanState::from_world(Arc::new(dry), &[BotId(1)]);
        let err = plan_plant(&s, &Position::new(0., 0.))
            .expect_err("no water, no plant, and it must say so");
        assert!(
            matches!(err, PlannerError::PowerPlantNeedsWater { .. }),
            "got {err:?}"
        );
    }

    /// The wide tier measures from the **bot**, not from the origin.
    ///
    /// The whole justification for removing the distance refusal is that the
    /// walk is priced -- and that is only worth anything if the plant is put
    /// on the water the walk is shortest to. This is the test that says so,
    /// and it needs two lakes because the shared fixture has one: with a
    /// single lake, a wide scan anchored on the origin and a wide scan
    /// anchored on the bot pick the same tile and no assertion can tell them
    /// apart.
    ///
    /// The bot stands at (0, 200). The near lake is at (0, 270) -- 70 tiles
    /// away, past the cheap scan and inside the wide one, and 270 tiles from
    /// the origin, so an origin-anchored search cannot see it at all. The
    /// fixture's own lake at (40, 40) is 165 tiles from the bot and invisible
    /// to it, but 57 tiles from the origin and so the *first* thing an
    /// origin-anchored search would find. The two anchors therefore disagree,
    /// and the assertion names which one is right.
    #[test]
    fn the_wide_scan_is_anchored_on_the_bot() {
        let near_lake = Position::new(0., 270.);
        let bot = Position::new(0., 200.);
        let world = fixture_world();
        let mut tiles = Vec::new();
        factorio_bot_core::test_utils::spawn_water(
            &mut tiles,
            factorio_bot_core::factorio::util::add_to_rect(
                &factorio_bot_core::types::Rect::from_wh(4., 4.),
                &near_lake,
            ),
        );
        world.update_chunk_tiles(tiles).expect("a second lake");
        let s = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        let plant = plan_plant(&s, &bot).expect("the near lake is inside the wide scan");
        let pump = plant
            .parts
            .iter()
            .find(|p| p.name == PUMP)
            .expect("a plant has a pump");
        let edge_of_that_lake = 2. + f64::from(SHORE_SEARCH_RADIUS);
        assert!(
            calculate_distance(&pump.position, &near_lake) <= edge_of_that_lake,
            "the pump landed at {}, which is not the lake nearest the bot",
            pump.position
        );
    }

    /// The negative control for
    /// `water_past_the_cheap_scan_is_built_against_rather_than_refused`:
    /// widening the search must not have changed *which water* a plant is
    /// built on.
    ///
    /// The fixture has exactly one lake, a four-by-four block centred on
    /// (40, 40). Both anchors below must land a pump on **that** lake's edge,
    /// so the second tier is reaching further to find the same water rather
    /// than finding different water.
    ///
    /// **The two plants are not identical, and should not be.** From (0, 0)
    /// the nearest tile of that lake is its north-west corner and the plant
    /// goes on the north shore; from (-40, 40) it is the west edge and the
    /// plant goes on the west shore. `nearest_water_tile` orders by distance
    /// from the bot, so the plant is built on the side the bot approaches
    /// from -- which is the behaviour to want, and is why this asserts a lake
    /// rather than a `Plant`.
    #[test]
    fn the_wide_scan_finds_the_same_lake_the_cheap_one_does() {
        let lake = Position::new(40., 40.);
        // The lake's half-width (2) plus the shoreline ring the search is
        // allowed to walk. Anything inside this is that lake's edge; the
        // fixture has no other water anywhere.
        let edge_of_that_lake = 2. + f64::from(SHORE_SEARCH_RADIUS);
        let s = state();
        for from in [Position::new(0., 0.), Position::new(-40., 40.)] {
            let plant = plan_plant(&s, &from).expect("the fixture's lake is reachable");
            let pump = plant
                .parts
                .iter()
                .find(|p| p.name == PUMP)
                .expect("a plant has a pump");
            assert!(
                calculate_distance(&pump.position, &lake) <= edge_of_that_lake,
                "from {from}, the pump landed at {} -- that is not the fixture's lake",
                pump.position
            );
        }
    }

    /// Run 32's refusal, as a test.
    ///
    /// `run-1788379071-00467` satisfied rungs 1-6 and refused rung 7 with
    /// *"the nearest water is 67.8 tiles away, and a power plant may not be
    /// sited more than 64 tiles from the bot that has to carry it there"* --
    /// by 3.8 tiles. Every bot in that run had already been further from
    /// spawn than that: 68.8, 69.9, 72.5 and 71.5 tiles, measured off the
    /// run's own `samples.jsonl`. The bound refused a journey the run was
    /// making routinely.
    ///
    /// The fixture's lake sits at about (40, 40); from (-40, 40) it is ~80
    /// tiles away, which is the same shape at a slightly larger number. The
    /// plant is built and the walk is priced, which is what `schedule` is for.
    #[test]
    fn water_past_the_cheap_scan_is_built_against_rather_than_refused() {
        let s = state();
        let plant = plan_plant(&s, &Position::new(-40., 40.))
            .expect("80 tiles of water is a longer walk, not an impossible plant");
        let pump = plant
            .parts
            .iter()
            .find(|p| p.name == PUMP)
            .expect("a plant has a pump");
        assert!(
            calculate_distance(&pump.position, &Position::new(-40., 40.)) > PLANT_WATER_SCAN_RADIUS,
            "the point of the test is that the plant is past the cheap scan, got {}",
            pump.position
        );
    }

    #[test]
    fn the_same_world_sites_the_same_plant_twice() {
        let s = state();
        let first = plan_plant(&s, &Position::new(0., 0.)).expect("a lake");
        for _ in 0..10 {
            assert_eq!(
                plan_plant(&s, &Position::new(0., 0.)).expect("a lake"),
                first,
                "the plant a world gets is a function of that world and nothing else"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Adoption
    // -----------------------------------------------------------------------

    /// The fixture's lake with the plant this module would site on it already
    /// standing, exactly as rung 1 leaves the world.
    ///
    /// Built through [`plan_plant`] rather than by hand, so the shoreline the
    /// standing plant occupies is the *same* shoreline a second one would be
    /// sited on — which is what `run-1788408407-02764` ran out of.
    fn state_with_a_standing_plant() -> (PlanState, Plant) {
        let mut s = state();
        let plant = plan_plant(&s, &Position::new(40., 40.)).expect("the fixture has a lake");
        for part in &plant.parts {
            let entity = entity_for(&s, part);
            s.create_entity(entity);
        }
        (s, plant)
    }

    /// **Run `run-1788408407-02764`, at the tile it went wrong.**
    ///
    /// Bot 1 stood at `[-51.25, 20.77]` and the plant it had just researched
    /// `automation` on had its pole at `[-13.5, -56.5]` — 86.0 tiles, which is
    /// past `ANCHOR_SEARCH_RADIUS` and `LAB_SEARCH_RADIUS` alike. The planner
    /// answered by siting a whole second plant on the other side of the map.
    /// The distance here is that run's, to two decimals.
    #[test]
    fn a_plant_that_already_stands_is_adopted_from_past_the_near_search() {
        let (s, plant) = state_with_a_standing_plant();
        // 86.0 tiles from the pole, on the axis, so the number is the run's
        // and not an artefact of the fixture's geometry.
        let from = Position::new(plant.pole.x(), plant.pole.y() + 86.0);
        assert!(
            s.nearest_supply_anchor(&from, 64., 189.).is_none(),
            "the premise: the near search cannot see this plant"
        );
        assert_eq!(
            supply_for(&s, &from, 64., 189.).expect("a standing plant is an answer"),
            Supply::Standing(plant.pole.clone()),
            "a working plant 86 tiles away is adopted, not duplicated"
        );
    }

    /// The refusal that closed the run, answered instead of raised.
    ///
    /// From here the planner can see **no water at all** — the fixture's one
    /// lake is 164 tiles off, past [`PLANT_WATER_WIDE_SCAN_RADIUS`] — so
    /// [`plan_plant`] refuses by name. That refusal was the whole of the old
    /// answer; it is now the *last* of three, and a plant that stands is
    /// reached first.
    #[test]
    fn a_standing_plant_answers_where_siting_a_new_one_refuses() {
        let (s, plant) = state_with_a_standing_plant();
        let from = Position::new(0., 200.);
        assert!(
            matches!(
                plan_plant(&s, &from),
                Err(PlannerError::PowerPlantNeedsWater { .. })
            ),
            "the premise: nothing here could site a plant of its own"
        );
        assert_eq!(
            supply_for(&s, &from, 64., 60.).expect("the standing plant answers"),
            Supply::Standing(plant.pole),
            "a milestone must not halt for want of a plant when one is standing"
        );
    }

    /// Nothing standing, so a plant still gets built — the old behaviour, whole.
    #[test]
    fn a_world_with_no_supply_still_builds_a_plant() {
        let s = state();
        let from = Position::new(0., 0.);
        assert_eq!(
            supply_for(&s, &from, 64., 60.).expect("the fixture has a lake"),
            Supply::Build(plan_plant(&s, &from).expect("a lake")),
            "adoption is a tier in front of the plant, not a replacement for it"
        );
    }

    /// **Coverage is not capacity, and adoption does not forget it.**
    ///
    /// A pole standing on its own is a pole with nothing behind it. Adopting
    /// it would put the cell on a network that generates zero, which is the
    /// failure this whole module exists to make impossible — so the pole is
    /// passed over and a plant is built.
    #[test]
    fn a_pole_with_nothing_generating_is_not_adopted() {
        let mut s = state();
        s.create_entity(FactorioEntity {
            name: POLE.into(),
            entity_type: "electric-pole".into(),
            position: Position::new(0., 60.),
            ..Default::default()
        });
        let from = Position::new(0., 0.);
        assert!(
            matches!(supply_for(&s, &from, 64., 60.), Ok(Supply::Build(_))),
            "a pole is not a power plant, however near it stands"
        );
    }

    /// The two tiers are one question with two bounds, and they cannot
    /// disagree.
    ///
    /// [`PlanState::nearest_supply_anchor`] orders candidates by distance and
    /// returns the first that qualifies, so widening the bound can only add
    /// candidates *behind* the one the near tier found. Splitting the search
    /// in two is therefore a read-cost decision and never a different answer —
    /// which is what lets [`supply_for`] keep each caller's own near radius
    /// without any of them meaning something different by it.
    #[test]
    fn the_near_tier_and_the_wide_tier_cannot_disagree() {
        let (s, plant) = state_with_a_standing_plant();
        for offset in [0., 10., 40., 63., 86., 150.] {
            let from = Position::new(plant.pole.x(), plant.pole.y() + offset);
            assert_eq!(
                supply_for(&s, &from, 64., 189.).expect("a standing plant is an answer"),
                Supply::Standing(
                    s.nearest_supply_anchor(&from, PLANT_ADOPT_RADIUS, 189.)
                        .expect("the plant is inside the wide bound")
                ),
                "the two-tier search must answer what one wide search would, at {offset} tiles"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Finishing a plant somebody started
    // -----------------------------------------------------------------------

    /// The fixture's plant with only the parts in `names` standing -- the
    /// world a plan cut short mid-build leaves behind.
    fn state_with_a_partial_plant(names: &[&str]) -> (PlanState, Plant) {
        let mut s = state();
        let plant = plan_plant(&s, &Position::new(40., 40.)).expect("the fixture has a lake");
        for part in plant.parts.iter().filter(|part| names.contains(&part.name)) {
            let entity = entity_for(&s, part);
            s.create_entity(entity);
        }
        (s, plant)
    }

    fn names(parts: &[PlantPart]) -> Vec<&'static str> {
        parts.iter().map(|part| part.name).collect()
    }

    /// **Run `run-1788608648-56109`, plan 2.** Plan 1 had placed the pump at
    /// `[46.5, -8.5]`, three pipes and the boiler at `[45, -5.5]`, and was
    /// cut before the engine. Plan 2 sited a whole second plant twenty
    /// tiles up the shore.
    #[test]
    fn a_plant_missing_its_engine_is_finished_rather_than_started_over() {
        let (s, plant) = state_with_a_partial_plant(&[PUMP, PIPE, BOILER]);
        let from = Position::new(plant.pole.x(), plant.pole.y() + 20.);
        let Supply::Build(finished) = supply_for(&s, &from, 64., 60.).expect("a lake") else {
            panic!("a pump and a boiler generate nothing; something must be built")
        };
        assert_eq!(
            names(&finished.standing),
            vec![PUMP, PIPE, PIPE, BOILER, PIPE],
            "what plan 1 left standing is what is reused"
        );
        assert_eq!(
            names(&finished.parts),
            vec![ENGINE, POLE],
            "only the engine and its pole are placed"
        );
        assert_eq!(
            finished.engine, plant.engine,
            "the engine goes where the pump's own layout puts it"
        );
        assert_eq!(finished.boiler, plant.boiler);
    }

    /// The pole went down before the engine -- the run's plan 1 had all four
    /// poles placed by tick 30,189 and the boiler at 33,341 -- so a standing
    /// pole that reaches the engine's ground is kept and only the engine is
    /// billed.
    #[test]
    fn a_standing_pole_that_reaches_the_engine_is_kept() {
        let (s, plant) = state_with_a_partial_plant(&[PUMP, PIPE, BOILER, POLE]);
        let finished = complete_plant(&s, &plant.pole, 60.).expect("finishable");
        assert_eq!(names(&finished.parts), vec![ENGINE]);
        assert_eq!(finished.pole, plant.pole);
        assert!(
            finished.standing.iter().any(|part| part.name == POLE),
            "the standing pole is reported as reused"
        );
    }

    /// A pump whose engine ground is taken by something else cannot be
    /// finished as designed, and a fresh plant is sited instead of a
    /// broken one.
    #[test]
    fn a_pump_whose_layout_is_blocked_is_passed_over() {
        let (mut s, plant) = state_with_a_partial_plant(&[PUMP, PIPE, BOILER]);
        s.create_entity(FactorioEntity {
            name: "iron-chest".into(),
            entity_type: "container".into(),
            position: plant.engine.clone(),
            ..Default::default()
        });
        assert!(
            complete_plant(&s, &plant.pole, 60.).is_none(),
            "a blocked layout is not a plant to finish"
        );
        let Supply::Build(fresh) = supply_for(&s, &plant.pole, 64., 60.).expect("a lake") else {
            panic!("nothing generates here")
        };
        assert!(fresh.standing.is_empty(), "a whole plant, somewhere else");
        assert_ne!(fresh.engine, plant.engine);
    }

    /// The finished plant's bill is the missing parts and the coal, not the
    /// whole seven.
    #[test]
    fn a_finished_plant_bills_only_what_it_places() {
        let (s, plant) = state_with_a_partial_plant(&[PUMP, PIPE, BOILER]);
        let finished = complete_plant(&s, &plant.pole, 60.).expect("finishable");
        assert_eq!(
            bill(&finished),
            vec![(ENGINE, 1), (POLE, 1), ("coal", PLANT_COAL)]
        );
        let whole = plan_plant(&state(), &Position::new(40., 40.)).expect("a lake");
        assert_eq!(
            bill(&whole),
            vec![
                (PUMP, 1),
                (PIPE, PIPE_COUNT),
                (BOILER, 1),
                (ENGINE, 1),
                (POLE, 1),
                ("coal", PLANT_COAL)
            ],
            "a plant sited from scratch still bills all of it"
        );
    }

    /// **Plan 4 of the same run.** The one working plant had 145 kW left, the
    /// cell wanted 189, and the half-built plant from plan 1 stood twenty
    /// tiles away wanting an engine. Plan 4 sited a third plant; plan 5 found
    /// no shoreline left and the run halted.
    ///
    /// A complete plant with nothing missing is not something to finish, so
    /// the half-built one beside it is.
    #[test]
    fn a_full_network_is_not_finished_but_the_half_built_plant_beside_it_is() {
        let (mut s, first) = state_with_a_standing_plant();
        let second = plan_plant(&s, &Position::new(40., 40.)).expect("room for a second");
        assert_ne!(second.engine, first.engine);
        for part in second
            .parts
            .iter()
            .filter(|part| part.name != ENGINE && part.name != POLE)
        {
            let entity = entity_for(&s, part);
            s.create_entity(entity);
        }
        // A lab on the first plant's network: 60 of its 900 kW spoken for.
        s.create_entity(FactorioEntity {
            name: "lab".into(),
            entity_type: "lab".into(),
            position: Position::new(first.pole.x(), first.pole.y() + 2.5),
            ..Default::default()
        });
        assert!(
            s.nearest_supply_anchor(&first.pole, PLANT_ADOPT_RADIUS, 900.)
                .is_none(),
            "the premise: the standing network cannot carry 900 kW more"
        );
        let Supply::Build(finished) = supply_for(&s, &first.pole, 64., 900.).expect("a lake")
        else {
            panic!("nothing standing carries 900 kW")
        };
        assert_eq!(
            finished.engine, second.engine,
            "the half-built plant is finished"
        );
        assert_eq!(names(&finished.parts), vec![ENGINE, POLE]);
    }

    /// The bound is wide enough that a plant is never built where one could
    /// have been adopted.
    ///
    /// The water may be [`PLANT_WATER_WIDE_SCAN_RADIUS`] away, the shoreline a
    /// further [`SHORE_SEARCH_RADIUS`] (diagonally, so `sqrt(2)` times it) from
    /// the water, and the pole a further plant's-extent-plus-ring-search from
    /// the shoreline. This asserts the last of those three against the plant
    /// the fixture actually sites, so the arithmetic in
    /// [`PLANT_ADOPT_RADIUS`]'s doc has a measurement under it rather than an
    /// estimate.
    #[test]
    fn the_adopt_bound_exceeds_the_furthest_a_plant_could_be_built() {
        let (_, plant) = state_with_a_standing_plant();
        let pump = &plant.parts[0].position;
        let spread = plant
            .parts
            .iter()
            .map(|part| calculate_distance(&part.position, pump))
            .fold(0., |a: f64, b| if a.total_cmp(&b).is_ge() { a } else { b });
        assert!(spread < 16., "a plant is a small rigid body: {spread}");
        let reach = PLANT_WATER_WIDE_SCAN_RADIUS
            + f64::from(SHORE_SEARCH_RADIUS) * std::f64::consts::SQRT_2
            + spread
            + f64::from(crate::method::util::FREE_TILE_SEARCH_RADIUS) * std::f64::consts::SQRT_2;
        assert!(
            PLANT_ADOPT_RADIUS > reach,
            "adoption must reach further than construction can, or the planner \
             can still build what it could have adopted: {PLANT_ADOPT_RADIUS} <= {reach}"
        );
    }
}

// -------------------------------------------------------------------------
// Capacity: the plant is sized against the demand it is asked for.
//
// Every literal in this module is written out rather than read from the
// constant under test, on the rule in
// `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`:
// a test asserting against the very constant the code reads proves nothing.
// So 900, 1800, 5 and the five-tile pitch are typed, not imported.
// -------------------------------------------------------------------------

#[cfg(test)]
mod capacity_tests {
    use super::*;
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    /// The vanilla figures this whole module is sized from, as literals.
    ///
    /// `steam-engine` 900 kW, and one boiler's 1.8 MW over it is two engines.
    const ENGINE_KW_LITERAL: f64 = 900.;
    const ONE_BOILER_KW_LITERAL: f64 = 1800.;

    #[test]
    fn the_engine_the_planner_credits_is_the_engine_vanilla_states() {
        // The anchor for every other number here. If this drifts, the plant
        // is sized against one figure and credited at another.
        let s = state();
        assert_eq!(
            s.generator_output_kw(ENGINE),
            Some(ENGINE_KW_LITERAL),
            "a steam engine is 900 kW: fluid_usage_per_tick 0.5 x 60 ticks x \
             (165 - 15) degrees x 0.2 kJ"
        );
    }

    #[test]
    fn a_demand_is_sized_into_engines_and_a_bigger_one_refuses_by_name() {
        let s = state();
        // Zero and anything up to one engine's worth is one engine -- the
        // plant this module has always built.
        assert_eq!(engines_for(&s, 0.).expect("zero"), 1);
        assert_eq!(engines_for(&s, 60.).expect("a lab"), 1);
        assert_eq!(engines_for(&s, 189.).expect("a red cell"), 1);
        assert_eq!(engines_for(&s, 900.).expect("exactly one engine"), 1);
        // Past one engine, two.
        assert_eq!(engines_for(&s, 900.5).expect("a hair over"), 2);
        assert_eq!(engines_for(&s, 1800.).expect("exactly two"), 2);
        // Past one boiler's worth, a named refusal rather than a short plant.
        let err = engines_for(&s, 1800.5).expect_err("more than one boiler drives");
        match err {
            PlannerError::PowerPlantTooSmall {
                needed_kw,
                plant_kw,
            } => {
                assert_eq!(needed_kw, 1800.5);
                assert_eq!(plant_kw, ONE_BOILER_KW_LITERAL);
            }
            other => panic!("expected PowerPlantTooSmall, got {other:?}"),
        }
    }

    #[test]
    fn a_second_engine_stands_five_tiles_on_and_costs_no_extra_pipe() {
        let s = state();
        for facing in Direction::orthogonal() {
            let one = layout(&Position::new(10.5, 10.5), facing, 1).expect("cardinal");
            let two = layout(&Position::new(10.5, 10.5), facing, 2).expect("cardinal");

            // Everything but the engines is untouched: same parts, same
            // order, same tiles. A second engine that moved the boiler would
            // move the plant under a caller that had already sited it.
            let trunk = |parts: &[PlantPart]| -> Vec<(String, Position)> {
                parts
                    .iter()
                    .filter(|part| part.name != ENGINE)
                    .map(|part| (part.name.to_string(), part.position.clone()))
                    .collect()
            };
            assert_eq!(trunk(&one), trunk(&two), "{facing:?}: the trunk moved");

            let engines: Vec<&PlantPart> = two.iter().filter(|part| part.name == ENGINE).collect();
            assert_eq!(engines.len(), 2, "{facing:?}");
            assert_eq!(
                engines[0].position,
                one[one.len() - 1].position,
                "{facing:?}: the first engine of two is the engine of one"
            );
            // Five tiles, the engine's own length, along one axis only.
            let dx = (engines[1].position.x() - engines[0].position.x()).abs();
            let dy = (engines[1].position.y() - engines[0].position.y()).abs();
            assert!(
                (dx + dy - 5.).abs() < 1e-9 && (dx < 1e-9 || dy < 1e-9),
                "{facing:?}: engines are {dx} by {dy} apart, wanted 5 along one axis"
            );
            // Away from the boiler, not on top of it.
            let boiler = two
                .iter()
                .find(|part| part.name == BOILER)
                .expect("a boiler")
                .position
                .clone();
            let near = calculate_distance(&engines[0].position, &boiler);
            let far = calculate_distance(&engines[1].position, &boiler);
            assert!(
                far > near,
                "{facing:?}: the row grew towards the boiler ({far} <= {near})"
            );
            // And each engine is on its own build grid, at every facing.
            for part in &two {
                assert!(
                    on_its_grid(&s, part),
                    "{facing:?}: {} at {} is off its build grid",
                    part.name,
                    part.position
                );
            }
        }
    }

    #[test]
    fn two_engines_never_overlap_each_other_or_the_trunk() {
        let s = state();
        for facing in Direction::orthogonal() {
            let parts = layout(&Position::new(10.5, 10.5), facing, 2).expect("cardinal");
            for (i, a) in parts.iter().enumerate() {
                for b in parts.iter().skip(i + 1) {
                    let box_a = s
                        .collision_area_facing(a.name, &a.position, a.direction)
                        .expect("prototype");
                    let box_b = s
                        .collision_area_facing(b.name, &b.position, b.direction)
                        .expect("prototype");
                    assert!(
                        !overlap(&box_a, &box_b),
                        "{facing:?}: {} at {} overlaps {} at {}",
                        a.name,
                        a.position,
                        b.name,
                        b.position
                    );
                }
            }
        }
    }

    /// Box against box. A local copy: the sibling module's helper is private
    /// to it, and this file's rule is that a test states its own geometry.
    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.left_top.x() < b.right_bottom.x()
            && a.right_bottom.x() > b.left_top.x()
            && a.left_top.y() < b.right_bottom.y()
            && a.right_bottom.y() > b.left_top.y()
    }

    /// The plant a demand of `kw` sites on the fixture's lake.
    fn plant_for(kw: f64) -> (PlanState, Plant) {
        let s = state();
        let plant = plan_plant_for(&s, &Position::new(0., 0.), kw).expect("the fixture has a lake");
        (s, plant)
    }

    /// A fork of `s` carrying every part of `plant` that is not standing yet.
    fn with_parts(s: &PlanState, plant: &Plant) -> PlanState {
        let mut built = s.fork();
        for part in &plant.parts {
            built.create_entity(entity_for(&built, part));
        }
        built
    }

    #[test]
    fn an_unsized_plant_is_still_the_one_engine_plant() {
        // The compatibility claim the three offline baselines rest on.
        let s = state();
        let old = plan_plant(&s, &Position::new(0., 0.)).expect("a lake");
        let (_, sized) = plant_for(60.);
        assert_eq!(old.engines.len(), 1);
        assert_eq!(
            old.parts, sized.parts,
            "a 60 kW demand sites the same plant"
        );
        assert_eq!(old.pole, sized.pole);
    }

    #[test]
    fn a_two_engine_plants_pole_supplies_both_engines() {
        // **The silent failure this test exists for.** A pole sited against
        // the first engine only leaves the second generating into nothing:
        // every entity places, the plan reads 1,800 kW, and the network
        // carries 900. Placement and function are separate concerns.
        let (s, plant) = plant_for(1000.);
        assert_eq!(plant.engines.len(), 2, "1,000 kW wants two engines");
        let built = with_parts(&s, &plant);
        for engine in &plant.engines {
            let area = built
                .collision_area(ENGINE, engine)
                .expect("the engine has a prototype");
            assert!(
                built.pole_would_supply(POLE, &plant.pole, &area),
                "the pole at {} does not reach the engine at {engine}",
                plant.pole
            );
        }
    }

    /// A tile that a small pole covering the **first** engine of `plant`'s row
    /// could stand on while leaving the **second** out of reach.
    ///
    /// The row runs along one axis with a five-tile pitch; a small pole's
    /// supply area is five by five and an engine's box is 4.7 along its long
    /// axis, so a pole four tiles behind the first engine reaches it and falls
    /// short of the second by a clear margin. Pushed three tiles sideways so
    /// it is not standing in the row itself.
    ///
    /// The test asserts both halves of that claim before using it, so a
    /// fixture that had quietly stopped discriminating fails loudly instead of
    /// passing vacuously.
    fn a_tile_reaching_only_the_first_engine(state: &PlanState, plant: &Plant) -> Position {
        let (a, b) = (&plant.engines[0], &plant.engines[1]);
        let axis = Position::new((b.x() - a.x()) / 5., (b.y() - a.y()) / 5.);
        let side = Position::new(-axis.y(), axis.x());
        let decoy = Position::new(
            a.x() - axis.x() * 4. + side.x() * 3.,
            a.y() - axis.y() * 4. + side.y() * 3.,
        );
        let area = |engine: &Position| {
            state
                .collision_area(ENGINE, engine)
                .expect("the engine has a prototype")
        };
        assert!(
            state.pole_would_supply(POLE, &decoy, &area(a)),
            "the decoy at {decoy} must reach the first engine, or it tests nothing"
        );
        assert!(
            !state.pole_would_supply(POLE, &decoy, &area(b)),
            "the decoy at {decoy} must NOT reach the second engine, or it tests nothing"
        );
        decoy
    }

    #[test]
    fn a_pole_site_refuses_when_only_one_engine_can_be_reached() {
        // **The discriminating test for `pole_site`'s rule.** On open ground
        // the ring search starts at the joint between the engines and lands on
        // a tile covering both by geometry, so weakening `all` to "the first
        // engine" changes nothing there and the search proves nothing about
        // the predicate. Here every tile that could reach both is taken, and
        // tiles that reach only the first are left free -- so the rule is the
        // only thing standing between the plant and a half-wired row.
        let (s, plant) = plant_for(1000.);
        let mut world = with_parts(&s, &plant);

        let areas: Vec<Rect> = plant
            .engines
            .iter()
            .map(|engine| {
                world
                    .collision_area(ENGINE, engine)
                    .expect("the engine has a prototype")
            })
            .collect();
        let origin = engine_row_centre(&plant.engines);

        // Take every tile in the ring search that reaches both engines.
        // `free_area_near_where` walks tile centres out to
        // `FREE_TILE_SEARCH_RADIUS`, so this is exactly its candidate set.
        let radius = crate::method::util::FREE_TILE_SEARCH_RADIUS;
        let mut blocked = 0;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let candidate = Position::new(
                    (origin.x().floor() as i32 + dx) as f64 + 0.5,
                    (origin.y().floor() as i32 + dy) as f64 + 0.5,
                );
                let reaches_both = areas
                    .iter()
                    .all(|area| world.pole_would_supply(POLE, &candidate, area));
                if reaches_both && world.is_area_free(POLE, &candidate) {
                    world.create_entity(FactorioEntity {
                        name: "stone-wall".to_string(),
                        entity_type: "wall".to_string(),
                        position: candidate,
                        ..Default::default()
                    });
                    blocked += 1;
                }
            }
        }
        assert!(
            blocked > 0,
            "the fixture blocked nothing, so it tests nothing"
        );

        // A tile reaching only the first engine is still free -- so a rule
        // asking about one engine would answer `Some` here.
        let decoy = a_tile_reaching_only_the_first_engine(&world, &plant);
        assert!(
            world.is_area_free(POLE, &decoy),
            "the decoy tile must be free, or the weakened rule could not take it \
             either and this test proves nothing"
        );

        match pole_site(&world, &areas, &origin) {
            None => {}
            Some(chosen) => panic!(
                "pole_site chose {chosen}, which cannot reach both engines: \
                 every tile that could was taken"
            ),
        }
    }

    #[test]
    fn a_standing_pole_reaching_only_one_engine_is_not_adopted_for_two() {
        // The discriminating half. The test above asserts a property of the
        // pole `fit` chose, and a ring search that starts at the joint between
        // the engines lands on a tile covering both **by luck** -- so
        // weakening the predicate to "covers the first engine" leaves it
        // green. `finish` chooses among poles that already stand, which is
        // where the predicate decides rather than the geometry.
        let (s, plant) = plant_for(1000.);
        assert_eq!(plant.engines.len(), 2);

        // Every part of the two-engine plant except its second engine, plus a
        // decoy pole that reaches the first engine and not the second.
        let mut world = s.fork();
        for part in &plant.parts {
            if part.name == ENGINE && part.position == plant.engines[1] {
                continue;
            }
            if part.name == POLE {
                continue;
            }
            world.create_entity(entity_for(&world, part));
        }
        let decoy = a_tile_reaching_only_the_first_engine(&world, &plant);
        world.create_entity(FactorioEntity {
            name: POLE.to_string(),
            entity_type: "electric-pole".to_string(),
            position: decoy.clone(),
            ..Default::default()
        });

        let finished = complete_plant(&world, &plant.engines[0], 1000.)
            .expect("the standing pump names a plant to finish");
        assert_ne!(
            finished.pole, decoy,
            "a pole covering one engine of two was adopted for a two-engine plant"
        );
        // And the pole it did choose reaches both.
        let mut all = world.fork();
        for part in &finished.parts {
            all.create_entity(entity_for(&all, part));
        }
        for engine in &finished.engines {
            let area = all
                .collision_area(ENGINE, engine)
                .expect("the engine has a prototype");
            assert!(
                all.pole_would_supply(POLE, &finished.pole, &area),
                "the chosen pole at {} does not reach the engine at {engine}",
                finished.pole
            );
        }
    }

    #[test]
    fn a_two_engine_plant_reads_eighteen_hundred_kilowatts_at_its_own_pole() {
        // Through `electric_supply_kw`, which is what `Condition::Powered`
        // asks -- not through a count of engines, which would be this
        // module's own arithmetic agreeing with itself.
        let (s, plant) = plant_for(1000.);
        let built = with_parts(&s, &plant);
        let area = built
            .collision_area(POLE, &plant.pole)
            .expect("the pole has a prototype");
        assert_eq!(
            built.electric_supply_kw(&area),
            ONE_BOILER_KW_LITERAL,
            "two engines on one pole are 1,800 kW"
        );

        // And the one-engine plant still reads 900, so the number above is a
        // consequence of the second engine and not of the fixture.
        let (s1, one) = plant_for(60.);
        let built1 = with_parts(&s1, &one);
        let area1 = built1
            .collision_area(POLE, &one.pole)
            .expect("the pole has a prototype");
        assert_eq!(built1.electric_supply_kw(&area1), ENGINE_KW_LITERAL);
    }

    #[test]
    fn the_coal_bill_follows_the_engine_row() {
        let (_, one) = plant_for(60.);
        let (_, two) = plant_for(1000.);
        let coal = |plant: &Plant| {
            bill(plant)
                .into_iter()
                .find(|(name, _)| *name == "coal")
                .expect("a plant bills coal")
                .1
        };
        assert_eq!(coal(&one), 5, "one engine, five coal");
        assert_eq!(coal(&two), 10, "two engines drink twice the steam");
        // And the engine line of the bill is the row itself.
        let engines = |plant: &Plant| {
            bill(plant)
                .into_iter()
                .find(|(name, _)| *name == ENGINE)
                .expect("a plant bills engines")
                .1
        };
        assert_eq!(engines(&one), 1);
        assert_eq!(engines(&two), 2);
        // The pipe count does not move: engines chain off each other.
        let pipes = |plant: &Plant| {
            bill(plant)
                .into_iter()
                .find(|(name, _)| *name == PIPE)
                .expect("a plant bills pipes")
                .1
        };
        assert_eq!(pipes(&one), 3);
        assert_eq!(pipes(&two), 3);
    }

    #[test]
    fn the_tier_that_builds_refuses_a_demand_no_plant_carries() {
        // The defect roadmap item 3 names: `supply_for`'s fourth tier used to
        // ignore `kw` entirely and hand back a 900 kW plant for any demand.
        let s = state();
        let err = supply_for(&s, &Position::new(0., 0.), 64., 5000.)
            .expect_err("5,000 kW is more than one boiler drives");
        assert!(
            matches!(err, PlannerError::PowerPlantTooSmall { .. }),
            "expected PowerPlantTooSmall, got {err:?}"
        );
        // Named, not silent: the message carries both numbers.
        let text = err.to_string();
        assert!(
            text.contains("5000") && text.contains("1800"),
            "the refusal must name what was asked and what is available: {text}"
        );
    }

    #[test]
    fn a_demand_one_plant_carries_is_still_built_rather_than_refused() {
        // The other side of the refusal, so the test above cannot pass by the
        // function refusing everything.
        let s = state();
        match supply_for(&s, &Position::new(0., 0.), 64., 1500.) {
            Ok(Supply::Build(plant)) => assert_eq!(plant.engines.len(), 2),
            other => panic!("expected a two-engine plant to build, got {other:?}"),
        }
    }

    #[test]
    fn the_same_demand_sites_the_same_plant_twice() {
        // Determinism, over the sized path: a centroid, a filter and a fold
        // are all new here, and the crate's contract is that identical inputs
        // give identical plans.
        let (_, a) = plant_for(1000.);
        let (_, b) = plant_for(1000.);
        assert_eq!(a.parts, b.parts);
        assert_eq!(a.pole, b.pole);
        assert_eq!(a.engines, b.engines);
    }

    // -----------------------------------------------------------------------
    // The world anchor
    //
    // The measured defect: siting was anchored on the caller -- a bot's
    // position or a machine site -- and a roster that had walked away could
    // no longer see a lake that had not moved. See `plant_world_anchor`.
    // -----------------------------------------------------------------------

    /// Where the fixture's one lake is. Read off
    /// `the_wide_scan_finds_the_same_lake_the_cheap_one_does`, which owns the
    /// claim.
    const FIXTURE_LAKE: (f64, f64) = (40., 40.);

    /// Far enough from that lake that **both** water scans miss it, and near
    /// enough to nothing else that the fixture has no other answer. 400 tiles
    /// north on the y axis is ~362 tiles from the lake against a wide scan of
    /// 128 -- the same shape as the bots parked at (255, 249) on
    /// `map-31337-explored.json`, 355 tiles from water the origin sees at 48.
    fn a_bot_that_walked_away() -> Position {
        Position::new(0., 400.)
    }

    fn pump_of(plant: &Plant) -> Position {
        plant
            .parts
            .iter()
            .find(|p| p.name == PUMP)
            .expect("a plant has a pump")
            .position
            .clone()
    }

    /// The control: the caller-anchored search really does fail from there.
    ///
    /// Without this the next test could pass by the local search quietly
    /// succeeding, and would then be asserting nothing about the fallback.
    #[test]
    fn the_caller_anchored_search_refuses_from_where_the_bot_parked() {
        let s = state();
        let err =
            plan_plant(&s, &a_bot_that_walked_away()).expect_err("362 tiles is past both scans");
        assert!(
            matches!(err, PlannerError::PowerPlantNeedsWater { .. }),
            "got {err:?}"
        );
    }

    /// The defect, as a test: a roster that walked away still gets a plant.
    ///
    /// `supply_for` refused here before 2026-09-06, and every power-needing
    /// goal on a charted dump refused with it.
    #[test]
    fn a_roster_that_walked_away_still_gets_a_plant_at_the_world_anchor() {
        let s = state();
        let supply = supply_for(&s, &a_bot_that_walked_away(), 64., 60.)
            .expect("the lake the origin can see is still a lake");
        let plant = match supply {
            Supply::Build(plant) => plant,
            other => panic!("nothing stands in the fixture, expected a build, got {other:?}"),
        };
        let lake = Position::new(FIXTURE_LAKE.0, FIXTURE_LAKE.1);
        let edge = 2. + f64::from(SHORE_SEARCH_RADIUS);
        assert!(
            calculate_distance(&pump_of(&plant), &lake) <= edge,
            "the pump landed at {} -- that is not the fixture's lake",
            pump_of(&plant)
        );
    }

    /// The fallback finds water, it does not invent it.
    ///
    /// A world with no water anywhere must still refuse by name, from the
    /// world anchor as from anywhere else -- otherwise the retry would have
    /// turned an honest refusal into a plan against nothing.
    #[test]
    fn the_world_anchor_does_not_invent_water_in_a_dry_world() {
        let world = fixture_world();
        let dry = factorio_bot_core::factorio::world::FactorioWorld::new();
        dry.update_entity_prototypes(
            world
                .entity_prototypes
                .iter()
                .map(|e| e.value().clone())
                .collect(),
        )
        .expect("prototypes");
        let s = PlanState::from_world(Arc::new(dry), &[BotId(1)]);
        let err = supply_for(&s, &a_bot_that_walked_away(), 64., 60.)
            .expect_err("no water anywhere is still no water");
        assert!(
            matches!(err, PlannerError::PowerPlantNeedsWater { .. }),
            "got {err:?}"
        );
    }

    /// A demand no plant can carry is **not** retried.
    ///
    /// `PowerPlantTooSmall` is a statement about the demand, true from every
    /// anchor on every map. Retrying it would read a quarter of a million
    /// terrain tiles to arrive at the identical error, so the retry is gated
    /// on the two refusals that are about *where the search stood*.
    #[test]
    fn a_demand_no_plant_can_carry_is_not_retried_from_the_world_anchor() {
        let s = state();
        let err = supply_for(&s, &a_bot_that_walked_away(), 64., 5_000.)
            .expect_err("5,000 kW is more than one boiler's engines");
        assert!(
            matches!(err, PlannerError::PowerPlantTooSmall { .. }),
            "got {err:?}"
        );
    }

    /// **The fallback is a fallback.** A caller standing beside water gets the
    /// plant its own search found, not the one the origin would have found.
    ///
    /// This is the property that keeps the three offline baselines still: on
    /// `map.json` the bots stand at the origin and the local search wins on
    /// every call, so the retry never runs at all.
    #[test]
    fn a_caller_that_can_site_locally_is_untouched_by_the_fallback() {
        // Two lakes, so "local" and "world anchor" have different answers and
        // the assertion can tell them apart -- the same construction
        // `the_wide_scan_is_anchored_on_the_bot` uses, for the same reason.
        let near_lake = Position::new(0., 270.);
        let bot = Position::new(0., 200.);
        let world = fixture_world();
        let mut tiles = Vec::new();
        factorio_bot_core::test_utils::spawn_water(
            &mut tiles,
            factorio_bot_core::factorio::util::add_to_rect(
                &factorio_bot_core::types::Rect::from_wh(4., 4.),
                &near_lake,
            ),
        );
        world.update_chunk_tiles(tiles).expect("a second lake");
        let two_lakes = PlanState::from_world(Arc::new(world), &[BotId(1)]);

        let supply = supply_for(&two_lakes, &bot, 64., 60.).expect("the near lake is reachable");
        let plant = match supply {
            Supply::Build(plant) => plant,
            other => panic!("expected a build, got {other:?}"),
        };
        let edge = 2. + f64::from(SHORE_SEARCH_RADIUS);
        assert!(
            calculate_distance(&pump_of(&plant), &near_lake) <= edge,
            "the pump landed at {} -- the fallback overrode a local answer",
            pump_of(&plant)
        );
    }

    // -----------------------------------------------------------------------
    // What a span costs, either way round
    // -----------------------------------------------------------------------

    /// The recipe numbers the module doc's retracted premise got wrong, read
    /// off the fixture's own prototypes rather than off this file's prose.
    ///
    /// The old paragraph priced piping at `pipe-to-ground`, 15 iron plates per
    /// **10** tiles. Two errors: plain pipe is the item you would use, and the
    /// pair spans 12 tiles rather than 10.
    #[test]
    fn the_pipe_and_pole_costs_are_the_recipes_own() {
        let s = state();
        let recipe = |name: &str| {
            s.base()
                .recipes
                .get(name)
                .unwrap_or_else(|| panic!("the fixture carries {name}"))
                .clone()
        };
        let ingredient = |name: &str, item: &str| -> u32 {
            recipe(name)
                .ingredients
                .as_ref()
                .unwrap_or_else(|| panic!("{name} has ingredients"))
                .iter()
                .find(|i| i.name == item)
                .unwrap_or_else(|| panic!("{name} wants {item}"))
                .amount
        };

        // pipe: 1 iron-plate -> 1 pipe, one tile.
        assert_eq!(
            ingredient(PIPE, "iron-plate"),
            PIPE_PLATES_PER_TILE,
            "a pipe is one iron plate and covers one tile"
        );

        // pipe-to-ground: 10 pipe + 5 iron-plate -> 2 pieces, and a pipe is a
        // plate, so a pair is 15 plates.
        let underground = "pipe-to-ground";
        let pair = ingredient(underground, PIPE) * PIPE_PLATES_PER_TILE
            + ingredient(underground, "iron-plate");
        assert_eq!(
            pair, PIPE_TO_GROUND_PLATES_PER_PAIR,
            "a pipe-to-ground pair is 10 pipe (a plate each) plus 5 plates"
        );

        // small-electric-pole: 1 wood + 2 copper-cable -> 2, and copper-cable
        // is 1 copper-plate -> 2 cable, so one craft is one copper plate.
        assert_eq!(ingredient(POLE, "wood"), POLE_CRAFT_WOOD, "a craft's wood");
        let cable = ingredient(POLE, "copper-cable");
        let cable_per_plate = recipe("copper-cable")
            .products
            .iter()
            .find(|p| p.name == "copper-cable")
            .expect("copper-cable makes copper-cable")
            .amount;
        assert_eq!(
            cable / cable_per_plate,
            POLE_CRAFT_COPPER_PLATES,
            "2 cable at 2 cable a plate is one copper plate, not two"
        );
    }

    /// The three transports, priced off the recipes, in the order the module
    /// doc's table gives them.
    ///
    /// This is the arithmetic behind *"wire the power, pipe the water, do not
    /// move the coal"*: if belt were not the dearest, the rule would be
    /// something else.
    #[test]
    fn the_three_transports_are_the_recipes_own() {
        let s = state();
        let ingredient = |name: &str, item: &str| -> u32 {
            s.base()
                .recipes
                .get(name)
                .unwrap_or_else(|| panic!("the fixture carries {name}"))
                .ingredients
                .as_ref()
                .unwrap_or_else(|| panic!("{name} has ingredients"))
                .iter()
                .find(|i| i.name == item)
                .unwrap_or_else(|| panic!("{name} wants {item}"))
                .amount
        };

        // transport-belt: 1 iron-plate + 1 iron-gear-wheel -> 2 belts, and a
        // gear is 2 plates. Three plates for two tiles.
        let gear = "iron-gear-wheel";
        let per_two = ingredient("transport-belt", "iron-plate")
            + ingredient("transport-belt", gear) * ingredient(gear, "iron-plate");
        assert_eq!(
            per_two, BELT_HALF_PLATES_PER_TILE,
            "a belt craft is 3 plates for 2 tiles"
        );

        // Per tile, over a distance long enough that the rounding in each
        // function is noise: belt dearest, pipe next, poles nearly free.
        let far = 700.;
        let belt = f64::from(belt_run_plates(far)) / far;
        let pipe = f64::from(pipe_run_plates(far)) / far;
        let (_, wood, copper) = pole_run_items(far);
        let poles = f64::from(wood + copper) / far;
        assert!(
            belt > pipe && pipe > poles,
            "belt {belt}, pipe {pipe}, poles {poles} -- the module doc's whole \
             argument is this ordering"
        );
        assert!(
            (belt - 1.5).abs() < 0.01,
            "belt should be about 1.5 plates a tile, got {belt}"
        );
    }

    /// A `pipe-to-ground` pair is **dearer** per tile than plain pipe.
    ///
    /// The claim the retracted premise inverted. It exists to cross an
    /// obstacle, not to save material.
    #[test]
    fn underground_pipe_is_dearer_per_tile_than_plain_pipe() {
        let underground =
            f64::from(PIPE_TO_GROUND_PLATES_PER_PAIR) / f64::from(PIPE_TO_GROUND_PAIR_SPAN_TILES);
        assert!(
            underground > f64::from(PIPE_PLATES_PER_TILE),
            "pipe-to-ground is {underground} plates a tile against plain pipe's \
             {PIPE_PLATES_PER_TILE}"
        );
    }

    #[test]
    fn a_pipe_run_costs_a_plate_a_tile() {
        assert_eq!(pipe_run_plates(0.), 0);
        assert_eq!(pipe_run_plates(1.), 1);
        assert_eq!(pipe_run_plates(60.), 60, "the module doc's 60-tile example");
        assert_eq!(pipe_run_plates(0.5), 1, "a part tile is still a whole pipe");
        assert_eq!(pipe_run_plates(355.), 355, "the explored dump's separation");
    }

    #[test]
    fn a_pole_run_comes_two_poles_to_a_craft() {
        assert_eq!(pole_run_items(0.), (0, 0, 0));
        // 7 tiles is one pole, and one craft makes two -- so one wood buys
        // fourteen tiles.
        assert_eq!(pole_run_items(7.), (1, 1, 1));
        assert_eq!(pole_run_items(14.), (2, 1, 1), "two poles, still one craft");
        assert_eq!(pole_run_items(15.), (3, 2, 2));
        assert_eq!(pole_run_items(355.), (51, 26, 26));
    }

    /// **There is no material crossover, and the one that binds is supply.**
    ///
    /// Poles are about seven times cheaper per tile than pipe at every
    /// distance -- so any claim that piping is ruled out *on price* is false.
    /// What rules the pole route out is wood: a four-bot run starts with four
    /// and this crate makes none, so eight poles is the whole budget and about
    /// 56 tiles is the whole reach.
    #[test]
    fn the_crossover_is_wood_rather_than_price() {
        for tiles in [7., 20., 56., 100., 355.] {
            let plates = f64::from(pipe_run_plates(tiles));
            let (poles, wood, copper) = pole_run_items(tiles);
            let pole_items = f64::from(wood + copper);
            assert!(
                pole_items < plates,
                "{tiles} tiles: {poles} poles cost {pole_items} items against \
                 {plates} plates of pipe -- poles are supposed to be cheaper"
            );
        }

        // Four wood is the roster's whole supply.
        const STARTING_WOOD: u32 = 4;
        let (_, wood_at_56, _) = pole_run_items(56.);
        assert!(
            wood_at_56 <= STARTING_WOOD,
            "56 tiles wants {wood_at_56} wood and a four-bot run has {STARTING_WOOD}"
        );
        let (_, wood_at_57, _) = pole_run_items(57.);
        assert!(
            wood_at_57 > STARTING_WOOD,
            "past 56 tiles the pole route must run out of wood, wanted {wood_at_57}"
        );
        // And the pipe route has no such ceiling: iron plate is what a run
        // mines by the hundred.
        assert_eq!(pipe_run_plates(357.), 357);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod ensure_powered_tests {
    //! [`ensure_powered`] is the one place that decides power for a site, and
    //! these are the two things a caller relies on that no baseline can show.
    //!
    //! The three offline plan reports pin that the extraction was *pure* --
    //! they are byte-identical across it -- but they only exercise the paths
    //! `researched:automation` and the two `producing:` goals happen to take.
    //! What a third caller (`method::blueprint`) needs is stated here instead:
    //! **an already-powered site costs nothing**, and **an unpowered one comes
    //! back genuinely powered rather than merely covered**.

    use super::*;
    use crate::ids::BotId;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    /// A 75 kW consumer -- `assembling-machine-1`, the draw
    /// `crate::action`'s own power tests use.
    const CONSUMER: &str = "assembling-machine-1";
    const CONSUMER_KW: f64 = 75.;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    /// A site a standing plant already covers with headroom needs **no steps
    /// and no ids at all**.
    ///
    /// This is the branch every caller would otherwise have to write for
    /// itself, and the one whose absence doubles a factory on a replan: a
    /// method that emitted a plant here would build a second one beside a
    /// working one. The ids matter as much as the steps -- a caller orders its
    /// own placement after every id it is handed, so a spurious id is a
    /// spurious edge.
    #[test]
    fn an_already_powered_site_costs_no_steps_and_no_ids() {
        let s = state();
        let plant = plan_plant(&s, &Position::new(0., 0.)).expect("the fixture has a lake");
        let mut built = s.fork();
        for part in &plant.parts {
            built.create_entity(entity_for(&built, part));
        }
        let site = plant.pole.clone();
        let area = built
            .collision_area(CONSUMER, &site)
            .expect("the consumer has a prototype");

        // Control: the site really is powered before the call, or this test
        // is about the other branch.
        assert!(
            Condition::Powered {
                pos: site.clone(),
                entity: CONSUMER.into(),
                kw: CONSUMER_KW,
            }
            .holds(&built, BotId(1)),
            "control: a 900 kW plant must already carry 75 kW at its own pole"
        );

        let mut ctx = ExpansionCtx::new(built, BotId(1));
        let powering = ensure_powered(
            &mut ctx,
            CONSUMER,
            &site,
            &area,
            CONSUMER_KW,
            SUPPLY_RADIUS,
            &[],
        )
        .expect("a standing plant is never a refusal")
        .expect("and it is never unreachable");

        assert!(
            powering.steps.is_empty(),
            "a powered site must cost nothing, got {} steps: {:?}",
            powering.steps.len(),
            powering.steps
        );
        assert!(
            powering.ids.is_empty(),
            "a powered site must hand back no ids to order against, got {:?}",
            powering.ids
        );
    }

    /// An unpowered site comes back **powered**, judged by the game's own
    /// rule against the state the call left behind.
    ///
    /// Not "some steps were emitted", and not "a pole stands within reach":
    /// coverage is not capacity, and this crate has paid for that distinction
    /// twice. The assertion is `Condition::Powered` -- a headroom test walking
    /// `PlanState`'s union-find over real wire distances -- evaluated on
    /// `ctx.state` after the call, which carries every plant part and every
    /// pole the run laid.
    #[test]
    fn an_unpowered_site_comes_back_powered_and_not_merely_covered() {
        let s = state();
        // Far enough from the lake that a plant has to be built *and* a run
        // of poles has to reach back to it.
        let site = Position::new(60.5, 60.5);
        let area = s
            .collision_area(CONSUMER, &site)
            .expect("the consumer has a prototype");
        let unpowered = Condition::Powered {
            pos: site.clone(),
            entity: CONSUMER.into(),
            kw: CONSUMER_KW,
        };
        assert!(
            !unpowered.holds(&s, BotId(1)),
            "control: the fixture must have no generation at {site}, or this \
             test is about the other branch"
        );

        let mut ctx = ExpansionCtx::new(s, BotId(1));
        let powering = ensure_powered(
            &mut ctx,
            CONSUMER,
            &site,
            &area,
            CONSUMER_KW,
            SUPPLY_RADIUS,
            &[],
        )
        .expect("the fixture has a lake to site a plant on")
        .expect("and open ground to run poles over");

        assert!(
            !powering.steps.is_empty(),
            "an unpowered site must cost something"
        );
        assert!(
            !powering.ids.is_empty(),
            "the caller must be given something to order its placement after"
        );

        // **The overlay is not the plan.** `plant_steps` creates its parts in
        // `ctx.state` as well as emitting them, so a version that built the
        // plant into the overlay and then dropped its steps on the floor still
        // passes the `Powered` check below -- verified by breaking exactly that
        // and watching this test stay green. What the run actually performs is
        // the steps, so the generator has to be *in* them.
        let placed: Vec<&str> = powering
            .steps
            .iter()
            .filter_map(|step| match step {
                Step::Act(action) => match &action.kind {
                    ActionKind::Place { entity } => Some(entity.name.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert!(
            placed.contains(&ENGINE),
            "the plant the site depends on must be in the emitted steps, not \
             only in the planning overlay; placed {placed:?}"
        );
        assert!(
            placed.contains(&POLE),
            "and so must the run that carries it there; placed {placed:?}"
        );

        // Every placement handed back is one the caller must wait for. An id
        // missing here is an edge nobody draws, which `infer_edges` cannot
        // recover because nothing satisfies `Condition::Powered`.
        let ids: Vec<ActionId> = powering.ids.clone();
        for step in &powering.steps {
            if let Step::Act(action) = step
                && matches!(action.kind, ActionKind::Place { .. })
            {
                assert!(
                    ids.contains(&action.id),
                    "{} is placed but its id is not one the caller is told to \
                     order against",
                    action.label
                );
            }
        }

        assert!(
            powering.powered.holds(&ctx.state, BotId(1)),
            "the site must be POWERED afterwards, not merely covered -- \
             {} steps and {} ids bought nothing",
            powering.steps.len(),
            powering.ids.len()
        );
    }

    /// The same radius `method::extract` passes, restated here rather than
    /// imported from it: these tests are about `ensure_powered`, and reaching
    /// into a caller for a constant would make a change there fail here for a
    /// reason that has nothing to do with this code.
    const SUPPLY_RADIUS: f64 = 64.;
}
