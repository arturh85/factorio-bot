//! Stage 2 of the starter factory: an assembling machine that makes red
//! science, fed by inserters, on the plant's own network.
//!
//! Stage 1 ([`crate::method::produce`]) is two burner machines and no
//! electricity: a drill on ore dropping straight into a stone furnace. This is
//! the next shape up, and every one of its differences is a thing that can
//! fail silently:
//!
//! * **an inserter carries the items**, and an inserter's `direction` names
//!   the side it *picks up* from. Backwards places 100 %, passes every
//!   geometry check and moves nothing. The rule is checked here through
//!   [`PlanState::delivers_into`], which models both ends of one;
//! * **the machines are electric**, so a pole has to reach them and the
//!   network has to have the capacity *left*. Coverage is not capacity;
//! * **an assembling machine needs a recipe on it.** A placed machine with no
//!   recipe costs materials, occupies ground, reads as built by every
//!   geometry check, and produces nothing at all.
//!
//! # The shape, and what it is general over
//!
//! ```text
//!            feed chest  -> inserter ->
//!                                       [intermediate] -> inserter -> [product] <- inserter <- supply chest
//!           [feed chest  -> inserter ->]
//! ```
//!
//! Red science is one copper plate and one iron gear wheel. The gear is
//! *craftable*, so the cell builds a machine for it; the copper plate is
//! **smelted**, so no assembling machine makes one and it arrives in a chest.
//! That is the rule [`assembly_spec`] applies, and it is stated over the
//! world's recipes rather than over the name `automation-science-pack`: a
//! two-ingredient crafting recipe, exactly one of whose ingredients has a
//! crafting recipe of its own with at most [`MAX_FEED`] ingredients.
//!
//! # One machine deep, and how wide
//!
//! The cell is **two machines and never three**. What changed for green
//! science is only how wide the intermediate machine's mouth is: red's gear
//! machine eats one item and green's transport-belt machine eats two, so the
//! feed side of the layout carries one chest per ingredient rather than one
//! chest. [`MAX_FEED`] is two, and it is the *pole* that says so.
//!
//! Green science is `transport-belt` + `inserter`, and **both** are craftable.
//! Only the belt fits an intermediate machine (an inserter takes three
//! ingredients, which is one chest more than one small pole can light), so the
//! choice is forced rather than picked -- and the consequence is stated
//! plainly: **the inserters green science eats arrive in a chest, hand-crafted
//! by a bot, exactly as red's copper plates do.** A green cell automates the
//! belts and the packs; it does not automate the inserters. That is a first
//! green cell, not a green factory, and [`CELL_CHARGE_TICKS`] is how long it
//! runs before somebody fills it again.
//!
//! # What this claims, and what it deliberately does not
//!
//! The same split stage 1 states: **the planner answers structure** — do the
//! machines stand, hold the right recipes, feed one another, and sit on a
//! network with the capacity to spare — and **the supervisor answers
//! duration**, by witnessing packs appear while every bot stands still. A
//! structurally satisfied cell can still be a cell whose chests have run out
//! (this crate reads no container contents), whose boiler has run dry
//! (`electric_supply_kw` counts nameplate), or whose output has backed up.
//!
//! And one more, stated plainly because the goal's name invites the opposite
//! reading: **the chests are filled by hand.** The cell is charged with
//! [`CELL_CHARGE_TICKS`] worth of ingredients when it is built and nothing
//! refills it. Making the inputs arrive by machine is a belt or a second cell
//! feeding this one, which is stage 3.

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ActionId, ItemId, Ticks};
use crate::method::have::{PLACE_TICKS, TRANSFER_TICKS};
use crate::method::power::{POLE, Supply, plant_steps, supply_for};
use crate::method::produce::cells_for;
use crate::method::util::{
    CRAFTING_CATEGORY, RecipeGate, ingredients_of, output_per_craft, recipe_for, recipe_gate,
    smelting_ticks, tile_alignment_facing,
};
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::{FromPrimitive, ToPrimitive};
use factorio_bot_core::types::{Direction, FactorioEntity, FactorioRecipe, Pos, Position};

/// The crafting machine a cell is built from.
///
/// `automation` — one lab research of ten red packs — unlocks exactly one
/// assembling machine, and this is it. `assembling-machine-2` is 1.5x the
/// speed for 2x the draw and sits behind `automation-2`, which needs green
/// science, which needs this cell to exist first.
pub const MACHINE: &str = "assembling-machine-1";

/// What carries items between the machines.
///
/// The electric `inserter`, not `burner-inserter`: a burner inserter needs no
/// power and no research, and it also needs *coal*, which means a fuel level
/// nothing in this crate reads and a second unattended-time budget. The cell
/// already has to be on a network for its machines, so putting the inserters
/// on the same network costs one pole it was going to place anyway. Both are
/// behind `electronics`, which is a trigger technology (craft ten copper
/// plates) the ladder passes long before it gets here.
pub const INSERTER: &str = "inserter";

/// How many ingredients the cell's intermediate machine may take, and so how
/// many feed chests the cell has.
///
/// **Two, and the bound is the pole rather than a preference.** A cell has one
/// small electric pole, at [`POLE_OFFSET`], whose supply area is 5x5; the
/// intermediate machine is 3x3 and presents three tiles to the west, at
/// `y = -1`, `0` and `1` in the north frame. The pole's area reaches
/// `y = -0.5 ..= 4.5`, so it lights the inserters on two of those three rows
/// and not the third. A third feed chest would therefore need a second pole,
/// and a pole costs one wood out of the four a whole run has (see
/// [`POLE_OFFSET`]).
///
/// What it forecloses is named in the module doc: an `inserter` machine, which
/// takes three ingredients, is not an intermediate this cell can build.
pub const MAX_FEED: usize = 2;

/// What the cell's inputs sit in.
///
/// `iron-chest`: eight iron plates, enabled from the start, 32 slots. A
/// `wooden-chest` is cheaper in iron and costs wood, which the planner reaches
/// only by chopping trees; the iron is the cheaper of the two here.
pub const CHEST: &str = "iron-chest";

/// How long a cell is charged with ingredients for, in ticks — two and a half
/// minutes.
///
/// **A starter charge, not autonomy, and the difference is the honest part.**
/// Stage 1's cell runs unattended because its drill mines its own input; this
/// one is filled by hand, so this number is how long it runs before somebody
/// has to fill it again. It is 3.75x the 2,400-tick window
/// `scripts/factory_stage2.lua`'s witness waits, which is what it has to
/// outlive to prove anything, and at one cell's six packs a minute it is
/// fifteen packs — thirty iron plates and fifteen copper plates, about a third
/// of what the `automation` research standing behind this cell already spends.
///
/// A larger number is not free: every plate in it is smelted by a bot before
/// the cell is switched on, and a charge that dominates the ladder would make
/// this rung a smelting test rather than a factory one.
///
/// **After it runs out, nothing detects it.** No container contents are read
/// anywhere in this crate, so the cell simply stops and the structural
/// predicate keeps holding. Only the supervisor's witness catches it, and only
/// at the next witness milestone.
pub const CELL_CHARGE_TICKS: Ticks = 9_000;

/// How far from the supplying pole a cell site is looked for, in tiles.
///
/// The same twelve `crate::method::produce` and
/// `crate::method::util::free_area_near` use. It is bounded by something
/// sharper here than a search budget: a small pole's wire reach is 7.5 tiles,
/// so a cell whose own pole ends up further than that from the anchor is not
/// wired to the plant at all and [`fit`] refuses it on the `Powered` check.
/// Twelve leaves room for the cell's pole to sit inside that reach while its
/// *origin* — the intermediate machine, one tile east and two tiles north of
/// the pole — is further out.
const CELL_SEARCH_RADIUS: i32 = 12;

/// How far a supplying pole is looked for, in tiles.
///
/// `crate::method::have`'s `LAB_SEARCH_RADIUS`, for the same reason and with
/// the same consequence: past this, the answer is "build a plant" rather than
/// "walk further".
const ANCHOR_SEARCH_RADIUS: f64 = 64.0;

/// How far from the supplying pole the boiler that feeds it is looked for.
///
/// `crate::method::power::layout` builds the pump, the boiler, the engine and
/// the pole as one rigid body a handful of tiles across, so a boiler within
/// sixteen tiles of the pole a cell hangs off is that plant's boiler. It is a
/// heuristic and it is named as one: a map with an unrelated boiler nearer
/// than the plant's would have the wrong one topped up. The top-up is coal
/// into a fuel slot, so the cost of being wrong is a few coal in the wrong
/// machine rather than a wrong plan.
const BOILER_SEARCH_RADIUS: f64 = 16.0;

/// What the boiler burns, in kJ per unit, and how many of them fit in its one
/// fuel slot.
///
/// Coal is 4 MJ and stacks to 50, so a full slot is 200 MJ. Written down here
/// rather than read, for the same reason `COAL_BURN_TICKS` is: **the mod does
/// not send `fuel_value` or `stack_size`**, so neither number can come out of
/// the world.
const COAL_KJ: u64 = 4_000;
const COAL_STACK: u32 = 50;

/// The machine a boiler is, by name.
const BOILER: &str = "boiler";

/// Time to put a recipe on a machine.
///
/// One RCON round trip and no walking beyond the reach the action already
/// states, so the same [`TRANSFER_TICKS`] an insert costs. It is an estimate
/// like every other duration in this crate; what makes it cheap to be wrong
/// about is that a `SetRecipe` is idempotent, so a retry costs another one of
/// these and nothing else.
const SET_RECIPE_TICKS: Ticks = TRANSFER_TICKS;

// ---------------------------------------------------------------------------
// What a cell for an item is
// ---------------------------------------------------------------------------

/// The half of the chain the cell builds a machine for.
#[derive(Clone, Debug, PartialEq)]
pub struct Intermediate {
    /// What the machine makes, e.g. `iron-gear-wheel`.
    pub item: ItemId,
    /// The recipe it runs.
    pub recipe: FactorioRecipe,
    /// How many of [`Intermediate::item`] one product needs.
    pub per_product: u32,
    /// How many [`Intermediate::item`] one run of its recipe yields.
    pub per_run: u32,
    /// What its recipe consumes, and how much of each **per run**, in the
    /// recipe's own order.
    ///
    /// One entry per feed chest, at least one and never more than
    /// [`MAX_FEED`]. Ordered rather than a map, because the order is the
    /// order the chests stand in and therefore the order the plan's steps
    /// come out in.
    pub ingredients: Vec<(ItemId, u32)>,
}

/// What a cell for one item is made of, resolved from the world's own recipes.
#[derive(Clone, Debug, PartialEq)]
pub struct AssemblySpec {
    /// What the cell produces, e.g. `automation-science-pack`.
    pub item: ItemId,
    /// The crafting recipe the product machine runs.
    pub recipe: FactorioRecipe,
    /// The ingredient the cell cannot make, and how much of it per product.
    ///
    /// This is what goes in the supply chest.
    pub supplied: (ItemId, u32),
    /// The ingredient it can, and the machine that does it.
    pub intermediate: Intermediate,
    /// Ticks per product for the whole cell: the **slower** of its two
    /// machines.
    ///
    /// Integer, and the whole reason [`Goal::Producing`] carries a `u32`
    /// rather than an `f64`: the cell count is
    /// `ceil(per_minute * ticks_per_item / 3600)`, exact integer arithmetic
    /// end to end, so it cannot land on a different side of a ceiling on a
    /// different run.
    pub ticks_per_item: Ticks,
}

impl AssemblySpec {
    /// How many products one charge is sized for.
    ///
    /// At least one: a cell charged for nothing at all would place perfectly
    /// and be dead before it started, which is the failure this whole stage is
    /// against.
    pub fn charge_products(&self) -> u32 {
        (CELL_CHARGE_TICKS / self.ticks_per_item.max(1)).max(1)
    }

    /// How many runs of the intermediate's recipe one charge is sized for.
    ///
    /// Rounded **up** at the run boundary rather than at the item one: a
    /// recipe yielding two per run needs `ceil(n / 2)` runs, and each run eats
    /// its whole ingredient amount whether or not the last one is fully used.
    pub fn intermediate_runs(&self) -> u32 {
        let wanted = self
            .charge_products()
            .saturating_mul(self.intermediate.per_product);
        wanted.div_ceil(self.intermediate.per_run.max(1))
    }

    /// What goes in each feed chest for one charge, in chest order.
    ///
    /// Integer end to end, one entry per ingredient of the intermediate's
    /// recipe, in that recipe's order -- which is the order
    /// [`Role::FeedChest`]'s indices are assigned in.
    pub fn feed_charges(&self) -> Vec<(ItemId, u32)> {
        let runs = self.intermediate_runs();
        self.intermediate
            .ingredients
            .iter()
            .map(|(item, amount)| (item.clone(), runs.saturating_mul(*amount)))
            .collect()
    }

    /// How much goes in the supply chest for one charge.
    pub fn supply_charge(&self) -> u32 {
        self.charge_products().saturating_mul(self.supplied.1)
    }
}

