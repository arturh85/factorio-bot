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
//! ## The supply crossover this section used to claim does not exist
//!
//! Until 2026-09-06 the paragraph above was followed by one that read:
//!
//! > *"**What binds is supply, not price.** Wood is the item this planner
//! > cannot make: a four-bot run starts with four, and no method in this crate
//! > mines a tree. Four wood is eight poles is about **56 tiles of wire,
//! > ever** [...] So the crossover is a **supply** crossover at roughly 56
//! > tiles [...] wire short runs and pipe long ones."*
//!
//! It is quoted rather than quietly deleted for the same reason the water
//! premise above is: a design rule — *"wire short runs and pipe long ones"* —
//! and half the case for the unbuilt pipe router were justified by it. **It is
//! false, and it was already false when it was written.** Three independent
//! things falsify it, and any one of them would be enough.
//!
//! **1. This crate chops.** [`crate::method::have::Chop`] swings at any
//! standing minable entity, trees and rocks alike, and is registered in the
//! production registry ahead of `Mine`. Its own doc records this exact cap
//! being removed after live run `run-1788396958-07935` halted on it: *"before
//! this method every wood in a run was wood a bot had been holding since it
//! spawned: four bots, four wood [...] That cap was a property of this model
//! and of nothing else."* [`crate::method::have::BUFFER_CHEST`] then picks a
//! `wooden-chest` over an iron one **because** wood is renewable, off the
//! reference map's 6,656 standing trees. So the constraint was real, was read
//! from a real place, and had been lifted in a file this pricing never
//! consulted. `the_planner_can_obtain_the_wood_a_long_pole_run_wants` asserts
//! it against the registry rather than against a constant in this file.
//!
//! **2. Wood is a tier-one artefact anyway.** The owner, who plays the game:
//! *"wood is only needed for the very first tier of power poles, we will
//! quickly research the better tiers which don't need wood at all."* Read off
//! `workspace/server/data/base/prototypes/recipe.lua` and
//! `entity/entities.lua` on 2026-09-06, with `iron-stick` (1 iron plate → 2),
//! `steel-plate` (5 iron plates → 1) and `copper-cable` (1 copper plate → 2)
//! folded in:
//!
//! | pole | ingredients | iron plates | wire reach | iron plates a tile |
//! |---|---|---|---|---|
//! | `small-electric-pole` | 1 wood + 2 copper-cable → **2** | **0** | 7.5 | **0** |
//! | `medium-electric-pole` | 4 iron-stick + 2 steel-plate + 2 copper-cable → 1 | 12 | 9 | 1.33 |
//! | `big-electric-pole` | 8 iron-stick + 5 steel-plate + 4 copper-cable → 1 | 29 | 32 | **0.91** |
//!
//! No wood past tier one. `electric-energy-distribution-1` (120 red + green)
//! unlocks the medium pole, the big pole and `iron-stick`, and it is already
//! on the oil ladder — so by the time power is being run to a well 350 tiles
//! out, the pole that needs no wood is a technology away rather than a forest
//! away. And note the last column: even paid for entirely in iron, a big pole
//! is **cheaper per tile than pipe**, and a medium pole is within a third of
//! it. There is no tier at which pipe becomes the material answer.
//!
//! **These three rows are prose, not a checked constant.** This crate builds
//! [`POLE`] and only [`POLE`], which is `small-electric-pole`; the fixture
//! recipes carry no `medium-electric-pole`, `big-electric-pole` or
//! `steel-plate`, so nothing in this crate can assert the two lower rows.
//! They are the game's numbers, dated, with their source named — see the rule
//! at the end of `docs/superpowers/notes/2026-09-06-stale-constraints.md`.
//!
//! **3. No crossover of any kind can exist between these routes, because
//! every one of them is linear in distance.** [`pipe_run_plates`] is
//! `ceil(N)` plates and [`pole_run_items`] is `ceil(ceil(N/7)/2)` crafts;
//! neither has a fixed cost, so their ratio is the same at 7 tiles and at 700
//! and no distance can reverse it. A crossover needs one route to carry a
//! setup charge the other amortises, and neither does. That is a property of
//! this module's own arithmetic and
//! `no_distance_turns_the_pole_route_into_the_dearer_one` asserts it directly
//! at the 56/57-tile boundary the retracted claim named.
//!
//! So the honest reading of *"poles run out at 56 tiles"* is **"you are still
//! holding the starting pole"** — a research problem, not a logistics one.
//! `docs/superpowers/notes/2026-09-06-piping-water-is-cheap.md` carries the
//! same correction; this module agrees with it.
//!
//! ## What survives, priced — and it is not a distance rule
//!
//! Nothing rules the pole route out. Three real residuals, none of which is a
//! crossover, and all three stated so the next reader can check them:
//!
//! * **A chop needs a charted standing minable.** `Chop::applicable` refuses
//!   without one (`PlanState::has_minable_source`), so on an unexplored or a
//!   genuinely bare map wood is unobtainable — and the failure is a
//!   `NoApplicableMethod { goal: "have N wood" }` at expansion, not a longer
//!   plan. The fix for it is charting, or research, not pipe. The treeless
//!   half of `the_planner_can_obtain_the_wood_a_long_pole_run_wants` pins
//!   both directions.
//! * **Time, where the wood is the cheap half.** Derived from the two figures
//!   [`crate::method::have::BUFFER_CHEST`] measured on the reference map: two
//!   wood off one dead tree is **372 ticks with the walk in it** (~186 a
//!   wood), and eight iron plates plan at **2,965 ticks** (~371 a plate, its
//!   furnace and fuel amortised). One pole craft is 1 wood + 1 copper plate
//!   and buys 14 tiles, so about 557 ticks — **~40 ticks a tile against
//!   pipe's ~371**. The pole route wins on time by about the same order it
//!   wins on materials, and **the expensive half of a pole craft is the
//!   copper plate, not the wood**: the item five paragraphs were once spent
//!   on is the cheaper one. (Arithmetic over two measurements taken
//!   elsewhere, not a measurement of this route.)
//! * **This crate can only build tier one.** [`POLE`] is hard-coded, so the
//!   wood-free poles of row 2 above are a fact about Factorio and not yet a
//!   capability here. A run that can find no tree cannot fall back on them
//!   today. That is a note for whoever wires the tier up, not a reason to
//!   pipe.
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
//! The reason that used to bind was `crate::state`'s: **a panel's output is a
//! function of the map clock.** Vanilla states `production = "60kW"`, which is
//! peak; the Nauvis daily average is about 42 kW and the value at night is
//! zero. This crate is pure and deterministic and has no in-game time of day
//! among its inputs, so crediting 60 plans a base that is dead for a third of
//! every day.
//!
//! **That objection is answered and the exclusion stands for a different
//! reason.** A surface reports its own daylight curve, and an *average* over
//! it is a function of surface constants — perfectly deterministic, whatever
//! hour the run starts. `PlanState::solar_average_kw` derives it (0.7 of
//! nameplate on Nauvis, from the curve and not from a table) and
//! `PlanState::accumulators_per_panel` derives the storage a day's shortfall
//! needs (0.85 per panel, the vanilla 25:21 scaled by the day length the game
//! actually reports). Both come out of one channel by two different integrals.
//!
//! What binds now is **storage**: an array credited its average keeps a base
//! alive only if the accumulators to carry the night are standing. That check
//! is [`solar_supply_kw`], and the owner's framing of the trade is why it is a
//! refusal rather than a credit — a refusal is visible at plan time, and a
//! base that dies at 03:00 is *coverage is not capacity with a clock
//! attached*, which reads in a run log as a stall nothing explains. The
//! conservative-looking option is the reckless one.
//!
//! **The last wire is not in this crate's reach.** Feasibility is decided by
//! `PlanState::electric_supply_kw`, which credits solar nothing; turning
//! [`solar_supply_kw`]'s `Ok` into supply is one call there. Until then this
//! arm's reach is the refusal path, where it turns "no pole run carries power
//! here" into "your solar farm has no batteries".

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

/// How many pipes a **one-boiler** plant lays. Derived by [`layout`], asserted
/// by a test — this is the bill, not the design.
///
/// Independent of the engine count: engines chain directly off each other's
/// steam connection, so a second engine adds no pipe. See [`layout`].
///
/// **It is not independent of the boiler count.** Each boiler has its own
/// steam joint and so its own pipe, which is why the general figure is
/// [`pipe_count`] and this constant is the one-boiler case it agrees with.
/// Boilers themselves chain with no pipe between them — see
/// [`BOILER_PITCH_TILES`].
pub const PIPE_COUNT: u32 = 3;

/// How many pipes a plant of `boilers` boilers lays.
///
/// One at the pump's own joint, then **two per boiler**: the water joint it
/// shares with the boiler before it (the step along the shore, for the first
/// one) and its own steam joint. `pipe_count(1)` is [`PIPE_COUNT`], and
/// `the_pipe_bill_is_one_plus_two_per_boiler` asserts that against the layout
/// rather than against this arithmetic.
#[must_use]
pub fn pipe_count(boilers: u32) -> u32 {
    1 + 2 * boilers
}

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
/// **Not a constraint, and this doc used to say it was.** It read *"the
/// binding constraint on the pole route [...] no method in this crate makes
/// wood"*, which `crate::method::have::Chop` had already falsified: wood is
/// renewable off any charted standing tree, and no pole above tier one wants
/// any. It is also the *cheap* half of the craft — the copper plate beside it
/// costs roughly twice as many ticks. The module doc's "the supply crossover
/// does not exist" section has the whole retraction.
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
/// 355-tile pipe run is 355 iron plates, where the same distance in poles is
/// 51 poles off 26 crafts — **26 wood and 26 copper plates**, an order of
/// magnitude fewer items.
///
/// This doc used to end *"which a four-bot run cannot obtain at all"*. It can:
/// `crate::method::have::Chop` takes 26 wood off seven trees, and
/// `the_planner_can_obtain_the_wood_a_long_pole_run_wants` asks the registry
/// for exactly this bill rather than asserting it here.
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
/// # This is a per-boiler bound, and it is no longer the plant's ceiling
///
/// Until 2026-09-06 this number *was* the plant's ceiling, because [`layout`]
/// laid out exactly one boiler. It now chains boilers along the shore
/// ([`BOILER_PITCH_TILES`]) and the plant's ceiling is
/// [`BOILERS_PER_PUMP`] boilers of `MAX_ENGINES_PER_BOILER` engines each. What
/// survives here is the ratio between *one* boiler and *its* engines, which is
/// the vanilla arithmetic above and nothing to do with our layout.
pub const MAX_ENGINES_PER_BOILER: u32 = 2;

/// Water an `offshore-pump` moves, in fluid units per second.
///
/// **Prose from the prototypes, not a checked constant** -- this crate's
/// fixtures carry no `pumping_speed`, so nothing here can assert it. Read off
/// `workspace/server/data/base/prototypes/entity/entities.lua` on 2026-09-06:
/// `offshore-pump` states `pumping_speed = 20`, and that field is fluid units
/// per **tick**, so `20 x 60 = 1200` a second.
///
/// Stated so that [`BOILERS_PER_PUMP`] can be *derived* here rather than
/// written down, and so the next reader re-derives it from the named file
/// instead of trusting this module. See the rule at the end of
/// `docs/superpowers/notes/2026-09-06-stale-constraints.md`.
pub const PUMP_WATER_PER_SECOND: u32 = 1_200;

/// Water one `boiler` consumes at full output, in fluid units per second.
///
/// Same file, same date, same standing as [`PUMP_WATER_PER_SECOND`]: `boiler`
/// states `energy_consumption = "1.8MW"` and `target_temperature = 165`. Water
/// carries 0.2 kJ per unit per degree, so lifting it the 150 °C from the 15 °C
/// default is 30 kJ a unit, and `1.8 MW / 30 kJ = 60` units a second.
pub const BOILER_WATER_PER_SECOND: u32 = 60;

/// How many boilers one offshore pump's water carries, and therefore how many
/// this planner chains onto one plant.
///
/// `1200 / 60 = 20`. **This one is a fact about the game**, not about our
/// layout, which is exactly why it is the number [`plant_size_for`] refuses
/// at: past it a twenty-first boiler would stand on the shore with nothing
/// to boil, and a plant that reads as generating and does not is the
/// coverage-is-not-capacity failure this module exists to avoid.
///
/// Twenty boilers of [`MAX_ENGINES_PER_BOILER`] engines is 40 engines, and at
/// the 900 kW [`crate::state::PlanState::generator_output_kw`] credits each
/// one that is **36 MW** -- twenty times what the single-boiler layout could
/// carry, and two orders of magnitude above anything a run in this repo's
/// record has yet drawn (every green run reads `roster-fed`, with 120 kW
/// drawn after the first eight minutes).
///
/// **An owner-supplied ratio of "1 pump : 200 boilers : 400 engines" does not
/// survive the prototypes** -- it is ten times this, and would put the ceiling
/// at ~360 MW. Recorded rather than quietly dropped, because the next reader
/// will meet both numbers; the arithmetic above is how to settle it.
///
/// What this bound is *not*: it is not a claim that twenty boilers will fit.
/// A plant that does not fit is refused by
/// [`PlannerError::PowerPlantNeedsShore`] after the shoreline search, which is
/// a fact about the map. This is the only bound stated in advance.
pub const BOILERS_PER_PUMP: u32 = PUMP_WATER_PER_SECOND / BOILER_WATER_PER_SECOND;

/// How many boilers and engines a plant carrying `kw` is built from.
///
/// A pair rather than a number, because since 2026-09-06 the plant grows in
/// both: engines come from the demand, boilers come from the engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlantSize {
    /// Boilers in the chain along the shore, at least one.
    pub boilers: u32,
    /// Steam engines across the whole plant, at least one.
    pub engines: u32,
}

impl PlantSize {
    /// How many engines stand on each boiler, boiler by boiler.
    ///
    /// **Greedy, and with [`MAX_ENGINES_PER_BOILER`] at two that is also the
    /// balanced answer** -- filling boilers to two leaves at most one boiler
    /// holding one engine, and no distribution of `n` engines over
    /// `ceil(n/2)` boilers can do better than that. The distinction would
    /// matter if the ratio were ever three or more, so it is stated rather
    /// than left to be rediscovered.
    ///
    /// The sum is [`engines`](Self::engines) exactly, because
    /// [`boilers`](Self::boilers) is `ceil(engines / MAX_ENGINES_PER_BOILER)`
    /// -- `an_engine_is_never_stranded_off_the_end_of_the_boiler_chain` is
    /// that claim as a test rather than as this sentence.
    #[must_use]
    pub fn engine_split(&self) -> Vec<u32> {
        let mut left = self.engines;
        (0..self.boilers)
            .map(|_| {
                let take = left.min(MAX_ENGINES_PER_BOILER);
                left -= take;
                take
            })
            .collect()
    }
}

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
///
/// # The ceiling here is the WATER's, and it moved on 2026-09-06
///
/// This function used to refuse above [`MAX_ENGINES_PER_BOILER`], because the
/// layout laid one boiler. It now refuses above
/// `BOILERS_PER_PUMP x MAX_ENGINES_PER_BOILER` = 40 engines = **36 MW**, and
/// that number is the pump's water divided by a boiler's thirst rather than
/// anything about our geometry. See [`BOILERS_PER_PUMP`].
pub fn engines_for(state: &PlanState, kw: f64) -> Result<u32, PlannerError> {
    plant_size_for(state, kw).map(|size| size.engines)
}