/// Can a stage-2 cell make `item`, and out of what?
///
/// Four conditions, all read from the world rather than named here:
///
/// * the recipe is a **crafting** recipe, because an assembling machine runs
///   no other category and a stone furnace runs no crafting one;
/// * it takes exactly **two** ingredients, because the product machine is fed
///   by one link inserter and one supply chest and there is nowhere to put a
///   third;
/// * **one** of them is the intermediate, which the cell builds a machine
///   for. To be one, an ingredient needs a crafting recipe of its own with at
///   most [`MAX_FEED`] ingredients — the width of the feed side;
/// * the other arrives in a chest, hand-filled, whether or not the game could
///   also craft it. There is no second intermediate machine: the cell is two
///   machines deep by construction.
///
/// # When both halves could be the intermediate
///
/// The **shallower** recipe wins, and a tie is a refusal. Each ingredient of
/// the intermediate is one more chest, one more inserter and one more thing
/// the pole has to light, so the smaller machine is the one more likely to fit
/// and the one with fewer ways to be half-fed; and refusing a tie is what
/// keeps this function from silently choosing between two cells that are
/// equally good on the only ground it can see.
///
/// **This ordering is why widening `MAX_FEED` from one to two changed no
/// existing answer.** Every item that resolved when only single-ingredient
/// intermediates counted has a candidate of length one, and length one is the
/// shallowest there is — so it still wins, against candidates that could not
/// even be considered before. `repair-pack` (gear + circuit) and
/// `fast-transport-belt` (gear + belt) are the two reachable cases where the
/// widening added a rival, and both still resolve to the gear.
///
/// In vanilla 2.1 red science is the case this exists for and green science is
/// the case [`MAX_FEED`] was widened for. `None` is not "impossible" — it is
/// "no *cell of this shape* makes it", and [`Goal::Have`] still reaches it by
/// hand.
pub fn assembly_spec(state: &PlanState, item: &str) -> Option<AssemblySpec> {
    let recipe = recipe_for(state, item)?;
    if recipe.category != CRAFTING_CATEGORY {
        return None;
    }
    let ingredients = ingredients_of(&recipe);
    let [(a, a_amount), (b, b_amount)] = ingredients.as_slice() else {
        return None;
    };
    // Which of the two the cell builds a machine for. None means both arrive
    // in chests and no machine of the cell's does anything; both is decided by
    // depth, and a tie is a refusal -- see this function's own doc.
    let a_made = intermediate_for(state, a, *a_amount);
    let b_made = intermediate_for(state, b, *b_amount);
    let (intermediate, supplied) = match (a_made, b_made) {
        (Some(made), None) => (made, (b.clone(), *b_amount)),
        (None, Some(made)) => (made, (a.clone(), *a_amount)),
        (Some(from_a), Some(from_b)) => {
            match from_a.ingredients.len().cmp(&from_b.ingredients.len()) {
                std::cmp::Ordering::Less => (from_a, (b.clone(), *b_amount)),
                std::cmp::Ordering::Greater => (from_b, (a.clone(), *a_amount)),
                std::cmp::Ordering::Equal => return None,
            }
        }
        (None, None) => return None,
    };
    // The product machine's own tempo, and the intermediate machine's
    // expressed in the same unit so the two are comparable. Both integer.
    let product_ticks =
        smelting_ticks(state, &recipe, MACHINE).div_ceil(output_per_craft(&recipe, item).max(1));
    let intermediate_ticks = smelting_ticks(state, &intermediate.recipe, MACHINE)
        .saturating_mul(intermediate.per_product)
        .div_ceil(intermediate.per_run.max(1));
    let ticks_per_item = product_ticks.max(intermediate_ticks);
    if ticks_per_item == 0 {
        return None;
    }
    Some(AssemblySpec {
        item: item.to_string(),
        recipe,
        supplied,
        intermediate,
        ticks_per_item,
    })
}

/// Is `item` something one assembling machine makes out of things the cell can
/// stand a chest in front of?
///
/// `None` for a recipe with more ingredients than [`MAX_FEED`]: that is not
/// "the game cannot make it", it is "this cell has nowhere to put the chests",
/// and the item then arrives in the *supply* chest instead — which is exactly
/// what happens to green science's inserters.
fn intermediate_for(state: &PlanState, item: &str, per_product: u32) -> Option<Intermediate> {
    let recipe = recipe_for(state, item)?;
    if recipe.category != CRAFTING_CATEGORY {
        return None;
    }
    let ingredients = ingredients_of(&recipe);
    if ingredients.is_empty() || ingredients.len() > MAX_FEED {
        return None;
    }
    // A machine that eats what it makes has no chest arrangement that works,
    // and would make `feed_charges` ask a bot to carry in the very thing the
    // cell exists to produce.
    if ingredients.iter().any(|(name, _)| name == item) {
        return None;
    }
    Some(Intermediate {
        item: item.to_string(),
        per_run: output_per_craft(&recipe, item).max(1),
        recipe,
        per_product,
        ingredients,
    })
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// What one building of a cell is for.
///
/// Named rather than positional because [`cell_steps`] has to find each of
/// them by role — which machine gets which recipe, which chest gets which
/// charge — and an index into a vector is exactly the kind of thing that keeps
/// working after somebody reorders the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// The cell's own supply, wired to the plant's.
    Pole,
    /// Makes the ingredient the cell can make.
    Intermediate,
    /// Makes the item the goal named.
    Product,
    /// Holds one of the things the intermediate machine eats.
    ///
    /// Indexed by that ingredient's position in the intermediate's own recipe,
    /// `0 .. MAX_FEED`. Named-and-indexed rather than positional for the same
    /// reason the rest of this enum is named: a cell has one chest per
    /// ingredient, and which chest gets which charge has to survive somebody
    /// reordering [`layout_table`].
    FeedChest(u8),
    /// Holds the ingredient nothing in the cell makes.
    SupplyChest,
    /// Feed chest `n` -> intermediate machine.
    FeedInserter(u8),
    /// Intermediate machine -> product machine.
    LinkInserter,
    /// Supply chest -> product machine.
    SupplyInserter,
}

impl Role {
    /// What stands in this role.
    fn name(self) -> &'static str {
        match self {
            Role::Pole => POLE,
            Role::Intermediate | Role::Product => MACHINE,
            Role::FeedChest(_) | Role::SupplyChest => CHEST,
            Role::FeedInserter(_) | Role::LinkInserter | Role::SupplyInserter => INSERTER,
        }
    }
}

/// One building of a cell, with the direction it stands in.
#[derive(Clone, Debug, PartialEq)]
pub struct CellPart {
    pub role: Role,
    pub position: Position,
    pub direction: Direction,
}

/// The cell, in build order, as north-frame offsets from the intermediate
/// machine's own position, with the direction each part stands in when the
/// cell faces north.
///
/// **The inserter directions are the whole design and each one points at what
/// it PICKS UP from.** `Direction::North` on the link inserter means it takes
/// from the tile *north* of itself — the intermediate machine — and drops one
/// tile south, into the product machine. `Direction::West` on the chest
/// inserters means they take from the chest to their west and drop east into
/// the machine. Neither is derived from the other; both are the same rule
/// `PlanState::pickup_position` and `delivery_offset` implement, and [`fit`]
/// checks every link with `delivers_into` rather than trusting this table.
///
/// The layout as a picture, north frame, `#` for the 3x3 machines, with the
/// second feed row in brackets — present only when the intermediate's recipe
/// has a second ingredient:
///
/// ```text
///        x: -3  -2  -1   0   1
///   y  0:  C   >   .  ###
///      1: [C] [>]  .  ###
///      2:  .   .   P   v
///      3:  .   .   .  ###
///      4:  C   >   .  ###
/// ```
///
/// (The machines are three tiles wide and centred on `x = 0`, so they occupy
/// `x = -1 .. 1`; the pole `P` at `(-1, 2)` sits in the one-tile gap between
/// them, where a 5x5 supply area reaches every consumer in the cell —
/// including both feed rows, which is what [`MAX_FEED`] is two for.)
///
/// **A one-feed cell comes out of this in exactly the order the fixed table it
/// replaced had**, which is why widening it moved no red-science plan: the
/// build order, and therefore every action id in the plan, is this sequence.
fn layout_table(feeds: usize) -> Vec<(Role, (f64, f64), Direction)> {
    let feeds = feeds.min(MAX_FEED);
    let mut out = vec![
        (Role::Intermediate, (0., 0.), Direction::North),
        (Role::Product, (0., 4.), Direction::North),
    ];
    for (index, y) in FEED_ROWS.iter().take(feeds).enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        out.push((Role::FeedChest(index as u8), (-3., *y), Direction::North));
    }
    out.push((Role::SupplyChest, (-3., 4.), Direction::North));
    for (index, y) in FEED_ROWS.iter().take(feeds).enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        out.push((Role::FeedInserter(index as u8), (-2., *y), Direction::West));
    }
    out.push((Role::LinkInserter, (0., 2.), Direction::North));
    out.push((Role::SupplyInserter, (-2., 4.), Direction::West));
    out
}

/// The rows a feed chest and its inserter stand on, in chest-index order.
///
/// `y = 0` is the intermediate machine's own row and `y = 1` the one south of
/// it. The machine presents a third tile to the west at `y = -1`, and it is
/// deliberately unused: the pole at [`POLE_OFFSET`] supplies
/// `y = -0.5 ..= 4.5`, so an inserter there would stand unpowered — see
/// [`MAX_FEED`].
const FEED_ROWS: [f64; MAX_FEED] = [0., 1.];

/// Where the cell puts a pole of its **own**, when it has to bring one.
///
/// The one-tile gap between the two machines, from which a 5x5 supply area
/// reaches all five consumers -- which is the whole reason the layout is an L
/// and not a row (`tests::the_cells_own_pole_covers_every_consumer_in_it`).
///
/// **It is not in [`LAYOUT`], and that is the point.** A small electric pole
/// costs one wood, and wood used to be the one item this planner could not
/// make: `Mine` sources only `EntityGraph::resources` and a tree is not one,
/// so four bots holding one each was four wood and eight poles for the whole
/// run -- **a limit of the model rather than of the game**, which renews wood
/// perfectly well and which the mod already deforests. Run
/// `run-1788396958-07935` spent both of bot 1's on power plants -- one craft
/// yields two, a replan re-sited the plant, and both went into the ground --
/// and the cell was then refused with `no method can satisfy goal: have 1
/// wood`, with three other bots each still holding one that
/// `Holder::Share(chain_actor)` cannot reach.
///
/// `crate::method::have::Chop` has since lifted that cap: a bot fells a tree
/// for wood, so a pole is priced like any other craft. The cheap pass below is
/// still asked first, because duplicating a supply area that already stands is
/// waste whatever it costs -- but "a pole is irreplaceable" is no longer a
/// reason to refuse a plan that needs one, and
/// `crate::method::have::lab_site_with_pole` is the second place that
/// reasoning had to be corrected.
///
/// So [`fit`] asks for the cell **without** a pole first, and only brings one
/// when no existing network already covers the ground. A cell sited in the
/// plant's own supply area genuinely does not need a pole, and asking for one
/// spends an irreplaceable item to duplicate something already standing.
const POLE_OFFSET: (f64, f64) = (-1., 2.);

/// Where a bot has to be able to stand to fill this cell's chests.
///
/// The column immediately west of both chests, and the tiles joining them.
/// **Stated rather than left to luck**, because the alternative was measured:
/// on the planner's 2-tile grid a character has `0.1015625` of slack against a
/// furnace, and 18 of 18 walk stalls in run 30 had the character pressed
/// against a box, eight of them at `1/256` of a tile. A chest is filled by a
/// bot standing next to it, so where that bot stands is part of the layout and
/// is asserted (`tests::every_lane_tile_admits_a_character`) rather than
/// hoped for.
const LANE: [(f64, f64); 5] = [(-4., 0.), (-4., 1.), (-4., 2.), (-4., 3.), (-4., 4.)];

/// A cell, sited and checked, ready to be turned into steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// The intermediate machine's position — the origin every offset is from.
    pub origin: Position,
    pub facing: Direction,
    /// Every building, in build order. Includes a pole of the cell's own only
    /// when no existing network already covers the ground -- see
    /// [`POLE_OFFSET`].
    pub parts: Vec<CellPart>,
    /// Every tile a bot must be able to stand on to charge this cell.
    pub lane: Vec<Position>,
    /// Bystanders `fit` found would be sealed into a pocket by this cell's
    /// own footprint, and where each must walk first. Empty in the ordinary
    /// case -- see [`crate::enclosure::check`].
    pub evacuate: Vec<crate::enclosure::Evacuation>,
}

impl Cell {
    /// Where the building in `role` stands.
    ///
    /// Every role is in [`LAYOUT`] exactly once, so this cannot miss — but it
    /// returns an `Option` rather than panicking, because a layout edited to
    /// drop a role should refuse a plan rather than take a process down.
    pub fn at(&self, role: Role) -> Option<&CellPart> {
        self.parts.iter().find(|part| part.role == role)
    }

    /// How many feed chests this cell has — one per ingredient of the
    /// intermediate's recipe.
    ///
    /// Counted off the parts rather than carried, so it cannot disagree with
    /// what actually stands.
    pub fn feeds(&self) -> usize {
        self.parts
            .iter()
            .filter(|part| matches!(part.role, Role::FeedChest(_)))
            .count()
    }

    /// Does this cell have to place a pole of its own?
    ///
    /// `false` when it stands inside a supply area that already exists, which
    /// is the cheap case and the one [`plan_cell`] looks for first. The bill
    /// reads this rather than assuming: a pole nobody needs is one wood spent
    /// out of a lifetime supply of four.
    pub fn brings_pole(&self) -> bool {
        self.at(Role::Pole).is_some()
    }

    /// The ground whose electric network this cell is on.
    ///
    /// The product machine, because it is a consumer by construction whether
    /// or not the cell brought a pole. Asking about the pole would answer
    /// `None` for exactly the cells that adopted somebody else's.
    pub fn on_network_at(&self) -> Option<&Position> {
        self.at(Role::Product).map(|part| &part.position)
    }
}

/// `direction` turned a further `by`.
///
/// A quarter turn is **four** on Factorio 2.x's sixteen-value scale, and a
/// half turn is eight; a cell only ever composes cardinals, so the sum is
/// always another cardinal. The same arithmetic `crate::method::power` does,
/// written again rather than shared because both are three lines and neither
/// crate boundary is crossed by it.
fn compose(direction: Direction, by: Direction) -> Option<Direction> {
    let sum = (Direction::to_u8(&direction)? + Direction::to_u8(&by)?) % 16;
    Direction::from_u8(sum)
}

/// The buildings of a cell whose intermediate machine stands at `origin`
/// facing `facing`, with `feeds` feed chests.
///
/// A rigid body rotated about the origin, which is a **tile centre**: every
/// part of this cell is one or three tiles across — odd on both axes — so
/// every one of them belongs on tile centres, and a quarter turn about a tile
/// centre takes tile centres to tile centres. That is what keeps all eight on
/// their own build grid at all four facings, and it is
/// `tests::every_facing_puts_every_building_on_its_own_grid` rather than a
/// comment.
fn layout(
    origin: &Position,
    facing: Direction,
    with_pole: bool,
    feeds: usize,
) -> Option<Vec<CellPart>> {
    // A cell with no feed chest has a machine nothing puts anything into, and
    // one with more than `MAX_FEED` has an inserter the pole cannot light.
    // Both are refusals rather than truncations: a truncated cell places
    // perfectly and starves.
    if feeds == 0 || feeds > MAX_FEED {
        return None;
    }
    let pole = with_pole.then_some((Role::Pole, POLE_OFFSET, Direction::North));
    layout_table(feeds)
        .into_iter()
        .chain(pole)
        .map(|(role, offset, direction)| {
            Some(CellPart {
                role,
                position: origin.add(&Position::new(offset.0, offset.1).turn(facing)?),
                direction: compose(direction, facing)?,
            })
        })
        .collect()
}

/// The lane tiles of a cell at `origin` facing `facing`.
fn lane(origin: &Position, facing: Direction) -> Option<Vec<Position>> {
    LANE.iter()
        .map(|offset| Some(origin.add(&Position::new(offset.0, offset.1).turn(facing)?)))
        .collect()
}

/// The `FactorioEntity` one part of a cell places.
///
/// `entity_type` is read from the prototype rather than guessed — an
/// assembling machine's type is `assembling-machine`, an iron chest's is
/// `container` and a small pole's is `electric-pole`, none of which is its
/// name, and `EntityGraph::add`'s whitelist and `PlanState::withdraw_slot` are
/// both keyed on the type.
fn entity_for(state: &PlanState, part: &CellPart) -> FactorioEntity {
    let name = part.role.name();
    let entity_type = state
        .base()
        .entity_prototypes
        .get(name)
        .map(|proto| proto.entity_type.clone())
        .unwrap_or_else(|| name.to_string());
    FactorioEntity {
        name: name.to_string(),
        entity_type,
        position: part.position.clone(),
        direction: Direction::to_u8(&part.direction).unwrap_or(0),
        ..Default::default()
    }
}

/// The links a cell is, in the order items travel them.
///
/// Returned as positions rather than checked in place so that [`fit`] and
/// [`cell_steps`] ask the *same* question — one is the expansion-time check
/// and the other is the `Condition::Feeds` the scheduler re-checks, and a
/// second copy of this list could disagree with the first.
fn links(cell: &Cell) -> Option<Vec<(Position, Position)>> {
    let p = |role: Role| cell.at(role).map(|part| part.position.clone());
    let mut out = Vec::new();
    for index in 0..cell.feeds() {
        #[allow(clippy::cast_possible_truncation)]
        let index = index as u8;
        out.push((p(Role::FeedChest(index))?, p(Role::FeedInserter(index))?));
        out.push((p(Role::FeedInserter(index))?, p(Role::Intermediate)?));
    }
    out.push((p(Role::Intermediate)?, p(Role::LinkInserter)?));
    out.push((p(Role::LinkInserter)?, p(Role::Product)?));
    out.push((p(Role::SupplyChest)?, p(Role::SupplyInserter)?));
    out.push((p(Role::SupplyInserter)?, p(Role::Product)?));
    Some(out)
}

/// Every part of a cell that draws from the network, with what it draws.
///
/// The draw comes from [`PlanState::consumer_draw_kw`] — the very table
/// `electric_demand_kw` charges — so the number a cell states in its
/// `Condition::Powered` is the number the budget will bill it. A second copy
/// of those figures here is exactly the drift that makes a plan pass its own
/// check and brown out the network.
fn consumers(state: &PlanState, cell: &Cell) -> Vec<(Position, &'static str, f64)> {
    cell.parts
        .iter()
        .filter_map(|part| {
            let name = part.role.name();
            let kw = state.consumer_draw_kw(name)?;
            Some((part.position.clone(), name, kw))
        })
        .collect()
}

/// Does a whole cell fit, work and run with its origin at `origin` facing
/// `facing`?
///
/// Four questions, and only the first is geometry:
///
/// 1. every building's ground is clear, and so is every tile of the servicing
///    lane — a cell nobody can reach to charge is a cell that runs once;
/// 2. with all eight standing, every one of the six links really delivers.
///    Asked of a fork with the cell placed, so it is the same predicate
///    [`Condition::Feeds`] will be checked with rather than a restatement of
///    it — and it is what refuses an inserter turned round, which places 100 %
///    and moves nothing;
/// 3. every electric part has the capacity it needs **left** on the network
///    the cell's own pole joins. Not "is there a pole": twelve assembling
///    machines on one 900 kW engine each pass a coverage test individually;
/// 4. and, implied by 3 through the union-find in `electric_network`, the
///    cell is actually wired to a generator — which is what bounds the search
///    to the anchor's wire reach without stating a distance here.
///
/// `with_pole` decides whether the cell brings a pole of its own. Asked
/// `false` first by [`plan_cell`]: question 3 then has to be answered by a
/// network that already exists, and a cell that passes it needs no pole at
/// all. See [`POLE_OFFSET`] for what one costs.
fn fit(
    state: &PlanState,
    origin: &Position,
    facing: Direction,
    with_pole: bool,
    feeds: usize,
) -> Option<Cell> {
    let parts = layout(origin, facing, with_pole, feeds)?;
    let lane = lane(origin, facing)?;
    for part in &parts {
        if !state.is_area_free_facing(part.role.name(), &part.position, part.direction) {
            return None;
        }
    }
    for tile in &lane {
        if !state.is_position_free(tile) {
            return None;
        }
    }
    let mut cell = Cell {
        origin: origin.clone(),
        facing,
        parts,
        lane,
        evacuate: Vec::new(),
    };
    let mut trial = state.fork();
    for part in &cell.parts {
        trial.create_entity(entity_for(state, part));
    }
    for (from, to) in links(&cell)? {
        if !trial.delivers_into(&from, &to) {
            return None;
        }
    }
    for (position, name, kw) in consumers(&trial, &cell) {
        if !(Condition::Powered {
            pos: position,
            entity: name.into(),
            kw,
        })
        .holds(&trial, crate::ids::BotId(0))
        {
            return None;
        }
    }
    // Last, because it is the most expensive check and every cheaper one
    // above has already had the chance to reject this candidate for free --
    // see `crate::enclosure::check`'s own doc for why `trial` (parts already
    // created) is exactly the fork that check wants.
    match crate::enclosure::check(state, &trial, origin) {
        crate::enclosure::EnclosurePrevention::Clear => {}
        crate::enclosure::EnclosurePrevention::Evacuate(evacuations) => {
            cell.evacuate = evacuations;
        }
        crate::enclosure::EnclosurePrevention::Refuse => return None,
    }
    Some(cell)
}

// ---------------------------------------------------------------------------
// Siting
// ---------------------------------------------------------------------------

/// What a whole cell draws, for the anchor search.
///
/// Two machines, and one inserter per feed chest plus the link and the supply
/// one. Read off the spec rather than a constant, because a two-feed cell has
/// a fourth inserter and an anchor sized for three would be sized short.
fn cell_demand_kw(state: &PlanState, spec: &AssemblySpec) -> f64 {
    let machines = state.consumer_draw_kw(MACHINE).unwrap_or(0.);
    let inserters = state.consumer_draw_kw(INSERTER).unwrap_or(0.);
    let count = inserter_count(spec).to_f64().unwrap_or(3.);
    2. * machines + count * inserters
}

/// How many inserters a cell for `spec` has: one per feed chest, one link, one
/// supply.
fn inserter_count(spec: &AssemblySpec) -> u32 {
    u32::try_from(spec.intermediate.ingredients.len()).unwrap_or(1) + 2
}

/// Find somewhere powered to put one cell, or say why not.
///
/// Deterministic by construction: the anchor is the pole
/// [`PlanState::nearest_supply_anchor`] orders first (distance, then `x`, then
/// `y`), the candidates come out of a ring search in a fixed order, and the
/// four facings are tried north, east, south, west. Nothing here reads a quad
/// tree's own order.
///
/// **Sited from the pole, not from the bot**, exactly as the lab is: a cell
/// has to be within one pole's wire reach of a network that already has
/// generation on it, and searching around a bot would only ever find power the
/// bot happens to be standing in.
pub fn plan_cell(
    state: &PlanState,
    anchor: &Position,
    spec: &AssemblySpec,
) -> Result<Cell, PlannerError> {
    let base = Pos::from(anchor);
    let feeds = spec.intermediate.ingredients.len();
    // The cheap pass first, and the whole ring search is repeated rather than
    // interleaved: a cell twelve tiles out that needs no pole beats one beside
    // the anchor that costs a wood, because wood is the one resource this
    // project has four of and cannot make more. Two full passes in a fixed
    // order are as deterministic as one.
    for with_pole in [false, true] {
        for radius in 0..=CELL_SEARCH_RADIUS {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    // Only the ring at exactly this radius; inner ones were done.
                    if dx.abs() != radius && dy.abs() != radius {
                        continue;
                    }
                    for facing in Direction::orthogonal() {
                        // The intermediate machine's own build grid, read rather
                        // than assumed: it is the tile-centre grid for a 3x3 at
                        // every cardinal, and reading it is what keeps this
                        // correct the day a cell's first machine is not 3x3.
                        let (offset_x, offset_y) = tile_alignment_facing(state, MACHINE, facing);
                        let candidate = Position::new(
                            f64::from(base.0 + dx) + offset_x,
                            f64::from(base.1 + dy) + offset_y,
                        );
                        if let Some(cell) = fit(state, &candidate, facing, with_pole, feeds) {
                            return Ok(cell);
                        }
                    }
                }
            }
        }
    }
    Err(PlannerError::NoRoomForCellNearPower {
        item: spec.item.clone(),
        radius: CELL_SEARCH_RADIUS,
    })
}