/// The plant [`plan_plant_for`] will lay out for a demand of `kw`, or why no
/// plant carries it.
///
/// Sizing is done here, before a single terrain tile is read, so a demand
/// nothing can carry costs nothing to discover.
///
/// # What refuses, and whose fact it is
///
/// * **an unpriceable engine** -- `plant_kw: 0.`, the direction an unknown
///   generator errs in everywhere else in this crate;
/// * **more than [`BOILERS_PER_PUMP`] boilers' worth of demand**, which is a
///   fact about `offshore-pump`'s `pumping_speed` against `boiler`'s
///   `energy_consumption` and **not** about this layout. Above it the plan
///   would chain boilers that no water reaches.
///
/// Everything else that can stop a plant being built -- no water in range, no
/// shoreline the chain fits on, ground in the way -- happens later, against
/// the map, and says so by name ([`PlannerError::PowerPlantNeedsWater`],
/// [`PlannerError::PowerPlantNeedsShore`]). **Those two are the layout's
/// refusals; this one is the game's.** Keeping them apart is the whole point:
/// a reader told "no plant generates that" should be able to tell whether to
/// ask for less or to look at the map.
pub fn plant_size_for(state: &PlanState, kw: f64) -> Result<PlantSize, PlannerError> {
    let each = state
        .generator_output_kw(ENGINE)
        .ok_or(PlannerError::PowerPlantTooSmall {
            needed_kw: kw,
            plant_kw: 0.,
        })?;
    let ceiling = f64::from(BOILERS_PER_PUMP * MAX_ENGINES_PER_BOILER);
    let wanted = (kw / each).ceil().max(1.);
    // `total_cmp` rather than `>`: this crate orders every float that way.
    if wanted.total_cmp(&ceiling).is_gt() {
        return Err(PlannerError::PowerPlantTooSmall {
            needed_kw: kw,
            plant_kw: each * ceiling,
        });
    }
    // In range by the test above, and `wanted >= 1`.
    let engines = wanted as u32;
    Ok(PlantSize {
        boilers: engines.div_ceil(MAX_ENGINES_PER_BOILER),
        engines,
    })
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
///   and the pole's one wood and one copper plate, together about 557 ticks
///   (`crate::method::have::BUFFER_CHEST`'s two measurements). This clause
///   used to read *"one wood, of which a four-bot run has exactly four and can
///   make no more"*; `crate::method::have::Chop` had already made wood
///   renewable, and the wood was never the expensive part of a plant.
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

/// How far apart consecutive boilers in a chain stand, in tiles along the
/// shore.
///
/// **Derived from [`BOILER_WATER`], not written down**: it is the separation
/// of a boiler's two water targets, so at this pitch boiler `k`'s far target
/// and boiler `k+1`'s near target are the **same tile**, and one pipe standing
/// there joins them. That is the module's own joining rule -- *two entities
/// are joined when a pipe stands on a tile that both of them name* -- applied
/// along the chain, which is why [`pipe_count`] grows with the boiler number.
/// A wrong number in `BOILER_WATER` moves the chain and fails a test rather
/// than sitting there being decorative.
///
/// # Why not three, which is the boiler's own width
///
/// The engine row does chain body-to-body -- consecutive engines five apart
/// have each one's far connection landing *inside* the next -- and the same
/// trick works for boilers at a pitch of three, with no linking pipe at all.
/// **It was built that way first and it could not be powered.** A steam
/// engine is three tiles wide against a three-tile pitch, so the engine
/// columns tile the ground with no gap between them, and a
/// `small-electric-pole`'s five-by-five supply area can then only reach a
/// two-engine column from *beside* the block. Two columns have a side; the
/// ones in the middle do not, and [`pole_chain`] refused every candidate at
/// four boilers and up -- surfacing as `PowerPlantNeedsShore` on a 200-tile
/// clean beach.
///
/// At four the columns leave a one-tile corridor between them, a pole in it
/// reaches the two engines on either side, and consecutive corridors are four
/// tiles apart -- inside [`WIRE_REACH`], so the chain is one network. One
/// extra pipe per boiler buys every boiler past the third.
pub const BOILER_PITCH_TILES: f64 = (BOILER_WATER[1].0 - BOILER_WATER[0].0).abs();

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
    ///
    /// Kept as a single position for every caller and test that wants "the
    /// boiler" of the one-boiler plant this planner built until 2026-09-06.
    /// The whole chain, which is what has to be **fuelled**, is
    /// [`boilers`](Self::boilers); this is its first entry, the one nearest
    /// the pump.
    pub boiler: Position,
    /// Every boiler in the chain, in build order, nearest the pump first.
    ///
    /// One entry for a plant sized under 1.8 MW, which is every demand this
    /// planner has asked for in a live run. See [`plant_size_for`].
    ///
    /// **Every one of them is fuelled**, not just [`boiler`](Self::boiler):
    /// coal in one boiler of four leaves three cold, and a plant delivering a
    /// quarter of its nameplate while every entity stands is the
    /// coverage-is-not-capacity failure again, one level down. `plant_steps`
    /// emits one `Insert` per entry and `bill` sizes the coal off the same
    /// list.
    pub boilers: Vec<Position>,
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
/// most [`MAX_ENGINES_PER_BOILER`] **per boiler**. **The extra engines cost no
/// extra pipe**:
/// a steam engine's two `ENGINE_STEAM` connections are one tile beyond each
/// end of its own five-tile length, so consecutive engines whose centres are
/// five apart along the row have each one's far connection landing inside the
/// next one's body — which is how the game joins two engines placed end to
/// end, and why [`PIPE_COUNT`] is independent of the engine count.
///
/// The row extends **away from the boiler**, i.e. further inland, along the
/// same axis `ENGINE_STEAM[1]` already chose for the first engine. Extending
/// the other way would put the second engine on top of the boiler.
///
/// # The boiler chain
///
/// `boilers` is how many boilers stand end to end along the shore, at least
/// one and at most [`BOILERS_PER_PUMP`]. Each carries its own steam pipe and
/// its own inland engine row, so the plant is a *grid* rather than a row --
/// but still one rigid body rotated about the pump's tile centre, which is
/// what keeps every building on its own grid at all four facings.
///
/// The chain runs along `lateral`, i.e. along the shore, at
/// [`BOILER_PITCH_TILES`]. It does not collide with the engine rows: an engine
/// is three tiles wide against a three-tile boiler pitch, so consecutive rows
/// touch and never overlap
/// (`the_plants_own_buildings_never_overlap_each_other` is that claim as a
/// test, over every size this planner will lay out).
///
/// **Build order is water-first, boiler by boiler**: pump, the two water
/// pipes, then for each boiler its own body, its steam pipe and its engines.
/// A boiler is built after the one whose far water joint feeds it, which is
/// the order a bot could actually build the thing in.
fn layout(
    pump: &Position,
    facing: Direction,
    boilers: u32,
    engines: u32,
) -> Option<Vec<PlantPart>> {
    let lateral = turned((1., 0.), facing)?;

    let joint_pump = pump.add(&turned(PUMP_OUTPUT, facing)?);
    let joint_boiler = joint_pump.add(&lateral);

    // Steam away from the water: the boiler faces opposite the pump.
    let boiler_facing = compose(Direction::South, facing)?;
    let first_boiler = subtract(&joint_boiler, &turned(BOILER_WATER[1], boiler_facing)?);

    let engine_facing = facing;

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
    // pipe reaches (it is what each `engine` is placed against, below), so
    // `ENGINE_STEAM[0]`'s sign is the inland one.
    let pitch = turned((0., length * ENGINE_STEAM[0].1.signum()), engine_facing)?;
    let chain = turned((BOILER_PITCH_TILES, 0.), facing)?;

    let split = PlantSize { boilers, engines }.engine_split();
    for (index, row) in split.iter().enumerate() {
        let step = index as f64;
        let boiler = Position::new(
            first_boiler.x() + chain.x() * step,
            first_boiler.y() + chain.y() * step,
        );
        // The water joint this boiler drinks from: the step along the shore
        // out of the pump for the first one, and for every one after it the
        // tile the *previous* boiler's far connection already names, so a
        // single pipe joins the pair.
        parts.push(PlantPart {
            name: PIPE,
            position: Position::new(
                joint_boiler.x() + chain.x() * step,
                joint_boiler.y() + chain.y() * step,
            ),
            direction: Direction::North,
        });
        let joint_engine = boiler.add(&turned(BOILER_STEAM, boiler_facing)?);
        let engine = subtract(&joint_engine, &turned(ENGINE_STEAM[1], engine_facing)?);
        parts.push(PlantPart {
            name: BOILER,
            position: boiler,
            direction: boiler_facing,
        });
        parts.push(PlantPart {
            name: PIPE,
            position: joint_engine,
            direction: Direction::North,
        });
        for row_index in 0..*row {
            let inland = f64::from(row_index);
            parts.push(PlantPart {
                name: ENGINE,
                position: Position::new(
                    engine.x() + pitch.x() * inland,
                    engine.y() + pitch.y() * inland,
                ),
                direction: engine_facing,
            });
        }
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
///    distance to, because building a second one costs about 45 iron plates,
///    which have to be mined and smelted. (This used to say "and one of a
///    run's four irreplaceable wood"; the wood is renewable and was never what
///    made a second plant expensive.) See [`PLANT_ADOPT_RADIUS`] for the run
///    that was killed by not doing this.
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
/// **That the plant is running.** Nothing in `FactorioSurface` says whether a
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
/// The failure this was built for: bots that had finished an exploration ring
/// and parked at `(255, 249)`, where every power-needing goal refused with *"a
/// power plant needs water, and the plan can see none within 128 tiles"* — a
/// true statement about what was looked at, and a false impression of a map
/// whose lake `score-map` reports at 48.1 tiles from the origin. The radii
/// were not the fault and neither was the map; the anchor was.
///
/// **The worked example this doc used to give is no longer reproducible, and
/// saying so is the honest form.** It tabulated `map-31337-explored.json` with
/// its bots at `(255, 249)` as *"355 tiles from water, refuses"*. Re-measured
/// on 2026-09-06 with `score-map --from 255,249`, the nearest water on that
/// dump — and on `map.json` too — is **47.4 tiles**, inside even the cheap
/// 64-tile scan, so neither dump refuses from there any more. Both dumps were
/// rewritten that day by the exploration work (`map-31337-explored.json` at
/// 13:01), and an exploration ring is exactly the thing that turns ungenerated
/// ground into charted water. The 355 was presumably read off the dump as it
/// stood earlier; it cannot be checked, because nothing kept that file.
///
/// The property is proven by fixture instead, which is what a test can hold:
/// `the_caller_anchored_search_refuses_from_where_the_bot_parked` and
/// `a_roster_that_walked_away_still_gets_a_plant_at_the_world_anchor`.
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
    let size = plant_size_for(state, kw).ok()?;
    let parts = layout(&pump.position, facing, size.boilers, size.engines)?;
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
    // Both lists, and in `layout`'s own order rather than in the order the
    // world happened to split them: `boilers` is what gets fuelled, one
    // `Insert` each, and a chain read out of `standing` first would fuel them
    // in an order that has nothing to do with the plant.
    let boiler_row: Vec<Position> = layout(&pump.position, facing, size.boilers, size.engines)?
        .iter()
        .filter(|part| part.name == BOILER)
        .map(|part| part.position.clone())
        .collect();
    let boiler = boiler_row.first()?.clone();
    // Both lists, because the row may be part standing and part missing: a
    // one-engine plant being grown to two has its first engine in `standing`
    // and its second in `missing`. The build order `layout` fixed is what
    // orders them, so `engine` stays the one nearest the boiler however the
    // row is split.
    let engine_parts: Vec<&PlantPart> = layout(&pump.position, facing, size.boilers, size.engines)?
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
    // Owned, because `standing` and `missing` are about to be pushed to and
    // `engine_parts` borrows both. The row is `layout`'s own order either way.
    let engine_parts_owned: Vec<PlantPart> = engine_parts.into_iter().cloned().collect();

    let mut trial = state.fork();
    for part in &missing {
        trial.create_entity(entity_for(&trial, part));
    }
    // One pole per boiler's engine row, adopting any that already stand --
    // the engine may have been the one part a cut-short plan never placed
    // while its pole went down early. `pole_chain` creates each placed pole in
    // `trial` as it goes; an adopted one is already there.
    let poles = pole_chain(&mut trial, &engine_groups(&engine_parts_owned, size), true)?;
    let pole = poles.first()?.position.clone();
    for choice in &poles {
        let part = PlantPart {
            name: POLE,
            position: choice.position.clone(),
            direction: Direction::North,
        };
        if choice.standing {
            standing.push(part);
        } else {
            missing.push(part);
        }
    }
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
        boilers: boiler_row,
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
    let size = plant_size_for(state, kw)?;
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
        None => match state.nearest_water_tile(from, PLANT_WATER_WIDE_SCAN_RADIUS) {
            Some(far) => far,
            // Only on the refusal path, so the seventeen extra tile-tree
            // probes cost nothing on a plan that works. They are what makes
            // the refusal say whether it was *dry* or *blind*: an
            // ungenerated chunk holds no tiles, so "no water here" and "no
            // ground here at all" are the same `None` out of
            // `nearest_water_tile` and only `charting` tells them apart.
            None => {
                let charting = state.charting(from, PLANT_WATER_WIDE_SCAN_RADIUS);
                return Err(PlannerError::PowerPlantNeedsWater {
                    radius: PLANT_WATER_WIDE_SCAN_RADIUS,
                    anchor_x: from.x(),
                    anchor_y: from.y(),
                    covered_probes: charting.covered,
                    probes: charting.probes,
                });
            }
        },
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
                    if let Some(plant) = fit(state, &tile_centre(&tile), facing, size) {
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
fn fit(state: &PlanState, pump: &Position, facing: Direction, size: PlantSize) -> Option<Plant> {
    let parts = layout(pump, facing, size.boilers, size.engines)?;
    for part in &parts {
        if !state.is_area_free_facing(part.name, &part.position, part.direction) {
            return None;
        }
    }
    let boiler_row: Vec<Position> = parts
        .iter()
        .filter(|part| part.name == BOILER)
        .map(|part| part.position.clone())
        .collect();
    let boiler = boiler_row.first()?.clone();
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
    // One pole per boiler's engine row, chained. `pole_chain` creates each in
    // `trial` as it goes -- which is what the enclosure check below needs,
    // since it reads `trial`'s own `added` map and a plant checked without its
    // poles would miss the parts sited last.
    let poles = pole_chain(&mut trial, &engine_groups(&parts, size), false)?;
    let pole = poles.first()?.position.clone();

    let mut parts = parts;
    for choice in &poles {
        parts.push(PlantPart {
            name: POLE,
            position: choice.position.clone(),
            direction: Direction::North,
        });
    }
    let mut plant = Plant {
        parts,
        standing: Vec::new(),
        boiler,
        boilers: boiler_row,
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

/// One pole of the plant's chain: where it is, and whether it was already
/// there.
#[derive(Clone, Debug, PartialEq)]
struct PoleChoice {
    position: Position,
    /// True when a pole already stood here and is being adopted rather than
    /// placed. [`finish`] puts these in `standing`; [`fit`] never produces one.
    standing: bool,
}

/// Where to put the plant's poles: **one per boiler's engine row**, each
/// covering every engine of its own row and each in wire reach of the one
/// before, so the whole plant is one network.
///
/// # One implementation, because a second one would be the failure
///
/// Both [`fit`] and [`finish`] site poles, and both used to ask their own
/// question about their own engine. With a one-engine row those two questions
/// are the same; with two they are not, and a pole that reaches the first
/// engine and not the second is 900 kW standing on the ground while the plan
/// reads 1,800 — placement and function are separate concerns, and this is
/// the class of failure that places 100 % and does nothing.
///
/// # Why it is a chain and not one pole
///
/// **This is where the boiler chain diverged from its design note.** The note
/// (`2026-09-06-one-place-that-decides-power.md`) listed five owners that had
/// to change and did not list this one, and a single pole is what actually
/// stopped the first three-boiler plant being sited: a `small-electric-pole`
/// supplies a **five-by-five** area, and three boilers' engine rows span six
/// tiles across and ten inland. No tile covers them all, so `pole_site`
/// returned `None`, `fit` refused every shoreline candidate, and the failure
/// surfaced as `PowerPlantNeedsShore` on an eighty-tile beach — the map blamed
/// for a fact about the layout, which is the defect class this change exists
/// to remove.
///
/// So each boiler's row gets its own pole, and consecutive poles are held
/// within [`WIRE_REACH`] of each other. At [`BOILER_PITCH_TILES`] the rows are
/// three tiles apart, so the constraint is slack in the ordinary case and only
/// bites when obstacles push a pole sideways — in which case the candidate is
/// refused rather than a plant built in two halves, one of which generates
/// into a network nothing draws from.
///
/// The search for each starts at [`engine_row_centre`] of that boiler's own
/// engines rather than at any one engine. With one engine that is the engine,
/// which is where this search has always started; with two it is the joint
/// between them, which is the only stretch of ground a five-by-five supply
/// area can overlap both from.
///
/// **The one-boiler plant is unchanged by construction**: one group, no
/// previous pole, one `free_area_near_where` call with exactly the old
/// predicate.
///
/// `adopt` asks whether a pole that already stands may be taken instead of
/// placed — [`finish`]'s case, where a cut-short plan may have got its pole
/// down before its engine. `groups` is each boiler's engines in build order.
/// Each pole is created in `trial` as it is chosen, so the next one's ring
/// search cannot land on it.
///
/// `None` when any row has no site — which refuses the whole plant candidate,
/// rather than building a row half of which generates into nothing.
fn pole_chain(
    trial: &mut PlanState,
    groups: &[Vec<&PlantPart>],
    adopt: bool,
) -> Option<Vec<PoleChoice>> {
    let mut chain: Vec<PoleChoice> = Vec::new();
    for group in groups {
        let areas = engine_areas(trial, group)?;
        let row: Vec<Position> = group.iter().map(|part| part.position.clone()).collect();
        let origin = engine_row_centre(&row);
        let previous = chain.last().map(|pole| pole.position.clone());
        let linked = |candidate: &Position| match &previous {
            // `total_cmp`, and `<=`: a pole at exactly the wire reach is on
            // the network, which is what `PlanState` itself answers -- see
            // `WIRE_REACH`'s doc for why that claim is pinned behaviourally.
            Some(before) => calculate_distance(before, candidate)
                .total_cmp(&WIRE_REACH)
                .is_le(),
            None => true,
        };
        // A pole that already reaches **every** engine of this row, nearest
        // first in `entities_within`'s fixed order; only otherwise one to
        // place. All of them, not the first: a standing pole that covers one
        // engine of two leaves the other generating into nothing.
        let standing = if adopt {
            trial
                .entities_within(
                    &origin,
                    f64::from(crate::method::util::FREE_TILE_SEARCH_RADIUS) + 4.,
                )
                .into_iter()
                .find(|entity| {
                    linked(&entity.position)
                        && areas.iter().all(|area| {
                            trial.pole_would_supply(&entity.name, &entity.position, area)
                        })
                })
                .map(|entity| entity.position)
        } else {
            None
        };
        match standing {
            Some(position) => chain.push(PoleChoice {
                position,
                standing: true,
            }),
            None => {
                let position = free_area_near_where(trial, &origin, POLE, |candidate| {
                    linked(candidate)
                        && areas
                            .iter()
                            .all(|area| trial.pole_would_supply(POLE, candidate, area))
                })?;
                let part = PlantPart {
                    name: POLE,
                    position: position.clone(),
                    direction: Direction::North,
                };
                trial.create_entity(entity_for(trial, &part));
                chain.push(PoleChoice {
                    position,
                    standing: false,
                });
            }
        }
    }
    Some(chain)
}

/// Each boiler's engines, in the chain's own order.
///
/// `groups[k]` is boiler `k`'s row. Read off [`layout`]'s output rather than
/// recomputed, so a change to the split moves both together.
fn engine_groups(parts: &[PlantPart], size: PlantSize) -> Vec<Vec<&PlantPart>> {
    let mut engines = parts.iter().filter(|part| part.name == ENGINE);
    size.engine_split()
        .into_iter()
        .map(|row| (0..row).filter_map(|_| engines.next()).collect())
        .collect()
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
        .globals
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
    // Per engine, because a boiler burns fuel in proportion to the steam its
    // engines draw: a two-engine plant was sized for roughly twice the load
    // and burns roughly twice the coal over the same window. A one-engine
    // plant -- which is every plant a live run in this repo has yet asked for
    // -- bills the unchanged `PLANT_COAL`.
    //
    // **The sum over the chain, and it is the sum `plant_steps` inserts.**
    // Read off `Plant::engines`, which is the whole row across every boiler
    // however the world split it into standing and missing: the coal is for
    // the whole plant, and a finished plant whose first engine already stands
    // still has two engines drinking steam. Before 2026-09-06 this line
    // billed `PLANT_COAL * engines` while `plant_steps` inserted a flat
    // `PLANT_COAL` into the one boiler, so a two-engine plant acquired ten
    // coal and put five of them nowhere.
    out.push(("coal", coal_charges(plant).iter().sum()));
    out
}

/// How much coal goes into each boiler of `plant`, in the chain's own order.
///
/// [`PLANT_COAL`] per engine that boiler feeds, so the charge follows the
/// steam: a boiler driving two engines is asked for twice what a boiler
/// driving one is. The sum is what [`bill`] acquires, which is what makes the
/// two agree by construction rather than by two people reading the same
/// paragraph.
///
/// A plant whose chain is empty -- which [`plant_size_for`] never produces,
/// since it floors at one boiler -- charges nothing, rather than dividing by
/// zero.
fn coal_charges(plant: &Plant) -> Vec<u32> {
    let boilers = plant.boilers.len() as u32;
    if boilers == 0 {
        return Vec::new();
    }
    let engines = (plant.engines.len() as u32).max(1);
    PlantSize { boilers, engines }
        .engine_split()
        .into_iter()
        .map(|row| PLANT_COAL * row.max(1))
        .collect()
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

    // **Every boiler in the chain, not the first.** A `Plant` carried a single
    // `boiler` until 2026-09-06 and this emitted a single `Insert`; the moment
    // the chain grew, fuelling `plant.boiler` alone would have left the rest
    // cold and the plant would have delivered a fraction of the nameplate
    // `Condition::Powered` credits it with -- everything standing, everything
    // wired, and the network browning out. `coal_charges` sizes each one off
    // the engines it feeds and `bill` acquires exactly their sum.
    for (boiler, count) in plant.boilers.iter().zip(coal_charges(plant)) {
        let fuel = ctx.ids.next();
        order_research_after.push(fuel);
        steps.push(Step::Act(Box::new(Action {
            id: fuel,
            kind: ActionKind::Insert {
                pos: boiler.clone(),
                entity: BOILER.into(),
                slot: InventorySlot::Fuel,
                item: "coal".into(),
                count,
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: boiler.clone(),
                    radius: reach,
                    min_radius: 0.0,
                },
                Condition::EntityAt {
                    pos: boiler.clone(),
                    name: BOILER.into(),
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: "coal".into(),
                    count,
                },
            ],
            eff: vec![Effect::LoseItem {
                who: Actor::Role,
                item: "coal".into(),
                count,
            }],
            duration: crate::method::have::TRANSFER_TICKS,
            pinned: None,
            label: format!("fuel the boiler with {} coal", count),
        })));
    }

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
/// **They are also the exclusion the headroom test is asked about.** `kw` is
/// their draw, so charging them as standing demand as well would count the
/// block twice; see [`headroom_condition`] for the arithmetic that made a
/// 624 kW block refuse on a 900 kW plant.
///
/// # Three answers, and the middle one is the interesting one
///
/// * `Err` -- supply itself is impossible, and [`supply_for`] says why by
///   name (`PowerPlantNeedsShore`, `PowerPlantTooSmall`, a `Have` shortfall);
///   **or the poles reach and what they reach is too small**, which is
///   [`PlannerError::PowerHeadroomShort`] and is raised here rather than by
///   the caller, because it is not a refusal about the site at all.
/// * `Ok(None)` -- supply exists but **no run of at most [`MAX_POLE_RUN`]
///   poles carries it there**, or the model cannot see the finished run
///   carrying power. **Routing only**, since 2026-09-06: it used to cover the
///   capacity case as well while its message named only this one, and a peer
///   session wiring `Goal::Built` spent two iterations on pole geometry for a
///   block whose poles routed perfectly. The caller names its own refusal,
///   because what an unreachable site means differs: `method::extract` calls
///   it `ExtractionNotModelled`.
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
    let powered = headroom_condition(ctx, consumer, site, kw, occupants);
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

/// The headroom condition [`ensure_powered`] checks and hands back: per-entity
/// when the caller is siting one machine, per-block when it is siting a group.
///
/// # The exclusion is DERIVED from `occupants`, and that is the point
///
/// `ensure_powered`'s contract is that `kw` is the draw of what the caller is
/// about to place, and `occupants` is *what the site will hold once the caller
/// places it* — the same set, stated once. So the ground whose consumers must
/// not be charged against `kw` is the ground those occupants stand on, and
/// asking the caller for it separately would let the two drift: a caller could
/// name a ground that is not the one whose pole-siting was protected, and the
/// resulting condition would be true of a world nobody built. This is the same
/// argument [`Powering`] makes for carrying the condition rather than letting
/// a caller construct a second copy.
///
/// # Why a block needs it at all
///
/// The occupants are created in the routing fork **before** the headroom check
/// — they have to be, or a pole is sited on ground the caller's own building is
/// about to take. For one machine that costs nothing: `Condition::Powered`
/// already excludes the consumer standing at `pos`. For a block it is the whole
/// defect: `FurnaceLine`'s 48 inserters are in the fork, the ledger charges
/// ~611 kW of them as *existing* demand, and the caller then asks for the
/// block's 624 kW on top. 900 − 611 = 289 < 624 — refused by its own
/// arithmetic on a plant with room to spare.
///
/// # A single occupant keeps the per-entity condition exactly
///
/// The ground of one machine contains one consumer — itself — so
/// `Excluded::Ground` of its footprint and `Excluded::Consumer` of its tile
/// name the same set. The per-entity branch is kept anyway, so that every
/// existing caller emits the condition it always emitted, byte for byte, and
/// nothing downstream (a precondition compared for equality, a `Display` in a
/// report) sees a new shape it never asked for.
fn headroom_condition(
    ctx: &ExpansionCtx,
    consumer: &str,
    site: &Position,
    kw: f64,
    occupants: &[FactorioEntity],
) -> Condition {
    let per_entity = Condition::Powered {
        pos: site.clone(),
        entity: consumer.into(),
        kw,
    };
    if occupants.len() < 2 {
        return per_entity;
    }
    let Some(own_ground) = occupied_ground(&ctx.state, occupants) else {
        return per_entity;
    };
    Condition::BlockPowered {
        pos: site.clone(),
        entity: consumer.into(),
        kw,
        own_ground,
    }
}

/// The smallest rectangle covering every occupant's **footprint**, or `None`
/// for an empty list.
///
/// Footprints and not positions, and that is one half of a pair with
/// [`crate::state::Excluded::Ground`]'s inclusive edges — an outermost entity's
/// position lies exactly on a positions-drawn rectangle's boundary and half a
/// collision box inside a footprint-drawn one. Either alone excludes it; break
/// both and the block's whole perimeter is charged against its own draw. That
/// is measured against a 48-inserter fixture, not reasoned: see the falsification
/// note on `Excluded::covers`.
fn occupied_ground(state: &PlanState, occupants: &[FactorioEntity]) -> Option<Rect> {
    let mut bounds: Option<Rect> = None;
    for occupant in occupants {
        let box_ = state.footprint_of(occupant);
        bounds = Some(match bounds {
            None => box_,
            Some(sofar) => Rect::new(
                &Position::new(
                    sofar.left_top.x().min(box_.left_top.x()),
                    sofar.left_top.y().min(box_.left_top.y()),
                ),
                &Position::new(
                    sofar.right_bottom.x().max(box_.right_bottom.x()),
                    sofar.right_bottom.y().max(box_.right_bottom.y()),
                ),
            ),
        });
    }
    bounds
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
        // Two different failures wear this one `false`, and they send a reader
        // to opposite ends of the map. Split them by the figures the decision
        // was actually made of.
        if let Some(err) = capacity_refusal(trial, powered) {
            return Err(err);
        }
        return Ok(None);
    }
    Ok(Some(path))
}

/// The Factorio `entity_type` of a solar panel.
///
/// **A type name, not a prototype name**, so a mod's `big-solar-panel` is
/// classified by what it *is* rather than by what this crate has heard of —
/// the same shape as `crate::state`'s `DETERMINISTIC_GENERATOR_TYPES`, and the
/// same reason.
///
/// # Why not the performance fields, which are the gate one level down
///
/// `PlanState::solar_average_kw` gates on the *presence* of
/// `solar_panel_performance_at_day`, and it must: those fields carry
/// `subclasses: ["SolarPanel"]`, so their presence is what says a prototype
/// follows the daylight curve, and crediting a steam engine that curve would
/// be a silent 30% under-count of a generator that runs all night.
///
/// This is a different question. **Classifying a standing entity must not
/// depend on the fields the pricing needs**, or a world that reports panels
/// without the curve — every dump this project archived before 2026-09-07 —
/// would answer "there are no solar panels here" rather than "there are panels
/// here that this world cannot price". The first is a lie and the second is
/// [`PlannerError::SolarBankNotSizable`].
const PANEL_TYPE: &str = "solar-panel";

/// The Factorio `entity_type` of an accumulator, on the same terms as
/// [`PANEL_TYPE`].
///
/// **Type rather than the buffer field**, and here the distinction bites
/// harder: 48 of this install's 1,028 prototypes carry an
/// `electric_buffer_capacity`, because a lab, a radar and an assembling
/// machine all have a small internal buffer. Classifying on the field would
/// count a lab as part of the bank.
const ACCUMULATOR_TYPE: &str = "accumulator";

/// The solar array and bank standing on one network.
///
/// Counts rather than entities: nothing downstream needs a position, and the
/// two numbers plus the prototype names are the whole of what the sizing
/// takes.
struct StandingSolar {
    /// The one panel prototype found on the network, and how many stand.
    /// `None` when no panel does.
    panel: Option<(String, u32)>,
    /// The one accumulator prototype found, and how many stand.
    accumulator: Option<(String, u32)>,
    /// Set when more than one prototype of either kind stands on the network,
    /// naming which kind. See [`solar_supply_kw`] for why that refuses.
    mixed: Option<String>,
}

/// Every prototype in this world whose `entity_type` is `entity_type`, sorted.
///
/// Sorted because the planner is pure and `DashMap`'s iteration order is not
/// stable: a plan that varied with hash order would not be reproducible, which
/// is the property every collection in this crate is ordered to keep.
fn prototype_names_of_type(state: &PlanState, entity_type: &str) -> BTreeSet<String> {
    state
        .base()
        .globals
        .entity_prototypes
        .iter()
        .filter(|prototype| prototype.entity_type == entity_type)
        .map(|prototype| prototype.key().clone())
        .collect()
}

/// The panels and accumulators wired to whatever occupies `area`.
///
/// # The network is the one the supply ledger already walked
///
/// [`PlanState::powering_entities`] returns the poles of every wire-connected
/// component whose supply area meets `area` — the *same* union-find walk
/// `PlanState::electric_supply_kw` credits generators over, not a second one.
/// A candidate is on that network exactly when one of those poles' supply
/// areas meets its footprint, which is
/// [`PlanState::pole_would_supply`], i.e. the same predicate again. So there
/// is one notion of "the same network" here and not two — the drift
/// `ElectricNetwork`'s own doc exists to prevent.
///
/// # It walks the world once per prototype, and that is why it is on the
/// refusal path
///
/// [`PlanState::entities_named`] has no centre to search around and reads the
/// whole entity tree. Vanilla has one panel prototype and one accumulator
/// prototype, so that is two walks — cheap enough for
/// [`capacity_refusal`], which runs only when a plan is already refusing, and
/// **not** cheap enough for a per-condition check. Crediting solar in the
/// supply ledger wants this classification done over the network's own entity
/// list instead, which `electric_supply_kw` already holds and this crate
/// cannot reach; see the module note.
fn standing_solar(state: &PlanState, area: &Rect) -> StandingSolar {
    let carriers = state.powering_entities(area);
    let on_network = |name: &str| -> u32 {
        state
            .entities_named(name)
            .iter()
            .filter(|entity| {
                let footprint = state.footprint_of(entity);
                carriers
                    .iter()
                    .any(|(pos, pole)| state.pole_would_supply(pole, pos, &footprint))
            })
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    };
    let found = |entity_type: &str| -> (Option<(String, u32)>, bool) {
        let standing: Vec<(String, u32)> = prototype_names_of_type(state, entity_type)
            .into_iter()
            .map(|name| {
                let count = on_network(&name);
                (name, count)
            })
            .filter(|(_, count)| *count > 0)
            .collect();
        let mixed = standing.len() > 1;
        (standing.into_iter().next(), mixed)
    };
    let (panel, panels_mixed) = found(PANEL_TYPE);
    let (accumulator, bank_mixed) = found(ACCUMULATOR_TYPE);
    StandingSolar {
        panel,
        accumulator,
        mixed: if panels_mixed {
            Some("more than one kind of solar panel stands on it".into())
        } else if bank_mixed {
            Some("more than one kind of accumulator stands on it".into())
        } else {
            None
        },
    }
}

/// How many accumulators of `accumulator` an array of `panels` panels of
/// `panel` needs to carry its own average load through the night.
///
/// # The unit trap this function is built around
///
/// An accumulator answers two questions that both sound like "how much power
/// does it hold", and they are integrals of different things:
///
/// | field | value on vanilla | what it is |
/// |---|---|---|
/// | `max_energy_production` | 300 kW | the **discharge rate**, a ceiling on delivery |
/// | `electric_buffer_capacity` | 5 MJ | the **store**, joules it can hold |
///
/// **This sizes the store, and nothing here reads the rate.** A bank sized on
/// 300 kW instead of 5 MJ is wrong by a factor that depends on how long the
/// night is — the same conflation that read the vanilla 25:21 ratio as an
/// output average one level up, and the reason
/// `a_bank_is_sized_on_stored_energy_and_not_on_discharge_rate` exists.
///
/// The whole derivation is
/// [`PlanState::accumulators_per_panel`](crate::state::PlanState::accumulators_per_panel):
/// the night's shortfall against a flat average load, integrated exactly over
/// the surface's own four day-phase boundaries, times the panel's joules per
/// tick and the surface's `ticks_per_day`, divided by the accumulator's
/// buffer. Nothing in this file multiplies or scales it, so there is one copy
/// of that arithmetic and this is not it.
///
/// # What it does not answer
///
/// **Whether the bank can deliver fast enough.** A bank with enough joules can
/// still be short of the 300 kW per unit the load wants at 03:00. That is the
/// rate question, it is deliberately not modelled, and sizing a general
/// storage model to answer it is out of scope — nothing needs it yet. Named
/// here so a reader does not mistake this for it.
///
/// Ceiling division: two thirds of an accumulator is an accumulator that is
/// not standing.
///
/// # Errors
///
/// [`PlannerError::SolarBankNotSizable`] when this world cannot answer — no
/// daylight curve on the surface, no day/night endpoints on `panel`, or no
/// buffer on `accumulator`. **Unknown, never zero**: a bank of zero would
/// credit the array in full, which is the midnight failure with an extra step.
pub fn solar_bank_for(
    state: &PlanState,
    panel: &str,
    accumulator: &str,
    panels: u32,
) -> Result<u32, PlannerError> {
    if panels == 0 {
        return Ok(0);
    }
    let per_panel = state
        .accumulators_per_panel(panel, accumulator)
        .ok_or_else(|| PlannerError::SolarBankNotSizable {
            panels,
            because: format!(
                "this surface reports no daylight curve, or `{panel}` carries no day and night \
                 performance figures, or `{accumulator}` carries no buffer capacity"
            ),
        })?;
    // `to_u32` saturating at the maximum rather than wrapping: a bank that
    // large is refused by the count comparison either way, and a wrapped
    // count would refuse for a number nobody can read.
    Ok((f64::from(panels) * per_panel)
        .ceil()
        .to_u32()
        .unwrap_or(u32::MAX))
}

/// What a solar array standing on the network reaching `area` may be credited,
/// in kW — **at the daily average, and only once its bank stands.**
///
/// The owner's ruling, as one function: a solar supply is worth its average
/// rather than its noon nameplate, and it is worth that only when the
/// accumulators to carry the night are there.
///
/// # The three answers
///
/// * `Ok(0.)` — no solar panel is on this network. Nothing to credit and
///   nothing wrong, which is every plan this project has ever made.
/// * `Ok(kw)` — panels stand, the bank stands, and `kw` is the array's daily
///   average. **Nothing credits this yet**; see "the last wire" below.
/// * `Err` — panels stand and the plan may not count them, by name.
///
/// # An accumulator without a panel is a buffer, not a supply
///
/// It answers `Ok(0.)` and refuses nothing. The brief this was built from
/// asked the refusal to name "which of the two is missing — panels or bank",
/// and one half of that is deliberately not a refusal: an accumulator on a
/// steam network is an ordinary thing to build for peak shaving, and refusing
/// a working base for owning one would be exactly the false refusal this arm
/// exists to remove. The refusal names the bank because the bank is the half
/// whose absence is silent.
///
/// # Why a mixed array refuses instead of guessing
///
/// [`solar_bank_for`] sizes one panel prototype against one accumulator
/// prototype. Two kinds of either on one network is a ratio this arm cannot
/// state, and the honest answer to a question with two answers is neither of
/// them. Vanilla has one of each, so this is a refusal nothing reaches today
/// and a guess nobody has to audit later.
///
/// # The last wire, which is not in this crate's reach
///
/// A `Condition::Powered` is decided by
/// `PlanState::electric_supply_kw`, which credits solar nothing — so an array
/// this function would credit is still invisible to every feasibility check.
/// Closing that is one call in `crates/planner/src/state.rs`, and it is left
/// as a handover rather than taken: crediting it *here* while the condition
/// disagrees would make [`supply_for`] adopt a network the scheduler then
/// refuses, which is a worse failure than the one it fixes.
///
/// What this does reach is [`capacity_refusal`], where an uncredited array is
/// the difference between "no pole run carries power here" and "your solar
/// farm has no batteries". That refusal is visible at plan time, which is the
/// whole argument: a base that dies at 03:00 is *coverage is not capacity with
/// a clock attached*, and it reads in a run log as a stall nothing explains.
///
/// # Errors
///
/// [`PlannerError::SolarBankShort`] when the bank is too small or absent, and
/// [`PlannerError::SolarBankNotSizable`] when this world cannot size one.
pub fn solar_supply_kw(state: &PlanState, area: &Rect) -> Result<f64, PlannerError> {
    let standing = standing_solar(state, area);
    let Some((panel, panels)) = standing.panel else {
        return Ok(0.);
    };
    if let Some(because) = standing.mixed {
        return Err(PlannerError::SolarBankNotSizable { panels, because });
    }
    let average_kw = state
        .solar_average_kw(&panel)
        .map(|each| each * f64::from(panels))
        .ok_or_else(|| PlannerError::SolarBankNotSizable {
            panels,
            because: format!(
                "this surface reports no daylight curve, or `{panel}` carries no day and night \
                 performance figures, so a panel's daily average cannot be derived"
            ),
        })?;
    // With no accumulator standing there is still a bank to size, and sizing
    // it needs an accumulator prototype to size against. The world's own,
    // lexicographically first for determinism -- and the count it is compared
    // against is zero, so which one it is changes the refusal's wording and
    // never its verdict.
    let (accumulator, accumulators_standing) = standing.accumulator.unwrap_or_else(|| {
        (
            prototype_names_of_type(state, ACCUMULATOR_TYPE)
                .into_iter()
                .next()
                .unwrap_or_else(|| ACCUMULATOR_TYPE.to_string()),
            0,
        )
    });
    let accumulators_needed = solar_bank_for(state, &panel, &accumulator, panels)?;
    if accumulators_standing < accumulators_needed {
        return Err(PlannerError::SolarBankShort {
            panels,
            average_kw,
            accumulators_needed,
            accumulators_standing,
        });
    }
    Ok(average_kw)
}

/// Which of the two failures a false headroom condition is, or `None` when it
/// is the routing one.
///
/// **`supply_kw` above zero is the discriminator**, and it is the honest one:
/// it says generation is wired to the ground the consumer stands on, which is
/// precisely what a pole run exists to achieve. If it arrived and the sum is
/// still short, the poles did their job and the plant did not — a fact about
/// capacity, not about geometry, and one no amount of re-routing will change.
/// At zero, nothing reached the site: the run failed to carry power, whatever
/// this module's [`POLE_STEP`] arithmetic believed, and that is
/// [`ensure_powered`]'s `Ok(None)`.
///
/// The numbers come from [`Condition::headroom_parts`], i.e. from the same
/// ledger the decision was made by, so the message cannot quote a figure the
/// refusal was not decided on.
///
/// # A third failure hides inside the zero, and it is a solar one
///
/// `supply_kw = 0` means *nothing the ledger credits* reached the site, which
/// is not the same as nothing being there. A solar array standing on this very
/// network reads as zero, because
/// `crate::state::PlanState::electric_supply_kw` credits solar nothing — so a
/// base with panels and no accumulators is reported as a **pole-routing**
/// failure, and a reader sent to look at geometry finds geometry that is
/// perfect.
///
/// [`solar_supply_kw`] is asked before that verdict is returned, and its
/// refusal replaces it when it has one. Only its refusal: an array whose bank
/// *is* standing answers `Ok`, and this still falls through to the routing
/// answer, because crediting it is the one call this crate cannot make. The
/// message is the change, not the plan — nothing here makes a refusal into an
/// acceptance.
fn capacity_refusal(state: &PlanState, powered: &Condition) -> Option<PlannerError> {
    let parts = powered.headroom_parts(state)?;
    if parts.supply_kw.total_cmp(&0.).is_le() {
        let area = state.collision_area(&parts.entity, &parts.pos)?;
        return solar_supply_kw(state, &area).err();
    }
    Some(PlannerError::PowerHeadroomShort {
        entity: parts.entity.clone(),
        site: parts.pos.to_string(),
        needed_kw: parts.needed_kw,
        supply_kw: parts.supply_kw,
        committed_kw: parts.committed_kw,
        headroom_kw: parts.headroom_kw(),
    })
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
                .globals
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
            let proto = s
                .base()
                .globals
                .entity_prototypes
                .get(name)
                .expect("prototype");
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
            let parts = layout(&Position::new(0.5, 0.5), facing, 1, 1).expect("a cardinal layout");
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
            // Every size this planner will lay out, not just the one-boiler
            // plant: the chain is a new axis and a quarter turn has to carry
            // it too. Before 2026-09-06 the only shape was a row.
            for size in every_size() {
                for part in layout(
                    &Position::new(10.5, 10.5),
                    facing,
                    size.boilers,
                    size.engines,
                )
                .expect("cardinal")
                {
                    assert!(
                        on_its_grid(&s, &part),
                        "{facing:?}, {size:?}: {} at {} is off its build grid",
                        part.name,
                        part.position
                    );
                }
            }
        }
    }

    /// Every plant shape [`plant_size_for`] can produce, from one boiler to
    /// the water's twenty, including the odd-engine chains.
    ///
    /// A test that only ever built the one-boiler plant is a test of the code
    /// that existed before the chain did.
    fn every_size() -> Vec<PlantSize> {
        (1..=BOILERS_PER_PUMP * MAX_ENGINES_PER_BOILER)
            .map(|engines| PlantSize {
                boilers: engines.div_ceil(MAX_ENGINES_PER_BOILER),
                engines,
            })
            .collect()
    }

    #[test]
    fn the_plants_own_buildings_never_overlap_each_other() {
        // `fit` checks each building against the world and not against its
        // siblings, which is only sound because of this.
        let s = state();
        for facing in Direction::orthogonal() {
            // **Every size, because the chain is where overlap gets hard.**
            // Three-tile-wide engines against a three-tile boiler pitch touch
            // and must not cross; a one-boiler plant could never have shown
            // that.
            for size in every_size() {
                let parts = layout(
                    &Position::new(10.5, 10.5),
                    facing,
                    size.boilers,
                    size.engines,
                )
                .expect("cardinal");
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
                            "{facing:?}, {size:?}: {} at {} overlaps {} at {}",
                            a.name, a.position, b.name, b.position
                        );
                    }
                }
            }
        }
    }

    /// The chain is a chain: consecutive boilers stand [`BOILER_PITCH_TILES`]
    /// apart along one axis, and that axis is the shore rather than inland.
    ///
    /// "Along the shore" is asserted as *perpendicular to the pump's facing*,
    /// which is what `shoreline_faces_water` means by facing: a chain that
    /// grew inland would walk away from the water it is drinking, and a chain
    /// that grew seaward would be refused by `fit` on every map rather than
    /// caught here.
    #[test]
    fn consecutive_boilers_stand_a_boiler_apart_along_the_shore() {
        for facing in Direction::orthogonal() {
            let parts = layout(&Position::new(10.5, 10.5), facing, 4, 8).expect("cardinal");
            let boilers: Vec<Position> = parts
                .iter()
                .filter(|part| part.name == BOILER)
                .map(|part| part.position.clone())
                .collect();
            assert_eq!(boilers.len(), 4, "{facing:?}");
            let inland = turned((0., 1.), facing).expect("cardinal");
            for pair in boilers.windows(2) {
                let step = subtract(&pair[1], &pair[0]);
                assert!(
                    (calculate_distance(&pair[0], &pair[1]) - BOILER_PITCH_TILES).abs() < 1e-9,
                    "{facing:?}: boilers are {} apart, wanted {BOILER_PITCH_TILES}",
                    calculate_distance(&pair[0], &pair[1])
                );
                // Perpendicular to `inland`: a zero dot product is "along the
                // shore" without this test having to name which way is left.
                assert!(
                    (step.x() * inland.x() + step.y() * inland.y()).abs() < 1e-9,
                    "{facing:?}: the chain steps {step}, which is not along the shore"
                );
            }
        }
    }

    /// One at the pump and two per boiler -- and [`PIPE_COUNT`] is the
    /// one-boiler case of it.
    ///
    /// Read off [`layout`] rather than off [`pipe_count`]'s own arithmetic,
    /// which would be the function agreeing with itself.
    #[test]
    fn the_pipe_bill_is_one_plus_two_per_boiler() {
        for facing in Direction::orthogonal() {
            for size in every_size() {
                let parts = layout(
                    &Position::new(10.5, 10.5),
                    facing,
                    size.boilers,
                    size.engines,
                )
                .expect("cardinal");
                let pipes = parts.iter().filter(|part| part.name == PIPE).count() as u32;
                assert_eq!(
                    pipes,
                    pipe_count(size.boilers),
                    "{facing:?}, {size:?}: the layout lays {pipes} pipes"
                );
            }
        }
        assert_eq!(pipe_count(1), PIPE_COUNT);
    }

    /// No engine is left off the end of the chain, and no boiler is left
    /// without one.
    ///
    /// The two failures this rules out are opposite and both silent: an engine
    /// the split never places is capacity the plan counted and never built,
    /// and a boiler with no engine is coal burnt into nothing.
    #[test]
    fn an_engine_is_never_stranded_off_the_end_of_the_boiler_chain() {
        for size in every_size() {
            let split = size.engine_split();
            assert_eq!(split.len() as u32, size.boilers, "{size:?}");
            assert_eq!(split.iter().sum::<u32>(), size.engines, "{size:?}");
            assert!(
                split
                    .iter()
                    .all(|row| *row >= 1 && *row <= MAX_ENGINES_PER_BOILER),
                "{size:?}: {split:?} has an empty or over-full boiler"
            );
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
                if let Some(plant) = fit(
                    s,
                    &pump,
                    facing,
                    PlantSize {
                        boilers: 1,
                        engines: 1,
                    },
                ) {
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
                fit(
                    &taken,
                    &pump,
                    facing,
                    PlantSize {
                        boilers: 1,
                        engines: 1
                    }
                )
                .is_none(),
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
        let dry = factorio_bot_core::factorio::world::FactorioSurface::new();
        dry.update_entity_prototypes(
            world
                .globals
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
    /// Twenty boilers is what one offshore pump's 1,200 water/s carries
    /// against a boiler's 60/s, so the whole plant's ceiling is 36 MW.
    const ENGINE_KW_LITERAL: f64 = 900.;
    const ONE_BOILER_KW_LITERAL: f64 = 1800.;
    const WHOLE_PLANT_KW_LITERAL: f64 = 36_000.;

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
        // **Past one boiler's worth, a second boiler rather than a refusal.**
        // This is the line that moved on 2026-09-06: 1,800.5 kW used to be
        // `PowerPlantTooSmall`, stating a limit that was the layout's.
        assert_eq!(engines_for(&s, 1800.5).expect("a hair over one boiler"), 3);
        let three = plant_size_for(&s, 1800.5).expect("a hair over one boiler");
        assert_eq!(
            (three.boilers, three.engines),
            (2, 3),
            "three engines is two boilers, one of them driving a single engine"
        );
        // The two named demands from the task that put this change on the
        // roadmap, sized rather than refused.
        let miner_line = plant_size_for(&s, 1_170.).expect("MinerLine's draw");
        assert_eq!((miner_line.boilers, miner_line.engines), (1, 2));
        let electric_furnaces = plant_size_for(&s, 4_320.).expect("24 electric furnaces");
        assert_eq!(
            (electric_furnaces.boilers, electric_furnaces.engines),
            (3, 5)
        );
        // The ceiling: twenty boilers of two engines, and it is the WATER's.
        let full = plant_size_for(&s, WHOLE_PLANT_KW_LITERAL).expect("exactly the pump's water");
        assert_eq!(
            (full.boilers, full.engines),
            (BOILERS_PER_PUMP, BOILERS_PER_PUMP * MAX_ENGINES_PER_BOILER),
            "36 MW is twenty boilers driving forty engines"
        );
        // Past it, a named refusal rather than a chain no water reaches.
        let err = plant_size_for(&s, WHOLE_PLANT_KW_LITERAL + 0.5)
            .expect_err("more water than one offshore pump moves");
        match err {
            PlannerError::PowerPlantTooSmall {
                needed_kw,
                plant_kw,
            } => {
                assert_eq!(needed_kw, WHOLE_PLANT_KW_LITERAL + 0.5);
                assert_eq!(
                    plant_kw, WHOLE_PLANT_KW_LITERAL,
                    "the ceiling reported is the pump's water, not one boiler's {ONE_BOILER_KW_LITERAL} kW"
                );
            }
            other => panic!("expected PowerPlantTooSmall, got {other:?}"),
        }
    }

    #[test]
    fn a_second_engine_stands_five_tiles_on_and_costs_no_extra_pipe() {
        let s = state();
        for facing in Direction::orthogonal() {
            let one = layout(&Position::new(10.5, 10.5), facing, 1, 1).expect("cardinal");
            let two = layout(&Position::new(10.5, 10.5), facing, 1, 2).expect("cardinal");

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
            let parts = layout(&Position::new(10.5, 10.5), facing, 1, 2).expect("cardinal");
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

        // `pole_chain` over the one boiler's row: the same question
        // `pole_site` used to be asked, through the function that replaced it.
        let mut trial = world.fork();
        let engines: Vec<PlantPart> = plant
            .engines
            .iter()
            .map(|position| PlantPart {
                name: ENGINE,
                position: position.clone(),
                direction: Direction::North,
            })
            .collect();
        let groups = vec![engines.iter().collect::<Vec<&PlantPart>>()];
        let _ = &areas;
        let _ = &origin;
        match pole_chain(&mut trial, &groups, false) {
            None => {}
            Some(chosen) => panic!(
                "pole_chain chose {:?}, which cannot reach both engines: \
                 every tile that could was taken",
                chosen
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

    /// **Every boiler of the chain is fuelled, and by an emitted step.**
    ///
    /// The correctness bug this whole change had to avoid: `plant_steps`
    /// emitted exactly one `Insert` while `Plant` carried exactly one boiler,
    /// and the moment the chain grew, three of four boilers would have stood
    /// cold while `Condition::Powered` credited the plan with the full
    /// nameplate. Everything places, everything is wired, and the network
    /// delivers a quarter of what the plan counted.
    ///
    /// **Asserted on the emitted steps, never on `ctx.state`.** `plant_steps`
    /// writes its parts into the planning overlay as well as emitting them, so
    /// a version that built the plant into the overlay and dropped the steps
    /// still leaves the site reading as powered -- the exact trap
    /// `docs/superpowers/notes/2026-09-06-one-place-that-decides-power.md`
    /// records catching a test in. The overlay is not the plan.
    #[test]
    fn every_boiler_in_the_chain_is_fuelled_and_the_bill_pays_for_it() {
        for kw in [60., 1000., 1800.5, 4_320., 36_000.] {
            let (s, plant) = plant_for(kw);
            let size = plant_size_for(&s, kw).expect("sized");
            assert_eq!(
                plant.boilers.len() as u32,
                size.boilers,
                "{kw} kW should site {} boilers",
                size.boilers
            );
            let mut ctx = ExpansionCtx::new(s, BotId(1));
            let (steps, _) = plant_steps(&mut ctx, &plant);

            // One `Insert` of coal per boiler, at that boiler's own tile.
            let mut fuelled: Vec<(Position, u32)> = Vec::new();
            for step in &steps {
                if let Step::Act(action) = step
                    && let ActionKind::Insert {
                        pos,
                        entity,
                        slot,
                        item,
                        count,
                    } = &action.kind
                    && entity == BOILER
                    && *slot == InventorySlot::Fuel
                    && item == "coal"
                {
                    fuelled.push((pos.clone(), *count));
                }
            }
            assert_eq!(
                fuelled.len(),
                plant.boilers.len(),
                "{kw} kW: {} boilers stand and {} are fuelled",
                plant.boilers.len(),
                fuelled.len()
            );
            for boiler in &plant.boilers {
                assert!(
                    fuelled.iter().any(|(pos, _)| pos == boiler),
                    "{kw} kW: the boiler at {boiler} is never fuelled"
                );
            }

            // And the bill pays for exactly what the inserts hand over -- a
            // bill short of the inserts stalls the plan at the last boiler,
            // and a bill over them is coal carried for nothing. Before
            // 2026-09-06 a two-engine plant billed ten coal and inserted five.
            let billed = bill(&plant)
                .into_iter()
                .find(|(name, _)| *name == "coal")
                .expect("a plant bills coal")
                .1;
            assert_eq!(
                billed,
                fuelled.iter().map(|(_, count)| count).sum::<u32>(),
                "{kw} kW: the coal bill and the coal inserts disagree"
            );
            // Per engine, so the charge follows the steam.
            assert_eq!(
                billed,
                PLANT_COAL * size.engines,
                "{kw} kW: {PLANT_COAL} coal an engine over {} engines",
                size.engines
            );
        }
    }

    /// **Every engine of a grown plant is inside some pole's supply area, and
    /// the poles are one network.**
    ///
    /// The failure this rules out places 100 % and does nothing: an engine
    /// nobody's pole reaches is 900 kW standing on the ground while
    /// `Condition::Powered` credits the plan with it, and a pole chain that
    /// breaks in the middle is a plant in two halves, one of which generates
    /// into a network no consumer draws from. Both are silent -- placement and
    /// function are separate concerns.
    ///
    /// It is asserted over the full 36 MW plant, twenty boilers and forty
    /// engines, because that is where a single pole stopped being enough. At
    /// one boiler this is the claim `pole_site` always made.
    ///
    /// # What it does NOT discriminate, stated rather than implied
    ///
    /// Deleting [`pole_chain`]'s `linked` predicate entirely leaves this test
    /// **green**. On open ground each row's ring search lands within a few
    /// tiles of the last one anyway, so the wire-reach constraint is slack and
    /// nothing here forces it to bind. Moving the emitted poles twenty tiles
    /// apart does fail this test, so the assertion is not vacuous -- but it
    /// pins the *outcome*, not the predicate, and the predicate is only load
    /// bearing on ground this fixture does not have. Said out loud because a
    /// falsification that passes is the finding, not the footnote; see
    /// `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`.
    #[test]
    fn every_engine_of_a_grown_plant_is_covered_and_the_poles_are_one_network() {
        for kw in [60., 1000., 4_320., 36_000.] {
            let (s, plant) = plant_for(kw);
            let built = with_parts(&s, &plant);
            let poles: Vec<Position> = plant
                .parts
                .iter()
                .chain(plant.standing.iter())
                .filter(|part| part.name == POLE)
                .map(|part| part.position.clone())
                .collect();
            assert!(!poles.is_empty(), "{kw} kW: a plant with no pole");

            for engine in &plant.engines {
                let area = built
                    .collision_area(ENGINE, engine)
                    .expect("the fixture prices a steam engine");
                assert!(
                    poles
                        .iter()
                        .any(|pole| built.pole_would_supply(POLE, pole, &area)),
                    "{kw} kW: the engine at {engine} is outside every pole's supply area"
                );
            }

            // One network: every pole reachable from the first by hops of at
            // most `WIRE_REACH`. A plain "consecutive poles are close" check
            // would pass a chain that `pole_chain` happened to emit in a
            // convenient order; this asks the connectivity question itself.
            let mut joined = vec![poles[0].clone()];
            loop {
                let grown: Vec<Position> = poles
                    .iter()
                    .filter(|pole| !joined.contains(pole))
                    .filter(|pole| {
                        joined
                            .iter()
                            .any(|had| calculate_distance(had, pole).total_cmp(&WIRE_REACH).is_le())
                    })
                    .cloned()
                    .collect();
                if grown.is_empty() {
                    break;
                }
                joined.extend(grown);
            }
            assert_eq!(
                joined.len(),
                poles.len(),
                "{kw} kW: {} of {} poles are on the first one's network -- the \
                 plant is wired in more than one piece",
                joined.len(),
                poles.len()
            );
        }
    }

    #[test]
    fn the_tier_that_builds_refuses_a_demand_no_plant_carries() {
        // The defect roadmap item 3 names: `supply_for`'s fourth tier used to
        // ignore `kw` entirely and hand back a 900 kW plant for any demand.
        let s = state();
        let err = supply_for(&s, &Position::new(0., 0.), 64., 40_000.)
            .expect_err("40,000 kW is more water than one offshore pump moves");
        assert!(
            matches!(err, PlannerError::PowerPlantTooSmall { .. }),
            "expected PowerPlantTooSmall, got {err:?}"
        );
        // Named, not silent: the message carries both numbers.
        let text = err.to_string();
        assert!(
            text.contains("40000") && text.contains("36000"),
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
    /// 128 -- the same shape as a roster that walked away from spawn and can
    /// no longer see the lake the origin sees at 48 tiles.
    ///
    /// This used to cite the bots parked at (255, 249) on
    /// `map-31337-explored.json` as "355 tiles from water". Re-measured
    /// 2026-09-06 (`score-map --from 255,249`), that dump reports **47.4**,
    /// so the citation is dropped rather than repeated; see
    /// [`plant_world_anchor`]. The fixture is what proves the property, and
    /// it is unaffected.
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
        let dry = factorio_bot_core::factorio::world::FactorioSurface::new();
        dry.update_entity_prototypes(
            world
                .globals
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

    // -----------------------------------------------------------------------
    // A water refusal says where it stood, and how much of that disc it saw
    //
    // `nearest_water_tile` answers `None` for two states that are not the
    // same thing: ground the mod wrote out that holds no water, and ground
    // that was never generated and holds no tiles at all. Both used to reach
    // the caller as the identical sentence.
    // -----------------------------------------------------------------------

    /// A dry world whose ground around `at` **has** been written out, to a
    /// tile either side of the wide scan.
    ///
    /// The seventeen probes sit at `at`, and on the eight compass points at
    /// half and at the full 128, so a square of half-width 129 covers every
    /// one of them with a tile to spare. Grass, not water: the point of this
    /// world is that the model has looked and there is nothing there.
    ///
    /// Built here rather than in `test_utils` because it is only this
    /// question that needs it, and because `update_chunk_tiles` is additive
    /// -- a world with a lake cannot have one removed.
    fn a_charted_but_dry_world(at: &Position, half_width: i32) -> PlanState {
        let world = factorio_bot_core::factorio::world::FactorioSurface::new();
        world
            .update_entity_prototypes(
                fixture_world()
                    .globals
                    .entity_prototypes
                    .iter()
                    .map(|e| e.value().clone())
                    .collect(),
            )
            .expect("prototypes");
        let (cx, cy) = (at.x().floor() as i32, at.y().floor() as i32);
        let mut tiles = Vec::new();
        for x in (cx - half_width)..=(cx + half_width) {
            for y in (cy - half_width)..=(cy + half_width) {
                tiles.push(factorio_bot_core::types::FactorioTile {
                    position: Position::new(f64::from(x), f64::from(y)),
                    name: "grass-1".to_owned(),
                    player_collidable: false,
                    color: None,
                    surface: None,
                });
            }
        }
        world.update_chunk_tiles(tiles).expect("charted ground");
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    /// Charted and dry: a statement about the **map**.
    ///
    /// Every probe found ground, so the refusal is not a gap in the dump and
    /// no amount of walking will change it. 17 is `CHARTING_PROBES` (the
    /// origin plus eight compass points at two radii), written out rather
    /// than imported on this crate's own fixture rule.
    #[test]
    fn a_water_refusal_over_charted_ground_says_the_ground_was_dry() {
        let at = Position::new(0., 0.);
        let s = a_charted_but_dry_world(&at, 129);
        let err = plan_plant(&s, &at).expect_err("grass is not water");
        match err {
            PlannerError::PowerPlantNeedsWater {
                covered_probes,
                probes,
                ..
            } => {
                assert_eq!(
                    (covered_probes, probes),
                    (17, 17),
                    "the whole 128-tile disc was written out, so the refusal is about the map"
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    /// Blind: a statement about the **dump**.
    ///
    /// The fixture's only tiles are its lake, so 400 tiles north nothing has
    /// ever been written out -- and this is the state that reads as "no
    /// water" while saying nothing at all about whether there is water. The
    /// anchor is asserted here too, because it is the caller's position and
    /// not [`plant_world_anchor`], and a reader chasing a lake needs to know
    /// which search refused.
    #[test]
    fn a_water_refusal_over_ungenerated_ground_says_it_was_blind() {
        let world = factorio_bot_core::factorio::world::FactorioSurface::new();
        world
            .update_entity_prototypes(
                fixture_world()
                    .globals
                    .entity_prototypes
                    .iter()
                    .map(|e| e.value().clone())
                    .collect(),
            )
            .expect("prototypes");
        let s = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        let away = a_bot_that_walked_away();
        let err = plan_plant(&s, &away).expect_err("no tiles at all is no water");
        match err {
            PlannerError::PowerPlantNeedsWater {
                anchor_x,
                anchor_y,
                covered_probes,
                probes,
                ..
            } => {
                assert_eq!(
                    (anchor_x, anchor_y),
                    (away.x(), away.y()),
                    "the refusal must name the position the search stood at"
                );
                assert_eq!(
                    (covered_probes, probes),
                    (0, 17),
                    "no chunk here was ever written out, so this refusal is about the dump"
                );
            }
            other => panic!("got {other:?}"),
        }
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
        let err = supply_for(&s, &a_bot_that_walked_away(), 64., 40_000.)
            .expect_err("40,000 kW is more water than one offshore pump moves");
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
                .globals
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
                .globals
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

    /// **There is no crossover of any kind, and 56 tiles is not special.**
    ///
    /// This replaces `the_crossover_is_wood_rather_than_price` (doclint-allow:
    /// deliberately names the deleted test, which is the point of the
    /// sentence), which typed
    /// `const STARTING_WOOD: u32 = 4` into itself and asserted arithmetic
    /// about `pole_run_items` against that literal. Registering ten `Chop`s
    /// would not have moved it: it was a fixture satisfying its own assertion
    /// by construction, the fourth shape in
    /// `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`.
    ///
    /// What is asserted instead is the property that kills the claim rather
    /// than a number that restates it: **both routes are linear in distance,
    /// so no distance can reverse their ordering.** Doubling the span doubles
    /// each bill, at absolute values written out here rather than computed
    /// from the constants under test, and the 56/57-tile boundary the
    /// retracted claim named is shown to be an ordinary pair of points.
    #[test]
    fn no_distance_turns_the_pole_route_into_the_dearer_one() {
        // Absolute bills, typed. A pole run of N tiles is ceil(N/7) poles off
        // ceil(poles/2) crafts, one wood and one copper plate each; a pipe run
        // is ceil(N) plates.
        assert_eq!(
            pole_run_items(56.),
            (8, 4, 4),
            "56 tiles: 8 poles, 4 crafts"
        );
        assert_eq!(pipe_run_plates(56.), 56);
        assert_eq!(
            pole_run_items(57.),
            (9, 5, 5),
            "57 tiles: 9 poles, 5 crafts"
        );
        assert_eq!(pipe_run_plates(57.), 57);
        assert_eq!(pole_run_items(700.), (100, 50, 50));
        assert_eq!(pipe_run_plates(700.), 700);

        // Linearity, which is what forbids a crossover: a route with no fixed
        // cost cannot overtake another that also has none. Doubling the span
        // doubles both bills, so their ratio is the same everywhere.
        for tiles in [14., 56., 112., 350.] {
            let (_, wood, copper) = pole_run_items(tiles);
            let (_, wood2, copper2) = pole_run_items(tiles * 2.);
            assert_eq!(
                (wood2, copper2),
                (wood * 2, copper * 2),
                "{tiles} tiles doubled must double the pole bill"
            );
            assert_eq!(
                pipe_run_plates(tiles * 2.),
                pipe_run_plates(tiles) * 2,
                "{tiles} tiles doubled must double the pipe bill"
            );
        }

        // And the ordering the linearity preserves, across two orders of
        // magnitude including the retracted boundary. A factor of three is the
        // margin the *shortest* run holds (one pole, two items, against seven
        // plates); rounding is what costs it, and it only improves from there.
        for tiles in [7., 20., 56., 57., 100., 355., 700.] {
            let plates = f64::from(pipe_run_plates(tiles));
            let (poles, wood, copper) = pole_run_items(tiles);
            let pole_items = f64::from(wood + copper);
            assert!(
                pole_items * 3. < plates,
                "{tiles} tiles: {poles} poles cost {pole_items} items against \
                 {plates} plates of pipe -- poles are cheaper by a wide margin \
                 at every distance, not just short ones"
            );
        }

        // Where rounding is noise, the asymptotic factor the module doc claims:
        // 1/14 wood + 1/14 copper a tile against a plate a tile, so seven.
        let (_, wood, copper) = pole_run_items(700.);
        let ratio = f64::from(pipe_run_plates(700.)) / f64::from(wood + copper);
        assert!(
            ratio > 6.9 && ratio < 7.1,
            "the module doc's factor of seven, measured over 700 tiles: {ratio}"
        );
    }

    /// **The planner can obtain the wood a 355-tile pole run wants**, and the
    /// only thing it needs is a standing tree.
    ///
    /// This is the assertion the retracted supply claim never had: it asks the
    /// **registry** whether `have 26 wood` expands, so any future change that
    /// really did make wood unobtainable turns it red and points at the
    /// sentence. Nothing here is computed from a constant in `power.rs`; 26 is
    /// the wood half of `pole_run_items(355.)`, typed.
    ///
    /// Both directions are asserted, because the residual is real and worth
    /// pinning: **a chop needs a charted standing minable**. The shared
    /// fixture's hundred trees are `tree-42`, a name the prototype fixture has
    /// no entry for, so they yield nothing and the treeless half refuses --
    /// which is also what makes the wooded half evidence about `Chop` rather
    /// than about the fixture's forest.
    #[test]
    fn the_planner_can_obtain_the_wood_a_long_pole_run_wants() {
        use crate::goal::{Goal, Holder};
        use crate::method::expand;
        use crate::method::have::registry_for;

        // 26 wood: what pole_run_items(355.) asks for, written out.
        let goal = Goal::Have {
            item: "wood".into(),
            count: 26,
            whose: Holder::Anyone,
        };
        let bots = [BotId(1)];

        // tree-01 is the prototype the fixture really carries a
        // `mine_result {wood: 4}` for; seven of them cover 26.
        let trees: Vec<factorio_bot_core::types::Position> = (0..8)
            .map(|k| factorio_bot_core::types::Position::new(6. + 2. * f64::from(k), 6.))
            .collect();
        let wooded = PlanState::from_world(
            Arc::new(crate::test_world::with_trees(fixture_world(), &trees)),
            &bots,
        );
        let net = expand(
            std::slice::from_ref(&goal),
            &wooded,
            &registry_for(&bots),
            BotId(1),
        )
        .expect(
            "a standing tree is all the pole route's wood needs -- if this \
             refuses, the module doc's retraction of the four-wood cap is \
             wrong and must be re-argued",
        );
        // Seven chops, not "some": `tree-01` yields 4 wood, so 26 wants seven
        // of the eight standing and the eighth is left alone. An absolute
        // count, so a bill that silently under-delivers cannot pass.
        assert_eq!(
            net.actions().count(),
            7,
            "26 wood off trees yielding 4 is seven swings"
        );

        // The control, and the residual: no readable tree, no wood.
        let treeless = PlanState::from_world(Arc::new(fixture_world()), &bots);
        let err = expand(&[goal], &treeless, &registry_for(&bots), BotId(1))
            .expect_err("the shared fixture's tree-42s yield nothing");
        // The property this control is for is that a bare map **refuses**
        // rather than quietly planning a shorter pole run. The variant that
        // carries it changed on 2026-09-07, when `products::NoProducer` was
        // registered last in `registry_for`: `NoApplicableMethod` ("no method
        // can satisfy goal: have 26 wood") became `ProductNotMakeable`, which
        // adds that no recipe produces wood and that it therefore comes out of
        // the ground. Nothing was lost -- both name the item -- so the
        // assertion moved to the property and the name rather than staying
        // pinned to a variant this test was never about.
        let text = err.to_string();
        assert!(
            matches!(err, PlannerError::ProductNotMakeable(_)),
            "an uncharted or bare map refuses wood by name rather than \
             planning a shorter run: got {err:?}"
        );
        assert!(text.contains("wood"), "and the refusal names it: {text}");
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod block_headroom_tests {
    //! **A block is not one machine**, and the demand ledger's single-position
    //! exclusion is what made that a refusal.
    //!
    //! The live failure these reduce: `FurnaceLine` draws 624 kW across 48
    //! inserters, all of which are in the routing fork by the time the headroom
    //! check runs -- they must be, or a pole is sited on ground the block is
    //! about to take. `Condition::Powered` excludes exactly one of them, so the
    //! ledger charged the block's own draw as *existing* demand and the caller
    //! then asked for it again on top. On a 900 kW plant that is
    //! 900 - 611 = 289 < 624: refused by its own arithmetic, with room to
    //! spare.
    //!
    //! **These fixtures were written by the same task as the code, and that is
    //! the trap `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md`
    //! names.** What they assume, said out loud: that 48 `inserter`s at 13 kW
    //! standing under one substation are a fair reduction of a decoded
    //! blueprint's consumers, and that a substation is a fair stand-in for the
    //! 13 small poles `FurnaceLine` actually carries. Both are geometry
    //! shortcuts; neither changes which set the ledger charges, which is the
    //! thing under test. Each assertion below was watched to fail for its own
    //! reason before being believed.

    use super::*;
    use crate::action::Condition;
    use crate::ids::BotId;
    use crate::state::Excluded;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    /// The block's own distribution, standing in for `FurnaceLine`'s 13 small
    /// poles: one 18x18 supply area, so every consumer below is demonstrably on
    /// one network without thirteen poles' worth of fixture.
    const HUB: &str = "substation";
    const CONSUMER: &str = "inserter";
    /// `INSERTER_DUTY_KW`, restated: these tests are about the ledger, and
    /// reaching into `state.rs` for the number would make them agree with it by
    /// construction.
    const CONSUMER_KW: f64 = 13.;
    /// `FurnaceLine`'s own inserter count, so the arithmetic here is the
    /// arithmetic of the live refusal.
    const CONSUMERS: usize = 48;
    const BLOCK_KW: f64 = CONSUMER_KW * CONSUMERS as f64;
    const SUPPLY_RADIUS: f64 = 64.;

    /// Where the block stands: clear of the fixture's lake, and far enough from
    /// spawn that a plant has to be built and a pole run has to reach back.
    fn hub_position() -> Position {
        Position::new(60., 56.)
    }

    /// The block: one hub and 48 consumers under it, in a fixed 8x6 grid below
    /// the hub so nothing overlaps the hub's own 2x2 box.
    ///
    /// Positions are tile centres, which is what a one-tile entity's build grid
    /// is -- the same convention `ring_search` uses and the reason a resource
    /// entity sits at `-40.5` and never at `-41`.
    fn block(state: &PlanState) -> Vec<FactorioEntity> {
        let hub = hub_position();
        let mut out = vec![entity_for(
            state,
            &PlantPart {
                name: HUB,
                position: hub.clone(),
                direction: Direction::North,
            },
        )];
        for index in 0..CONSUMERS {
            let column = (index % 8) as f64;
            let row = (index / 8) as f64;
            out.push(entity_for(
                state,
                &PlantPart {
                    name: CONSUMER,
                    position: Position::new(hub.x() - 3.5 + column, hub.y() + 2.5 + row),
                    direction: Direction::North,
                },
            ));
        }
        out
    }

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    /// A world with a finished 900 kW plant, the block standing, and the two
    /// joined by one pole -- the state the routing fork is in when the headroom
    /// check runs.
    fn plant_and_block_standing() -> PlanState {
        let s = state();
        let hub = hub_position();
        let plant = plan_plant(&s, &Position::new(0., 0.)).expect("the fixture has a lake");
        let mut built = s.fork();
        for part in &plant.parts {
            built.create_entity(entity_for(&built, part));
        }
        for entity in block(&built) {
            built.create_entity(entity);
        }
        // The one pole that joins the plant's network to the block's hub. Sited
        // by the same `pole_run` the real path uses, so this fixture cannot
        // assume a wire the model would not draw.
        let area = built
            .collision_area(HUB, &hub)
            .expect("the substation has a prototype");
        let joined = Condition::BlockPowered {
            pos: hub.clone(),
            entity: HUB.into(),
            kw: 0.,
            own_ground: Rect::new(&Position::new(0., 0.), &Position::new(0., 0.)),
        };
        let run = pole_run(&mut built, &plant.pole, &hub, &area, &joined, BotId(1))
            .expect("no shortfall is possible here")
            .expect("open ground between the lake and the block");
        assert!(
            !run.is_empty(),
            "control: the plant and the block must be joined by a real run, or \
             every assertion below is about an unwired world"
        );
        built
    }

    /// The double count, stated as arithmetic: the same network, the same
    /// ground, two exclusions, and only one of them answers the question the
    /// caller asked.
    ///
    /// Exact figures, not inequalities. An assertion that headroom is "enough"
    /// passes for a ledger that charges nothing at all.
    #[test]
    fn a_blocks_own_consumers_are_not_charged_against_its_own_draw() {
        let built = plant_and_block_standing();
        let hub = hub_position();
        let area = built.collision_area(HUB, &hub).expect("a prototype");

        assert_eq!(
            built.electric_supply_kw(&area),
            900.,
            "one steam engine, which is what `engines_for` sizes for 624 kW"
        );
        assert_eq!(
            built.electric_demand_kw_excluding(&area, Excluded::Consumer(&hub)),
            BLOCK_KW,
            "excluding one position charges the block's whole draw, because the \
             one position excluded is the hub, which draws nothing"
        );
        assert_eq!(
            built.electric_demand_kw_excluding(
                &area,
                Excluded::Ground(&occupied_ground(&built, &block(&built)).expect("48 occupants")),
            ),
            0.,
            "excluding the block's ground charges nobody: every consumer on \
             this network is the block's own"
        );

        assert!(
            !(Condition::Powered {
                pos: hub.clone(),
                entity: HUB.into(),
                kw: BLOCK_KW,
            })
            .holds(&built, BotId(1)),
            "the defect, pinned: 900 - 624 = 276 < 624, so the per-entity \
             condition refuses a block the plant covers twice over"
        );
        assert!(
            (Condition::BlockPowered {
                pos: hub.clone(),
                entity: HUB.into(),
                kw: BLOCK_KW,
                own_ground: occupied_ground(&built, &block(&built)).expect("48 occupants"),
            })
            .holds(&built, BotId(1)),
            "and the fix: 900 - 0 >= 624"
        );
    }

    /// `ensure_powered` plans the block end to end, and the generator is in the
    /// **steps** rather than only in the overlay.
    ///
    /// **The overlay is not the plan.** `plant_steps` creates its parts in
    /// `ctx.state` as well as emitting them, so a version that built the plant
    /// into the overlay and dropped its steps would still leave the condition
    /// holding -- the sibling test above this module records the same trap. The
    /// generator is therefore asserted in the emitted placements.
    #[test]
    fn ensure_powered_plans_a_block_whose_own_draw_it_no_longer_double_counts() {
        let s = state();
        let hub = hub_position();
        let occupants = block(&s);
        let area = s.collision_area(HUB, &hub).expect("a prototype");

        let mut ctx = ExpansionCtx::new(s, BotId(1));
        let powering = ensure_powered(
            &mut ctx,
            HUB,
            &hub,
            &area,
            BLOCK_KW,
            SUPPLY_RADIUS,
            &occupants,
        )
        .expect(
            "624 kW is inside one 900 kW plant -- a refusal here is the double \
             count back",
        )
        .expect("and the fixture has open ground between the lake and the block");

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
            placed.contains(&"steam-engine"),
            "the plan must BUILD the generator, not merely leave one in the \
             overlay; placed: {placed:?}"
        );

        assert!(
            matches!(powering.powered, Condition::BlockPowered { .. }),
            "a 49-occupant call states the block condition, got {}",
            powering.powered
        );

        // The game's own rule, over the world the call leaves plus the block
        // the caller is about to place -- which is the state the caller's own
        // `Place` steps produce.
        let mut finished = ctx.state.fork();
        for occupant in &occupants {
            finished.create_entity(occupant.clone());
        }
        assert!(
            powering.powered.holds(&finished, BotId(1)),
            "the block must be POWERED once it stands, not merely covered"
        );
    }

    /// One occupant states the **per-entity** condition, unchanged.
    ///
    /// Every existing caller of `ensure_powered` sites one machine, and a
    /// condition that changed shape under them would change what a report
    /// prints, what a precondition compares equal to, and what the three
    /// offline baselines plan. The ground of one machine holds one consumer --
    /// itself -- so the two exclusions name the same set here and the choice is
    /// about the shape, not about the answer.
    #[test]
    fn a_single_occupant_still_states_the_per_entity_condition() {
        let s = state();
        let site = Position::new(60.5, 60.5);
        let area = s
            .collision_area("assembling-machine-1", &site)
            .expect("a prototype");
        let only = entity_for(
            &s,
            &PlantPart {
                name: "assembling-machine-1",
                position: site.clone(),
                direction: Direction::North,
            },
        );

        let mut ctx = ExpansionCtx::new(s, BotId(1));
        let powering = ensure_powered(
            &mut ctx,
            "assembling-machine-1",
            &site,
            &area,
            75.,
            SUPPLY_RADIUS,
            std::slice::from_ref(&only),
        )
        .expect("the fixture has a lake")
        .expect("and open ground");

        assert_eq!(
            powering.powered,
            Condition::Powered {
                pos: site,
                entity: "assembling-machine-1".into(),
                kw: 75.,
            },
            "one machine must still emit the condition it always emitted"
        );
    }

    /// `Ok(None)` said "no pole run carries it there" for a site whose poles
    /// routed perfectly and whose plant was short. The discriminator is whether
    /// any supply reached the site at all.
    ///
    /// Both directions, because one alone proves nothing: a `capacity_refusal`
    /// that always answered `Some` would pass a test that only asked for the
    /// error, and one that always answered `None` would pass a test that only
    /// asked for routing.
    #[test]
    fn a_reachable_network_that_is_too_small_refuses_by_capacity_not_by_routing() {
        let built = plant_and_block_standing();
        let hub = hub_position();

        // Reachable and short: 900 kW generated, the block's 624 kW charged as
        // somebody else's, and 900 kW asked for on top.
        let short = Condition::Powered {
            pos: hub.clone(),
            entity: HUB.into(),
            kw: 900.,
        };
        assert!(
            !short.holds(&built, BotId(1)),
            "control: this condition must be false, or the split below is about \
             nothing"
        );
        let err = capacity_refusal(&built, &short).expect("supply reached the site");
        match err {
            PlannerError::PowerHeadroomShort {
                needed_kw,
                supply_kw,
                committed_kw,
                headroom_kw,
                ..
            } => {
                assert_eq!(needed_kw, 900.);
                assert_eq!(supply_kw, 900.);
                assert_eq!(
                    committed_kw, BLOCK_KW,
                    "the figure quoted must be the one the decision was made by"
                );
                assert_eq!(headroom_kw, 900. - BLOCK_KW);
            }
            other => panic!("expected PowerHeadroomShort, got {other:?}"),
        }

        // Unreachable: the same question asked about ground no wire reaches.
        // Nothing generated there, so nothing routed there, and the refusal is
        // the caller's own.
        let far = Position::new(500.5, 500.5);
        let unreachable = Condition::Powered {
            pos: far.clone(),
            entity: "assembling-machine-1".into(),
            kw: 75.,
        };
        assert!(
            !unreachable.holds(&built, BotId(1)),
            "control: nothing is out there"
        );
        assert_eq!(
            built.electric_supply_kw(
                &built
                    .collision_area("assembling-machine-1", &far)
                    .expect("a prototype")
            ),
            0.,
            "control: no supply reaches 500,500, which is what makes it routing"
        );
        assert!(
            capacity_refusal(&built, &unreachable).is_none(),
            "no supply reached the site, so this is routing and must stay \
             `Ok(None)` for the caller to name"
        );
    }

    /// A draw past what this planner's layout can generate is refused **by
    /// name, before a pole is sited** -- the `MinerLine` shape, 1,170 kW
    /// against a plant that tops out at 1.8 MW only with two engines and at
    /// 900 kW with one boiler's worth here.
    ///
    /// Kept beside the headroom test because the two are the pair a reader has
    /// to tell apart: `PowerPlantTooSmall` is about the **layout**, true from
    /// every anchor on every map; `PowerHeadroomShort` is about **one network
    /// at one moment**.
    #[test]
    fn a_draw_past_the_layout_is_refused_by_the_layout_and_not_by_the_poles() {
        let s = state();
        let hub = hub_position();
        let occupants = block(&s);
        let area = s.collision_area(HUB, &hub).expect("a prototype");

        // **Derived from the ceiling, never written down.** This asked for a
        // literal nine megawatts, which was past every layout on the day it
        // was written and stopped being so the moment the plant grew to
        // `BOILERS_PER_PUMP x MAX_ENGINES_PER_BOILER` engines = 36 MW. The
        // test kept passing on the branch that grew the plant and on the
        // branch that wrote this assertion, and only failed once they were
        // merged -- `cargo check` cannot see it, because it compiles.
        // The engine's output comes from the fixture's own prototype, not from
        // a second literal beside the first one.
        let each_kw = s.generator_output_kw(ENGINE).expect("an engine prototype");
        let ceiling_kw = f64::from(BOILERS_PER_PUMP * MAX_ENGINES_PER_BOILER) * each_kw;
        let past_every_layout = ceiling_kw + each_kw;

        let mut ctx = ExpansionCtx::new(s, BotId(1));
        let err = match ensure_powered(
            &mut ctx,
            HUB,
            &hub,
            &area,
            past_every_layout,
            SUPPLY_RADIUS,
            &occupants,
        ) {
            Err(err) => err,
            Ok(_) => panic!(
                "{past_every_layout} kW is one engine past a {ceiling_kw} kW ceiling and must refuse"
            ),
        };
        assert!(
            matches!(err, PlannerError::PowerPlantTooSmall { .. }),
            "a demand no plant can meet must say so about the plant: got {err:?}"
        );
    }

    /// The exclusion covers its own boundary, and the direction is deliberate.
    ///
    /// `Rect::contains` is strict on all four edges. The ground here is the
    /// union of the occupants' **footprints**, so a consumer sitting exactly on
    /// an edge is one whose footprint the block already paid for, and charging
    /// it would put back a slice of the double count round the whole perimeter.
    /// Asserted through the ledger rather than by calling the private predicate,
    /// so what is pinned is the behaviour a caller sees.
    #[test]
    fn the_block_ground_excludes_a_consumer_standing_on_its_edge() {
        let built = plant_and_block_standing();
        let hub = hub_position();
        let area = built.collision_area(HUB, &hub).expect("a prototype");
        let ground = occupied_ground(&built, &block(&built)).expect("48 occupants");

        // The lowest row of consumers, at y = hub + 2.5 + 5, sits 0.1484 above
        // the ground's own lower edge. Cut the rectangle back to their exact
        // centre line: under a strict test that row falls out of the exclusion
        // and is charged.
        let edge_y = hub.y() + 2.5 + 5.;
        let trimmed = Rect::new(
            &ground.left_top,
            &Position::new(ground.right_bottom.x(), edge_y),
        );
        assert!(
            trimmed.right_bottom.y() < ground.right_bottom.y(),
            "control: the trim must actually cut the rectangle"
        );
        assert_eq!(
            built.electric_demand_kw_excluding(&area, Excluded::Ground(&trimmed)),
            0.,
            "a consumer whose centre is exactly on the edge is inside the \
             block's own ground and must not be charged"
        );
    }
}

// -------------------------------------------------------------------------
// The solar arm's tests.
//
// The vanilla results this arm reproduces -- 0.7 of nameplate averaged, 0.85
// accumulators per panel -- are pinned where they are derived, by
// `crate::state`'s own tests against the live 2.1.17 capture. Restating them
// here would be a second copy of an answer, and the rule in
// `docs/superpowers/notes/2026-09-06-fixtures-agree-with-their-code.md` cuts
// the other way for it: what these assert is what this arm does *with* those
// numbers, relationally, so a change to the derivation moves one file and not
// two.
// -------------------------------------------------------------------------

#[cfg(test)]
mod solar_tests {
    use super::*;
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{SurfaceDaylight, SurfaceId};
    use std::sync::Arc;

    /// Vanilla Nauvis' daylight curve, as `LuaSurface` reports it in 2.1.17.
    /// The capture is `crates/core/tests/live-2.1.17-daylight.json`.
    fn nauvis_daylight() -> SurfaceDaylight {
        SurfaceDaylight {
            surface: Some(SurfaceId::nauvis()),
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

    /// A panel's NOON output, in joules per tick: the 60 kW on a vanilla
    /// panel's tooltip, which is exactly the figure this arm must not credit.
    const PANEL_NOON_JOULES_PER_TICK: f64 = 1000.;
    /// A vanilla accumulator's STORE, in joules. 5 MJ.
    const ACCUMULATOR_BUFFER_JOULES: f64 = 5_000_000.;
    /// A vanilla accumulator's DISCHARGE RATE, in joules per tick: 300 kW.
    /// Present in the fixture precisely so a test can prove nothing reads it.
    const ACCUMULATOR_DISCHARGE_JOULES_PER_TICK: f64 = 5000.;

    /// Where the arm's tests put things. The pole's supply area is 5x5 around
    /// it, so everything below is inside it and the one entity at
    /// `OFF_NETWORK` is not.
    const POLE_AT: (f64, f64) = (10.5, 10.5);
    const PANELS_AT: [(f64, f64); 2] = [(13.5, 10.5), (13.5, 7.5)];
    const ACCUMULATORS_AT: [(f64, f64); 3] = [(7.5, 10.5), (7.5, 12.5), (7.5, 8.5)];
    const OFF_NETWORK: (f64, f64) = (40.5, 40.5);
    const CONSUMER_AT: (f64, f64) = (9.5, 12.5);

    /// A world with a small pole, `panels` solar panels and `accumulators`
    /// accumulators all inside its supply area, and optionally the surface's
    /// daylight curve.
    ///
    /// The fixture ships both prototypes with none of the energy fields, so
    /// they are set here for the reason `state.rs`'s own solar fixture gives:
    /// a number that came from the world is distinguishable from a number that
    /// happens to match a table. The accumulator gets its **discharge rate**
    /// as well as its buffer, so a sizing that reached for the wrong one would
    /// find a plausible number waiting.
    fn solar_state(daylight: bool, panels: usize, accumulators: usize) -> PlanState {
        let world = fixture_world();

        let mut panel = world
            .globals
            .entity_prototypes
            .get("solar-panel")
            .expect("the fixture ships a solar panel")
            .clone();
        panel.max_energy_production = Some(PANEL_NOON_JOULES_PER_TICK);
        panel.solar_panel_performance_at_day = Some(1.0);
        panel.solar_panel_performance_at_night = Some(0.0);
        world
            .globals
            .entity_prototypes
            .insert("solar-panel".into(), panel);

        let mut accumulator = world
            .globals
            .entity_prototypes
            .get("accumulator")
            .expect("the fixture ships an accumulator")
            .clone();
        accumulator.max_energy_production = Some(ACCUMULATOR_DISCHARGE_JOULES_PER_TICK);
        accumulator.electric_buffer_capacity = Some(ACCUMULATOR_BUFFER_JOULES);
        world
            .globals
            .entity_prototypes
            .insert("accumulator".into(), accumulator);

        if daylight {
            world.update_daylight(nauvis_daylight());
        }

        let mut state = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        state.create_entity(FactorioEntity {
            name: POLE.into(),
            position: Position::new(POLE_AT.0, POLE_AT.1),
            ..Default::default()
        });
        for (x, y) in PANELS_AT.iter().take(panels) {
            state.create_entity(FactorioEntity {
                name: "solar-panel".into(),
                position: Position::new(*x, *y),
                ..Default::default()
            });
        }
        for (x, y) in ACCUMULATORS_AT.iter().take(accumulators) {
            state.create_entity(FactorioEntity {
                name: "accumulator".into(),
                position: Position::new(*x, *y),
                ..Default::default()
            });
        }
        state
    }

    /// [`solar_state`] with the accumulator's buffer taken away and everything
    /// else left alone — a world that prices the panel and cannot size the
    /// bank.
    fn bufferless_accumulator_state(panels: usize, accumulators: usize) -> PlanState {
        let state = solar_state(true, panels, accumulators);
        let mut proto = state
            .base()
            .globals
            .entity_prototypes
            .get("accumulator")
            .expect("an accumulator")
            .clone();
        proto.electric_buffer_capacity = None;
        state
            .base()
            .globals
            .entity_prototypes
            .insert("accumulator".into(), proto);
        state
    }

    /// The condition [`capacity_refusal`] is asked about: a consumer inside
    /// the pole's supply area, on a network with no generator the ledger
    /// credits, so `supply_kw` is zero and the routing branch is the one
    /// taken.
    fn unpowered_site() -> Condition {
        Condition::Powered {
            pos: Position::new(CONSUMER_AT.0, CONSUMER_AT.1),
            entity: "stone-furnace".into(),
            kw: 90.,
        }
    }

    /// The ground the arm is asked about: one consumer inside the pole's
    /// supply area.
    fn consumer_area(state: &PlanState) -> Rect {
        state
            .collision_area(
                "stone-furnace",
                &Position::new(CONSUMER_AT.0, CONSUMER_AT.1),
            )
            .expect("the fixture carries a stone-furnace prototype")
    }

    /// **The owner's ruling as an assertion**: an array whose bank stands is
    /// credited, and credited its *daily average*.
    ///
    /// The average is not restated here -- it is taken from
    /// `solar_average_kw`, which is where it is derived and where it is
    /// pinned. What this asserts is the two things that could only be wrong
    /// here: that the credit is the average times the panel *count*, and that
    /// it is strictly between darkness and the noon nameplate. A pass-through
    /// of nameplate would fail the upper bound; crediting nothing, the lower.
    #[test]
    fn an_array_whose_bank_stands_is_credited_its_daily_average() {
        let s = solar_state(true, 2, 2);
        let each = s
            .solar_average_kw("solar-panel")
            .expect("a surface with a curve prices its panels");
        let credited = solar_supply_kw(&s, &consumer_area(&s)).expect("the bank stands");
        assert_eq!(
            credited,
            each * 2.,
            "two panels on the network, credited one average each"
        );
        let noon_kw = PANEL_NOON_JOULES_PER_TICK * 60. / 1000.;
        assert!(
            credited > 0. && credited < noon_kw * 2.,
            "an average is less than noon and more than midnight: {credited} kW against a \
             nameplate of {} kW",
            noon_kw * 2.
        );
    }

    /// The refusal, and it names the bank.
    ///
    /// One accumulator short of what two panels need, which on Nauvis is two.
    /// The figures in the error are the ones the decision was made on, so a
    /// reader can check the arithmetic without re-deriving it.
    #[test]
    fn an_array_whose_bank_is_short_refuses_and_says_it_is_the_bank() {
        let s = solar_state(true, 2, 1);
        match solar_supply_kw(&s, &consumer_area(&s)) {
            Err(PlannerError::SolarBankShort {
                panels,
                accumulators_needed,
                accumulators_standing,
                average_kw,
            }) => {
                assert_eq!(panels, 2);
                assert_eq!(accumulators_standing, 1);
                assert_eq!(
                    accumulators_needed, 2,
                    "0.85 accumulators a panel, twice, rounded up to a whole battery"
                );
                assert!(average_kw > 0., "the array it refuses to credit is real");
            }
            other => panic!("a short bank must refuse by name, got {other:?}"),
        }
        // And with the second accumulator standing, the same array is
        // credited -- so this is about the bank and not about the panels.
        assert!(solar_supply_kw(&solar_state(true, 2, 2), &consumer_area(&s)).is_ok());
    }

    /// **An accumulator without a panel is a buffer, not a supply.**
    ///
    /// Credited nothing, and refusing nothing: an accumulator on a steam
    /// network is an ordinary thing to build, and refusing a working base for
    /// owning one would be exactly the false refusal this arm exists to
    /// remove. The empty network is the same answer for the same reason, and
    /// it is the answer every plan this project has ever made gets.
    #[test]
    fn a_bank_with_no_panels_is_credited_nothing_and_refuses_nothing() {
        let s = solar_state(true, 0, 3);
        assert_eq!(
            solar_supply_kw(&s, &consumer_area(&s)).expect("a buffer is not a fault"),
            0.
        );
        let empty = solar_state(true, 0, 0);
        assert_eq!(
            solar_supply_kw(&empty, &consumer_area(&empty)).expect("nothing solar, nothing wrong"),
            0.
        );
        // And through the refusal path, which is where it would do harm: a
        // site with no supply and no solar must still be the routing verdict.
        // Every plan this project has ever made is this case, so a solar arm
        // that spoke here would speak on every refusal in the archive.
        assert!(
            capacity_refusal(&empty, &unpowered_site()).is_none(),
            "no panels, no solar verdict"
        );
    }

    /// The **second** guard on "unknown, never zero", and the one the
    /// no-daylight test above cannot reach.
    ///
    /// A surface with a perfectly good curve and an accumulator prototype that
    /// carries no buffer is still a bank nobody can size. It matters because
    /// the two guards are otherwise redundant: without daylight *both*
    /// `solar_average_kw` and `accumulators_per_panel` answer `None`, and the
    /// first one alone carries that test. This is the case where only the
    /// second one can.
    #[test]
    fn an_accumulator_with_no_buffer_cannot_size_a_bank() {
        let s = bufferless_accumulator_state(2, 2);
        assert!(
            s.solar_average_kw("solar-panel").is_some(),
            "the premise: the panel is priced, so this is about the battery"
        );
        assert!(
            matches!(
                solar_supply_kw(&s, &consumer_area(&s)),
                Err(PlannerError::SolarBankNotSizable { panels: 2, .. })
            ),
            "two accumulators that store nothing are not a bank, and a bank of \
             zero would credit the array in full"
        );
    }

    /// **Unknown, never zero.** A world that never reported its daylight
    /// cannot size a bank, and the tempting reading -- no curve, no night, no
    /// accumulators needed -- credits the array in full and is the midnight
    /// failure with an extra step.
    ///
    /// This is the state of every world this project archived before the
    /// daylight channel landed, so it is the common case rather than an edge.
    ///
    /// **Two independent guards hold this, and falsification is how that was
    /// found**: without a curve, `solar_average_kw` and
    /// `accumulators_per_panel` both answer `None`, so breaking either one
    /// alone leaves this test green. What it therefore pins on its own is the
    /// *classification*: a panel must still be recognised as a panel on a
    /// world that cannot price it, or this world answers "there is no solar
    /// here" — a lie — instead of "there is solar here I cannot size".
    /// `an_accumulator_with_no_buffer_cannot_size_a_bank` is the case that
    /// reaches the second guard by itself.
    #[test]
    fn a_world_with_no_daylight_refuses_rather_than_sizing_a_bank_of_zero() {
        let s = solar_state(false, 2, 3);
        assert!(
            matches!(
                solar_supply_kw(&s, &consumer_area(&s)),
                Err(PlannerError::SolarBankNotSizable { panels: 2, .. })
            ),
            "three accumulators is more than enough for two panels on any curve, and \
             without a curve there is no 'enough' to be more than"
        );
    }

    /// **The unit trap, as an experiment.**
    ///
    /// An accumulator answers two questions that both sound like "how much
    /// power does it hold": `electric_buffer_capacity` is its 5 MJ **store**
    /// and `max_energy_production` is its 300 kW **discharge rate**. A bank
    /// sized on the rate is wrong by a factor that depends on how long the
    /// night is.
    ///
    /// So: doubling the store halves the bank, and changing the discharge rate
    /// by any amount changes nothing at all.
    #[test]
    fn a_bank_is_sized_on_stored_energy_and_not_on_discharge_rate() {
        let s = solar_state(true, 0, 0);
        let baseline = solar_bank_for(&s, "solar-panel", "accumulator", 100).expect("sizable");

        let bigger_store = solar_state(true, 0, 0);
        let mut proto = bigger_store
            .base()
            .globals
            .entity_prototypes
            .get("accumulator")
            .expect("an accumulator")
            .clone();
        proto.electric_buffer_capacity = Some(ACCUMULATOR_BUFFER_JOULES * 2.);
        bigger_store
            .base()
            .globals
            .entity_prototypes
            .insert("accumulator".into(), proto);
        assert_eq!(
            solar_bank_for(&bigger_store, "solar-panel", "accumulator", 100).expect("sizable"),
            baseline.div_ceil(2),
            "an accumulator that stores twice as much halves the bank"
        );

        let faster = solar_state(true, 0, 0);
        let mut proto = faster
            .base()
            .globals
            .entity_prototypes
            .get("accumulator")
            .expect("an accumulator")
            .clone();
        proto.max_energy_production = Some(ACCUMULATOR_DISCHARGE_JOULES_PER_TICK * 10.);
        faster
            .base()
            .globals
            .entity_prototypes
            .insert("accumulator".into(), proto);
        assert_eq!(
            solar_bank_for(&faster, "solar-panel", "accumulator", 100).expect("sizable"),
            baseline,
            "an accumulator that discharges ten times faster stores no more, so the bank \
             is the same size; a sizing that read the rate would be a tenth of it"
        );
    }

    /// A panel is part of the array when the **wire** says so, not when it is
    /// nearby. The one off the network is credited to nobody and, crucially,
    /// refuses nobody -- otherwise a panel somebody built across the map would
    /// refuse every unrelated plan.
    #[test]
    fn a_panel_off_the_network_is_not_part_of_the_array() {
        // Stated as a difference between two worlds rather than against the
        // average, so this asserts the exclusion and nothing else: what the
        // credit *is* belongs to the test above, and restating it here would
        // make one of the two redundant.
        let without = solar_state(true, 2, 2);
        let before =
            solar_supply_kw(&without, &consumer_area(&without)).expect("two panels, two batteries");

        let mut with = solar_state(true, 2, 2);
        with.create_entity(FactorioEntity {
            name: "solar-panel".into(),
            position: Position::new(OFF_NETWORK.0, OFF_NETWORK.1),
            ..Default::default()
        });
        let after = solar_supply_kw(&with, &consumer_area(&with))
            .expect("a panel across the map is not this network's problem");

        assert_eq!(
            before, after,
            "the third panel is on no pole's network and is credited to nobody -- had it \
             counted, its own bank would be short and this would refuse instead"
        );
    }

    /// **The refusal a reader actually meets.** A site with a solar farm and
    /// no batteries used to be reported as a pole-routing failure, because
    /// `electric_supply_kw` credits solar nothing and zero supply is what a
    /// failed pole run looks like. It sent a reader to look at geometry that
    /// was perfect.
    #[test]
    fn a_refusing_site_blames_the_missing_batteries_and_not_the_poles() {
        let s = solar_state(true, 2, 0);
        assert!(
            !unpowered_site().holds(&s, BotId(1)),
            "the premise: an uncredited array leaves the site unpowered"
        );
        assert!(
            matches!(
                capacity_refusal(&s, &unpowered_site()),
                Some(PlannerError::SolarBankShort { .. })
            ),
            "an uncredited array is a battery fault, not a geometry one"
        );
    }
}