/// Site `count` cells, each clear of the ones before it.
///
/// Each cell is reserved on a fork as it is chosen, so the next search sees it
/// standing there — and, since the reservation includes its pole and its five
/// consumers, the *second* cell's `Powered` check is asked against a network
/// the first cell has already spent capacity on. Without that, two cells on
/// one 900 kW engine would each be sized against the whole of it.
pub fn plan_cells(
    state: &PlanState,
    anchor: &Position,
    spec: &AssemblySpec,
    count: u32,
) -> Result<Vec<Cell>, PlannerError> {
    let mut trial = state.fork();
    let mut out = Vec::new();
    for _ in 0..count {
        let cell = plan_cell(&trial, anchor, spec)?;
        for part in &cell.parts {
            trial.create_entity(entity_for(&trial, part));
        }
        out.push(cell);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// What already stands
// ---------------------------------------------------------------------------

/// How far from the world origin [`cells_standing`] looks.
///
/// **A bound on what `holds` can see, stated rather than hidden.** A cell built
/// further out than this is not counted and its goal is re-planned, which
/// over-builds rather than over-claims — the direction this crate chooses
/// everywhere. It is generous because the sweep is cheap: `entities_within`
/// reads the entity tree, which holds machines, containers, poles and two
/// kinds of rock, and *not* the trees, water and ore that make a charted map
/// large.
const CELL_SCAN_RADIUS: f64 = 512.0;

/// How many complete cells for `spec` already stand in this state.
///
/// A cell counts when a machine **set to the product's recipe** stands,
/// has the capacity it needs left on its network, and has one loaded inserter
/// per ingredient of that recipe. All four clauses are load-bearing:
///
/// * *set to the recipe*, because a machine with no recipe on it is the
///   placed-but-dead machine this stage exists to make impossible;
/// * *powered with headroom*, because coverage is not capacity, and because a
///   machine on a network with nothing left runs at a fraction of its rate
///   with no error anywhere;
/// * *one inserter per ingredient*, because a machine fed copper and no gears
///   makes nothing at all, and the ingredient count is the only handle this
///   crate has on that — nothing here models which *item* an inserter carries;
/// * *each of those inserters is itself fed*, which is what makes this a claim
///   about the whole chain rather than about the last link of it. The
///   intermediate machine is counted by this clause and not by name — see
///   [`is_supplied`], which is where the recursion into it happens.
///
/// Order-independent by construction: it counts, and it dedupes by tile
/// through the `(x, y, name)` order `entities_within` already imposes.
pub fn cells_standing(state: &PlanState, spec: &AssemblySpec) -> u32 {
    let ingredients = ingredients_of(&spec.recipe).len();
    let mut count = 0u32;
    let nearby = state.entities_within(&Position::new(0., 0.), CELL_SCAN_RADIUS);
    for machine in &nearby {
        if machine.name != MACHINE {
            continue;
        }
        if machine.recipe.as_deref() != Some(spec.recipe.name.as_str()) {
            continue;
        }
        let Some(kw) = state.consumer_draw_kw(MACHINE) else {
            continue;
        };
        if !(Condition::Powered {
            pos: machine.position.clone(),
            entity: MACHINE.into(),
            kw,
        })
        .holds(state, crate::ids::BotId(0))
        {
            continue;
        }
        if loaded_feeders(state, &nearby, machine, CHAIN_DEPTH) >= ingredients {
            count += 1;
        }
    }
    count
}

/// How deep [`loaded_feeders`] follows the chain back from the product
/// machine.
///
/// One, because the cell is two machines deep and never three: the product
/// machine's feeders come from chests and from the intermediate machine, and
/// the intermediate machine's feeders come from chests. It is stated as a
/// bound rather than left to the shape of the world because the world is not
/// a cell — a real factory has machines feeding machines feeding machines, and
/// an unbounded walk over `entities_within(512)` would be neither cheap nor
/// obviously terminating.
const CHAIN_DEPTH: u8 = 1;

/// How many inserters put something *supplied* into `machine`.
///
/// The inserter has to deliver into the machine, and it has to be taking from
/// something else that has something to give — which is [`is_supplied`], and
/// which is where "the whole chain" stops being a figure of speech.
fn loaded_feeders(
    state: &PlanState,
    nearby: &[FactorioEntity],
    machine: &FactorioEntity,
    depth: u8,
) -> usize {
    nearby
        .iter()
        .filter(|inserter| inserter.name == INSERTER)
        .filter(|inserter| state.delivers_into(&inserter.position, &machine.position))
        .filter(|inserter| {
            nearby.iter().any(|source| {
                source.position != inserter.position
                    && source.position != machine.position
                    && state.delivers_into(&source.position, &inserter.position)
                    && is_supplied(state, nearby, source, depth)
            })
        })
        .count()
}

/// Has `source` got anything to hand on?
///
/// A chest is taken on trust: **nothing in this crate reads a container's
/// contents**, which is stated in this module's header and is unchanged here.
///
/// A *machine* is not taken on trust, and that is the clause a two-feed cell
/// made load-bearing. A machine hands on only what it makes, and it makes
/// nothing unless it has a recipe on it and every ingredient of that recipe
/// arriving. A one-feed cell could not tell the difference — its intermediate
/// has one ingredient, and an intermediate with no feeder at all already fails
/// the "taking from something" clause. A **two**-feed cell can: a transport-
/// belt machine fed iron plates and no gears stands, is powered, delivers
/// nothing, and would otherwise have counted as a whole green cell.
///
/// `false` for a machine whose recipe the world does not report, which is the
/// safe direction: an unreadable cell is re-built rather than claimed. That is
/// the same failure `30b28846` fixed at the source, kept honest here as well.
fn is_supplied(
    state: &PlanState,
    nearby: &[FactorioEntity],
    source: &FactorioEntity,
    depth: u8,
) -> bool {
    if source.name != MACHINE {
        return true;
    }
    if depth == 0 {
        return false;
    }
    let Some(recipe) = source
        .recipe
        .as_deref()
        .and_then(|name| recipe_for(state, name))
    else {
        return false;
    };
    loaded_feeders(state, nearby, source, depth - 1) >= ingredients_of(&recipe).len()
}

/// Does `Goal::Producing { item, per_minute }` hold as an *assembly* cell?
///
/// The stage-2 half of the answer `crate::method::have::holds` gives for that
/// goal; [`crate::method::produce::holds_producing`] is the stage-1 half. The
/// two are disjoint by construction — one wants a smelting recipe and the
/// other a crafting one — so no item is answered by both.
///
/// `false` for an item no cell of this shape can make: the arrangement does
/// not exist, which is a fact, not an absence of one.
pub fn holds_assembling(state: &PlanState, item: &str, per_minute: u32) -> bool {
    let Some(spec) = assembly_spec(state, item) else {
        return false;
    };
    match cells_for(per_minute, spec.ticks_per_item) {
        Ok(needed) => cells_standing(state, &spec) >= needed,
        // A rate nothing could build cannot be held by anything standing.
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// What `count` cells need in the acting bot's hands, in emission order.
///
/// `Goal::Have` and not `Goal::Produced`: `Produced` deliberately ignores
/// inventory — that is its entire reason to exist — so a bill written with it
/// re-crafts machines the bot is already carrying, every replan.
///
/// `Holder::Share`, for the same reason `power::bill` uses it: one bot places
/// these, so one bot has to be holding them, and `Anyone` sizes its shortfall
/// against the sum across the roster.
/// **Merged by item, first occurrence keeping its place.** Green science is
/// why: its supply chest is charged with `inserter`s, which is also what three
/// of the cell's own links are made of, and two separate `Goal::Have` subgoals
/// naming the same item are each satisfied by the *same* items in the bot's
/// inventory — so the cell would be built with the inserters its chest was
/// supposed to hold. Nothing in red's bill repeats, which is why the merge
/// changes no plan that already worked.
fn bill(spec: &AssemblySpec, count: u32, poles: u32, coal: u32) -> Vec<(ItemId, u32)> {
    let mut out = vec![
        (MACHINE.to_string(), 2 * count),
        (INSERTER.to_string(), inserter_count(spec) * count),
        (
            CHEST.to_string(),
            (u32::try_from(spec.intermediate.ingredients.len()).unwrap_or(1) + 1) * count,
        ),
    ];
    for (item, amount) in spec.feed_charges() {
        out.push((item, amount.saturating_mul(count)));
    }
    out.push((
        spec.supplied.0.clone(),
        spec.supply_charge().saturating_mul(count),
    ));
    // Only the cells that could not adopt supply that already stands. A pole
    // in a bill nobody places is one of four wood, spent for nothing.
    if poles > 0 {
        out.push((POLE.to_string(), poles));
    }
    if coal > 0 {
        out.push(("coal".to_string(), coal));
    }
    let mut merged: Vec<(ItemId, u32)> = Vec::with_capacity(out.len());
    for (item, amount) in out {
        match merged.iter_mut().find(|(name, _)| *name == item) {
            Some((_, total)) => *total = total.saturating_add(amount),
            None => merged.push((item, amount)),
        }
    }
    merged
}

/// How much coal keeps a network drawing `demand_kw` running for one charge.
///
/// Integer from the first line: the demand is a sum of table constants, so
/// rounding it up to a whole kilowatt before dividing costs at most one coal
/// and removes every float from the answer. Capped at one stack, because a
/// boiler's fuel inventory is **one slot** and asking the game to accept 51
/// coal puts one of them nowhere.
fn boiler_coal(demand_kw: f64) -> u32 {
    let kw = demand_kw.max(0.).ceil().to_u64().unwrap_or(0);
    let kj = kw.saturating_mul(u64::from(CELL_CHARGE_TICKS)) / 60;
    let coal = kj.div_ceil(COAL_KJ);
    u32::try_from(coal).unwrap_or(COAL_STACK).min(COAL_STACK)
}

/// The boiler this cell's power comes out of, if the plan can see one.
///
/// `None` is not a refusal: a world powered by something this planner did not
/// build has no boiler to top up, and the cell is perfectly buildable on it.
/// What `None` costs is the fuel guarantee, and that is stated in
/// [`CELL_CHARGE_TICKS`]'s doc rather than hidden here.
fn boiler_near(state: &PlanState, anchor: &Position) -> Option<Position> {
    let mut candidates: Vec<(f64, Position)> = state
        .entities_within(anchor, BOILER_SEARCH_RADIUS)
        .into_iter()
        .filter(|entity| entity.name == BOILER)
        .map(|entity| {
            (
                calculate_distance(&entity.position, anchor),
                entity.position,
            )
        })
        .collect();
    candidates.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.x.total_cmp(&b.1.x))
            .then(a.1.y.total_cmp(&b.1.y))
    });
    candidates.into_iter().next().map(|(_, position)| position)
}

/// A `Place` step for one part, with its site reserved as it is emitted.
fn place_step(ctx: &mut ExpansionCtx, part: &CellPart) -> Step {
    let entity = entity_for(&ctx.state, part);
    let name = entity.name.clone();
    let position = part.position.clone();
    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);
    // The annulus's inner bound, exactly as the furnace, the lab and the plant
    // use it: standing *on* the tile a building is going for satisfies a plain
    // disc and then has the game refuse the build with
    // `player_blocks_placement`.
    let min_radius = ctx.state.placement_clearance(&name).unwrap_or(0.0);
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
                min_radius,
            },
            Condition::AreaFree {
                pos: position.clone(),
                entity: name.clone(),
                direction: entity.direction,
            },
            Condition::HasItem {
                who: Actor::Role,
                item: name.clone(),
                count: 1,
            },
        ],
        eff: vec![
            Effect::LoseItem {
                who: Actor::Role,
                item: name.clone(),
                count: 1,
            },
            Effect::CreateEntity(Box::new(entity.clone())),
        ],
        duration: PLACE_TICKS,
        pinned: None,
        label: format!("place {} at {}", name, position),
    }));
    ctx.state.create_entity(entity);
    step
}

/// The steps that build `cells`, and the ids a caller must order its own work
/// after.
///
/// Every site is **reserved in `ctx.state` as its `Place` is emitted**, the
/// same discipline `power::plant_steps` uses: `expand` returns its whole step
/// list before `run_steps` executes any of it, so a site left unreserved would
/// be chosen twice by two subtrees of the same plan.
///
/// # Where the structural conditions sit, and why there
///
/// Nothing produces a `Condition::Feeds` or a `Condition::Powered` — no
/// `Effect` satisfies either — so neither orders anything by itself, and
/// neither can be true before the buildings it talks about stand. They ride on
/// the **charge inserts**, the last actions of the cell, alongside a
/// `Condition::EntityAt` for every building they name. Those `EntityAt`s are
/// world-scoped, so `ActionNetwork::infer_edges` draws the edge from each
/// placement on its own, and by the time the scheduler checks a charge insert
/// the whole cell is standing. Charging a chest that feeds nothing is the
/// placed-but-dead machine in its stage-2 form.
///
/// The split between the charge inserts is by *branch*: each feed chest's
/// insert asserts the four-link chain from that chest through the intermediate
/// machine, and the supply chest's asserts the two-link one. All of them name
/// the product machine and its recipe, because all of them are claims about
/// it.
fn cell_steps(
    ctx: &mut ExpansionCtx,
    spec: &AssemblySpec,
    cells: &[Cell],
    coal: u32,
    boiler: Option<Position>,
) -> Result<(Vec<Step>, Vec<ActionId>), PlannerError> {
    let mut steps: Vec<Step> = Vec::new();
    // The actions that cannot run before the network exists: the ones carrying
    // a `Condition::Powered`, which no effect satisfies and which therefore
    // orders nothing by itself.
    let mut needs_power: Vec<ActionId> = Vec::new();
    let count = cells.len() as u32;

    // A recipe the force has not unlocked will not go on a machine, and a
    // trigger technology lands *after* the craft that fires it settles --
    // measured at 16 ticks on a real run. So the condition is stated as well as
    // the subgoal, which is what `await_research` reads off the action.
    let gate_pre = |recipe: &FactorioRecipe, steps: &mut Vec<Step>| -> Vec<Condition> {
        match recipe_gate(&ctx.state, recipe) {
            RecipeGate::NeedsResearch(tech) => {
                steps.push(Step::Subgoal(Goal::Researched(tech.clone())));
                vec![Condition::Researched(tech)]
            }
            RecipeGate::PlannedResearch(tech) => vec![Condition::Researched(tech)],
            RecipeGate::Open | RecipeGate::Unobtainable => Vec::new(),
        }
    };
    let product_gate = gate_pre(&spec.recipe, &mut steps);
    let intermediate_gate = gate_pre(&spec.intermediate.recipe, &mut steps);

    let poles = cells.iter().filter(|cell| cell.brings_pole()).count() as u32;
    for (item, amount) in bill(spec, count, poles, coal) {
        steps.push(Step::Subgoal(Goal::Have {
            item,
            count: amount,
            whose: Holder::Share(ctx.chain_actor),
        }));
    }

    let reach = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.reach_distance)
        .unwrap_or(10.0);

    for cell in cells {
        // Every bystander `fit` found would be sealed in by this cell walks
        // clear before any of the cell's own parts go down -- see
        // `crate::enclosure::check`.
        let evacuation_ids: Vec<ActionId> = cell
            .evacuate
            .iter()
            .map(|evacuation| {
                let (step, id) = crate::method::util::evacuation_step(
                    ctx,
                    evacuation,
                    &format!("the assembly cell at {}", cell.origin),
                );
                steps.push(step);
                id
            })
            .collect();
        let mut part_ids: Vec<ActionId> = Vec::new();
        for part in &cell.parts {
            let step = place_step(ctx, part);
            if let Step::Act(action) = &step {
                part_ids.push(action.id);
            }
            steps.push(step);
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

        let Some(chain) = links(cell) else {
            continue;
        };
        let entity_at = |role: Role| -> Option<Condition> {
            cell.at(role).map(|part| Condition::EntityAt {
                pos: part.position.clone(),
                name: part.role.name().into(),
            })
        };
        // Read before the loop below starts mutating `ctx.state`, and read
        // from the ledger rather than restated: the kilowatts a cell claims
        // are the kilowatts `electric_demand_kw` will bill it.
        let machine_kw = ctx.state.consumer_draw_kw(MACHINE);
        let inserter_kw = ctx.state.consumer_draw_kw(INSERTER);
        let powered = |role: Role| -> Option<Condition> {
            let part = cell.at(role)?;
            let name = part.role.name();
            let kw = match name {
                MACHINE => machine_kw,
                INSERTER => inserter_kw,
                _ => None,
            }?;
            Some(Condition::Powered {
                pos: part.position.clone(),
                entity: name.into(),
                kw,
            })
        };
        let recipe_set = |role: Role, recipe: &str| -> Option<Condition> {
            cell.at(role).map(|part| Condition::RecipeSet {
                pos: part.position.clone(),
                recipe: recipe.to_string(),
            })
        };

        for (role, recipe, gate) in [
            (
                Role::Intermediate,
                &spec.intermediate.recipe,
                &intermediate_gate,
            ),
            (Role::Product, &spec.recipe, &product_gate),
        ] {
            let Some(part) = cell.at(role) else {
                continue;
            };
            let mut pre = vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: part.position.clone(),
                    radius: reach,
                    min_radius: 0.0,
                },
                Condition::EntityAt {
                    pos: part.position.clone(),
                    name: MACHINE.into(),
                },
            ];
            pre.extend(gate.iter().cloned());
            let id = ctx.ids.next();
            steps.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::SetRecipe {
                    pos: part.position.clone(),
                    entity: MACHINE.into(),
                    recipe: recipe.name.clone(),
                },
                pre,
                eff: vec![Effect::SetRecipe {
                    pos: part.position.clone(),
                    recipe: recipe.name.clone(),
                }],
                duration: SET_RECIPE_TICKS,
                pinned: None,
                label: format!("set {} to {}", MACHINE, recipe.name),
            })));
            // Kept in step with the emission, so a second cell's `Powered`
            // and a later `Condition::RecipeSet` both read a state that
            // already carries this recipe. It cannot fail: the machine was
            // created three lines up by `place_step`, and `set_recipe` refuses
            // only empty ground.
            ctx.state.set_recipe(&part.position, &recipe.name)?;
        }

        // One branch per chest, in chest order: each feed chest asserts the
        // whole chain through the intermediate machine, and the supply chest
        // asserts the two-link one. Both name the product machine and its
        // recipe, because both are claims about it.
        let mut branches: Vec<(Role, ItemId, u32, Vec<Role>)> = Vec::new();
        for (index, (item, amount)) in spec.feed_charges().into_iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let index = index as u8;
            branches.push((
                Role::FeedChest(index),
                item,
                amount,
                vec![
                    Role::FeedInserter(index),
                    Role::Intermediate,
                    Role::LinkInserter,
                    Role::Product,
                ],
            ));
        }
        branches.push((
            Role::SupplyChest,
            spec.supplied.0.clone(),
            spec.supply_charge(),
            vec![Role::SupplyInserter, Role::Product],
        ));
        for (chest_role, item, amount, branch) in branches {
            let Some(chest) = cell.at(chest_role) else {
                continue;
            };
            let mut pre = vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: chest.position.clone(),
                    radius: reach,
                    min_radius: 0.0,
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: item.clone(),
                    count: amount,
                },
            ];
            for role in std::iter::once(chest_role).chain(branch.iter().copied()) {
                pre.extend(entity_at(role));
                pre.extend(powered(role));
            }
            // Every link this branch depends on, from the chest to the machine
            // whose output the witness will read.
            let branch_positions: Vec<Position> = std::iter::once(chest_role)
                .chain(branch.iter().copied())
                .filter_map(|role| cell.at(role).map(|part| part.position.clone()))
                .collect();
            for (from, to) in &chain {
                if branch_positions.contains(from) && branch_positions.contains(to) {
                    pre.push(Condition::Feeds {
                        from: from.clone(),
                        to: to.clone(),
                    });
                }
            }
            pre.extend(recipe_set(Role::Product, &spec.recipe.name));
            if branch.contains(&Role::Intermediate) {
                pre.extend(recipe_set(
                    Role::Intermediate,
                    &spec.intermediate.recipe.name,
                ));
            }
            let id = ctx.ids.next();
            needs_power.push(id);
            steps.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::Insert {
                    pos: chest.position.clone(),
                    entity: CHEST.into(),
                    slot: InventorySlot::Chest,
                    item: item.clone(),
                    count: amount,
                },
                pre,
                eff: vec![Effect::LoseItem {
                    who: Actor::Role,
                    item: item.clone(),
                    count: amount,
                }],
                duration: TRANSFER_TICKS,
                pinned: None,
                label: format!(
                    "charge the {} chest with {} {}",
                    chest_role_name(chest_role),
                    amount,
                    item
                ),
            })));
        }
    }

    // The plant's fuel, which is the cell's fuel: a boiler's slot holds one
    // stack and at this cell's draw `power::PLANT_COAL`'s five coal is under
    // two minutes. Topping it up is what makes the difference between a cell
    // that stands and a cell a witness can watch.
    if let (Some(boiler), true) = (boiler, coal > 0) {
        let id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id,
            kind: ActionKind::Insert {
                pos: boiler.clone(),
                entity: BOILER.into(),
                slot: InventorySlot::Fuel,
                item: "coal".into(),
                count: coal,
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
                    count: coal,
                },
            ],
            eff: vec![Effect::LoseItem {
                who: Actor::Role,
                item: "coal".into(),
                count: coal,
            }],
            duration: TRANSFER_TICKS,
            pinned: None,
            label: format!("top the boiler up with {} coal", coal),
        })));
    }

    Ok((steps, needs_power))
}

/// The word a charge-insert label uses for a chest.
fn chest_role_name(role: Role) -> &'static str {
    match role {
        Role::SupplyChest => "supply",
        _ => "feed",
    }
}

// ---------------------------------------------------------------------------
// The method
// ---------------------------------------------------------------------------

/// Build enough assembly cells to produce an item at a rate.
pub struct BuildAssemblyCell;

impl Method for BuildAssemblyCell {
    fn name(&self) -> &'static str {
        "build-assembly-cell"
    }

    /// Claims [`Goal::Producing`] and nothing else, and only when a cell for
    /// the item exists as a *shape*.
    ///
    /// Deliberately no siting here, exactly as in stage 1: `applicable`
    /// answers "can I satisfy this goal at all", and a goal this method
    /// understands but cannot place is a **refusal with a reason** —
    /// `NoRoomForCellNearPower`, or one of the plant's own — which is strictly
    /// better than the `NoApplicableMethod` a false answer here would produce.
    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        match goal {
            Goal::Producing { item, per_minute } => {
                *per_minute > 0 && assembly_spec(state, item).is_some()
            }
            _ => false,
        }
    }

    /// One bot builds one plan's worth of cells.
    ///
    /// Eight buildings, two recipes and two chest charges have to meet in one
    /// pair of hands: three parts of a cell arriving on three bots is a cell
    /// nobody can assemble.
    fn converges(&self, _goal: &Goal, _state: &PlanState) -> bool {
        true
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Producing { item, per_minute } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let spec = assembly_spec(&ctx.state, item)
            .ok_or_else(|| PlannerError::NoCellProduces { item: item.clone() })?;
        let needed = cells_for(*per_minute, spec.ticks_per_item)?;
        // What already stands counts towards the goal, or a replan after a
        // partial build doubles the factory. `AlreadySatisfied` covers only the
        // case where *all* of it stands, and it answers from the very same two
        // functions, so the two cannot disagree about how far along we are.
        let build = needed.saturating_sub(cells_standing(&ctx.state, &spec));
        if build == 0 {
            return Ok(Vec::new());
        }

        let from = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.position.clone())
            .unwrap_or_default();
        // Somewhere with the capacity *left* to run a whole cell -- a network
        // that already stands wherever `power::supply_for` can find one, and
        // only otherwise a plant, built inline the way `Researched` builds one.
        // It has to be inline rather than a subgoal: a subgoal is expanded
        // after this method returns, and the cell's site is chosen from the
        // pole, so a cell planned against a state with no plant in it has
        // nowhere to be.
        //
        // **The adoption tier is not an optimisation.** This search used to
        // stop at `ANCHOR_SEARCH_RADIUS` from the bot and build a plant when it
        // found nothing there, which is how `run-1788408407-02764` came to
        // plan a second offshore pump, boiler and steam engine 86 tiles from a
        // working plant it had just researched `automation` on. See
        // `power::PLANT_ADOPT_RADIUS`.
        let mut plant_steps_taken: Vec<Step> = Vec::new();
        let mut power_links: Vec<ActionId> = Vec::new();
        let want_kw = cell_demand_kw(&ctx.state, &spec) * f64::from(build);
        let anchor = match supply_for(&ctx.state, &from, ANCHOR_SEARCH_RADIUS, want_kw)? {
            Supply::Standing(anchor) => anchor,
            Supply::Build(plant) => {
                let anchor = plant.pole.clone();
                let (built, links) = plant_steps(ctx, &plant);
                plant_steps_taken = built;
                power_links = links;
                anchor
            }
        };
        let cells = plan_cells(&ctx.state, &anchor, &spec, build)?;
        let (coal, boiler) = fuel_for(&ctx.state, &anchor, &cells);
        let (built, needs_power) = cell_steps(ctx, &spec, &cells, coal, boiler)?;
        let mut steps = plant_steps_taken;
        steps.extend(built);
        // Every id, not just the generator's: an engine with no steam produces
        // nothing and a boiler with no water makes no steam, so the cell waits
        // for the whole plant. `plant_steps` returns them all for exactly this
        // reason and says so in its own doc.
        for from in &power_links {
            for to in &needs_power {
                steps.push(Step::Link {
                    from: *from,
                    to: *to,
                    lag: 0,
                });
            }
        }
        Ok(steps)
    }
}

/// How much coal the cells' own network needs for one charge, and where it
/// goes.
///
/// The demand is read off a fork with every cell standing, so it is the whole
/// network's draw — the cells, and whatever else was already on it — rather
/// than the cells' own. A boiler fuelled for the cells alone would run the lab
/// beside them dry.
fn fuel_for(state: &PlanState, anchor: &Position, cells: &[Cell]) -> (u32, Option<Position>) {
    let mut trial = state.fork();
    for cell in cells {
        for part in &cell.parts {
            trial.create_entity(entity_for(state, part));
        }
    }
    let Some(ground) = cells.first().and_then(Cell::on_network_at) else {
        return (0, None);
    };
    let Some(area) = trial.collision_area(MACHINE, ground) else {
        return (0, None);
    };
    let demand = trial.electric_demand_kw(&area, None);
    let boiler = boiler_near(state, anchor);
    match boiler {
        Some(boiler) => (boiler_coal(demand), Some(boiler)),
        None => (0, None),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::method::expand;
    use crate::method::have::registry_for;
    use crate::network::ActionNetwork;
    use factorio_bot_core::factorio::world::FactorioWorld;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    const PACK: &str = "automation-science-pack";
    const GREEN: &str = "logistic-science-pack";

    /// The shared fixture plus the two recipes the 1.1 capture never had.
    ///
    /// Ingredients and energy are the **live 2.1.17** ones, asserted against
    /// the capture in `tests/red_science_cell.rs`. They are added `enabled` so
    /// these tests are about the layout rather than about the research ladder.
    fn world() -> FactorioWorld {
        let world = fixture_world();
        let green: factorio_bot_core::types::FactorioRecipe =
            factorio_bot_core::serde_json::from_str(
                r#"{
              "name": "logistic-science-pack",
              "valid": true,
              "enabled": true,
              "category": "crafting",
              "ingredients": [
                { "name": "transport-belt", "ingredient_type": "item", "amount": 1 },
                { "name": "inserter", "ingredient_type": "item", "amount": 1 }
              ],
              "products": [
                { "name": "logistic-science-pack", "product_type": "item", "amount": 1, "probability": 1.0 }
              ],
              "hidden": false,
              "energy": 6.0,
              "order": "b",
              "group": "intermediate-products",
              "subgroup": "science-pack"
            }"#,
            )
            .expect("the green science recipe parses");
        world
            .update_recipes(vec![green])
            .expect("update_recipes cannot fail for a well-formed recipe");
        let recipe: factorio_bot_core::types::FactorioRecipe =
            factorio_bot_core::serde_json::from_str(
                r#"{
              "name": "assembling-machine-1",
              "valid": true,
              "enabled": true,
              "category": "crafting",
              "ingredients": [
                { "name": "iron-plate", "ingredient_type": "item", "amount": 9 },
                { "name": "iron-gear-wheel", "ingredient_type": "item", "amount": 5 },
                { "name": "electronic-circuit", "ingredient_type": "item", "amount": 3 }
              ],
              "products": [
                { "name": "assembling-machine-1", "product_type": "item", "amount": 1, "probability": 1.0 }
              ],
              "hidden": false,
              "energy": 0.5,
              "order": "a",
              "group": "production",
              "subgroup": "production-machine"
            }"#,
            )
            .expect("the assembling machine recipe parses");
        world
            .update_recipes(vec![recipe])
            .expect("update_recipes cannot fail for a well-formed recipe");
        world
    }

    fn bare(bots: &[BotId]) -> PlanState {
        PlanState::from_world(Arc::new(world()), bots)
    }

    /// A state with a plant standing in the overlay, and one wood per bot.
    ///
    /// The pole and the engine are where `crate::test_world::with_steam_power`
    /// puts them; the boiler is beside them so the top-up has something to
    /// aim at. The wood is the pole's own ingredient, and the fixture has no
    /// players to carry it -- see `tests/red_science_cell.rs` for the cap it
    /// represents.
    fn powered(bots: &[BotId]) -> PlanState {
        let mut state = bare(bots);
        for bot in bots {
            state.gain(*bot, "wood", 1);
        }
        for (name, position) in [
            (POLE, Position::new(10.5, 10.5)),
            ("steam-engine", Position::new(12.5, 10.5)),
            (BOILER, Position::new(12.5, 14.5)),
        ] {
            let entity_type = state
                .base()
                .entity_prototypes
                .get(name)
                .map(|p| p.entity_type.clone())
                .unwrap_or_else(|| name.to_string());
            state.create_entity(FactorioEntity {
                name: name.into(),
                entity_type,
                position,
                ..Default::default()
            });
        }
        state
    }

    fn spec() -> AssemblySpec {
        assembly_spec(&bare(&[BotId(1)]), PACK).expect("red science is a two-ingredient craft")
    }

    /// A cell standing in `state`, built the way the planner would build it,
    /// recipes and all.
    fn stand_a_cell(state: &mut PlanState) -> Cell {
        stand_a_cell_for(state, &spec())
    }

    fn stand_a_cell_for(state: &mut PlanState, spec: &AssemblySpec) -> Cell {
        let cell = plan_cell(state, &Position::new(10.5, 10.5), spec)
            .expect("the fixture has room beside its plant");
        for part in &cell.parts {
            let entity = entity_for(state, part);
            state.create_entity(entity);
        }
        state
            .set_recipe(
                &cell.at(Role::Intermediate).unwrap().position,
                &spec.intermediate.recipe.name,
            )
            .expect("the machine was just placed");
        state
            .set_recipe(&cell.at(Role::Product).unwrap().position, &spec.recipe.name)
            .expect("the machine was just placed");
        cell
    }

    fn plan(per_minute: u32) -> Result<ActionNetwork, PlannerError> {
        let bots = [BotId(1)];
        let state = powered(&bots);
        expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
    }

    // ---- what a cell is ---------------------------------------------------

    /// The shape rule, stated over recipes and checked on four items that are
    /// each a different *reason* to answer.
    #[test]
    fn only_a_two_ingredient_craft_with_exactly_one_makeable_half_is_a_cell() {
        let s = bare(&[BotId(1)]);
        let pack = assembly_spec(&s, PACK).expect("red science is the case this exists for");
        assert_eq!(pack.intermediate.item, "iron-gear-wheel");
        assert_eq!(pack.supplied, ("copper-plate".to_string(), 1));

        // Generality, not a special case: an electronic circuit is one iron
        // plate and three copper cables, and a cable is crafted from one
        // copper plate. Same shape, different numbers.
        let circuit = assembly_spec(&s, "electronic-circuit").expect("a circuit is the same shape");
        assert_eq!(circuit.intermediate.item, "copper-cable");
        assert_eq!(circuit.intermediate.per_product, 3);
        assert_eq!(circuit.intermediate.per_run, 2, "a cable recipe yields two");
        assert_eq!(circuit.supplied, ("iron-plate".to_string(), 1));

        // And the four ways to not be one.
        assert!(
            assembly_spec(&s, "iron-plate").is_none(),
            "a plate is smelted; a stone furnace makes it and stage 1 builds that"
        );
        assert!(
            assembly_spec(&s, "iron-gear-wheel").is_none(),
            "one ingredient, so the cell would have a chest and nothing to put in the other"
        );
        assert!(
            assembly_spec(&s, "lab").is_none(),
            "three ingredients, and the layout has two chests"
        );
        assert!(
            assembly_spec(&s, "transport-belt").is_some(),
            "iron plate plus gear is the same shape as red science and must plan"
        );
        assert!(assembly_spec(&s, "not-a-thing").is_none());
    }

    /// Green science, and the forced choice that makes it a cell.
    ///
    /// Both of its ingredients are craftable, which is what refused it before
    /// `MAX_FEED` was two. Only the belt fits an intermediate machine — an
    /// inserter takes three ingredients and the cell has room for two chests —
    /// so nothing is *chosen* here: the inserters arrive in the supply chest
    /// because there is nowhere else for them to come from.
    #[test]
    fn green_science_puts_the_belt_in_a_machine_and_the_inserters_in_a_chest() {
        let s = bare(&[BotId(1)]);
        let green = assembly_spec(&s, GREEN).expect("green science is a two-feed cell");
        assert_eq!(green.intermediate.item, "transport-belt");
        assert_eq!(
            green.intermediate.ingredients,
            vec![
                ("iron-plate".to_string(), 1),
                ("iron-gear-wheel".to_string(), 1)
            ],
            "one feed chest per ingredient, in the recipe's own order"
        );
        assert_eq!(green.intermediate.per_run, 2, "a belt recipe yields two");
        assert_eq!(
            green.supplied,
            ("inserter".to_string(), 1),
            "the half no machine of this cell makes"
        );
        assert!(
            intermediate_for(&s, "inserter", 1).is_none(),
            "three ingredients is one chest more than one pole can light"
        );

        // 6 s of recipe at crafting speed 0.5 is 720 ticks a pack: five a
        // minute per cell, so six a minute is two cells and not one.
        assert_eq!(green.ticks_per_item, 720);
        assert_eq!(cells_for(5, green.ticks_per_item).unwrap(), 1);
        assert_eq!(cells_for(6, green.ticks_per_item).unwrap(), 2);

        // The charge, integer end to end: twelve packs, twelve inserters by
        // hand, and six runs of a belt recipe that yields two.
        assert_eq!(green.charge_products(), 12);
        assert_eq!(green.intermediate_runs(), 6);
        assert_eq!(
            green.feed_charges(),
            vec![
                ("iron-plate".to_string(), 6),
                ("iron-gear-wheel".to_string(), 6)
            ]
        );
        assert_eq!(green.supply_charge(), 12);
    }

    /// Widening the feed side added rivals for the intermediate and changed no
    /// existing answer.
    ///
    /// `repair-pack` is two gears and two electronic circuits, and a circuit
    /// is a two-ingredient craft that a one-ingredient rule could not see. It
    /// resolved to the gear before the widening and it must resolve to the
    /// gear after it, or every plan for it moves.
    ///
    /// A tie is a refusal rather than a coin toss, which is asserted on a spec
    /// the fixture cannot supply: `transport-belt` and `burner-inserter` are
    /// both iron plate plus gear, so a recipe taking one of each has two
    /// candidates of equal depth and no ground to choose between them.
    #[test]
    fn the_shallower_half_stays_the_intermediate_and_a_tie_is_a_refusal() {
        let s = bare(&[BotId(1)]);
        let pack = assembly_spec(&s, "repair-pack")
            .expect("repair-pack was a cell before MAX_FEED widened");
        assert_eq!(
            pack.intermediate.item, "iron-gear-wheel",
            "the gear, not the deeper circuit"
        );

        let world = world();
        let tie: factorio_bot_core::types::FactorioRecipe =
            factorio_bot_core::serde_json::from_str(
                r#"{
              "name": "a-tie",
              "valid": true,
              "enabled": true,
              "category": "crafting",
              "ingredients": [
                { "name": "transport-belt", "ingredient_type": "item", "amount": 1 },
                { "name": "burner-inserter", "ingredient_type": "item", "amount": 1 }
              ],
              "products": [
                { "name": "a-tie", "product_type": "item", "amount": 1, "probability": 1.0 }
              ],
              "hidden": false,
              "energy": 1.0,
              "order": "z",
              "group": "other",
              "subgroup": "other"
            }"#,
            )
            .expect("the tie recipe parses");
        world
            .update_recipes(vec![tie])
            .expect("a well-formed recipe");
        let tied = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert!(
            assembly_spec(&tied, "a-tie").is_none(),
            "two candidates of equal depth is no ground to choose, so no cell"
        );
    }

    /// Six packs a minute per cell, and the arithmetic that says so.
    #[test]
    fn a_cell_is_six_packs_a_minute_because_the_machine_is_half_speed() {
        let spec = spec();
        // 5 s of recipe divided by the machine's 0.5 crafting speed is 10 s.
        assert_eq!(spec.ticks_per_item, 600, "3600 / 600 = 6 a minute");
        // The gear machine is nowhere near the bottleneck: 0.5 s at speed 0.5
        // is one second a gear against ten seconds a pack.
        assert_eq!(
            smelting_ticks(&bare(&[BotId(1)]), &spec.intermediate.recipe, MACHINE),
            60
        );
        assert_eq!(cells_for(6, spec.ticks_per_item).unwrap(), 1);
        assert_eq!(cells_for(7, spec.ticks_per_item).unwrap(), 2);
        assert_eq!(cells_for(12, spec.ticks_per_item).unwrap(), 2);
    }

    /// The charge, integer end to end and rounded at the run boundary.
    #[test]
    fn the_charge_is_integer_arithmetic_from_the_recipe() {
        let spec = spec();
        assert_eq!(spec.charge_products(), 15, "9000 ticks / 600 a pack");
        assert_eq!(
            spec.feed_charges(),
            vec![("iron-plate".to_string(), 30)],
            "fifteen gears at two iron plates each, in one chest"
        );
        assert_eq!(spec.supply_charge(), 15, "one copper plate a pack");

        // The yield-of-two case, where the intermediate machine and not the
        // product machine is the bottleneck: three cables a circuit at 60
        // ticks per two-cable run is 90 ticks, against the circuit's own 60.
        let s = bare(&[BotId(1)]);
        let circuit = assembly_spec(&s, "electronic-circuit").unwrap();
        assert_eq!(
            circuit.ticks_per_item, 90,
            "the cable machine is the slower half"
        );
        assert_eq!(circuit.charge_products(), 100, "9000 ticks / 90 a circuit");
        assert_eq!(
            circuit.feed_charges(),
            vec![("copper-plate".to_string(), 150)],
            "300 cables is 150 runs, and a run eats one copper plate"
        );
    }

    /// The last run of a charge is paid for in full.
    ///
    /// **No reachable recipe exercises this**, which is why it is asked of a
    /// hand-built spec rather than of red science: a charge of 300 cables is
    /// exactly 150 runs, so floor and ceiling agree and a `div_ceil` dropped
    /// from `feed_charges` would change no plan the game can produce. An odd
    /// count is the case that separates them, and a machine does not run half
    /// a craft.
    #[test]
    fn the_last_run_of_a_charge_is_paid_for_in_full() {
        let s = bare(&[BotId(1)]);
        let mut spec = assembly_spec(&s, PACK).unwrap();
        // 9,000 / 1,000 is nine products, each wanting one intermediate, out
        // of a recipe that makes two per run: five runs, not four and a half.
        spec.ticks_per_item = 1_000;
        spec.intermediate.per_product = 1;
        spec.intermediate.per_run = 2;
        spec.intermediate.ingredients = vec![("iron-plate".to_string(), 3)];
        assert_eq!(spec.charge_products(), 9);
        assert_eq!(
            spec.feed_charges(),
            vec![("iron-plate".to_string(), 15)],
            "five runs of three plates, not four"
        );
    }

    // ---- geometry ---------------------------------------------------------

    #[test]
    fn every_facing_puts_every_building_on_its_own_grid() {
        // The rigid-body claim: a quarter turn about a tile centre takes tile
        // centres to tile centres, so every one of the eight is legal at every
        // facing. A wrong offset moves a building half a tile and the game
        // refuses the placement -- after the bot has walked there.
        let s = bare(&[BotId(1)]);
        for feeds in 1..=MAX_FEED {
            for facing in Direction::orthogonal() {
                let origin = Position::new(10.5, 10.5);
                for part in layout(&origin, facing, true, feeds).expect("a cardinal facing") {
                    let name = part.role.name();
                    let (offset_x, offset_y) = tile_alignment_facing(&s, name, part.direction);
                    assert!(
                        (part.position.x() - offset_x).fract().abs() < 1. / 512.
                            && (part.position.y() - offset_y).fract().abs() < 1. / 512.,
                        "{name} at {} facing {facing:?} with {feeds} feeds is off its own \
                         build grid",
                        part.position
                    );
                }
            }
        }
    }

    #[test]
    fn no_two_buildings_of_a_cell_overlap_at_any_facing() {
        let s = bare(&[BotId(1)]);
        for (feeds, facing) in
            (1..=MAX_FEED).flat_map(|f| Direction::orthogonal().into_iter().map(move |d| (f, d)))
        {
            let parts = layout(&Position::new(10.5, 10.5), facing, true, feeds).unwrap();
            for (i, a) in parts.iter().enumerate() {
                for b in parts.iter().skip(i + 1) {
                    let a_box = s
                        .collision_area_facing(a.role.name(), &a.position, a.direction)
                        .unwrap();
                    let b_box = s
                        .collision_area_facing(b.role.name(), &b.position, b.direction)
                        .unwrap();
                    assert!(
                        a_box.right_bottom.x() <= b_box.left_top.x()
                            || b_box.right_bottom.x() <= a_box.left_top.x()
                            || a_box.right_bottom.y() <= b_box.left_top.y()
                            || b_box.right_bottom.y() <= a_box.left_top.y(),
                        "{:?} and {:?} overlap at {facing:?}",
                        a.role,
                        b.role
                    );
                }
            }
        }
    }

    /// **The inserter-direction claim, at all four facings.**
    ///
    /// Six links for a one-feed cell and eight for a two-feed one, and every
    /// one of them a chance for a rotation that is wrong by a quarter turn to
    /// place perfectly and move nothing. Asked of a fork with the cell
    /// standing, so it is the same `delivers_into` the `Condition::Feeds` on
    /// the charge inserts is checked with.
    #[test]
    fn every_link_of_the_chain_delivers_at_every_facing() {
        for feeds in 1..=MAX_FEED {
            for facing in Direction::orthogonal() {
                let s = bare(&[BotId(1)]);
                let origin = Position::new(10.5, 10.5);
                let cell = Cell {
                    origin: origin.clone(),
                    facing,
                    parts: layout(&origin, facing, true, feeds).unwrap(),
                    lane: lane(&origin, facing).unwrap(),
                    evacuate: Vec::new(),
                };
                let mut trial = s.fork();
                for part in &cell.parts {
                    trial.create_entity(entity_for(&s, part));
                }
                let chain = links(&cell).unwrap();
                assert_eq!(chain.len(), 4 + 2 * feeds, "two links per chest");
                for (from, to) in chain {
                    assert!(
                        trial.delivers_into(&from, &to),
                        "at {facing:?} with {feeds} feeds, {from} does not deliver into {to}"
                    );
                }
            }
        }
    }

    /// An inserter turned round places 100 %, passes every geometry check, and
    /// moves nothing.
    ///
    /// The trap CLAUDE.md paid for twice, asserted as a difference rather than
    /// as a rule: the *only* thing changed here is one inserter's direction,
    /// and the placement stays legal while the link stops holding.
    #[test]
    fn an_inserter_turned_round_places_perfectly_and_feeds_nothing() {
        let roles = (0..MAX_FEED)
            .map(|index| {
                #[allow(clippy::cast_possible_truncation)]
                Role::FeedInserter(index as u8)
            })
            .chain([Role::LinkInserter, Role::SupplyInserter]);
        for role in roles {
            let s = bare(&[BotId(1)]);
            let origin = Position::new(10.5, 10.5);
            let mut parts = layout(&origin, Direction::North, true, MAX_FEED).unwrap();
            for part in parts.iter_mut() {
                if part.role == role {
                    part.direction = compose(part.direction, Direction::South).unwrap();
                }
            }
            let cell = Cell {
                origin: origin.clone(),
                facing: Direction::North,
                parts,
                lane: lane(&origin, Direction::North).unwrap(),
                evacuate: Vec::new(),
            };
            let turned = cell.at(role).unwrap();
            assert!(
                s.is_area_free_facing(turned.role.name(), &turned.position, turned.direction),
                "{role:?} turned round still places: that is the whole trap"
            );
            let mut trial = s.fork();
            for part in &cell.parts {
                trial.create_entity(entity_for(&s, part));
            }
            let broken = links(&cell)
                .unwrap()
                .into_iter()
                .filter(|(from, to)| !trial.delivers_into(from, to))
                .count();
            assert_eq!(
                broken, 2,
                "{role:?} turned round breaks both of its own links and no others"
            );
        }
    }

    /// The servicing lane, asserted as a clearance rather than hoped for.
    ///
    /// Run 30's measurement is the argument: 18 of 18 walk stalls had the
    /// character pressed against a collision box, eight of them at 1/256 of a
    /// tile. A cell whose chests a bot cannot reach is a cell that runs once.
    #[test]
    fn every_lane_tile_admits_a_character() {
        let s = bare(&[BotId(1)]);
        // Half a vanilla character's collision box on each axis, which is what
        // `PlanState` reads out of the `character` prototype.
        let half = 0.19921875_f64;
        let margin = 1. / 64.;
        for (feeds, facing) in
            (1..=MAX_FEED).flat_map(|f| Direction::orthogonal().into_iter().map(move |d| (f, d)))
        {
            let origin = Position::new(10.5, 10.5);
            let parts = layout(&origin, facing, true, feeds).unwrap();
            let lane = lane(&origin, facing).unwrap();
            assert_eq!(lane.len(), 5);
            for tile in &lane {
                for part in &parts {
                    let box_ = s
                        .collision_area_facing(part.role.name(), &part.position, part.direction)
                        .unwrap();
                    let dx = (box_.left_top.x() - tile.x()).max(tile.x() - box_.right_bottom.x());
                    let dy = (box_.left_top.y() - tile.y()).max(tile.y() - box_.right_bottom.y());
                    assert!(
                        dx.max(dy) >= half + margin,
                        "a character standing at {tile} clears {:?} by only {:.6}",
                        part.role,
                        dx.max(dy)
                    );
                }
            }
        }
        // And the lane really does reach both chests: a bot standing on it is
        // within a vanilla reach distance of each.
        let parts = layout(&Position::new(10.5, 10.5), Direction::North, true, MAX_FEED).unwrap();
        let lane = lane(&Position::new(10.5, 10.5), Direction::North).unwrap();
        let chests = (0..MAX_FEED)
            .map(|index| {
                #[allow(clippy::cast_possible_truncation)]
                Role::FeedChest(index as u8)
            })
            .chain([Role::SupplyChest]);
        for role in chests {
            let chest = parts.iter().find(|p| p.role == role).unwrap();
            assert!(
                lane.iter().any(|tile| {
                    factorio_bot_core::factorio::util::calculate_distance(tile, &chest.position)
                        <= 2.
                }),
                "no lane tile is next to the {role:?}"
            );
        }
    }

    /// A pole and a steam engine on one network, with **clear ground around
    /// the pole** for a cell to be sited in.
    ///
    /// The generator is seven tiles south, on a second pole wired to the first
    /// (a small pole reaches 7.5), which is what leaves the first pole's own
    /// supply area empty. That is not contrivance: it is the difference
    /// between a plant whose engine sits in the ground a cell wants and one
    /// whose does not, and `powered()` above is the first case.
    fn powered_with_room(bots: &[BotId]) -> PlanState {
        let mut state = bare(bots);
        for bot in bots {
            state.gain(*bot, "wood", 1);
        }
        for (name, entity_type, position) in [
            (POLE, "electric-pole", Position::new(30.5, 32.5)),
            (POLE, "electric-pole", Position::new(30.5, 39.5)),
            ("steam-engine", "generator", Position::new(32.5, 39.5)),
        ] {
            state.create_entity(FactorioEntity {
                name: name.into(),
                entity_type: entity_type.into(),
                position,
                ..Default::default()
            });
        }
        state
    }

    /// **A cell inside supply that already stands brings no pole of its own.**
    ///
    /// The finding of run `run-1788396958-07935`, as a test. That run
    /// researched `automation` and then refused the cell with `no method can
    /// satisfy goal: have 1 wood (a share sized for bot 1)`: one craft yields
    /// two poles, a replan re-sited the power plant, both of bot 1's went into
    /// the ground, and the planner has no way to hand it one of the three the
    /// other bots were each still carrying. Four bots, four wood, eight poles
    /// ever -- so a pole the cell does not need is not a rounding error.
    #[test]
    fn a_cell_inside_an_existing_supply_area_brings_no_pole_of_its_own() {
        let bots = [BotId(1)];
        let state = powered_with_room(&bots);
        let cell = plan_cell(&state, &Position::new(30.5, 32.5), &spec())
            .expect("there is room beside that pole");
        assert!(
            !cell.brings_pole(),
            "an existing pole covers this cell; bringing another spends a wood to duplicate it"
        );
        assert_eq!(
            cell.parts.len(),
            7,
            "the eight-building cell minus the pole"
        );

        // ...and it is genuinely powered, so this is adoption rather than the
        // check being skipped.
        let mut trial = state.fork();
        for part in &cell.parts {
            trial.create_entity(entity_for(&state, part));
        }
        for (position, name, kw) in consumers(&trial, &cell) {
            assert!(
                (Condition::Powered {
                    pos: position,
                    entity: name.into(),
                    kw,
                })
                .holds(&trial, BotId(0)),
                "{name} is not actually on the network it adopted"
            );
        }

        // The whole point: the bill does not ask for one either.
        let net = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a cell that needs no pole plans");
        // Nothing about a pole anywhere in the plan -- not a placement and not
        // a craft. Asserting only on placements would pass against a bill that
        // still asks for one and simply never puts it down, which spends the
        // wood just the same.
        let mentions: Vec<&str> = net
            .actions()
            .filter(|a| a.label.contains(POLE))
            .map(|a| a.label.as_str())
            .collect();
        assert!(
            mentions.is_empty(),
            "no pole is placed and none is crafted, so no wood is spent: {mentions:?}"
        );
    }

    /// And a cell no existing pole reaches still brings one.
    ///
    /// The control for the test above: in `powered()` the steam engine stands
    /// in exactly the ground a pole-less cell would need, so the cheap pass
    /// finds nothing and the cell pays for its own supply. Without this, the
    /// test above would pass just as well against a planner that never places
    /// a pole at all.
    #[test]
    fn a_cell_no_existing_pole_reaches_brings_one() {
        let cell = plan_cell(&powered(&[BotId(1)]), &Position::new(10.5, 10.5), &spec())
            .expect("the fixture has room for a cell that carries its own pole");
        assert!(cell.brings_pole());
        assert_eq!(cell.parts.len(), 8);
    }

    /// One pole, and it reaches every consumer in the cell.
    ///
    /// The reason the layout is this shape and not a row: a small pole's
    /// supply area is 5x5, and a cell laid out along one axis would need two
    /// poles and two wood. `pole_would_supply` is the game's own overlap rule,
    /// shared with `electric_supply_kw` rather than restated.
    #[test]
    fn the_cells_own_pole_covers_every_consumer_in_it() {
        let s = bare(&[BotId(1)]);
        for (feeds, facing) in
            (1..=MAX_FEED).flat_map(|f| Direction::orthogonal().into_iter().map(move |d| (f, d)))
        {
            let parts = layout(&Position::new(10.5, 10.5), facing, true, feeds).unwrap();
            let pole = parts.iter().find(|p| p.role == Role::Pole).unwrap();
            let consumers = parts
                .iter()
                .filter(|p| s.consumer_draw_kw(p.role.name()).is_some());
            assert_eq!(
                consumers.clone().count(),
                4 + feeds,
                "two machines, one inserter per feed chest, a link and a supply"
            );
            for part in consumers {
                let area = s
                    .collision_area_facing(part.role.name(), &part.position, part.direction)
                    .unwrap();
                assert!(
                    s.pole_would_supply(POLE, &pole.position, &area),
                    "the cell's own pole does not reach its {:?} at {facing:?}",
                    part.role
                );
            }
        }
    }

    // ---- power ------------------------------------------------------------

    /// Two 375 kW machines, wired to the plant or not.
    ///
    /// Placed **21 tiles east** of the anchor deliberately: that is outside
    /// the twelve-tile ring a cell is sited in, so neither variant takes any
    /// ground the cell wants, and inside `POWER_SEARCH_RADIUS`, so both are
    /// visible to the network walk. The only difference between the two is the
    /// pole chain, which is the difference the test is about.
    fn with_a_big_load(state: &mut PlanState, wired: bool) {
        let mut poles = vec![Position::new(31.5, 10.5)];
        if wired {
            // 7 tiles apart, inside a small pole's 7.5 wire reach, so this
            // really is one network with the plant's own pole.
            poles.push(Position::new(17.5, 10.5));
            poles.push(Position::new(24.5, 10.5));
        }
        for position in poles {
            state.create_entity(FactorioEntity {
                name: POLE.into(),
                entity_type: "electric-pole".into(),
                position,
                ..Default::default()
            });
        }
        for y in [7.5, 13.5] {
            state.create_entity(FactorioEntity {
                name: "assembling-machine-3".into(),
                entity_type: "assembling-machine".into(),
                position: Position::new(31.5, y),
                ..Default::default()
            });
        }
    }

    /// Coverage is not capacity, one level up: a network with nothing left
    /// refuses the cell rather than siting it and browning out.
    ///
    /// Two 375 kW machines is 750 kW of the engine's 900, leaving 150 against
    /// a cell's 189. The **control** is the identical load on an island of
    /// poles the plant's own pole does not reach: same entities, same ground,
    /// same distance, and it must still plan — otherwise this test would be
    /// measuring occupancy or search radius rather than headroom.
    #[test]
    fn a_cell_is_refused_on_a_network_that_is_already_spent() {
        let bots = [BotId(1)];

        let mut island = powered(&bots);
        with_a_big_load(&mut island, false);
        assert!(
            plan_cell(&island, &Position::new(10.5, 10.5), &spec()).is_ok(),
            "750 kW on a network the plant does not reach spends none of its budget"
        );

        let mut committed = powered(&bots);
        with_a_big_load(&mut committed, true);
        assert!(
            matches!(
                plan_cell(&committed, &Position::new(10.5, 10.5), &spec()),
                Err(PlannerError::NoRoomForCellNearPower { .. })
            ),
            "the same 750 kW, wired to the plant, leaves 150 of the engine's 900 and a \
             cell needs 189"
        );
    }

    /// A cell that stands on a network with nothing generating on it makes
    /// nothing, and `holds` has to say so.
    ///
    /// The other side of the same coin: a boiler-less, engine-less base reads
    /// as 0 kW, and eight buildings standing on it are eight buildings.
    #[test]
    fn a_cell_that_stands_on_a_dead_network_does_not_hold() {
        let bots = [BotId(1)];
        let mut s = powered(&bots);
        let cell = stand_a_cell(&mut s);
        assert!(
            holds_assembling(&s, PACK, 6),
            "it holds while the engine stands"
        );
        s.remove_entity(&Position::new(12.5, 10.5));
        assert!(
            !holds_assembling(&s, PACK, 6),
            "with the engine gone the cell is eight buildings on a dead wire"
        );
        // ...and the cell itself is untouched, so this is about power and not
        // about the buildings.
        assert!(
            s.entity_at(&cell.at(Role::Product).unwrap().position)
                .is_some()
        );
    }

    /// The kilowatts a cell claims are the kilowatts the ledger bills it.
    ///
    /// A cell that budgeted 75 kW for a machine `electric_demand_kw` charges
    /// 150 for would pass its own check and brown out the network — which is
    /// *coverage is not capacity* with the two halves swapped.
    #[test]
    fn every_powered_condition_states_the_draw_the_ledger_charges() {
        let s = powered(&[BotId(1)]);
        let net = plan(6).expect("a powered fixture can build a cell");
        let mut seen = 0;
        for action in net.actions() {
            for condition in &action.pre {
                if let Condition::Powered { entity, kw, .. } = condition {
                    assert_eq!(
                        Some(*kw),
                        s.consumer_draw_kw(entity),
                        "{entity} is claimed at {kw} kW and billed at something else"
                    );
                    seen += 1;
                }
            }
        }
        assert_eq!(
            seen, 6,
            "the feed branch states four consumers and the supply branch two"
        );
    }

    // ---- what the plan says -----------------------------------------------

    /// Both machines get a recipe, and the charge inserts assert the whole
    /// chain that stands behind them.
    #[test]
    fn the_charge_inserts_assert_the_whole_chain() {
        let net = plan(6).expect("a powered fixture can build a cell");
        let charge: Vec<&Action> = net
            .actions()
            .filter(|a| matches!(&a.kind, ActionKind::Insert { entity, .. } if entity == CHEST))
            .collect();
        assert_eq!(charge.len(), 2, "one insert per chest");
        let feeds: usize = charge
            .iter()
            .flat_map(|a| a.pre.iter())
            .filter(|c| matches!(c, Condition::Feeds { .. }))
            .count();
        assert_eq!(
            feeds, 6,
            "four links on the feed branch and two on the supply branch"
        );
        let recipes: usize = charge
            .iter()
            .flat_map(|a| a.pre.iter())
            .filter(|c| matches!(c, Condition::RecipeSet { .. }))
            .count();
        assert_eq!(
            recipes, 3,
            "both machines on the feed branch, the product on both"
        );
    }

    /// The boiler that runs the cell is topped up, and by how much.
    #[test]
    fn the_plants_boiler_is_topped_up_for_the_charge() {
        let net = plan(6).expect("a powered fixture can build a cell");
        let coal: Vec<u32> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Insert {
                    entity,
                    slot: InventorySlot::Fuel,
                    count,
                    ..
                } if entity == BOILER => Some(*count),
                _ => None,
            })
            .collect();
        // 189 kW for 9000 ticks is 28.35 MJ, and coal is 4 MJ.
        assert_eq!(
            coal,
            vec![8],
            "one top-up, sized from the network's own draw"
        );
    }

    /// The arithmetic on its own, including the stack bound a fuel slot is.
    #[test]
    fn the_coal_bill_is_bounded_by_the_one_slot_it_goes_in() {
        assert_eq!(boiler_coal(0.), 0);
        assert_eq!(boiler_coal(189.), 8);
        assert_eq!(boiler_coal(900.), 34);
        assert_eq!(
            boiler_coal(100_000.),
            COAL_STACK,
            "a boiler's fuel inventory is one slot; the 51st coal goes nowhere"
        );
    }

    /// A world with no boiler is planned, and asks for no coal.
    #[test]
    fn a_world_whose_power_this_planner_did_not_build_asks_for_no_coal() {
        let bots = [BotId(1)];
        let mut state = bare(&bots);
        state.gain(BotId(1), "wood", 1);
        for (name, entity_type, position) in [
            (POLE, "electric-pole", Position::new(10.5, 10.5)),
            ("steam-engine", "generator", Position::new(12.5, 10.5)),
        ] {
            state.create_entity(FactorioEntity {
                name: name.into(),
                entity_type: entity_type.into(),
                position,
                ..Default::default()
            });
        }
        let net = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("no boiler is not a refusal");
        assert!(
            !net.actions().any(|a| matches!(
                &a.kind,
                ActionKind::Insert { entity, .. } if entity == BOILER
            )),
            "there is no boiler to top up, and inventing one would be worse than not"
        );
    }

    // ---- what already stands ----------------------------------------------

    #[test]
    fn a_cell_that_stands_with_its_recipes_on_it_holds() {
        let mut s = powered(&[BotId(1)]);
        assert!(
            !holds_assembling(&s, PACK, 6),
            "nothing stands yet; a goal that held here would report a factory on bare ground"
        );
        stand_a_cell(&mut s);
        assert!(holds_assembling(&s, PACK, 6));
        assert!(
            !holds_assembling(&s, PACK, 7),
            "one cell is six a minute, and seven needs two"
        );
        // And the wiring: `Goal::Producing` reaches this through
        // `have::holds`, which used to answer only stage 1's shape. A
        // `holds` that missed this one would let `AlreadySatisfied` rebuild a
        // standing factory every replan.
        assert_eq!(
            crate::method::have::holds(
                &Goal::Producing {
                    item: PACK.into(),
                    per_minute: 6,
                },
                &s
            ),
            Some(true)
        );
    }

    /// A machine with no recipe on it is the placed-but-dead machine of stage 2.
    #[test]
    fn a_cell_whose_product_machine_has_no_recipe_does_not_hold() {
        let mut s = powered(&[BotId(1)]);
        let spec = spec();
        let cell = plan_cell(&s, &Position::new(10.5, 10.5), &spec).unwrap();
        for part in &cell.parts {
            let entity = entity_for(&s, part);
            s.create_entity(entity);
        }
        s.set_recipe(
            &cell.at(Role::Intermediate).unwrap().position,
            &spec.intermediate.recipe.name,
        )
        .unwrap();
        assert!(
            !holds_assembling(&s, PACK, 6),
            "every building stands and the product machine is empty; it makes nothing"
        );
    }

    /// One inserter per ingredient, and each of them fed by something.
    #[test]
    fn a_product_machine_short_of_a_feeder_does_not_count() {
        for missing in [Role::SupplyInserter, Role::SupplyChest, Role::LinkInserter] {
            let mut s = powered(&[BotId(1)]);
            let cell = stand_a_cell(&mut s);
            assert!(holds_assembling(&s, PACK, 6), "the whole cell holds first");
            s.remove_entity(&cell.at(missing).unwrap().position);
            assert!(
                !holds_assembling(&s, PACK, 6),
                "a cell with no {missing:?} is fed by fewer things than the recipe has \
                 ingredients, and makes nothing"
            );
        }
    }

    /// **The same cell, arriving from the world instead of from this plan.**
    ///
    /// Every other test here stands its cell with `PlanState::create_entity`
    /// and `PlanState::set_recipe`, which write the overlay. A *replan* has no
    /// overlay: `PlanState::from_world` starts empty and everything standing
    /// comes back out of `FactorioWorld`'s entity graph. So the overlay tests
    /// could all pass while the predicate was unsatisfiable against a real
    /// world, and that is exactly what happened -- in `run-1788485718-45723`
    /// four consecutive replans each built a whole new cell, every action
    /// succeeded, and the goal was never met, because a recipe set over RCON
    /// had no route into the entity graph and every stored machine read
    /// `recipe: None`.
    ///
    /// This test is the seam: the cell is pushed into the base world through
    /// `update_chunk_entities`, the same door the mod's entity events use.
    #[test]
    fn a_cell_the_world_reports_holds_as_well_as_one_this_plan_built() {
        let bots = [BotId(1)];
        let spec = spec();

        // Lay the cell out exactly as the planner would, then take its parts.
        let mut planned = powered(&bots);
        let cell = stand_a_cell(&mut planned);
        let recipes = cell_recipes(&cell, &spec);

        // A world that has never heard of this plan, told only what a game
        // would have told it.
        let observed = |with_recipes: bool| {
            observed_world(
                &planned,
                &cell.parts,
                if with_recipes { &recipes } else { &[] },
                &bots,
            )
        };

        assert!(
            !holds_assembling(&observed(false), PACK, 6),
            "machines with no recipe on them are the placed-but-dead cell, however \
             complete the rest of it is -- this is the state every replan used to see"
        );
        assert!(
            holds_assembling(&observed(true), PACK, 6),
            "the cell stands, is powered, is fed, and the world knows what each \
             machine is set to; a replan that rebuilt it would build a second factory"
        );
    }

    /// The recipes a standing cell has on it, as the entity graph takes them.
    fn cell_recipes(cell: &Cell, spec: &AssemblySpec) -> Vec<(Position, String)> {
        vec![
            (
                cell.at(Role::Intermediate).unwrap().position.clone(),
                spec.intermediate.recipe.name.clone(),
            ),
            (
                cell.at(Role::Product).unwrap().position.clone(),
                spec.recipe.name.clone(),
            ),
        ]
    }

    /// A `PlanState` built the way a **game** builds one.
    ///
    /// `parts` go in through `update_chunk_entities` — the door the mod's
    /// entity events use — and the recipes go on through the entity graph,
    /// not through a plan's overlay. That distinction is the whole point:
    /// `30b28846` was three complete cells producing science while
    /// `cells_standing` read zero, and every test that missed it stood its
    /// cell through the overlay, which a replan never has.
    fn observed_world(
        planned: &PlanState,
        parts: &[CellPart],
        recipes: &[(Position, String)],
        bots: &[BotId],
    ) -> PlanState {
        let mut standing: Vec<FactorioEntity> =
            parts.iter().map(|part| entity_for(planned, part)).collect();
        for (name, position) in [
            (POLE, Position::new(10.5, 10.5)),
            ("steam-engine", Position::new(12.5, 10.5)),
            (BOILER, Position::new(12.5, 14.5)),
        ] {
            let world = world();
            let entity_type = world
                .entity_prototypes
                .get(name)
                .map(|p| p.entity_type.clone())
                .unwrap_or_else(|| name.to_string());
            standing.push(FactorioEntity {
                name: name.into(),
                entity_type,
                bounding_box: planned
                    .collision_area(name, &position)
                    .expect("the fixture sizes everything it places"),
                position,
                ..Default::default()
            });
        }
        // A bounding box, because `EntityGraph::add` drops anything with a
        // zero-width one -- the overlay needs none and the graph does.
        for entity in &mut standing {
            if entity.bounding_box.width() == 0.
                && let Some(area) = planned.collision_area(&entity.name, &entity.position)
            {
                entity.bounding_box = area;
            }
        }
        let world = world();
        world
            .update_chunk_entities(standing)
            .expect("a fixture world accepts a cell");
        for (position, recipe) in recipes {
            assert!(
                world.entity_graph.set_recipe(position, recipe),
                "the world has a machine at {position} to set to {recipe}"
            );
        }
        PlanState::from_world(Arc::new(world), bots)
    }

    /// **The same seam, for the shape green science added**: a cell whose
    /// intermediate machine has two mouths.
    ///
    /// Two claims, and only the second is new. A green cell the *world*
    /// reports — recipes set through the entity graph, entities through
    /// `update_chunk_entities` — has to hold, or every replan builds another
    /// one. And a green cell **short one feed chest** must not: it stands, it
    /// is powered, both recipes are on it and every inserter is turned the
    /// right way, and it produces nothing forever, because a transport belt is
    /// not made of iron plates alone.
    ///
    /// A one-feed cell cannot express that second case at all — its
    /// intermediate has one ingredient, so losing its chest also loses the
    /// "taking from something" clause that has always been checked. Widening
    /// the feed side is what made [`is_supplied`] load-bearing.
    #[test]
    fn a_green_cell_holds_at_the_seam_and_a_half_fed_one_does_not() {
        let bots = [BotId(1)];
        let spec = assembly_spec(&bare(&bots), GREEN).expect("green science is a cell");
        assert_eq!(spec.intermediate.ingredients.len(), 2, "two feed chests");

        let mut planned = powered(&bots);
        let cell = stand_a_cell_for(&mut planned, &spec);
        assert_eq!(cell.feeds(), 2);
        let recipes = cell_recipes(&cell, &spec);

        // One cell is five packs a minute, which is what `cells_for` says.
        assert!(
            holds_assembling(
                &observed_world(&planned, &cell.parts, &recipes, &bots),
                GREEN,
                5
            ),
            "a whole green cell the world reports must hold, or a replan doubles it"
        );

        // The same cell with the gear chest never built. Everything else is
        // identical, down to the tile.
        let starved: Vec<CellPart> = cell
            .parts
            .iter()
            .filter(|part| part.role != Role::FeedChest(1))
            .cloned()
            .collect();
        assert_eq!(starved.len(), cell.parts.len() - 1);
        assert!(
            !holds_assembling(
                &observed_world(&planned, &starved, &recipes, &bots),
                GREEN,
                5
            ),
            "a belt machine with no gears arriving makes no belts, and a cell whose \
             intermediate makes nothing makes no science"
        );
    }

    /// The plan a green goal produces, at the shape rather than the tile.
    #[test]
    fn a_green_plan_builds_two_cells_and_charges_three_chests_each() {
        let bots = [BotId(1)];
        let state = powered_with_room(&bots);
        let net = expand(
            &[Goal::Producing {
                item: GREEN.into(),
                per_minute: 5,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("green science plans");

        let placed: Vec<&str> = net
            .actions()
            .filter_map(|action| match &action.kind {
                ActionKind::Place { entity } => Some(entity.name.as_str()),
                _ => None,
            })
            .collect();
        let count = |name: &str| placed.iter().filter(|n| **n == name).count();
        assert_eq!(count(MACHINE), 2, "an intermediate and a product machine");
        assert_eq!(count(CHEST), 3, "two feed chests and one supply chest");
        assert_eq!(count(INSERTER), 4, "two feed, one link, one supply");

        // Three charges, one per chest, and the supply chest's is inserters.
        let charges: Vec<(String, u32)> = net
            .actions()
            .filter_map(|action| match &action.kind {
                ActionKind::Insert {
                    entity,
                    item,
                    count,
                    ..
                } if entity == CHEST => Some((item.clone(), *count)),
                _ => None,
            })
            .collect();
        assert_eq!(
            charges,
            vec![
                ("iron-plate".to_string(), 6),
                ("iron-gear-wheel".to_string(), 6),
                ("inserter".to_string(), 12),
            ],
            "one charge per chest, in chest order"
        );
    }

    /// A cell that stands is not built twice.
    #[test]
    fn a_replan_against_a_standing_cell_builds_nothing() {
        let bots = [BotId(1)];
        let mut s = powered(&bots);
        stand_a_cell(&mut s);
        let net = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a satisfied goal is not a refusal");
        assert_eq!(net.len(), 0, "the cell already stands");
    }

    // ---- determinism ------------------------------------------------------

    /// The same state plans the same cell, down to the tile and the facing.
    #[test]
    fn the_same_state_sites_the_same_cell_twice() {
        let a = plan_cell(&powered(&[BotId(1)]), &Position::new(10.5, 10.5), &spec()).unwrap();
        let b = plan_cell(&powered(&[BotId(1)]), &Position::new(10.5, 10.5), &spec()).unwrap();
        assert_eq!(a, b);
    }

    /// Two cells do not land on one another, and the second is sized against a
    /// network the first has already spent capacity on.
    #[test]
    fn a_second_cell_stands_clear_of_the_first() {
        let cells = plan_cells(
            &powered(&[BotId(1)]),
            &Position::new(10.5, 10.5),
            &spec(),
            2,
        )
        .expect("the fixture has room for two");
        assert_eq!(cells.len(), 2);
        assert_ne!(cells[0].origin, cells[1].origin);
        let s = bare(&[BotId(1)]);
        for a in &cells[0].parts {
            for b in &cells[1].parts {
                let a_box = s
                    .collision_area_facing(a.role.name(), &a.position, a.direction)
                    .unwrap();
                let b_box = s
                    .collision_area_facing(b.role.name(), &b.position, b.direction)
                    .unwrap();
                assert!(
                    a_box.right_bottom.x() <= b_box.left_top.x()
                        || b_box.right_bottom.x() <= a_box.left_top.x()
                        || a_box.right_bottom.y() <= b_box.left_top.y()
                        || b_box.right_bottom.y() <= a_box.left_top.y(),
                    "two cells overlap at {:?} / {:?}",
                    a.role,
                    b.role
                );
            }
        }
    }
}
