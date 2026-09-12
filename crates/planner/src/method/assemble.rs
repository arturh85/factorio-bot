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
//!   geometry check, and produces nothing at all;
//! * **and something has to take the product away.** An assembling machine
//!   stops when its own output slot backs up, and for a machine making one
//!   item per craft that is *three or four items* -- not a stack, and not a
//!   minute's work. See [`Role::OutputInserter`].
//!
//! # The shape, and what it is general over
//!
//! ```text
//!   iron-plate belt  -> inserter -> [intermediate] -> inserter -> [product] <- inserter <- copper-plate belt
//!  (from a standing                                                  |               (from a standing
//!   stage-1 cell)                                                    v                stage-1 cell)
//!                                                   output chest <- inserter
//! ```
//!
//! Red science is one copper plate and one iron gear wheel. The gear is
//! *craftable*, so the cell builds a machine for it; the plates are
//! **smelted**, so no assembling machine makes one -- and since 2026-09-09
//! **a smelted ingredient arrives on a belt, straight into the machine that
//! eats it, from a standing stage-1 cell** ([`AssemblySpec::belted`],
//! [`Mouth`], [`belted_link_steps`]). There is no chest in the line: owner,
//! *"inside a factory every chest would be a huge bottleneck because the
//! slow inserters at the beginning are way slower than a belt"*. A cell
//! asked for with no such source is refused by name
//! ([`crate::error::PlannerError::AssemblyNoStandingSource`]); the script
//! composes `goal.all{ sustain(iron), sustain(copper), producing(red) }`.
//! That is the rule [`assembly_spec`] applies, and it is stated over the
//! world's recipes rather than over the name `automation-science-pack`: a
//! two-ingredient crafting recipe, exactly one of whose ingredients has a
//! crafting recipe of its own with at most [`MAX_FEED`] ingredients.
//!
//! **A chest stands only where nothing here can belt** -- green's gears and
//! inserters, which are crafted rather than smelted -- and those are still
//! charged by hand, exactly as below. That is the phase-2 gap the design
//! note names (`docs/superpowers/notes/2026-09-09-no-chests-in-the-line.md`).
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
//! ingredients, which is one chest more than the pole can light *on the
//! intermediate's side*), so the choice is forced rather than picked -- and
//! the consequence is stated plainly: **the inserters green science eats
//! arrive in a chest, hand-crafted by a bot, exactly as red's copper plates
//! do.** A green cell automates the belts and the packs; it does not automate
//! the inserters. That is a first green cell, not a green factory, and
//! [`CELL_CHARGE_TICKS`] is how long it runs before somebody fills it again.
//!
//! The two sides are not symmetric, and the asymmetry has since been spent.
//! The product machine has a **third** powered mouth at north-frame `(-2, 3)`,
//! drawing from free ground at `(-3, 3)` that the existing lane already
//! services, which the layout used to leave empty. **The output inserter and
//! its chest stand there now**, so the third mouth is no longer spare and
//! "a three-ingredient product costs no second pole" is **no longer true**:
//! a second supply chest and the output path want the same tile, and the
//! output path is not optional.
//! `tests::the_pole_lights_the_output_mouth_but_no_third_feed_row` pins both
//! halves at all four facings, because the difference has already been
//! misread once: an `inserter`-producing cell composed onto this one was
//! proposed as a way to "avoid the pole geometry entirely", and it does not --
//! it moves the third ingredient from the intermediate to the product, which
//! used to be the half with room.
//!
//! What such a composition would *not* buy is worth writing down beside it.
//! An inserter cell's own three inputs are hand-filled chests, so it converts
//! "fill one chest with twelve inserters" into "fill three chests" and adds a
//! machine, three chests and four inserters of build cost. **Chest-fed cells
//! compose into more hand-fill points, never fewer.** Autonomy comes from
//! attaching an input to something that renews -- a drill on ore, a furnace --
//! which is `crate::method::produce`'s shape, not from stacking another
//! charged cell behind this one.
//!
//! # What this claims, and what it deliberately does not
//!
//! The same split stage 1 states: **the planner answers structure** — do the
//! machines stand, hold the right recipes, feed one another, and sit on a
//! network with the capacity to spare — and **the supervisor answers
//! duration**, by witnessing packs appear while every bot stands still. A
//! structurally satisfied cell can still be a cell whose chests have run out
//! (this crate reads no container contents) or whose boiler has run dry
//! (`electric_supply_kw` counts nameplate).
//!
//! **"Whose output has backed up" used to be on that list, and it should never
//! have been.** A backed-up output is not a state a cell drifts into after a
//! while; it is where a cell with no output inserter arrives after four crafts
//! and stays. That is structure, not duration, so it is answered here -- by
//! the [`Role::OutputInserter`] every cell now places and by the drain clause
//! [`cells_standing`] now requires -- rather than deferred to a witness that
//! cannot see it either.
//!
//! And one more, stated plainly because the goal's name invites the opposite
//! reading: **what chests remain are filled by hand.** A chest-fed ingredient
//! is charged with [`CELL_CHARGE_TICKS`] worth when the cell is built and
//! nothing refills it. A belted one has no charge at all -- not even an
//! ignition charge, which is measured before it is added back (see
//! [`belted_link_steps`]) -- and the cell is bounded by its sources' ore
//! instead ([`SupplyHorizon`]).
//!
//! What that does **not** do is make a cell autonomous on its own: the goal
//! has to have been asked in a world where the sources stand, which today
//! means composing it (`goal.all{sustain(<plate>, ...), producing(...)}`), and
//! `Researched` asks for a cell only where they do
//! (`crate::method::have::machine_made_packs`).

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ActionId, BotId, ItemId, Ticks};
use crate::method::connect::connect_steps_reserving;
use crate::method::have::{COAL_BURN_TICKS, fuel_visits};
use crate::method::have::{
    HANDOVER_WALK_TICKS, PLACE_TICKS, TRANSFER_TICKS, participants_that_can_work,
};
use crate::method::power::{POLE, supply_anchor};
use crate::method::produce::{FURNACE, cell_spec, cells_for, fuel_for_duration};
use crate::method::util::{
    BEACON, CRAFTING_CATEGORY, RecipeGate, SMELTING_CATEGORY, beacon_geometry, ingredients_of,
    output_per_craft, recipe_for, recipe_gate, smelting_ticks, tile_alignment,
    tile_alignment_facing,
};
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::{FromPrimitive, ToPrimitive};
use factorio_bot_core::types::{Direction, FactorioEntity, FactorioRecipe, Pos, Position};
use std::collections::{BTreeMap, BTreeSet};

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
///
/// **This is a bound on the feed side alone.** The product machine's west face
/// has a third mouth inside the same pole's area, at `(-2, 3)`, which the
/// layout left empty until the cell grew an output path -- so the same pole
/// that refuses a third feed chest lights [`Role::OutputInserter`]. Both facts
/// are checked together in
/// `tests::the_pole_lights_the_output_mouth_but_no_third_feed_row`, so a
/// change to [`POLE_OFFSET`] cannot quietly make either of them wrong.
///
/// That mouth is now **taken**. A three-ingredient product -- one made by the
/// intermediate, two arriving in chests -- would want it for a second supply
/// chest, and it cannot have it: an unremoved product jams the machine after
/// four crafts, so the output path outranks a second supply chest for the one
/// tile they both need.
///
/// # It is an artifact of CHESTS, and does not survive a belt-fed layout
///
/// Everything above is a correct proof about **this** design and says nothing
/// about Factorio. It is chest-per-ingredient: `k` ingredients means `k`
/// chests, `k` inserters, and therefore `k` powered mouths spread **along**
/// the machine's face -- one per row. A `small-electric-pole` covers five
/// tile-rows (`supply_area_distance` 2.5, read off the live prototype), the
/// cell already spends five on feed/link/output/supply, and a sixth is the
/// wall. That is where the two comes from.
///
/// **Belt-feeding stacks the ingredients perpendicular to the face instead**,
/// and then the row budget stops binding. Derived from
/// [`crate::state`]'s prototype-fed reach (2026-09-08), counting rows outward
/// from the machine face:
///
/// ```text
///   row 0  the machine's own edge tile
///   row 1  inserter row     normal reaches row 2 (1 tile)
///   row 1  inserter row     long   reaches row 3 (2 tiles)
///   row 2  inserter row     long   reaches row 4, dropping into row 0
///   row 2                   normal drops into row 1, NOT the machine
/// ```
///
/// So **one machine face serves three belt rows out of two tile-rows of
/// inserter space**, and there is no `normal@2` because it cannot reach the
/// machine. That is an independent derivation of a rule measured over 4,300
/// machine-serving inserters in the world-record base with zero exceptions,
/// where `long-handed-inserter` is 20.7 % of all inserters and 0 of 1,172
/// touch a furnace -- it is purely an assembly tool.
///
/// Two consequences worth stating plainly:
///
/// - **The consumers are two rows deep whatever `k` is**, so a single small
///   pole covers them with three rows to spare. The pole was never a bound on
///   arity; it was a bound on *chests*.
/// - **`long@1` drops two tiles inward**, which lands inside a 3x3
///   `assembling-machine-1` (`collision_box` +/-1.199) and would **overshoot a
///   1x1 or 2x2**. A layout using reach 2 has to check the drop lands in the
///   footprint; nothing does today.
///
/// And the ceiling people expect from Factorio 1.x is **gone**: there is no
/// `ingredient_count` on any prototype in this install (0 files under
/// `workspace/data/base/prototypes`, against a `crafting_speed` control that
/// hits), so an `assembling-machine-1` will run a three-ingredient recipe.
///
/// **This constant is therefore right where it stands and must not be raised
/// on the strength of the paragraph above.** Widening it widens the *chest*
/// layout, which the pole genuinely refuses. The belt-fed layout is a
/// different method, sequenced separately, and it needs a lane model that
/// does not exist -- `graph::route` and [`super::connect`] contain the word
/// "lane" zero times, so which of a belt's two lines a commodity rides is
/// unplanned, and getting it wrong places 100 % and moves one plate.
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
/// `crate::method::power::layout` builds the pump, the boilers, the engines and
/// the pole as one rigid body a handful of tiles across, so a boiler within
/// sixteen tiles of the pole a cell hangs off is that plant's boiler. It is a
/// heuristic and it is named as one: a map with an unrelated boiler nearer
/// than the plant's would have the wrong one topped up. The top-up is coal
/// into a fuel slot, so the cost of being wrong is a few coal in the wrong
/// machine rather than a wrong plan.
const BOILER_SEARCH_RADIUS: f64 = 16.0;

/// How far past the nearest boiler [`boilers_near`] keeps collecting, in
/// tiles.
///
/// **A plant is a chain now, and this is its length.** Since 2026-09-06
/// `power::layout` stands up to `power::BOILERS_PER_PUMP` boilers end to end
/// along the shore at `power::BOILER_PITCH_TILES`, so the last boiler of a
/// full chain is `4 x 19 = 76` tiles from the first. A search that stopped at
/// [`BOILER_SEARCH_RADIUS`] would find the near end of a long plant and fuel
/// that, leaving the far end cold -- everything standing, everything wired,
/// and a fraction of the nameplate delivered.
///
/// It is deliberately measured **from the nearest boiler**, not from the
/// anchor: widening `BOILER_SEARCH_RADIUS` itself to 92 tiles would start
/// sweeping in unrelated plants (`run-1788408407-02764`'s second pump stood 86
/// tiles from the first, and a 92-tile disc from a cell between them reaches
/// both). Anchoring on the nearest boiler and reaching one chain-length past
/// it keeps the group to one plant on every map where two plants are further
/// apart than a plant is long -- which is the case this planner creates,
/// since `power::PLANT_ADOPT_RADIUS` is 256.
const BOILER_CHAIN_SPAN: f64 = crate::method::power::BOILER_PITCH_TILES
    * ((crate::method::power::BOILERS_PER_PUMP - 1) as f64);

/// What the boiler burns, in kJ per unit, and how many of them fit in its one
/// fuel slot.
///
/// Coal is 4 MJ and stacks to 50, so a full slot is 200 MJ.
///
/// # These are readable, and this comment used to say they were not
///
/// It said "**the mod does not send `fuel_value` or `stack_size`**, so neither
/// number can come out of the world", and that was false in both halves.
/// `serialize_item_prototype` (`mods/BotBridge/types.lua`) sends both,
/// `FactorioItemPrototype` (`crates/core/src/types.rs`) carries both, and
/// `workspace/scripts/map.json` has them for all 342 item prototypes — coal's
/// `fuel_value` is `4000000` and its `stack_size` is `50`, which is exactly
/// these two constants. So the numbers are right and the reason given for
/// hardcoding them was not.
///
/// `COAL_STACK` is now what [`boiler_coal`] falls back to rather than what it
/// caps with: the cap comes from `slot_capacity(InventorySlot::Fuel, "coal")`,
/// and the constant answers for a world with no coal prototype. That is
/// deliberately *not* the "unknown means unbounded" rule the rest of the
/// capacity work follows, and the reason is that this cap only ever makes the
/// bill smaller -- a fixture that fell back to unbounded would ask a boiler
/// for more coal than a slot holds, which is the defect, not the fallback.
///
/// `COAL_KJ` is still a constant. Replacing it with `fuel_value / 1000` needs
/// an answer for an unknown fuel that changes plan *arithmetic* rather than
/// bounding it, so it stays out of scope here.
///
/// The precedent this comment used to invoke does not say what it was quoted
/// as saying: `COAL_BURN_TICKS`' own doc names the machine's `energy_usage` as
/// the field the mod withholds, and that is correct and is a different field.
/// `fuel_value` and `stack_size` were never in that sentence.
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
    /// Additional ingredients for recipes with 3+ inputs (chemical-science-pack).
    pub extra_supplied: Vec<(ItemId, u32)>,
    /// The ingredient it can, and the machine that does it — `None` for a
    /// **one-machine** cell, whose single machine is fed from the supply chest
    /// and nothing else.
    ///
    /// A steel furnace is the case this became an `Option` for: `steel-plate`
    /// is five iron plates and nothing else, so there is no second ingredient
    /// for the cell to build a machine for and no link inserter to carry it.
    /// The layout is the same body with the intermediate half left out — see
    /// [`layout_table`].
    pub intermediate: Option<Intermediate>,
    /// What the product machine is.
    ///
    /// [`MACHINE`] for a **crafting** recipe and [`FURNACE`] for a
    /// **smelting** one. It is a field rather than a constant because the two
    /// differ in more than a name: a furnace is 2x2 and an assembling machine
    /// 3x3 (see [`AssemblySpec::product_offset`]), a furnace burns coal where
    /// an assembling machine draws kilowatts, and a furnace **takes no
    /// recipe** at all (see [`AssemblySpec::sets_recipe`]).
    pub machine: &'static str,
    /// Where [`Role::Product`] stands, as a north-frame offset from the
    /// cell's origin.
    ///
    /// Derived from the machine's own [`tile_alignment`], never chosen. The
    /// origin is a tile centre, so a 3x3 machine sits at `(0, 4)` and a 2x2
    /// one at `(-0.5, 3.5)` — the north-west 2x2 of the same 3x3 footprint,
    /// which is what keeps [`Role::SupplyInserter`]'s drop tile and
    /// [`Role::OutputInserter`]'s pickup tile inside the machine at both
    /// sizes. Both components are half-integers or both are integers, so a
    /// quarter turn about the origin preserves the machine's build grid
    /// exactly as it does for every other part.
    pub product_offset: (f64, f64),
    /// Ticks per product for the whole cell: the **slower** of its two
    /// machines.
    ///
    /// Integer, and the whole reason [`Goal::Producing`] carries a `u32`
    /// rather than an `f64`: the cell count is
    /// `ceil(per_minute * ticks_per_item / 3600)`, exact integer arithmetic
    /// end to end, so it cannot land on a different side of a ceiling on a
    /// different run.
    pub ticks_per_item: Ticks,
    /// The ingredients that arrive on a **belt**, straight into the machine
    /// that eats them, rather than in a chest a bot fills.
    ///
    /// **Every smelted one** -- every ingredient a stage-1 cell
    /// ([`crate::method::produce::cell_spec`]) can make out of ore -- and it
    /// is decided from the world's recipes, never from what happens to be
    /// standing, so the layout of a cell for an item is one layout on every
    /// plan and every replan. Red science is the case: its copper plate and
    /// the iron plate behind its gears are both here, so a red cell has
    /// **no chest but the output one**. Green's gears and inserters are
    /// crafted, not smelted, and nothing in this method makes them, so they
    /// stay in hand-filled chests -- see [`Role::FeedChest`].
    ///
    /// What a belted ingredient costs the plan is a standing source: a cell
    /// asked for with none is refused by name
    /// ([`PlannerError::AssemblyNoStandingSource`]) rather than planned as a
    /// cell that dies when a hand charge runs out.
    pub belted: BTreeSet<ItemId>,
}

/// One ingredient a cell takes on a belt: what it is, which machine eats it,
/// and how much one product of the cell costs in it.
#[derive(Clone, Debug, PartialEq)]
pub struct BeltedInput {
    pub item: ItemId,
    /// [`Role::Intermediate`] or [`Role::Product`].
    pub into: Role,
    /// Which mouth of the machine it enters by: the feed row for an
    /// intermediate's ingredient (its index in that recipe), `None` for the
    /// product machine's own supplied ingredient.
    pub feed_row: Option<u8>,
    /// Units of the item per product of the cell, rounded up at the run
    /// boundary the same way [`AssemblySpec::intermediate_runs`] rounds.
    pub per_product: u32,
}

impl AssemblySpec {
    /// How many products one hand charge is sized for.
    ///
    /// **Only a chest-fed ingredient is charged**, and it is what bounds a
    /// cell that still has one: green's gears and inserters. A cell whose
    /// every input is belted has no charge and is bounded by its sources
    /// instead -- see [`SupplyHorizon`].
    ///
    /// At least one: a cell charged for nothing at all would place perfectly
    /// and be dead before it started, which is the failure this whole stage is
    /// against.
    pub fn charge_products(&self) -> u32 {
        (CELL_CHARGE_TICKS / self.ticks_per_item.max(1)).max(1)
    }

    /// Does `item` arrive on a belt?
    pub fn is_belted(&self, item: &str) -> bool {
        self.belted.contains(item)
    }

    /// Every ingredient that arrives on a belt, in the order its mouth stands
    /// in: the intermediate's rows first, the product machine's last.
    pub fn belted_inputs(&self) -> Vec<BeltedInput> {
        let mut out = Vec::new();
        if let Some(made) = self.intermediate.as_ref() {
            for (index, (item, amount)) in made.ingredients.iter().enumerate() {
                if !self.is_belted(item) {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation)]
                let feed_row = index as u8;
                out.push(BeltedInput {
                    item: item.clone(),
                    into: Role::Intermediate,
                    feed_row: Some(feed_row),
                    per_product: amount
                        .saturating_mul(made.per_product)
                        .div_ceil(made.per_run.max(1)),
                });
            }
        }
        if self.is_belted(&self.supplied.0) {
            out.push(BeltedInput {
                item: self.supplied.0.clone(),
                into: Role::Product,
                feed_row: None,
                per_product: self.supplied.1,
            });
        }
        out
    }

    /// What one cell eats of a belted input, per minute, when it makes
    /// `cell_per_minute` products a minute.
    ///
    /// **The goal's rate, not the cell's tempo.** A belt cell can make 120
    /// belts a minute and a stone furnace makes 15 plates; sized at the
    /// tempo, `producing:transport-belt:6` would ask a source for eight
    /// furnaces' worth of iron to make six belts. The cell runs at what its
    /// belt delivers, and what was asked for is the rate the source has to
    /// cover -- [`SupplyHorizon`]'s ticks are still counted at the tempo,
    /// which is the honest lower bound on the wait and is stated as one in
    /// `cellstock`.
    pub fn demand_per_minute(&self, input: &BeltedInput, cell_per_minute: u32) -> u32 {
        input.per_product.saturating_mul(cell_per_minute)
    }

    /// One cell's own tempo, in products a minute: what `Researched` asks a
    /// cell for, and the most a cell can ever be asked for.
    pub fn tempo_per_minute(&self) -> u32 {
        (3600 / self.ticks_per_item.max(1)).max(1)
    }

    /// The intermediate's ingredients that arrive in a chest, with the feed
    /// row (their index in the recipe) each one's chest stands on.
    pub fn chest_fed_feeds(&self) -> Vec<(u8, ItemId)> {
        let Some(made) = self.intermediate.as_ref() else {
            return Vec::new();
        };
        made.ingredients
            .iter()
            .enumerate()
            .filter(|(_, (item, _))| !self.is_belted(item))
            .map(|(index, (item, _))| {
                #[allow(clippy::cast_possible_truncation)]
                let index = index as u8;
                (index, item.clone())
            })
            .collect()
    }

    /// Does the product machine's own supplied ingredient arrive in a chest?
    pub fn has_supply_chest(&self) -> bool {
        !self.is_belted(&self.supplied.0)
    }

    /// How many runs of the intermediate's recipe one charge is sized for.
    ///
    /// Rounded **up** at the run boundary rather than at the item one: a
    /// recipe yielding two per run needs `ceil(n / 2)` runs, and each run eats
    /// its whole ingredient amount whether or not the last one is fully used.
    pub fn intermediate_runs(&self) -> u32 {
        let Some(intermediate) = self.intermediate.as_ref() else {
            return 0;
        };
        let wanted = self
            .charge_products()
            .saturating_mul(intermediate.per_product);
        wanted.div_ceil(intermediate.per_run.max(1))
    }

    /// How many feed chests a cell for this spec has — one per **chest-fed**
    /// ingredient of the intermediate's recipe, and **zero** when there is no
    /// intermediate or every ingredient is belted.
    pub fn feeds(&self) -> usize {
        self.chest_fed_feeds().len()
    }

    /// Does this cell's product machine take a recipe?
    ///
    /// **Only an assembling machine does.** A furnace picks its recipe from
    /// what is put into it and the game refuses `set_recipe` on one, so a
    /// `SetRecipe` action emitted for a furnace would be an action that
    /// cannot run — and a `Condition::RecipeSet` on one would be a condition
    /// that can never hold. Read off the recipe's category rather than off
    /// the machine's name, because the category is what decides which machine
    /// runs it in the first place.
    pub fn sets_recipe(&self) -> bool {
        self.recipe.category == CRAFTING_CATEGORY
    }

    /// What goes in each feed chest for one charge, as `(feed row, item,
    /// count)` in chest order.
    ///
    /// Integer end to end, one entry per **chest-fed** ingredient of the
    /// intermediate's recipe, in that recipe's order -- which is the order
    /// [`Role::FeedChest`]'s indices are assigned in. A belted ingredient
    /// has no chest and no charge.
    pub fn feed_charges(&self) -> Vec<(u8, ItemId, u32)> {
        let runs = self.intermediate_runs();
        let Some(intermediate) = self.intermediate.as_ref() else {
            return Vec::new();
        };
        self.chest_fed_feeds()
            .into_iter()
            .map(|(index, item)| {
                let amount = intermediate
                    .ingredients
                    .get(usize::from(index))
                    .map_or(0, |(_, amount)| *amount);
                (index, item, runs.saturating_mul(amount))
            })
            .collect()
    }

    /// How much goes in the supply chest for one charge -- **zero** when the
    /// supplied ingredient is belted, because then there is no chest.
    pub fn supply_charge(&self) -> u32 {
        if !self.has_supply_chest() {
            return 0;
        }
        self.charge_products().saturating_mul(self.supplied.1)
    }
}

/// Which of a recipe's ingredients a cell takes on a belt: the smelted ones.
///
/// Asked of [`crate::method::produce::cell_spec`] rather than of a name, so
/// the rule is "a stage-1 cell can make it from ore" and nothing else --
/// `iron-plate` and `copper-plate` in vanilla, and whatever a mod adds an ore
/// and a smelting recipe for.
fn belted_among<'a>(
    state: &PlanState,
    ingredients: impl IntoIterator<Item = &'a ItemId>,
) -> BTreeSet<ItemId> {
    ingredients
        .into_iter()
        .filter(|item| cell_spec(state, item).is_some())
        .cloned()
        .collect()
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
    if recipe.category == SMELTING_CATEGORY {
        return furnace_spec(state, item, recipe);
    }
    if recipe.category != CRAFTING_CATEGORY {
        return None;
    }
    let ingredients = ingredients_of(&recipe);
    // Handle 2+ ingredients. First 2 follow the existing pattern;
    // remaining go into extra_supplied for 3+ input recipes.
    let mut iter = ingredients.iter();
    let (a, a_amount) = iter.next()?;
    let (b, b_amount) = iter.next()?;
    let extra_supplied: Vec<(ItemId, u32)> = iter.map(|(i, a)| (i.clone(), *a)).collect();
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
    let belted = belted_among(
        state,
        intermediate
            .ingredients
            .iter()
            .map(|(item, _)| item)
            .chain(std::iter::once(&supplied.0)),
    );
    Some(AssemblySpec {
        item: item.to_string(),
        product_offset: product_offset(state, MACHINE),
        machine: MACHINE,
        recipe,
        supplied,
        extra_supplied,
        intermediate: Some(intermediate),
        ticks_per_item,
        belted,
    })
}

/// Where [`Role::Product`] stands for a cell whose product machine is
/// `machine`, in the north frame.
///
/// The 3x3 case is `(0, 4)` — the offset this layout was written with — and
/// every other size falls out of the machine's own [`tile_alignment`]. The
/// derivation is one sentence: the machine has to cover the tile
/// [`Role::SupplyInserter`] drops into (north-frame `(-1, 4)`) and the tile
/// [`Role::OutputInserter`] picks up from (`(-1, 3)`), and it has to sit on
/// its own build grid. Pinning the machine's north-west corner to the
/// north-west corner of the 3x3 footprint does both for any size, and that
/// corner is `(-1.5, 2.5)`: a machine `n` tiles across is centred half its
/// width from it, which is exactly what `alignment - 0.5` reads out for the
/// only two sizes a crafting machine has (`0.5 -> 0`, `0 -> -0.5`).
fn product_offset(state: &PlanState, machine: &str) -> (f64, f64) {
    let (ax, ay) = tile_alignment(state, machine);
    (ax - 0.5, ay + 3.5)
}

/// Can a **one-machine** cell make `item` in a furnace, and out of what?
///
/// Three conditions, and the third is the one that keeps this method and
/// [`crate::method::produce`] disjoint:
///
/// * the recipe takes exactly **one** ingredient, because the cell has one
///   supply chest and one inserter into the machine. The *amount* is free —
///   `steel-plate` is five iron plates and an inserter carries five as
///   happily as one, which is precisely the constraint a drill-fed cell
///   cannot relax;
/// * that ingredient is **not something a drill can stand on**, which is
///   `cell_spec` answering for itself rather than a second copy of its rule.
///   `iron-plate` and `copper-plate` are stage 1's cells and stay stage 1's:
///   a drill mining its own input is autonomous where a hand-filled chest is
///   charged once, so where both shapes exist the drill wins;
/// * and the furnace has a tempo, so the cell count is a division by
///   something.
///
/// In vanilla 2.1 what this admits is `steel-plate` (five iron plates) and
/// `stone-brick` (two stone) — the two smelting recipes stage 1 refuses, and
/// for the same reason in both cases. **Steel is why it exists**: nothing in
/// this project has ever made a steel plate, and steel gates the rocket silo,
/// solar panels and every electric furnace past them.
///
/// # What fuels it
///
/// A stone furnace is a **burner**, and the block's own rule is that a burner
/// works where coal flows *through* it — which a steel furnace's does not, its
/// input being iron plates and its output steel. So the coal is hand-loaded,
/// the same way [`crate::method::produce`]'s furnace is: [`CELL_CHARGE_TICKS`]
/// worth at [`COAL_BURN_TICKS`] each, billed with the rest of the cell and
/// inserted beside the boiler top-up. It runs out when the charge does, and
/// nothing detects either — see [`CELL_CHARGE_TICKS`].
///
/// The **inserters** are electric ([`INSERTER`]), so the cell still wants a
/// pole and a network with capacity left. Only two of them, and no electric
/// machine at all, so a furnace cell's draw is ~26 kW against a red cell's
/// ~200.
fn furnace_spec(state: &PlanState, item: &str, recipe: FactorioRecipe) -> Option<AssemblySpec> {
    let ingredients = ingredients_of(&recipe);
    let [(input, amount)] = ingredients.as_slice() else {
        return None;
    };
    // Stage 1's cells stay stage 1's. Asked of `cell_spec` itself rather than
    // restated here, so the two can never drift into claiming the same item.
    if cell_spec(state, item).is_some() {
        return None;
    }
    let ticks_per_item =
        smelting_ticks(state, &recipe, FURNACE).div_ceil(output_per_craft(&recipe, item).max(1));
    if ticks_per_item == 0 {
        return None;
    }
    let belted = belted_among(state, std::iter::once(input));
    Some(AssemblySpec {
        item: item.to_string(),
        product_offset: product_offset(state, FURNACE),
        machine: FURNACE,
        recipe,
        supplied: (input.clone(), *amount),
        extra_supplied: vec![],
        intermediate: None,
        ticks_per_item,
        belted,
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
    /// Holds what the cell has made, so the product machine keeps running.
    ///
    /// **It is a buffer, not a store, and it cannot fill inside one charge.**
    /// A [`CHEST`] is 32 slots and a science pack stacks to 200, so it holds
    /// thousands where [`CELL_CHARGE_TICKS`] is sized for fifteen. Whoever
    /// empties it is whoever refills the feed chests -- the same hand, on the
    /// same visit -- so this does not add an unattended-time budget to the one
    /// [`CELL_CHARGE_TICKS`] already states. What it does *not* solve is
    /// stated there in full: the cell still stops when its inputs run out, and
    /// nothing in this crate detects that.
    OutputChest,
    /// Feed chest `n` -> intermediate machine.
    FeedInserter(u8),
    /// Intermediate machine -> product machine.
    LinkInserter,
    /// Supply chest -> product machine.
    SupplyInserter,
    /// Product machine -> output chest.
    ///
    /// **The part whose absence made every rate claim of this stage false.**
    /// An assembling machine holds its finished items in one output slot and
    /// halts on `full_output` when it backs up; for a one-item-per-craft
    /// recipe that is three or four items. A cell without this inserter runs
    /// four crafts and then stands `working`-shaped and dead: every building
    /// present, every recipe set, every input link live, every geometry check
    /// green, and a `Goal::Producing` rate that has never been achieved and
    /// cannot be.
    ///
    /// It faces **east** in the north frame, because an inserter's direction
    /// names the side it *picks up* from and the machine is east of it. That
    /// is the one inserter in the cell whose direction is not shared with any
    /// other, and it is the one the "backwards places 100 % and moves nothing"
    /// rule is most likely to catch -- [`fit`] checks it with `delivers_into`
    /// like every other link.
    ///
    /// **Where it stands depends on the [`Sink`].** Into a chest it stands on
    /// the product machine's west face; into a lab it stands on the south
    /// face, facing north, because a 3x3 lab does not fit where a 1x1 chest
    /// did (the tile west of the chest's is the supplied plate's mouth).
    OutputInserter,
    /// The `k`-th lab of the chain the cell's output feeds -- see [`Sink`].
    ///
    /// Lab `0` is the sink of [`Role::OutputInserter`]; every further lab
    /// stands one [`LAB_PITCH`] further south with a [`Role::LabInserter`]
    /// between it and the one before, because **labs pass science packs to
    /// each other**: a lab hands on what it is not using itself, so a chain
    /// is fed from one end and no belt or chest stands anywhere in the line.
    Lab(u8),
    /// Lab `k` -> lab `k + 1`, facing north (it picks up from the lab to
    /// its north and drops into the one to its south).
    LabInserter(u8),
    /// The pole that lights lab `k` and the arm that feeds it.
    ///
    /// One per lab, on the east side of the chain and outside the beacon
    /// flank, each within a small pole's wire reach of the one before and
    /// the first within reach of the cell's own [`POLE_OFFSET`] -- so the
    /// chain is wired by construction whatever the network it hangs off.
    LabPole(u8),
}

impl Role {
    /// What stands in this role, given the cell's product machine.
    ///
    /// `machine` is [`AssemblySpec::machine`] and is the whole reason this
    /// takes an argument: a cell's machine roles are an
    /// `assembling-machine-1` for a crafting recipe and a `stone-furnace` for
    /// a smelting one, and a constant here would have placed an assembling
    /// machine and asked it to smelt. Callers read [`CellPart::name`]
    /// instead, which is this answer recorded at layout time so that a part
    /// and its prototype cannot disagree.
    fn name(self, machine: &'static str) -> &'static str {
        match self {
            Role::Pole => POLE,
            Role::Intermediate | Role::Product => machine,
            Role::FeedChest(_) | Role::SupplyChest | Role::OutputChest => CHEST,
            Role::FeedInserter(_)
            | Role::LinkInserter
            | Role::SupplyInserter
            | Role::OutputInserter
            | Role::LabInserter(_) => INSERTER,
            Role::Lab(_) => LAB,
            Role::LabPole(_) => POLE,
        }
    }
}

/// The building research happens in -- `crate::method::have::LAB`, restated
/// here because the two crates' constants are private to their modules and a
/// lab placed by a cell has to be the lab a research looks for.
pub const LAB: &str = "lab";

/// Where a cell's output goes.
///
/// **Into a lab, for a science pack -- there is no chest in the line.** The
/// output arm drops straight into the first lab of a chain, and every further
/// lab is fed by the one before it through one inserter, which is how labs
/// pass packs along in the game. A chest is what remains for an item nothing
/// researches with (steel, belts): those are drawn by hand
/// (`crate::method::cellstock::DrawFromCell`), and a chest is the split-off
/// a hand can reach into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sink {
    /// An [`Role::OutputChest`] on the product machine's west face.
    Chest,
    /// A chain of this many labs south of the product machine, `1` at least.
    Labs(u8),
}

impl Sink {
    /// How many labs stand in this sink -- zero for a chest.
    pub fn labs(self) -> u8 {
        match self {
            Sink::Chest => 0,
            Sink::Labs(n) => n.max(1),
        }
    }
}

/// The sink a cell for `spec` gets when nobody says otherwise: one lab for a
/// science pack, a chest for anything else.
///
/// A science pack is an item some technology of the acting force researches
/// with -- asked of the technology table, not of a name, exactly as
/// `crate::method::have` asks it.
pub fn default_sink(state: &PlanState, spec: &AssemblySpec) -> Sink {
    if is_science_pack(state, &spec.item) {
        Sink::Labs(1)
    } else {
        Sink::Chest
    }
}

/// Does any technology of the acting force research with `item`?
pub fn is_science_pack(state: &PlanState, item: &str) -> bool {
    state.technology_names().into_iter().any(|name| {
        state.technology(&name).is_some_and(|tech| {
            tech.research_unit_ingredients
                .iter()
                .any(|ingredient| ingredient.name == item)
        })
    })
}

/// How many labs of a chain one cell keeps fed, when it makes
/// `cell_per_minute` packs a minute for `tech`.
///
/// **Labs pass packs forward only, and a lab hands on only what it is not
/// using itself**, so the far end of a long chain starves exactly as the far
/// furnace on a partial belt does (the 91/43/7/0 % gradient CLAUDE.md
/// measured). A lab researches one unit per `research_unit_energy` ticks
/// (that field is in ticks -- see `research_ticks_in_labs`) and eats the
/// unit's `amount` of each pack, so it consumes `amount * 3600 / energy`
/// packs a minute; the chain the cell feeds is the cell's rate over that,
/// floored, and one at least. On shipped 2.1.17 a red cell makes six a
/// minute and `automation` burns six a minute in one lab, so **one cell
/// feeds one lab** for every `automation`-era research; a chain only earns
/// its length on a research slower than 600 ticks a unit.
pub fn labs_fed_by(
    cell_per_minute: u32,
    pack: &str,
    tech: &factorio_bot_core::types::FactorioTechnology,
) -> u32 {
    use factorio_bot_core::num_traits::ToPrimitive;
    let amount = tech
        .research_unit_ingredients
        .iter()
        .find(|ingredient| ingredient.name == pack)
        .map_or(1, |ingredient| ingredient.amount.max(1));
    let energy = tech.research_unit_energy.to_f64().unwrap_or(0.).max(1.);
    let per_lab = f64::from(amount) * 3600. / energy;
    if per_lab <= 0. {
        return 1;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let labs = (f64::from(cell_per_minute) / per_lab).floor() as u32;
    labs.max(1)
}

/// One building of a cell, with the direction it stands in.
#[derive(Clone, Debug, PartialEq)]
pub struct CellPart {
    pub role: Role,
    pub position: Position,
    pub direction: Direction,
    /// The prototype that stands here — [`Role::name`] resolved against the
    /// cell's own product machine, carried rather than recomputed so that no
    /// caller has to have the spec in hand to ask what a part is.
    pub name: &'static str,
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
///      3:  O   <   .  ###
///      4:  C   >   .  ###
/// ```
///
/// (The machines are three tiles wide and centred on `x = 0`, so they occupy
/// `x = -1 .. 1`; the pole `P` at `(-1, 2)` sits in the one-tile gap between
/// them, where a 5x5 supply area reaches every consumer in the cell —
/// including both feed rows, which is what [`MAX_FEED`] is two for.
/// `O` is the output chest and `<` the [`Role::OutputInserter`] that fills it:
/// it is the one inserter in the picture pointing the *other* way, because it
/// picks up from the machine to its east rather than from a chest to its
/// west.)
///
/// **The output pair is appended after the parts that were here before it**,
/// so a plan's action ids are the old sequence with two placements on the end
/// rather than the old sequence renumbered. The plan still moves — it has two
/// more buildings in it and a longer bill — but the diff is an addition rather
/// than a permutation.
fn layout_table(spec: &AssemblySpec, sink: Sink) -> Vec<(Role, (f64, f64), Direction)> {
    let product_offset = spec.product_offset;
    // **The intermediate half is present only when there is one to build.**
    // A one-machine cell (a furnace: see `furnace_spec`) has no feed chest,
    // no feed inserter, no intermediate machine and no link inserter -- its
    // single machine is fed from the supply chest and drained into the output
    // chest, which is this same body with one half left out rather than a
    // second layout. The ground the intermediate would have stood on is
    // simply left clear.
    let mut out = if spec.intermediate.is_none() {
        vec![(Role::Product, product_offset, Direction::North)]
    } else {
        vec![
            (Role::Intermediate, (0., 0.), Direction::North),
            (Role::Product, product_offset, Direction::North),
        ]
    };
    // **A chest stands only where an ingredient is not belted.** A belted
    // ingredient's row holds nothing of the layout's: its arm and the belt
    // tile beyond it are a [`Mouth`], placed by `method::connect` when the
    // run from its source is laid, and kept clear by [`fit`] until then.
    let feeds: Vec<(u8, f64)> = spec
        .chest_fed_feeds()
        .into_iter()
        .filter_map(|(index, _)| FEED_ROWS.get(usize::from(index)).map(|y| (index, *y)))
        .collect();
    let supply_chest = spec.has_supply_chest();
    for (index, y) in &feeds {
        out.push((Role::FeedChest(*index), (-3., *y), Direction::North));
    }
    if supply_chest {
        out.push((Role::SupplyChest, (-3., SUPPLY_ROW), Direction::North));
    }
    for (index, y) in &feeds {
        out.push((Role::FeedInserter(*index), (-2., *y), Direction::West));
    }
    if spec.intermediate.is_some() {
        out.push((Role::LinkInserter, (0., 2.), Direction::North));
    }
    if supply_chest {
        out.push((Role::SupplyInserter, (-2., SUPPLY_ROW), Direction::West));
    }
    match sink {
        Sink::Chest => {
            out.push((Role::OutputChest, OUTPUT_CHEST_OFFSET, Direction::North));
            out.push((
                Role::OutputInserter,
                OUTPUT_INSERTER_OFFSET,
                Direction::East,
            ));
        }
        Sink::Labs(labs) => {
            // The output arm on the product machine's SOUTH face, and the
            // chain running south from it: lab, arm, lab, arm ... with a
            // pole beside each lab. See `LAB_FIRST_OFFSET`.
            out.push((
                Role::OutputInserter,
                LAB_OUTPUT_INSERTER_OFFSET,
                Direction::North,
            ));
            for k in 0..labs.max(1) {
                let y = LAB_FIRST_OFFSET.1 + f64::from(k) * LAB_PITCH;
                out.push((Role::Lab(k), (LAB_FIRST_OFFSET.0, y), Direction::North));
                out.push((
                    Role::LabPole(k),
                    (
                        LAB_POLE_OFFSET.0,
                        LAB_POLE_OFFSET.1 + f64::from(k) * LAB_PITCH,
                    ),
                    Direction::North,
                ));
                if k + 1 < labs.max(1) {
                    out.push((
                        Role::LabInserter(k),
                        (LAB_FIRST_OFFSET.0, y + LAB_PITCH / 2.),
                        Direction::North,
                    ));
                }
            }
        }
    }
    out
}

/// Where a lab chain stands, in the north frame.
///
/// The product machine is at `(0, 4)` and covers `y = 3 ..= 5`, so its south
/// face is `y = 6`: the output arm stands at `(0, 6)` facing north (it picks
/// up from the machine to its north and drops south), and the first lab --
/// 3x3, so on a tile centre -- at `(0, 8)`, covering `y = 7 ..= 9`. Each
/// further lab is [`LAB_PITCH`] further south with its arm on the row
/// between, and each lab's pole stands one column east of the lab's edge:
/// `(2, 7)` supplies `x = -0.5 ..= 4.5`, `y = 4.5 ..= 9.5`, which is the arm
/// and the lab, and it is 5.8 tiles from the cell's own pole at
/// [`POLE_OFFSET`] and 4 from the next -- inside a small pole's 7.5 wire
/// reach either way. `x = 2` is the beacon flank's column, but the flank
/// spans `y = -1 ..= 5` ([`BEACON_FLANK_ROWS`]) and the first pole is at
/// `y = 7`, so the two never meet.
///
/// **Why south and not west, where the chest stood.** The chest's tile at
/// `(-3, 3)` has the supplied plate's mouth at `(-3, 4)` below it, and a
/// 3x3 lab centred anywhere the output arm at `(-2, 3)` could reach covers
/// that mouth's belt tile. The south face is the one face of the product
/// machine nothing else claims.
const LAB_OUTPUT_INSERTER_OFFSET: (f64, f64) = (0., 6.);
const LAB_FIRST_OFFSET: (f64, f64) = (0., 8.);
const LAB_PITCH: f64 = 4.;
const LAB_POLE_OFFSET: (f64, f64) = (2., 7.);

/// Where the product machine's output goes, in the north frame.
///
/// The mouth at `(-2, 3)` and the ground at `(-3, 3)` — the pair
/// `31c8d579` measured as powered-and-free at all four facings with
/// `pole_would_supply`, the game's own overlap rule. Using it is what makes
/// the output path cost **no second pole**: it is inside the supply area of
/// the pole at [`POLE_OFFSET`] that the cell was going to place anyway, and
/// the chest is serviced by a [`LANE`] tile that already exists.
const OUTPUT_INSERTER_OFFSET: (f64, f64) = (-2., 3.);
const OUTPUT_CHEST_OFFSET: (f64, f64) = (-3., 3.);

/// The rows a feed chest and its inserter stand on, in chest-index order.
///
/// `y = 0` is the intermediate machine's own row and `y = 1` the one south of
/// it. The machine presents a third tile to the west at `y = -1`, and it is
/// deliberately unused: the pole at [`POLE_OFFSET`] supplies
/// `y = -0.5 ..= 4.5`, so an inserter there would stand unpowered — see
/// [`MAX_FEED`].
const FEED_ROWS: [f64; MAX_FEED] = [0., 1.];

/// The row the product machine's supplied ingredient enters on, in the north
/// frame: the machine's southernmost west-face tile, one row below the
/// output path at [`OUTPUT_INSERTER_OFFSET`].
const SUPPLY_ROW: f64 = 4.;

/// The column an ingredient's arm stands in, and the column the belt it
/// picks from ends in: immediately west of the machines, and one further.
const MOUTH_ARM_COLUMN: f64 = -2.;
const MOUTH_BELT_COLUMN: f64 = -3.;

/// Where a belted ingredient enters the cell: the tile its unload arm will
/// stand on and the tile the belt it picks from will end on, one row of the
/// machine's west face.
///
/// **Not a part.** Nothing in [`layout_table`] stands here; the arm and the
/// belt are placed by `method::connect` when the run from the ingredient's
/// source is laid (see [`belted_link_steps`]), and this is the reservation
/// that keeps the run's own end free of the cell's other parts, of the
/// cell's other runs, and of anything else this plan puts down. The rows are
/// the same rows a chest would have stood on, so the pole at [`POLE_OFFSET`]
/// lights every arm exactly as it lit every chest inserter --
/// `tests::the_pole_lights_the_output_mouth_but_no_third_feed_row` is the
/// proof, unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct Mouth {
    /// What arrives here.
    pub item: ItemId,
    /// The machine the arm drops into.
    pub into: Role,
    pub arm: Position,
    pub belt: Position,
}

/// The mouths of a cell at `origin` facing `facing`, in [`BeltedInput`]
/// order.
fn mouths(origin: &Position, facing: Direction, spec: &AssemblySpec) -> Option<Vec<Mouth>> {
    let mut out = Vec::new();
    for input in spec.belted_inputs() {
        let row = match input.feed_row {
            Some(index) => *FEED_ROWS.get(usize::from(index))?,
            None => SUPPLY_ROW,
        };
        let at = |x: f64| Some(origin.add(&Position::new(x, row).turn(facing)?));
        out.push(Mouth {
            item: input.item,
            into: input.into,
            arm: at(MOUTH_ARM_COLUMN)?,
            belt: at(MOUTH_BELT_COLUMN)?,
        });
    }
    Some(out)
}

/// Where the cell puts a pole of its **own**, when it has to bring one.
///
/// The one-tile gap between the two machines, from which a 5x5 supply area
/// reaches all six consumers -- which is the whole reason the layout is an L
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
const LANE_COLUMN: f64 = -4.;

/// The rows of [`LANE_COLUMN`] a bot needs for this cell: one behind every
/// chest it fills by hand, and the output chest's. A belted row has no chest
/// and no bot ever stands behind it; that ground is the belt's.
fn lane_rows(spec: &AssemblySpec, sink: Sink) -> Vec<f64> {
    let mut rows: Vec<f64> = spec
        .chest_fed_feeds()
        .into_iter()
        .filter_map(|(index, _)| FEED_ROWS.get(usize::from(index)).copied())
        .collect();
    // A lab is emptied by nobody: only a chest wants a bot behind it.
    if sink == Sink::Chest {
        rows.push(OUTPUT_CHEST_OFFSET.1);
    }
    if spec.has_supply_chest() {
        rows.push(SUPPLY_ROW);
    }
    rows
}

/// The ground east of the machine column, left clear so a beacon can be
/// added later without tearing the cell down.
///
/// # Why the cell reserves ground for a machine it cannot yet use
///
/// The owner's ask is *"leave space for beacons from the beginning so we can
/// cheaply improve the production rates/productivity later"*. Empty ground is
/// cheap today; a beacon lane retrofitted into a built and belted base is a
/// teardown. So this is a **deliberate gap, not an oversight** -- if you are
/// reading this because a layout has a hole in it, the hole is the feature,
/// and deleting it costs the teardown it exists to avoid.
///
/// # Why east, and why one column rather than a lane between the machines
///
/// The two machines sit at `(0, 0)` and `(0, 4)`, four tiles apart, and that
/// distance is **forced**: [`Role::LinkInserter`] at `(0, 2)` carries the
/// intermediate's output into the product machine, and an inserter reaches
/// exactly one tile. Widening the gap to fit a beacon between them would
/// break the one link the cell is built around. The machines are therefore a
/// single column, both faces on the same side, and **one beacon column to
/// their east reaches both** -- the "does it reach both rows" question
/// [`crate::method::util::BeaconGeometry`] exists to answer does not even
/// arise here, because there is only one row.
///
/// East rather than west because west is taken: the feed and supply chests
/// are at `x = -3` and [`LANE`], the ground a bot stands on to fill them, is
/// at `x = -4`.
///
/// # Width is derived, not chosen
///
/// The column is [`crate::method::util::BeaconGeometry::lane_tiles`] wide --
/// the beacon's own footprint, read off its `collision_box`. That is the
/// narrowest lane that can ever work and it needs no `supply_area_distance`,
/// which is just as well because the mod does not send one; see that type's
/// doc for the derivation and for what is still missing. A world with no
/// beacon prototype reserves **nothing**, so a mod that removes beacons pays
/// no footprint for them.
///
/// # It spans the machines' own rows and no more
///
/// `y = -1 ..= 5` -- the seven rows the two 3x3 machines occupy. Reserving
/// less would leave a beacon standing beside only part of the column;
/// reserving more would buy nothing, and every tile is a tile that can refuse
/// a site.
const BEACON_FLANK_ROWS: [f64; 7] = [-1., 0., 1., 2., 3., 4., 5.];

/// The first column east of the machines, whose east faces are at `x = 1.5`.
///
/// Flush against them, so the gap between beacon footprint and machine is
/// zero and the beacon reaches the column for any `supply_area_distance`
/// above zero -- see [`crate::method::util::BeaconGeometry`].
const BEACON_FLANK_FIRST_COLUMN: f64 = 2.;

/// The tiles a cell at `origin` facing `facing` keeps clear for a beacon.
///
/// Empty when the world carries no beacon prototype, which is the only way
/// this costs nothing.
fn beacon_flank(state: &PlanState, origin: &Position, facing: Direction) -> Option<Vec<Position>> {
    let Some(geometry) = beacon_geometry(state, BEACON) else {
        return Some(Vec::new());
    };
    let columns = geometry.lane_tiles().max(0.).round();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let columns = columns as usize;
    let mut tiles = Vec::with_capacity(columns * BEACON_FLANK_ROWS.len());
    for column in 0..columns {
        #[allow(clippy::cast_precision_loss)]
        let x = BEACON_FLANK_FIRST_COLUMN + column as f64;
        for y in BEACON_FLANK_ROWS {
            tiles.push(origin.add(&Position::new(x, y).turn(facing)?));
        }
    }
    Some(tiles)
}

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
    /// The roles whose building the world already holds, on the tile and
    /// under the name [`parts`](Self::parts) gives them.
    ///
    /// Empty for a cell [`plan_cell`] sites on clear ground. A cell
    /// [`complete_cell`] finishes lists here what a cut-short plan left
    /// behind -- two machines and nothing else, in `run-1788608648-56109`'s
    /// third plan -- and [`cell_steps`] neither bills nor places those parts
    /// again. See [`Cell::stands`].
    pub standing: Vec<Role>,
    /// Every tile a bot must be able to stand on to charge this cell.
    pub lane: Vec<Position>,
    /// Where each belted ingredient's run ends -- see [`Mouth`].
    pub mouths: Vec<Mouth>,
    /// Where the output goes -- see [`Sink`].
    pub sink: Sink,
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
    /// is the cheap case and the one [`plan_cell`] looks for first -- and
    /// `false` when its own pole is one of the parts already standing. The
    /// bill reads this rather than assuming: a pole nobody needs is one wood
    /// spent out of a lifetime supply of four.
    pub fn brings_pole(&self) -> bool {
        self.at(Role::Pole).is_some() && !self.stands(Role::Pole)
    }

    /// Is the building in `role` already in the world?
    pub fn stands(&self, role: Role) -> bool {
        self.standing.contains(&role)
    }

    /// The parts this cell still has to place, in build order.
    pub fn missing(&self) -> impl Iterator<Item = &CellPart> {
        self.parts.iter().filter(|part| !self.stands(part.role))
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
/// centre takes tile centres to tile centres. That is what keeps all ten on
/// their own build grid at all four facings, and it is
/// `tests::every_facing_puts_every_building_on_its_own_grid` rather than a
/// comment.
fn layout(
    origin: &Position,
    facing: Direction,
    with_pole: bool,
    spec: &AssemblySpec,
    sink: Sink,
) -> Option<Vec<CellPart>> {
    let feeds = spec.feeds();
    // A cell with more than `MAX_FEED` feed chests has an inserter the pole
    // cannot light. A refusal rather than a truncation: a truncated cell
    // places perfectly and starves. **Zero is not that case** -- it is a cell
    // with no intermediate machine at all, whose one machine is fed from the
    // supply chest (see `layout_table`), and `assembly_spec` only ever
    // answers zero together with `intermediate: None`.
    if feeds > MAX_FEED {
        return None;
    }
    let pole = with_pole.then_some((Role::Pole, POLE_OFFSET, Direction::North));
    layout_table(spec, sink)
        .into_iter()
        .chain(pole)
        .map(|(role, offset, direction)| {
            Some(CellPart {
                role,
                position: origin.add(&Position::new(offset.0, offset.1).turn(facing)?),
                direction: compose(direction, facing)?,
                name: role.name(spec.machine),
            })
        })
        .collect()
}

/// The lane tiles of a cell at `origin` facing `facing`.
fn lane(
    origin: &Position,
    facing: Direction,
    spec: &AssemblySpec,
    sink: Sink,
) -> Option<Vec<Position>> {
    lane_rows(spec, sink)
        .into_iter()
        .map(|row| Some(origin.add(&Position::new(LANE_COLUMN, row).turn(facing)?)))
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
    let name = part.name;
    let entity_type = state
        .base()
        .globals
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
    // Only the chests that stand: a belted row has no chest and no arm of
    // the layout's, and its link is checked by `method::connect` when it is
    // laid rather than by this table.
    let feed_indices: Vec<u8> = cell
        .parts
        .iter()
        .filter_map(|part| match part.role {
            Role::FeedChest(index) => Some(index),
            _ => None,
        })
        .collect();
    for index in feed_indices {
        out.push((p(Role::FeedChest(index))?, p(Role::FeedInserter(index))?));
        out.push((p(Role::FeedInserter(index))?, p(Role::Intermediate)?));
    }
    // Only when there is an intermediate machine. A one-machine cell's items
    // go chest -> inserter -> machine and no further in, so asking for a link
    // through a part that does not exist would refuse every furnace cell on
    // the `?`.
    if let Some(intermediate) = p(Role::Intermediate) {
        out.push((intermediate, p(Role::LinkInserter)?));
        out.push((p(Role::LinkInserter)?, p(Role::Product)?));
    }
    if let Some(supply) = p(Role::SupplyChest) {
        out.push((supply, p(Role::SupplyInserter)?));
        out.push((p(Role::SupplyInserter)?, p(Role::Product)?));
    }
    // Out of the machine, not into it: the last link of the cell and the only
    // one that runs the other way. `delivers_into` answers it through its
    // *pull* disjunct — the inserter's pickup tile is one the machine covers —
    // so an output inserter turned round fails here exactly as a feed one
    // does.
    out.push((p(Role::Product)?, p(Role::OutputInserter)?));
    match cell.sink {
        Sink::Chest => out.push((p(Role::OutputInserter)?, p(Role::OutputChest)?)),
        Sink::Labs(labs) => {
            out.push((p(Role::OutputInserter)?, p(Role::Lab(0))?));
            for k in 0..labs.max(1).saturating_sub(1) {
                out.push((p(Role::Lab(k))?, p(Role::LabInserter(k))?));
                out.push((p(Role::LabInserter(k))?, p(Role::Lab(k + 1))?));
            }
        }
    }
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
            let name = part.name;
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
/// 2. with all ten standing, every one of the eight links really delivers.
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
    spec: &AssemblySpec,
    sink: Sink,
) -> Option<Cell> {
    let parts = layout(origin, facing, with_pole, spec, sink)?;
    let lane = lane(origin, facing, spec, sink)?;
    let mouths = mouths(origin, facing, spec)?;
    for part in &parts {
        if !state.is_area_free_facing(part.name, &part.position, part.direction) {
            return None;
        }
        // Refuse placement in uncharted territory -- the game will reject it.
        if !state.is_charted(&part.position) {
            return None;
        }
    }
    for tile in &lane {
        if !state.is_charted(tile) {
            return None;
        }
        if !state.is_position_free(tile) {
            return None;
        }
    }
    // A mouth's two tiles are the cell's ground as much as a chest's would
    // have been: the arm stands on one and the run ends on the other.
    for mouth in &mouths {
        if !state.is_position_free(&mouth.arm) || !state.is_position_free(&mouth.belt) {
            return None;
        }
    }
    // Ground kept clear for a beacon that no plan places yet -- see
    // `BEACON_FLANK_ROWS` for why a cell refuses a site over a machine it
    // cannot use, and `tests::a_cell_leaves_room_for_a_beacon_to_its_east`
    // for the shape. Checked here and NOT in `fit_partial`: a cell already
    // standing was sited before this rule existed, and refusing to finish it
    // would strand a half-built cell to protect ground that is already gone.
    for tile in &beacon_flank(state, origin, facing)? {
        if !state.is_position_free(tile) {
            return None;
        }
    }
    let cell = Cell {
        origin: origin.clone(),
        facing,
        parts,
        standing: Vec::new(),
        lane,
        mouths,
        sink,
        evacuate: Vec::new(),
    };
    works(state, cell, spec)
}

/// Questions 2 to 4 of [`fit`], asked of a cell whose ground is already
/// settled: with its missing parts standing on a fork, does every link
/// deliver, does every consumer have its capacity, and does the cell seal
/// nobody in?
///
/// Shared by [`fit`] and [`fit_partial`] so that a cell being finished is
/// held to exactly the checks a cell being sited is -- a standing inserter
/// turned the wrong way fails `delivers_into` here the same as a planned one
/// would, which is what makes reuse safe to prefer.
fn works(state: &PlanState, mut cell: Cell, spec: &AssemblySpec) -> Option<Cell> {
    let mut trial = state.fork();
    reserve_in(&mut trial, &cell, spec).ok()?;
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
    match crate::enclosure::check(state, &trial, &cell.origin) {
        crate::enclosure::EnclosurePrevention::Clear => {}
        crate::enclosure::EnclosurePrevention::Evacuate(evacuations) => {
            cell.evacuate = evacuations;
        }
        crate::enclosure::EnclosurePrevention::Refuse => return None,
    }
    if !supply_chest_is_reachable(&trial, &cell) {
        return None;
    }
    if !mouths_are_reachable(&trial, &cell) {
        return None;
    }
    Some(cell)
}

/// Can a belt be laid INTO every mouth of this cell from open ground?
///
/// [`supply_chest_is_reachable`]'s question, asked of the one side a belted
/// ingredient has: the mouth's arm tile must be free on the trial and its
/// belt tile must reach the window's edge on the surface with the arm
/// standing -- or the run already stands there, which is the replan's case
/// and is proven by the belt (see [`mouth_is_linked`]).
fn mouths_are_reachable(trial: &PlanState, cell: &Cell) -> bool {
    cell.mouths.iter().all(|mouth| {
        mouth_is_linked(trial, cell, mouth) || door_is_open(trial, &mouth.arm, &mouth.belt)
    })
}

/// Is a run already standing at this mouth: an arm on its tile that picks
/// up from something and drops into the machine the mouth serves?
///
/// The discriminator is direction, exactly as in [`already_filled_by_machine`]:
/// the cell's own output arm stands one row up and picks up FROM the
/// machine, so it is not an answer here; only an arm whose drop lands in the
/// machine is. What makes this idempotent across a replan.
fn mouth_is_linked(state: &PlanState, cell: &Cell, mouth: &Mouth) -> bool {
    let Some(machine) = cell.at(mouth.into) else {
        return false;
    };
    state.entity_at(&mouth.arm).is_some_and(|arm| {
        Pos::from(&arm.position) == Pos::from(&mouth.arm)
            && state.pickup_position(&arm).is_some()
            && state.delivers_into(&arm.position, &machine.position)
    })
}

/// Can a run end at `belt` with its unload arm on `arm`?
///
/// One side of a chest, or one mouth of a machine: the arm's tile free, and
/// the belt's tile either already the tail of a standing run or free and
/// reaching the window's edge on the surface with the arm standing. See
/// [`supply_chest_is_reachable`] for the run that paid for each clause.
fn door_is_open(trial: &PlanState, arm: &Position, belt: &Position) -> bool {
    if !trial.is_area_free(INSERTER, arm) {
        return false;
    }
    // A belt standing at the door with the arm's tile free is the run that
    // reached it, wanting its arm -- `connect::standing_run` -- but only a
    // belt whose flow ENDS here. A belt passing the chest on its way
    // elsewhere (a coal run, on `run-1788941729-70024` at `[29.5,-30.5]`)
    // reaches nothing.
    if belt_ends_at(trial, belt) {
        return true;
    }
    if !trial.is_area_free("transport-belt", belt) {
        return false;
    }
    // With the arm STANDING: the belt tile's way out must not be the tile
    // the arm will occupy. In `run-1788936524-99544` the belt tile was the
    // one-tile gap between the engine and the boiler, and its only free
    // neighbour was the arm's tile.
    let Some(area) = trial.collision_area_facing(INSERTER, arm, Direction::North) else {
        return false;
    };
    let mut with_arm = trial.fork();
    with_arm.create_entity(FactorioEntity {
        name: INSERTER.into(),
        entity_type: "inserter".into(),
        position: arm.clone(),
        direction: 0,
        bounding_box: area,
        ..Default::default()
    });
    let ctx = crate::method::ExpansionCtx::new(with_arm, crate::ids::BotId(0));
    crate::method::connect::belt_reaches_open_ground(&ctx, belt)
}

/// Can a belt be laid INTO this cell's supply chest from open ground?
///
/// The supply chest is the one part of a cell that some other method's run
/// has to reach -- a link lays a belt into it from a cell that
/// makes its ingredient -- and every other check here is about the cell's
/// own ground. Measured in `run-1788936524-99544` (seed 31337, replan at
/// tick 62,222): the science cell was sited on the shore beside the standing
/// plant, its supply chest's three free-looking sides were the plant's pipe,
/// water and its own inserter, and the one side left opened onto a two-tile
/// pocket. The link refused, the run was `stuck`, and the refusal said
/// "an underground span of 7 tiles" about a wall that was never on the
/// source's side. The same question `method::sustain::choose_exit` asks of
/// a plate chest's exit, asked of the chest a link arrives at: one side must
/// have the arm's tile and the belt's tile free on the trial (the cell's own
/// parts stand on it), and the belt's tile must reach the window's edge on
/// the surface.
///
/// Asked of the trial with the cell standing rather than of the bare state,
/// so a side the cell's own inserter takes is not counted. On clean ground
/// every side qualifies and the search is unchanged; the four canonical
/// baselines and the science bundle are byte-identical with this in place.
///
/// # A chest a belt already reaches is reachable
///
/// Asked of a cell being FINISHED rather than sited, the question can
/// already be answered by the ground: a link that stands whole fills the
/// chest by machine (`already_filled_by_machine`), and one that lost its
/// arms to an abandoned batch has a standing belt on one side's belt tile
/// with the arm's tile free. Both are "a belt can reach this chest", proven
/// by the belt. Without this, `fit_partial` on `run-1788941729-70024`'s
/// world rejected the true layout of the half-built cell -- its supply
/// chest's free sides all opened onto the coal run -- and recovered a
/// mirror of it with the feed chest as supply, which then asked for a
/// second link out of a plate chest with no side left.
fn supply_chest_is_reachable(trial: &PlanState, cell: &Cell) -> bool {
    let Some(chest) = cell.at(Role::SupplyChest) else {
        return true;
    };
    if already_filled_by_machine(trial, &chest.position) {
        return true;
    }
    [(0., -1.), (1., 0.), (0., 1.), (-1., 0.)]
        .into_iter()
        .any(|(dx, dy)| {
            let arm = Position::new(chest.position.x() + dx, chest.position.y() + dy);
            let belt = Position::new(chest.position.x() + 2. * dx, chest.position.y() + 2. * dy);
            door_is_open(trial, &arm, &belt)
        })
}

/// Does a `transport-belt` stand centred on `at` with no belt on the tile it
/// flows into -- the tail of a chain, where an unload arm would pick up?
fn belt_ends_at(state: &PlanState, at: &Position) -> bool {
    use factorio_bot_core::num_traits::FromPrimitive;
    let Some(belt) = state
        .entity_at(at)
        .filter(|e| e.name == "transport-belt" && Pos::from(&e.position) == Pos::from(at))
    else {
        return false;
    };
    let Some(facing) = Direction::from_u8(belt.direction) else {
        return false;
    };
    let Some(step) = Position::new(0., -1.).turn(facing) else {
        return false;
    };
    let next = at.add(&step);
    !state
        .entity_at(&next)
        .is_some_and(|e| e.name == "transport-belt" && Pos::from(&e.position) == Pos::from(&next))
}

/// Put `cell` into `state` the way the plan will: its missing parts created,
/// and **both machines set to their recipes**.
///
/// The recipes are the point. `PlanState::electric_demand_kw` charges a
/// crafting machine only once it has a recipe -- a machine with none can
/// never draw more than its drain -- so a reservation without them would let
/// a second cell be sized against capacity the first has already spoken for,
/// and [`fuel_for`] would size the boiler's coal for an empty network. Every
/// fork that stands a cell up goes through here so that the three cannot
/// disagree: [`works`], [`plan_cells`] and [`fuel_for`].
///
/// Standing parts are left as they are, recipes included: a standing machine
/// is only accepted by [`fit_partial`] with the right recipe or none, and
/// `set_recipe` on one already set to it is a no-op.
fn reserve_in(state: &mut PlanState, cell: &Cell, spec: &AssemblySpec) -> Result<(), PlannerError> {
    for part in cell.missing() {
        state.create_entity(entity_for(state, part));
    }
    // **Only machines that take one.** A furnace picks its recipe from what
    // is put into it and the game refuses `set_recipe` on one, so the model
    // must not carry a recipe the world could never hold -- see
    // `AssemblySpec::sets_recipe`.
    if spec.sets_recipe() {
        let intermediate = spec.intermediate.as_ref().map(|made| &made.recipe);
        for (role, recipe) in [
            (Role::Intermediate, intermediate),
            (Role::Product, Some(&spec.recipe)),
        ] {
            if let (Some(part), Some(recipe)) = (cell.at(role), recipe) {
                state.set_recipe(&part.position, &recipe.name)?;
            }
        }
    }
    Ok(())
}

/// The recipe a standing machine in `role` may already carry.
fn recipe_for_role(spec: &AssemblySpec, role: Role) -> Option<&str> {
    if !spec.sets_recipe() {
        return None;
    }
    match role {
        Role::Intermediate => spec
            .intermediate
            .as_ref()
            .map(|made| made.recipe.name.as_str()),
        Role::Product => Some(spec.recipe.name.as_str()),
        _ => None,
    }
}

/// The cell `machine` is part of, finished: every layout that puts a machine
/// on that tile is tried, and the one with the most of it already standing
/// wins.
///
/// # What "part of a cell" is taken to mean
///
/// A standing machine does not say which cell it was meant for -- an
/// assembling machine has no facing, and the mod reports every one at
/// direction 0 -- so the layout is recovered by trial: the machine is taken as
/// the [`Role::Intermediate`] and as the [`Role::Product`] of a cell at each
/// of the four facings, with and without a pole of the cell's own, and each
/// of those sixteen layouts is checked against the world part by part:
///
/// * an entity **of the part's name on the part's tile** is that part,
///   standing. A machine also has to carry the role's recipe **or none**; a
///   machine set to something else belongs to some other arrangement;
/// * **empty ground** is a part to place;
/// * **anything else** means this layout is not the cell, and it is dropped.
///
/// Every tile of the lane has to be free too, exactly as for a new cell. A
/// layout with nothing of it standing is not a cell to finish, and a layout
/// whose product machine `exclude` names is somebody's complete cell already.
///
/// The survivors are then held to [`works`] -- links, capacity, enclosure --
/// and the one with the **most standing parts** is the answer, ties going to
/// the first in the fixed order `(facing, role, with_pole)`. Most, not first:
/// two machines four tiles apart are the two machines of one cell at exactly
/// one facing, and the other fifteen layouts each claim one of them and
/// would build the rest of a different cell around it.
///
/// # And since 2026-09-09, around any standing PART, not only a machine
///
/// `seed` is whatever stands; `roles` are the roles of this layout that
/// stand under its name, and the seed is taken as each of them in turn. A
/// machine's roles are the two machine roles, as before. A chest's are the
/// feed, supply and output chests; an inserter's the four arms; a pole's the
/// cell's own pole (only the with-pole layouts, since only they have one).
/// `least` is how many parts must stand for a layout to count -- **one for a
/// machine, two for anything else**, so a lone chest anywhere in the world
/// is never the seed of a cell, while two parts on the exact tiles one
/// layout puts them on are that layout with its machines not yet placed.
///
/// Why it is needed: the machines are the LAST parts of a cell to stand.
/// Both need `automation` researched before they can be crafted, and every
/// electric arm needs a circuit off the copper the cell's own supply take
/// fetches, so a batch cut short upstream of either leaves the chests, some
/// arms, the poles and the whole supply link standing and no machine at all
/// -- `run-1788941729-70024` at tick 53,546: three chests, four inserters,
/// four poles, a lab and a complete belt link into the supply chest, and the
/// replan sited a fresh cell beside all of it, then refused because the
/// fresh cell's supply chest needed a second link out of a plate chest with
/// no side left. Recovering the cell from its chests finishes it instead,
/// and `already_filled_by_machine` then finds the standing link.
fn fit_partial(
    state: &PlanState,
    seed: &FactorioEntity,
    roles: &[Role],
    least: usize,
    spec: &AssemblySpec,
    sink: Sink,
    exclude: &BTreeSet<Pos>,
) -> Option<Cell> {
    let table = layout_table(spec, sink);
    let mut best: Option<(usize, Cell)> = None;
    for facing in Direction::orthogonal() {
        for &role in roles {
            let offset = if role == Role::Pole {
                POLE_OFFSET
            } else {
                let Some((_, offset, _)) = table.iter().find(|(r, _, _)| *r == role) else {
                    continue;
                };
                *offset
            };
            let Some(turned) = Position::new(offset.0, offset.1).turn(facing) else {
                continue;
            };
            let origin = Position::new(
                seed.position.x() - turned.x(),
                seed.position.y() - turned.y(),
            );
            // A pole seed is only a part of the layouts that have a pole.
            let pole_choices: &[bool] = if role == Role::Pole {
                &[true]
            } else {
                &[false, true]
            };
            for &with_pole in pole_choices {
                let Some(parts) = layout(&origin, facing, with_pole, spec, sink) else {
                    continue;
                };
                let Some(lane) = lane(&origin, facing, spec, sink) else {
                    continue;
                };
                let Some(mouths) = mouths(&origin, facing, spec) else {
                    continue;
                };
                if parts.iter().any(|part| {
                    matches!(part.role, Role::Intermediate | Role::Product)
                        && exclude.contains(&Pos::from(&part.position))
                }) {
                    continue;
                }
                let Some(standing) = standing_parts(state, &parts, spec) else {
                    continue;
                };
                if standing.is_empty() {
                    continue;
                }
                // The lane must be free -- except behind a supply chest a
                // machine already fills: a bot never charges that chest by
                // hand, and the tile behind it is where the link's unload
                // arm stands (`run-1788941729-70024`, arm at `[31.5,-31.5]`
                // behind the supply chest at `[31.5,-30.5]`).
                let supply = parts
                    .iter()
                    .find(|part| part.role == Role::SupplyChest)
                    .map(|part| part.position.clone());
                let fills_supply = |tile: &Position| {
                    supply.as_ref().is_some_and(|chest| {
                        state.entity_at(tile).is_some_and(|arm| {
                            state.pickup_position(&arm).is_some()
                                && state.delivers_into(&arm.position, chest)
                        })
                    })
                };
                if lane
                    .iter()
                    .any(|tile| !state.is_position_free(tile) && !fills_supply(tile))
                {
                    continue;
                }
                let count = standing.len();
                if count < least {
                    continue;
                }
                if best.as_ref().is_some_and(|(most, _)| *most >= count) {
                    continue;
                }
                let cell = Cell {
                    origin: origin.clone(),
                    facing,
                    parts,
                    standing,
                    lane,
                    mouths,
                    sink,
                    evacuate: Vec::new(),
                };
                // A mouth holds the run that reached it, or nothing at all.
                // An arm there that drops somewhere other than this layout's
                // machine, or a belt passing through, is another arrangement
                // and this layout is not the cell.
                if !cell.mouths.iter().all(|mouth| {
                    mouth_is_linked(state, &cell, mouth)
                        || (state.is_position_free(&mouth.arm)
                            && (state.is_position_free(&mouth.belt)
                                || belt_ends_at(state, &mouth.belt)))
                }) {
                    continue;
                }
                if let Some(cell) = works(state, cell, spec) {
                    best = Some((count, cell));
                }
            }
        }
    }
    best.map(|(_, cell)| cell)
}

/// Which of `parts` already stand, or `None` when one of them is blocked by
/// something that is not it.
fn standing_parts(state: &PlanState, parts: &[CellPart], spec: &AssemblySpec) -> Option<Vec<Role>> {
    let mut standing = Vec::new();
    for part in parts {
        let name = part.name;
        match state.entity_at(&part.position) {
            // The same name **centred on the same tile**: `entity_at` answers
            // for any entity covering the point, and a machine one tile off
            // the layout is somebody else's.
            Some(entity)
                if entity.name == name
                    && Pos::from(&entity.position) == Pos::from(&part.position) =>
            {
                if let Some(wanted) = recipe_for_role(spec, part.role)
                    && entity.recipe.as_deref().is_some_and(|set| set != wanted)
                {
                    return None;
                }
                standing.push(part.role);
            }
            Some(_) => return None,
            None => {
                if !state.is_area_free_facing(name, &part.position, part.direction) {
                    return None;
                }
            }
        }
    }
    Some(standing)
}

/// How far from the anchor [`complete_cell`] looks for a machine to finish a
/// cell around: the cell search itself, plus the furthest a cell's own part
/// stands from its origin, so a machine at the edge of where a new cell
/// could go is still a candidate.
const PARTIAL_CELL_SCAN_RADIUS: f64 = CELL_SEARCH_RADIUS as f64 + 6.;

/// A cell to finish rather than build, if a machine near `anchor` is part of
/// one -- nearest machine first.
///
/// Asked by [`plan_cells`] **before** [`plan_cell`], because a cell whose two
/// machines stand is two machines the plan does not have to craft, and a
/// planner that only ever sites on clear ground walks past them every replan:
/// `run-1788608648-56109`'s third plan placed four assembling machines beside
/// the two its second plan had already put down, and its fourth plan two
/// more beside those. Six machines for a goal that needs four, none of them
/// ever fed.
///
/// `exclude` names both machines of every cell that is already whole (see
/// [`cell_machines`]) and of every cell this plan has just chosen, so a cell
/// is neither finished twice, nor "finished" when it needs nothing, nor built
/// through a machine some other cell is using.
///
/// **Machines first, then every other part**, each nearest first: a
/// standing machine is a cell on its own evidence, and a chest or an arm is
/// only with a second part beside it (see [`fit_partial`]'s `least`). The
/// order is what keeps every plan with a machine standing byte-identical to
/// the plan made before parts other than machines could seed a cell.
///
/// If `memory` is provided, the [`CellIntent`] list is consulted before the
/// geometry scan: a matching anchor and item supplies the facing and a
/// pre-filtered standing-parts list so the cell is recovered by intent rather
/// than by scanning every entity in the neighbourhood. The memory is
/// advisory — every claim is verified against the world before use.
pub fn complete_cell(
    state: &PlanState,
    anchor: &Position,
    spec: &AssemblySpec,
    sink: Sink,
    exclude: &BTreeSet<Pos>,
    memory: Option<&crate::memory::ReplanMemory>,
) -> Option<Cell> {
    // Check memory before the geometry scan: if we have a CellIntent whose
    // anchor and item match, use the known facing and parts list as seeds.
    if let Some(memory) = memory {
        if let Some(cell_intent) = memory
            .cells
            .iter()
            .find(|ci| ci.anchor == *anchor && ci.item == spec.item)
        {
            for &with_pole in &[false, true] {
                let parts = layout(anchor, cell_intent.facing, with_pole, spec, sink)?;
                let lane = lane(anchor, cell_intent.facing, spec, sink)?;
                let mouths = mouths(anchor, cell_intent.facing, spec)?;
                // The machines of a cell already claimed are excluded.
                if parts
                    .iter()
                    .any(|part| matches!(part.role, Role::Intermediate | Role::Product)
                        && exclude.contains(&Pos::from(&part.position)))
                {
                    continue;
                }
                // Check every part position: standing parts are accepted,
                // missing parts must have free ground.
                if let Some(standing) = standing_parts(state, &parts, spec) {
                    if !standing.is_empty() {
                        let cell = Cell {
                            origin: anchor.clone(),
                            facing: cell_intent.facing,
                            parts,
                            standing,
                            lane,
                            mouths,
                            sink,
                            evacuate: Vec::new(),
                        };
                        if let Some(cell) = works(state, cell, spec) {
                            return Some(cell);
                        }
                    }
                }
            }
        }
    }

    // Original geometry scan — unchanged.
    let table = layout_table(spec, sink);
    let roles_of = |name: &str| -> Vec<Role> {
        table
            .iter()
            .map(|(role, _, _)| *role)
            .chain([Role::Pole])
            .filter(|role| role.name(spec.machine) == name)
            .collect()
    };
    let mut seeds: Vec<(bool, f64, FactorioEntity)> = state
        .entities_within(anchor, PARTIAL_CELL_SCAN_RADIUS)
        .into_iter()
        .filter(|entity| !roles_of(&entity.name).is_empty())
        .filter(|entity| !exclude.contains(&Pos::from(&entity.position)))
        .map(|entity| {
            (
                entity.name != spec.machine,
                calculate_distance(&entity.position, anchor),
                entity,
            )
        })
        .collect();
    seeds.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.position.x.total_cmp(&b.2.position.x))
            .then(a.2.position.y.total_cmp(&b.2.position.y))
    });
    seeds.into_iter().find_map(|(other, _, seed)| {
        let least = if other { 2 } else { 1 };
        fit_partial(
            state,
            &seed,
            &roles_of(&seed.name),
            least,
            spec,
            sink,
            exclude,
        )
    })
}

// ---------------------------------------------------------------------------
// Siting
// ---------------------------------------------------------------------------

/// What a whole cell draws, for the anchor search.
///
/// Two machines, and one inserter per feed chest plus the link and the supply
/// one. Read off the spec rather than a constant, because a two-feed cell has
/// a fourth inserter and an anchor sized for three would be sized short.
fn cell_demand_kw(state: &PlanState, spec: &AssemblySpec, sink: Sink) -> f64 {
    // `unwrap_or(0.)` is the right answer and not a fallback here: a burner
    // furnace draws nothing from the network, so a furnace cell's whole
    // demand is its two inserters.
    let machines = state.consumer_draw_kw(spec.machine).unwrap_or(0.);
    let inserters = state.consumer_draw_kw(INSERTER).unwrap_or(0.);
    let labs = state.consumer_draw_kw(LAB).unwrap_or(0.);
    let count = inserter_count(spec, sink).to_f64().unwrap_or(3.);
    let machine_count = if spec.intermediate.is_some() { 2. } else { 1. };
    machine_count * machines + count * inserters + f64::from(sink.labs()) * labs
}

/// How many inserters a cell for `spec` draws through: one per feed chest,
/// one link, one supply arm if the supply is a chest, one output -- and two
/// per belted ingredient, the load arm at its source and the unload arm at
/// its mouth.
///
/// The load arm stands on a stage-1 cell that has no network of its own, so
/// its draw lands wherever `ensure_powered` runs a wire from; counting it
/// here sizes the anchor for the worst case, which is that wire coming off
/// the cell's own plant.
fn inserter_count(spec: &AssemblySpec, sink: Sink) -> u32 {
    let link = u32::from(spec.intermediate.is_some());
    let supply = u32::from(spec.has_supply_chest());
    let belted = u32::try_from(spec.belted_inputs().len()).unwrap_or(0);
    // One between each pair of chained labs.
    let chain = u32::from(sink.labs()).saturating_sub(1);
    u32::try_from(spec.feeds()).unwrap_or(1) + link + supply + 1 + belted.saturating_mul(2) + chain
}

/// How many chests a cell for `spec` has: one per feed chest, one supply if
/// the supply is a chest, one output.
///
/// Only a test asks now: [`bill`] counts chests off the cell's own missing
/// parts, so this is the number a cell sited on clear ground has to place.
#[cfg(test)]
fn chest_count(spec: &AssemblySpec) -> u32 {
    u32::try_from(spec.feeds()).unwrap_or(1) + u32::from(spec.has_supply_chest()) + 1
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
    memory: Option<&crate::memory::ReplanMemory>,
) -> Result<Cell, PlannerError> {
    plan_cell_near(state, anchor, spec, default_sink(state, spec), &[], memory)
}

/// How far, on either axis, a cell's origin may stand from a source it is
/// belted from: one `connect` window's half-side, less room for the run to
/// enter the window's edge and turn.
///
/// A run is planned in one window centred on its SOURCE, and the sink has to
/// be inside it; a cell sited without this bound was put 29 tiles from its
/// iron chest and refused "further apart than one search window reaches"
/// while ground 20 tiles nearer stood empty (seed 31337, 2026-09-09).
const SOURCE_REACH: f64 = crate::enclosure::SEARCH_RADIUS - 4.;

/// [`plan_cell`], preferring a candidate within [`SOURCE_REACH`] of each of
/// `sources` -- the chests the cell's belted inputs come from -- and taking
/// the plain search only when no such site fits. Empty means unconstrained,
/// which is `plan_cell`.
///
/// Prefer-then-fall-back rather than a hard bound: the bound is a
/// conservative reading of the window, and a site just outside it can still
/// route (`sustain:iron-plate:30` composed with red found one 29 tiles from
/// its iron chest that linked). The run's own refusal, not this filter, is
/// what says a site is out of reach.
pub fn plan_cell_near(
    state: &PlanState,
    anchor: &Position,
    spec: &AssemblySpec,
    sink: Sink,
    sources: &[Position],
    memory: Option<&crate::memory::ReplanMemory>,
) -> Result<Cell, PlannerError> {
    let _ = memory;
    if !sources.is_empty()
        && let Ok(cell) = plan_cell_bounded(state, anchor, spec, sink, sources)
    {
        return Ok(cell);
    }
    plan_cell_bounded(state, anchor, spec, sink, &[])
}

fn plan_cell_bounded(
    state: &PlanState,
    anchor: &Position,
    spec: &AssemblySpec,
    sink: Sink,
    sources: &[Position],
) -> Result<Cell, PlannerError> {
    let base = Pos::from(anchor);
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
                        if sources.iter().any(|source| {
                            (source.x() - candidate.x()).abs() > SOURCE_REACH
                                || (source.y() - candidate.y()).abs() > SOURCE_REACH
                        }) {
                            continue;
                        }
                        if let Some(cell) = fit(state, &candidate, facing, with_pole, spec, sink) {
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

/// Site `count` cells, each clear of the ones before it -- finishing what
/// already stands before siting anything new.
///
/// Each cell is reserved on a fork as it is chosen, so the next search sees it
/// standing there — and, since the reservation includes its pole, its five
/// consumers **and their recipes** (see [`reserve_in`]), the *second* cell's
/// `Powered` check is asked against a network the first cell has already
/// spent capacity on. Without that, two cells on one 900 kW engine would each
/// be sized against the whole of it.
///
/// [`complete_cell`] is asked first for every one of the `count`, with the
/// product machines of whole cells and of the cells chosen so far excluded,
/// so a half-built cell is finished exactly once and a plan needing two cells
/// with two half-built ones standing finishes both.
pub fn plan_cells(
    state: &PlanState,
    anchor: &Position,
    spec: &AssemblySpec,
    count: u32,
    memory: Option<&crate::memory::ReplanMemory>,
) -> Result<Vec<Cell>, PlannerError> {
    plan_cells_near(state, anchor, spec, default_sink(state, spec), count, &[], memory)
}

/// [`plan_cells`], with the `i`th cell kept within [`SOURCE_REACH`] of the
/// chests in `sources[i]` -- see [`plan_cell_near`].
pub fn plan_cells_near(
    state: &PlanState,
    anchor: &Position,
    spec: &AssemblySpec,
    sink: Sink,
    count: u32,
    sources: &[Vec<Position>],
    memory: Option<&crate::memory::ReplanMemory>,
) -> Result<Vec<Cell>, PlannerError> {
    let mut trial = state.fork();
    let mut out = Vec::new();
    // **Both** machines of every cell already spoken for, not just the
    // product: a layout at another facing through a cell's intermediate
    // machine has a product tile of its own, and excluding products alone
    // let the second cell of a green plan be "finished" around the first
    // cell's intermediate -- one machine feeding two link inserters, and a
    // plan three machines long for a factory that needs four. Measured on
    // `workspace/scripts/map.json` the moment this tier existed.
    let mut exclude: BTreeSet<Pos> = cell_machines(state, spec).iter().map(Pos::from).collect();
    for index in 0..count {
        let near: &[Position] = sources
            .get(usize::try_from(index).unwrap_or(usize::MAX))
            .map_or(&[], Vec::as_slice);
        let cell = match complete_cell(&trial, anchor, spec, sink, &exclude, memory) {
            Some(cell) => cell,
            None => plan_cell_near(&trial, anchor, spec, sink, near, memory)?,
        };
        reserve_in(&mut trial, &cell, spec)?;
        for role in [Role::Intermediate, Role::Product] {
            if let Some(part) = cell.at(role) {
                exclude.insert(Pos::from(&part.position));
            }
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
///   [`is_supplied`], which is where the recursion into it happens;
/// * and *something takes the product away* — see [`is_drained`]. This is the
///   fifth clause and the newest, and it is the one whose absence made the
///   other four add up to a false claim: a cell satisfying all of them still
///   halts on `full_output` after four crafts, and `Goal::Producing` names a
///   **rate**.
///
/// # Adding the drain clause stops old cells counting, on purpose
///
/// A cell built before the output path existed now reads as not standing, so a
/// replan builds another one rather than adopting it. That is the direction
/// this crate chooses everywhere (see [`CELL_SCAN_RADIUS`]): **over-build
/// rather than over-claim.** The alternative is a predicate that keeps
/// answering `true` for the exact arrangement this clause was written for,
/// which is the defect and not a mitigation of it.
///
/// It is genuinely a whole extra cell rather than a two-part repair, because
/// nothing here plans repairs: a cell is planned as a unit. The honest cost is
/// two machines, three chests and four inserters wasted once, against a goal
/// that would otherwise never be met at all.
///
/// Order-independent by construction: it counts, and it dedupes by tile
/// through the `(x, y, name)` order `entities_within` already imposes.
pub fn cells_standing(state: &PlanState, spec: &AssemblySpec) -> u32 {
    u32::try_from(complete_cells(state, spec, None).len()).unwrap_or(u32::MAX)
}

/// The product machines of every complete cell for `spec` -- the positions
/// behind [`cells_standing`]'s count, so that [`plan_cells`] can keep a whole
/// cell out of [`complete_cell`]'s candidates. Same five clauses, same order.
pub fn complete_cells(state: &PlanState, spec: &AssemblySpec, memory: Option<&crate::memory::ReplanMemory>) -> Vec<Position> {
    let _ = memory;
    let ingredients = ingredients_of(&spec.recipe).len();
    let mut out = Vec::new();
    let nearby = state.entities_within(&Position::new(0., 0.), CELL_SCAN_RADIUS);
    for machine in &nearby {
        if machine.name != spec.machine {
            continue;
        }
        // **A machine that takes a recipe must carry this one; a furnace,
        // which takes none, must merely not be busy with something else.**
        // A furnace reports whatever it last smelted, so `Some(other)` is a
        // furnace in somebody else's arrangement and `None` is one that has
        // not run -- which, standing between a loaded feeder and a drain, is
        // this cell.
        let recipe_ok = if spec.sets_recipe() {
            machine.recipe.as_deref() == Some(spec.recipe.name.as_str())
        } else {
            machine
                .recipe
                .as_deref()
                .is_none_or(|set| set == spec.recipe.name.as_str())
        };
        if !recipe_ok {
            continue;
        }
        // A burner draws nothing, so there is no capacity clause to check for
        // one -- `Powered` is asked of exactly the machines that consume.
        if let Some(kw) = state.consumer_draw_kw(spec.machine)
            && !(Condition::Powered {
                pos: machine.position.clone(),
                entity: spec.machine.into(),
                kw,
            })
            .holds(state, crate::ids::BotId(0))
        {
            continue;
        }
        if !is_drained(state, &nearby, machine) {
            continue;
        }
        if loaded_feeders(state, &nearby, machine, CHAIN_DEPTH) >= ingredients {
            out.push(machine.position.clone());
        }
    }
    out
}

/// Where a complete cell for `spec` puts what it makes: the sink each cell's
/// [`Role::OutputInserter`] delivers into.
///
/// **Found the way [`is_drained`] finds it, not by the role table**, and that
/// asymmetry is deliberate: `is_drained` takes the sink on trust and never
/// names it, because the claim it answers is "the output has somewhere to go".
/// This asks the narrower question a *drawer* needs -- where is the container
/// -- and so it does name one, filtering to [`CHEST`]: a belt or a second
/// machine is a perfectly good sink for `Goal::Producing` and is not something
/// [`crate::method::cellstock::DrawFromCell`] can walk up to and empty.
///
/// Deduped and in the `(x, y, name)` order `entities_within` already imposes,
/// so the list is order-independent for the same reason `complete_cells` is.
pub fn cell_output_chests(state: &PlanState, spec: &AssemblySpec) -> Vec<Position> {
    let nearby = state.entities_within(&Position::new(0., 0.), CELL_SCAN_RADIUS);
    let mut out: Vec<Position> = Vec::new();
    for product in complete_cells(state, spec, None) {
        for inserter in nearby
            .iter()
            .filter(|inserter| inserter.name == INSERTER)
            .filter(|inserter| state.delivers_into(&product, &inserter.position))
        {
            for sink in nearby.iter().filter(|sink| {
                sink.name == CHEST
                    && sink.position != inserter.position
                    && sink.position != product
                    && state.delivers_into(&inserter.position, &sink.position)
            }) {
                if !out.contains(&sink.position) {
                    out.push(sink.position.clone());
                }
            }
        }
    }
    out
}

/// Every machine that belongs to a complete cell for `spec`: each product
/// machine [`complete_cells`] names, and the machine feeding it through its
/// link inserter -- found the way [`loaded_feeders`] finds it, as the
/// [`MACHINE`] source of an inserter that delivers into the product.
pub fn cell_machines(state: &PlanState, spec: &AssemblySpec) -> Vec<Position> {
    let nearby = state.entities_within(&Position::new(0., 0.), CELL_SCAN_RADIUS);
    let mut out = Vec::new();
    for product in complete_cells(state, spec, None) {
        for inserter in nearby
            .iter()
            .filter(|inserter| inserter.name == INSERTER)
            .filter(|inserter| state.delivers_into(&inserter.position, &product))
        {
            for source in nearby.iter().filter(|source| {
                source.name == spec.machine
                    && source.position != product
                    && state.delivers_into(&source.position, &inserter.position)
            }) {
                out.push(source.position.clone());
            }
        }
        out.push(product);
    }
    out
}

/// Does anything take `machine`'s product away?
///
/// An inserter whose **pickup** tile the machine covers, and which delivers
/// into something that is neither itself nor the machine. The asymmetry with
/// [`loaded_feeders`] is the whole content: a feeder is an inserter that
/// `delivers_into` the machine, a drain is an inserter the *machine*
/// `delivers_into` — which `PlanState::delivers_into` answers through its pull
/// disjunct, since an assembling machine has no drop point of its own and only
/// an inserter has a pickup one.
///
/// That disjunct is also what keeps the cell's own feed and supply inserters
/// from answering this. Their pickup tiles are chests, not the machine, so
/// `delivers_into(machine, feeder)` is false — the direction of an inserter is
/// load-bearing here exactly as it is everywhere else in this module.
///
/// The sink is taken on trust and is deliberately not named: a chest is what
/// this module builds, but a belt, a second machine or anything else the
/// entity graph reports counts too, because the claim is "the output has
/// somewhere to go", not "the output goes where I would have put it".
fn is_drained(state: &PlanState, nearby: &[FactorioEntity], machine: &FactorioEntity) -> bool {
    nearby
        .iter()
        .filter(|inserter| inserter.name == INSERTER)
        .filter(|inserter| state.delivers_into(&machine.position, &inserter.position))
        .any(|inserter| {
            nearby.iter().any(|sink| {
                sink.position != inserter.position
                    && sink.position != machine.position
                    && state.delivers_into(&inserter.position, &sink.position)
            })
        })
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
///
/// **The buildings are counted off the cells' missing parts**, not off the
/// cell count: a cell being finished bills what it lacks, and a bill that
/// still said "two machines per cell" would have the bot craft the two it is
/// standing next to. The charges are per cell whatever stands, and the pole
/// is [`Cell::brings_pole`]'s answer.
fn bill(spec: &AssemblySpec, cells: &[Cell], coal: u32) -> Vec<(ItemId, u32)> {
    let count = cells.len() as u32;
    let mut out: Vec<(ItemId, u32)> = Vec::new();
    for name in [spec.machine, INSERTER, CHEST, LAB] {
        let missing = cells
            .iter()
            .flat_map(Cell::missing)
            .filter(|part| part.name == name && part.role != Role::Pole)
            .count() as u32;
        if missing > 0 {
            out.push((name.to_string(), missing));
        }
    }
    // The cell's own pole when it brings one, and every lab's -- the chain's
    // poles are parts like any other and are billed as they are missing.
    let poles = cells.iter().filter(|cell| cell.brings_pole()).count() as u32
        + cells
            .iter()
            .flat_map(Cell::missing)
            .filter(|part| matches!(part.role, Role::LabPole(_)))
            .count() as u32;
    for (_, item, amount) in spec.feed_charges() {
        out.push((item, amount.saturating_mul(count)));
    }
    // Nothing for a belted supply: the plates come down a belt, and a bill
    // that still asked for them would have a bot smelt a charge nobody
    // pours anywhere.
    let supply_charge = spec.supply_charge();
    if supply_charge > 0 {
        out.push((spec.supplied.0.clone(), supply_charge.saturating_mul(count)));
    }
    // Extra supplied items for recipes with 3+ inputs (chemical-science-pack).
    for (item, per_product) in &spec.extra_supplied {
        let charge = spec.charge_products().saturating_mul(*per_product);
        if charge > 0 {
            out.push((item.clone(), charge.saturating_mul(count)));
        }
    }
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

/// How much coal keeps a network drawing `demand_kw` running for `ticks`.
///
/// Integer from the first line: the demand is a sum of table constants, so
/// rounding it up to a whole kilowatt before dividing costs at most one coal
/// and removes every float from the answer.
///
/// **Not capped at one fuel slot.** The caller is responsible for using
/// [`crate::method::have::fuel_visits`] to split the total into per-visit
/// loads that fit in a boiler\'s single fuel slot. Before 2026-09-09 this
/// function capped at [`COAL_STACK`] and the boiler ran out well before the
/// horizon, leaving a lab-fed cell dead mid-research.
fn boiler_coal(_state: &PlanState, demand_kw: f64, ticks: Ticks) -> u32 {
    let kw = demand_kw.max(0.).ceil().to_u64().unwrap_or(0);
    let kj = kw.saturating_mul(u64::from(ticks)) / 60;
    let coal = kj.div_ceil(COAL_KJ);
    u32::try_from(coal).unwrap_or(u32::MAX)
}

/// Every boiler this cell's power comes out of, nearest first.
///
/// Empty is not a refusal: a world powered by something this planner did not
/// build has no boiler to top up, and the cell is perfectly buildable on it.
/// What empty costs is the fuel guarantee, and that is stated in
/// [`CELL_CHARGE_TICKS`]'s doc rather than hidden here.
///
/// # A list, because a plant is a chain
///
/// This returned *the* boiler until 2026-09-06, which was right while
/// `power::Plant` carried exactly one. It now silently found *a* boiler of
/// several, and topping one up out of four leaves three cold: the plant reads
/// as built at its full nameplate and delivers a quarter of it. So the nearest
/// boiler anchors a group and everything within [`BOILER_CHAIN_SPAN`] of *it*
/// joins -- see that constant for why the reach hangs off the boiler rather
/// than off `anchor`.
///
/// Deterministic: candidates are ordered by `(distance from the anchor, x, y)`
/// with `total_cmp`, exactly as the single-boiler version was, so a one-boiler
/// world gets the identical one-element answer.
fn boilers_near(state: &PlanState, anchor: &Position) -> Vec<Position> {
    let order = |a: &(f64, Position), b: &(f64, Position)| {
        a.0.total_cmp(&b.0)
            .then(a.1.x.total_cmp(&b.1.x))
            .then(a.1.y.total_cmp(&b.1.y))
    };
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
    candidates.sort_by(order);
    let Some((_, nearest)) = candidates.first().cloned() else {
        return Vec::new();
    };
    let mut chain: Vec<(f64, Position)> = state
        .entities_within(&nearest, BOILER_CHAIN_SPAN)
        .into_iter()
        .filter(|entity| entity.name == BOILER)
        .map(|entity| {
            (
                calculate_distance(&entity.position, anchor),
                entity.position,
            )
        })
        .collect();
    chain.sort_by(order);
    chain.into_iter().map(|(_, position)| position).collect()
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
    let mut pre = vec![
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
    ];
    // **An electric part claims its power on its own placement.** The claim
    // used to ride on the chest charges, the last actions of a cell; a cell
    // with no chest has no charge, so each consumer states it here, with the
    // draw read off the same ledger `electric_demand_kw` bills
    // (`consumer_draw_kw`). `PlanState::powering_entities` turns the claim
    // into `EntityAt` edges from the pole and the generators that satisfy
    // it, so a part placed before the cell's own pole waits for the pole.
    // `crate::powered::audit` reads exactly this.
    if let Some(kw) = ctx.state.consumer_draw_kw(&name) {
        pre.push(Condition::Powered {
            pos: position.clone(),
            entity: name.clone(),
            kw,
        });
    }
    let step = Step::Act(Box::new(Action {
        id,
        kind: ActionKind::Place {
            entity: Box::new(entity.clone()),
        },
        pre,
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
/// insert asserts the chain from that chest through the intermediate machine,
/// and the supply chest's asserts the short one. All of them name the product
/// machine and its recipe, because all of them are claims about it — and all
/// of them carry the **output tail** as well, because a charge poured into a
/// machine that cannot empty itself buys four crafts. A branch is the whole
/// path an item takes, chest to chest.
fn cell_steps(
    ctx: &mut ExpansionCtx,
    spec: &AssemblySpec,
    cells: &[Cell],
    coal: u32,
    boilers: &[Position],
    roster: &[BotId],
    feeds: &[CellFeed<'_>],
) -> Result<CellSteps, PlannerError> {
    let mut steps: Vec<Step> = Vec::new();
    // The action each cell is complete at -- see `ledger_carrier`.
    let mut carriers: Vec<ActionId> = Vec::new();
    // What one burner product machine burns over the horizon, and zero for
    // an electric one. Billed with the boiler's coal so that a single
    // `Goal::Have` covers both -- two separate `Have`s for the same item in
    // the same hands are satisfied by the same items, which is the merge
    // `bill` exists to do.
    let horizon_ticks = feeds
        .iter()
        .map(|feed| feed.horizon.ticks(spec))
        .min()
        .unwrap_or(CELL_CHARGE_TICKS);
    let furnace_coal = furnace_charge_coal(&ctx.state, spec, horizon_ticks);
    // The tile in front of every run laid so far, closed to every later run:
    // a belt whose last tile faces a foreign belt pours its plates onto it.
    // See `belted_link_steps`.
    let mut laid_forward: Vec<Position> = Vec::new();
    // `coal` is per boiler and every boiler of the chain gets it, so the
    // bill is the chain's worth. It used to be one boiler's, which the
    // four-boiler fixture only survived because a bot happened to hold
    // nine coal against a bill of two.
    let coal_bill = coal
        .saturating_mul(u32::try_from(boilers.len()).unwrap_or(u32::MAX))
        .saturating_add(
            furnace_coal.saturating_mul(u32::try_from(cells.len()).unwrap_or(u32::MAX)),
        );
    // The actions that cannot run before the network exists: the ones carrying
    // a `Condition::Powered`, which no effect satisfies and which therefore
    // orders nothing by itself.
    let mut needs_power: Vec<ActionId> = Vec::new();

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
    let intermediate_gate = match spec.intermediate.as_ref() {
        Some(made) => gate_pre(&made.recipe.clone(), &mut steps),
        None => Vec::new(),
    };

    let reach = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.reach_distance)
        .unwrap_or(10.0);

    // Every cell's steps are *built* first -- ids allocated, sites and
    // recipes reserved in `ctx.state`, in exactly the order they were when
    // they were emitted as they went -- and emitted afterwards, so that a
    // roster can deal them out by item (see the end of this function). A
    // roster of one emits them in the order it always did.
    let mut builds: Vec<CellBuild> = Vec::with_capacity(cells.len());
    for (index, cell) in cells.iter().enumerate() {
        let mut build = CellBuild { product_place_id: None, ..Default::default() };
        let horizon = feeds
            .get(index)
            .map(|feed| feed.horizon)
            .unwrap_or(SupplyHorizon {
                products: spec.charge_products(),
            });
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
                build.evacuations.push(step);
                id
            })
            .collect();
        let mut part_ids: Vec<ActionId> = Vec::new();
        // Only what is missing: a part the world already holds is neither
        // billed (the bundle's `need` counts these steps) nor placed.
        for part in cell.missing() {
            let step = place_step(ctx, part);
            if let Step::Act(action) = &step {
                part_ids.push(action.id);
                // Carries a `Condition::Powered` -- see `place_step` -- so
                // the plant has to stand before it.
                if ctx.state.consumer_draw_kw(part.name).is_some() {
                    needs_power.push(action.id);
                }
                // Remember the Product machine's Place ID for the recipe link.
                if part.role == Role::Product {
                    build.product_place_id = Some(action.id);
                }
            }
            build.places.push((part.name.to_string(), step));
        }
        for evacuation_id in &evacuation_ids {
            for part_id in &part_ids {
                build.links.push(Step::Link {
                    from: *evacuation_id,
                    to: *part_id,
                    lag: 0,
                });
            }
        }

        let Some(chain) = links(cell) else {
            builds.push(build);
            continue;
        };

        // THE BELTED INPUTS, laid now that the machines stand in the overlay
        // (`place_step` reserves each site as it is emitted), so that the
        // recipe step below can name their arms. A mouth a run already
        // reaches is left alone -- the replan's case.
        let laid = match feeds.get(index).map(|feed| feed.supply) {
            Some(supply) => {
                let (run, laid) = belted_link_steps(ctx, spec, cell, supply, &mut laid_forward)?;
                build.supply = run;
                laid
            }
            None => Vec::new(),
        };

        let entity_at = |role: Role| -> Option<Condition> {
            cell.at(role).map(|part| Condition::EntityAt {
                pos: part.position.clone(),
                name: part.name.into(),
            })
        };
        // Read before the loop below starts mutating `ctx.state`, and read
        // from the ledger rather than restated: the kilowatts a cell claims
        // are the kilowatts `electric_demand_kw` will bill it.
        let machine_kw = ctx.state.consumer_draw_kw(spec.machine);
        let inserter_kw = ctx.state.consumer_draw_kw(INSERTER);
        let powered = |role: Role| -> Option<Condition> {
            let part = cell.at(role)?;
            let name = part.name;
            // `None` for a burner furnace, which is the honest answer and not
            // a missing row: it draws nothing, so it has no capacity clause.
            let kw = match name {
                _ if name == spec.machine => machine_kw,
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
            if !spec.sets_recipe() {
                return None;
            }
            cell.at(role).map(|part| Condition::RecipeSet {
                pos: part.position.clone(),
                recipe: recipe.to_string(),
            })
        };

        // Nothing to set on a furnace -- see `AssemblySpec::sets_recipe`.
        let settings: Vec<(Role, &FactorioRecipe, &Vec<Condition>)> = if spec.sets_recipe() {
            spec.intermediate
                .as_ref()
                .map(|made| (Role::Intermediate, &made.recipe, &intermediate_gate))
                .into_iter()
                .chain(std::iter::once((
                    Role::Product,
                    &spec.recipe,
                    &product_gate,
                )))
                .collect()
        } else {
            Vec::new()
        };
        for (role, recipe, gate) in settings {
            let Some(part) = cell.at(role) else {
                continue;
            };
            // A standing machine already set to this recipe needs no visit;
            // `fit_partial` accepts a standing machine only with this recipe
            // or none, so anything else here is a machine still to set.
            if cell.stands(role)
                && ctx
                    .state
                    .entity_at(&part.position)
                    .and_then(|entity| entity.recipe)
                    .as_deref()
                    == Some(recipe.name.as_str())
            {
                continue;
            }
            let mut pre = vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: part.position.clone(),
                    radius: reach,
                    min_radius: 0.0,
                },
                Condition::EntityAt {
                    pos: part.position.clone(),
                    name: spec.machine.into(),
                },
            ];
            pre.extend(gate.iter().cloned());
            // **The product machine's recipe is set last, and it asserts the
            // whole cell.** Every part by name, every link of the chain, and
            // every belted arm with the link it makes into its machine --
            // the structural claims that used to ride on the chest charges,
            // on the one action every cell has. `EntityAt` is world-scoped,
            // so `ActionNetwork::infer_edges` draws the edge from each
            // placement, and by the time this runs the cell is standing;
            // `Feeds` is re-checked by the scheduler exactly as `fit`
            // checked it with `delivers_into`.
            if role == Role::Product {
                for other in &cell.parts {
                    pre.push(Condition::EntityAt {
                        pos: other.position.clone(),
                        name: other.name.into(),
                    });
                }
                for (from, to) in &chain {
                    pre.push(Condition::Feeds {
                        from: from.clone(),
                        to: to.clone(),
                    });
                }
                for link in &laid {
                    pre.push(Condition::EntityAt {
                        pos: link.unload_arm.clone(),
                        name: INSERTER.into(),
                    });
                    pre.push(Condition::Feeds {
                        from: link.unload_arm.clone(),
                        to: link.into.clone(),
                    });
                }
            }
            let id = ctx.ids.next();
            // Link this recipe after its machine's placement, so the recipe
            // is never configured before the machine stands. The ID was
            // recorded during place creation in the loop above.
            if role == Role::Product {
                if let Some(place_id) = build.product_place_id {
                    build.links.push(Step::Link {
                        from: place_id,
                        to: id,
                        lag: 0,
                    });
                }
            }
            build.recipes.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::SetRecipe {
                    pos: part.position.clone(),
                    entity: spec.machine.into(),
                    recipe: recipe.name.clone(),
                },
                pre,
                eff: vec![Effect::SetRecipe {
                    pos: part.position.clone(),
                    recipe: recipe.name.clone(),
                }],
                duration: SET_RECIPE_TICKS,
                pinned: None,
                label: format!("set {} to {}", spec.machine, recipe.name),
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
        for (index, item, amount) in spec.feed_charges() {
            branches.push((
                Role::FeedChest(index),
                item,
                amount,
                vec![
                    Role::FeedInserter(index),
                    Role::Intermediate,
                    Role::LinkInserter,
                    Role::Product,
                    Role::OutputInserter,
                    Role::OutputChest,
                ],
            ));
        }
        if spec.has_supply_chest() {
            branches.push((
                Role::SupplyChest,
                spec.supplied.0.clone(),
                spec.supply_charge(),
                vec![
                    Role::SupplyInserter,
                    Role::Product,
                    Role::OutputInserter,
                    Role::OutputChest,
                ],
            ));
        }
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
            if let Some(made) = spec.intermediate.as_ref()
                && branch.contains(&Role::Intermediate)
            {
                pre.extend(recipe_set(Role::Intermediate, &made.recipe.name));
            }
            let id = ctx.ids.next();
            let label = format!(
                "charge the {} chest with {} {}",
                chest_role_name(chest_role),
                amount,
                item
            );
            let eff = vec![Effect::LoseItem {
                who: Actor::Role,
                item: item.clone(),
                count: amount,
            }];
            build.charges.push((
                item.clone(),
                amount,
                Box::new(Action {
                    id,
                    kind: ActionKind::Insert {
                        pos: chest.position.clone(),
                        entity: CHEST.into(),
                        slot: InventorySlot::Chest,
                        item: item.clone(),
                        count: amount,
                    },
                    pre,
                    eff,
                    duration: TRANSFER_TICKS,
                    pinned: None,
                    label,
                }),
            ));
        }

        // THE CELL'S LEDGER, on the last action that completes it.
        //
        // **This is what makes a cell's output spendable by another goal.**
        // Until it existed a cell was a structure and nothing more:
        // `Goal::Producing` is satisfied by machines standing, no method
        // could ask a standing cell for an item, and so production plateaued
        // at whatever the plan's bill had already hand-crafted.
        // `crate::method::cellstock::DrawFromCell` spends against exactly
        // this and cannot over-draw, because `Condition::BufferHas` is
        // checked against it.
        //
        // The count is the [`SupplyHorizon`]: what the standing sources will
        // deliver, over a hand charge where a chest remains -- not
        // `charge_products` unconditionally, which was one charge and
        // fifteen packs for ever. It is a claim about the future -- the
        // machines still have to run -- and the wait is on the drawing
        // action's edge, where the tempo that decides it lives.
        //
        // **Only when spending the output cannot pay for the cell**, which
        // is asked of the recipe graph rather than of a name. The limit was
        // bought by a measured regression: with the ledger written for
        // every product, `producing:transport-belt:6` went from a
        // 307-action plan to `4 transport-belt in the buffer at
        // [33.5, -12.5] does not hold there` -- `Withdraw` spent the belts
        // the cell had yet to make on building the cell, because
        // `method::sustain` belts its fuel. A `transport-belt` is in its
        // own cell's bill and a `steel-plate` is not, and
        // `crate::method::cellstock::output_is_spendable` is what tells
        // them apart.
        //
        // It rides on the product machine's `SetRecipe` when there is one
        // (the last thing a crafting cell does, and the action that asserts
        // the whole cell), else on the last arm of the last run laid, else
        // on the last part placed -- a furnace cell with its run already
        // standing. A cell with nothing to place and nothing to set has no
        // ledger to write; `build == 0` never reaches here.
        //
        // **Into a lab there is no ledger at all**: nothing draws from a
        // lab, the research that eats the packs is linked from this same
        // action by `crate::method::have::Researched` (see `CellSteps::carriers`),
        // and a `BufferGain` on a lab would be a stock nothing spends.
        let spendable = crate::method::cellstock::output_is_spendable(&ctx.state, spec);
        if let Some(carrier) = ledger_carrier(&mut build, cell) {
            carriers.push(carrier.id);
            if spendable && let Some(sink) = cell.at(Role::OutputChest) {
                carrier.eff.push(Effect::BufferGain {
                    pos: sink.position.clone(),
                    entity: CHEST.into(),
                    slot: InventorySlot::Chest,
                    item: spec.item.clone(),
                    count: horizon.products,
                });
            }
        }
        builds.push(build);
    }

    // Who builds what. With one bot there is nobody to deal to, and the bill
    // and the steps go out exactly as they always have; with a roster the
    // cell is dealt out by item -- see [`deal_bundles`] for the rule and
    // [`BuildAssemblyCell`]'s `converges` doc for why this is not the welding
    // it replaces.
    let builders = participants_that_can_work(&ctx.state, roster.to_vec());
    if builders.len() < 2 {
        for (item, amount) in bill(spec, cells, coal_bill) {
            steps.push(Step::Subgoal(Goal::Have {
                item,
                count: amount,
                whose: Holder::Share(ctx.chain_actor),
                via: None,
            }));
        }
        for build in builds {
            steps.extend(build.evacuations);
            steps.extend(build.places.into_iter().map(|(_, step)| step));
            steps.extend(build.links);
            steps.extend(build.supply);
            steps.extend(build.recipes);
            for (_, _, action) in build.charges {
                needs_power.push(action.id);
                steps.push(Step::Act(action));
            }
        }
    } else {
        // The coal first, and inline: it is the one bill left on the chain
        // this method opened, and the first `Holder::Share` a chain meets is
        // what names its owner (`expand_goal_body`), so it must be met
        // before any dealt block opens a chain of its own.
        if coal_bill > 0 {
            steps.push(Step::Subgoal(Goal::Have {
                item: "coal".into(),
                count: coal_bill,
                whose: Holder::Share(ctx.chain_actor),
                via: None,
            }));
        }
        let mut bundles: BTreeMap<ItemId, Bundle> = BTreeMap::new();
        let mut links: Vec<Step> = Vec::new();
        let mut supply: Vec<Step> = Vec::new();
        let mut recipes: Vec<Step> = Vec::new();
        for build in builds {
            steps.extend(build.evacuations);
            for (item, step) in build.places {
                let bundle = bundles.entry(item).or_default();
                bundle.need = bundle.need.saturating_add(1);
                bundle.places.push(step);
            }
            for (item, amount, action) in build.charges {
                let bundle = bundles.entry(item).or_default();
                bundle.need = bundle.need.saturating_add(amount);
                bundle.charges.push(*action);
            }
            links.extend(build.links);
            supply.extend(build.supply);
            recipes.extend(build.recipes);
        }
        for (item, bundle, bot) in deal_bundles(&ctx.state, bundles, &builders) {
            let reach = ctx.state.bot(bot).map(|b| b.reach_distance).unwrap_or(10.0);
            let mut block: Vec<Step> =
                Vec::with_capacity(1 + bundle.places.len() + bundle.charges.len());
            // `Holder::Share(bot)`: this bot places and charges these, so
            // this bot's inventory is what the shortfall is sized against,
            // and -- through `Step::Owned` -- this bot is who runs it.
            block.push(Step::Subgoal(Goal::Have {
                item: item.clone(),
                count: bundle.need,
                whose: Holder::Share(bot),
                via: None,
            }));
            block.extend(bundle.places);
            for mut action in bundle.charges {
                // The charger's own reach, now that the charger is known.
                for condition in &mut action.pre {
                    if let Condition::AtPosition {
                        who: Actor::Role,
                        radius,
                        ..
                    } = condition
                    {
                        *radius = reach;
                    }
                }
                needs_power.push(action.id);
                block.push(Step::Act(Box::new(action)));
            }
            steps.push(Step::Owned {
                whose: Holder::Share(bot),
                steps: block,
            });
        }
        // The runs stay on the chain actor, as the supply link always was:
        // a run is one bot's job (`connect` bands it across the roster
        // itself when it is long enough to be worth a second walk).
        steps.extend(supply);
        steps.extend(links);
        steps.extend(recipes);
    }

    // The plant's fuel, which is the cell's fuel: belt it from a coal buffer
    // chest to each boiler. A hand-charge of one slot (50 coal) lasts roughly
    // 63,000 ticks at 200 kW, but a red science research like `logistics`
    // needs 200 packs at 600 ticks each -- 120,000 ticks, nearly double what
    // a single slot covers.
    //
    // So the full horizon's coal is split across visits by `fuel_visits`:
    // each visit fills the boiler's single fuel slot (50 max), and the visits
    // are chained with a lag so the next one runs only after the current load
    // has burned. The first load keeps the plant running across the refuels
    // -- the boiler never goes cold, and a cell that would have starved at
    // 63,000 ticks now runs for the full horizon.
    //
    // **Every boiler of the chain, each with its share.** Topping up only the
    // nearest -- which is what this did while `power::Plant` carried a single
    // `boiler` -- leaves the rest of a grown plant cold, and a plant delivering
    // a fraction of its nameplate while every entity stands is exactly the
    // failure `Condition::Powered`'s capacity accounting exists to prevent.
    if coal > 0 {
        for boiler in boilers {
            let visits = fuel_visits(&ctx.state, "coal", coal);
            let mut ids: Vec<ActionId> = Vec::with_capacity(visits.len());
            for &load in &visits {
                let id = ctx.ids.next();
                ids.push(id);
                steps.push(Step::Act(Box::new(Action {
                    id,
                    kind: ActionKind::Insert {
                        pos: boiler.clone(),
                        entity: BOILER.into(),
                        slot: InventorySlot::Fuel,
                        item: "coal".into(),
                        count: load,
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
                            count: load,
                        },
                    ],
                    eff: vec![Effect::LoseItem {
                        who: Actor::Role,
                        item: "coal".into(),
                        count: load,
                    }],
                    duration: TRANSFER_TICKS,
                    pinned: None,
                    label: format!("top the boiler up with {} coal", load),
                })));
            }
            // Chain visit `j + 1` behind visit `j` by how long visit `j`
            // burns, so the next load arrives after the slot has room again.
            for (pair, &load) in ids.windows(2).zip(&visits) {
                steps.push(Step::Link {
                    from: pair[0],
                    to: pair[1],
                    lag: load.saturating_mul(COAL_BURN_TICKS),
                });
            }
        }
    }

    // The product machine's own fuel, when it is a burner.
    //
    // A stone furnace smelting steel is fed iron plates and hands out steel,
    // so **no coal ever flows through it** and the burner-inserter trick that
    // makes stage 1's belted blocks self-fuelling does not apply. It is
    // hand-loaded, exactly as `produce`'s furnace is, and it runs out when
    // the charge does -- see `CELL_CHARGE_TICKS`. Split across visits by
    // `fuel_visits`, because a fuel inventory is one slot and a single
    // oversized insert puts the remainder nowhere.
    if furnace_coal > 0 {
        for cell in cells {
            let Some(part) = cell.at(Role::Product) else {
                continue;
            };
            for load in fuel_visits(&ctx.state, "coal", furnace_coal) {
                let id = ctx.ids.next();
                steps.push(Step::Act(Box::new(Action {
                    id,
                    kind: ActionKind::Insert {
                        pos: part.position.clone(),
                        entity: part.name.into(),
                        slot: InventorySlot::Fuel,
                        item: "coal".into(),
                        count: load,
                    },
                    pre: vec![
                        Condition::AtPosition {
                            who: Actor::Role,
                            pos: part.position.clone(),
                            radius: reach,
                            min_radius: 0.0,
                        },
                        Condition::EntityAt {
                            pos: part.position.clone(),
                            name: part.name.into(),
                        },
                        Condition::HasItem {
                            who: Actor::Role,
                            item: "coal".into(),
                            count: load,
                        },
                    ],
                    eff: vec![Effect::LoseItem {
                        who: Actor::Role,
                        item: "coal".into(),
                        count: load,
                    }],
                    duration: TRANSFER_TICKS,
                    pinned: None,
                    label: format!("fuel the {} with {} coal", part.name, load),
                })));
            }
        }
    }

    Ok(CellSteps {
        steps,
        needs_power,
        carriers,
    })
}

/// What [`cell_steps`] emits.
struct CellSteps {
    steps: Vec<Step>,
    /// The actions carrying a `Condition::Powered`, which the plant has to
    /// stand before.
    needs_power: Vec<ActionId>,
    /// Per cell, the action the cell is complete at: the product machine's
    /// `SetRecipe`, else the last arm of the last run laid, else the last
    /// part placed. What a research linked to the cell waits on.
    carriers: Vec<ActionId>,
}

/// How much coal one cell's product machine burns over `ticks`, and zero
/// when it burns none.
///
/// **A burner is a machine the network does not power.** That is the test —
/// `consumer_draw_kw` answering `None` for a machine this crate is about to
/// place — rather than the name `stone-furnace`, so a world whose smelting
/// machine is something else is priced by the same rule. `fuel_for_duration`
/// and [`COAL_BURN_TICKS`] are `produce`'s, so a furnace in a stage-2 cell
/// and a furnace in a stage-1 cell are fuelled off one arithmetic.
///
/// Capped at one fuel slot, as [`boiler_coal`] is: a belted furnace cell's
/// horizon is its source's ore, which is far more than a slot burns, and a
/// hand can fill a slot once. What a longer horizon needs is the coal belted
/// in, which is `method::sustain`'s shape and not a bigger hand charge.
fn furnace_charge_coal(state: &PlanState, spec: &AssemblySpec, ticks: Ticks) -> u32 {
    if state.consumer_draw_kw(spec.machine).is_some() {
        return 0;
    }
    let cap = state
        .slot_capacity(InventorySlot::Fuel, "coal")
        .unwrap_or(COAL_STACK);
    fuel_for_duration(ticks, COAL_BURN_TICKS).min(cap)
}

/// What feeds one cell: its belted inputs' sources, and how much they will
/// deliver.
struct CellFeed<'a> {
    supply: &'a CellSupply,
    horizon: SupplyHorizon,
}

/// One cell's steps, built before any is emitted -- see `cell_steps`.
#[derive(Default)]
struct CellBuild {
    evacuations: Vec<Step>,
    /// Each placement, with the item it places.
    places: Vec<(ItemId, Step)>,
    /// Evacuation-before-placement edges.
    links: Vec<Step>,
    /// The belt runs into the cell's mouths, arms and wires included.
    supply: Vec<Step>,
    recipes: Vec<Step>,
    /// Each chest charge, with the item and count the charger has to hold.
    charges: Vec<(ItemId, u32, Box<Action>)>,
    /// The ActionId of the Product machine's Place step, stored during place
    /// creation so the recipe loop can link to it without matching by name
    /// (both Intermediate and Product machines have the same name).
    product_place_id: Option<ActionId>,
}

/// The action a cell's ledger rides on -- see the ledger note in
/// [`cell_steps`] for the order of preference.
fn ledger_carrier<'a>(build: &'a mut CellBuild, cell: &Cell) -> Option<&'a mut Action> {
    let product = cell.at(Role::Product).map(|part| part.position.clone());
    let is_product_recipe = |step: &Step| {
        matches!(step, Step::Act(action)
            if matches!(&action.kind, ActionKind::SetRecipe { pos, .. }
                if product.as_ref().is_some_and(|at| Pos::from(pos) == Pos::from(at))))
    };
    let is_arm_place = |step: &Step| {
        matches!(step, Step::Act(action)
            if matches!(&action.kind, ActionKind::Place { entity } if entity.name == INSERTER))
    };
    let pick =
        |steps: &'a mut Vec<Step>, wanted: &dyn Fn(&Step) -> bool| -> Option<&'a mut Action> {
            let index = steps.iter().rposition(wanted)?;
            match steps.get_mut(index)? {
                Step::Act(action) => Some(action.as_mut()),
                _ => None,
            }
        };
    if build.recipes.iter().any(is_product_recipe) {
        return pick(&mut build.recipes, &is_product_recipe);
    }
    if build.supply.iter().any(is_arm_place) {
        return pick(&mut build.supply, &is_arm_place);
    }
    let last = build.places.len().checked_sub(1)?;
    match &mut build.places.get_mut(last)?.1 {
        Step::Act(action) => Some(action.as_mut()),
        _ => None,
    }
}

/// Everything a cell needs of one item: the placements of it and the charges
/// of it, and the count that has to be in one bot's hands for both.
///
/// **Merged by item, exactly as [`bill`] merges.** Green's supply chest is
/// charged with inserters, which is also what the cell's own links are made
/// of; a bot handed the inserters to place and a different bot the inserters
/// to charge would each be sized against its own inventory and clash with
/// nobody, but a bot handed both must see one bill, or the cell is built
/// with the inserters its chest was supposed to hold.
#[derive(Default)]
struct Bundle {
    need: u32,
    places: Vec<Step>,
    charges: Vec<Action>,
}

/// Deal a cell's bundles across `builders`, heaviest first, each to whoever
/// is lightest at that moment. Returns them in the order dealt.
///
/// # Why the cell is dealt at all
///
/// `BuildAssemblyCell::converges` welds the cell to one bot, and that was the
/// whole of the green plan's tail: `run-1788604520-39283` left bot 1 alone
/// for the last 47 actions and ~35,000 ticks -- eight chests, ten inserters,
/// four machines and the crafting behind them -- while bots 2, 3 and 4 had
/// finished at 36,027 / 46,079 / 59,367. With the research off that chain
/// (`Action::tied_to_runner`) and the packs dealt evenly, bot 1 was still
/// last by 21,000 ticks: 23,668 of them idle, waiting on its own single
/// drills for the plates behind 34 inserters and 8 chests.
///
/// The distinction that makes dealing safe is the one `have::furnace_suppliers`
/// draws for a furnace: a placed chest, inserter or machine is a **map
/// fact**. Everything that comes after it -- the next placement's `AreaFree`,
/// the charge's `EntityAt` and `Feeds`, the recipe's `EntityAt` -- names a
/// position and no bot, so the item, its craft and its placement can be one
/// other bot's errand end to end; and a charge is inventory-convergent only
/// with its own `Have`, which travels with it. Each bundle's bill is sized
/// against its builder (`Holder::Share(builder)`) and bound to it
/// (`Step::Owned` always names an owner), so sizing and binding still agree:
/// several independently correct chains, not one chain with a relaxed owner.
///
/// # The rule
///
/// Load is [`PlanState::planned_ticks`] -- what this expansion has already
/// committed each bot to -- plus, as each bundle is dealt, its price **from
/// raw** (`produce::craft_ticks`), its placements and transfers, and one
/// [`HANDOVER_WALK_TICKS`] for the trip. Heaviest bundle first, each to the
/// lightest `(load, BotId)`, so the deal is a function of its inputs alone.
/// The taker is a candidate like anyone else, and a walled-in bot is not
/// (`participants_that_can_work`).
///
/// From raw and not in hand time, unlike the pack deal, and that was
/// measured. A supplier has no cell: a thirty-plate charge is thirty ore it
/// mines and smelts itself, and priced in hand time (zero) it was dealt as
/// if free. On `workspace/scripts/map.json` over four bots, hand time gave
/// `producing:logistic-science-pack:6` 54,847 ticks but
/// `producing:automation-science-pack:6` 31,296 -- worse than the undealt
/// 26,066, because the bot handed the chests and the plate charge waited
/// 4,950 ticks on a hand furnace before crafting its pack share and the
/// research slid behind it. From raw the two are 57,752 and 26,162, against
/// 71,167 and 28,885 before the cell was dealt at all.
fn deal_bundles(
    state: &PlanState,
    bundles: BTreeMap<ItemId, Bundle>,
    builders: &[BotId],
) -> Vec<(ItemId, Bundle, BotId)> {
    let mut loads: BTreeMap<BotId, Ticks> = builders
        .iter()
        .map(|bot| (*bot, state.planned_ticks(*bot)))
        .collect();
    let mut order: Vec<(Ticks, ItemId, Bundle)> = bundles
        .into_iter()
        .map(|(item, bundle)| {
            let price = crate::method::produce::craft_ticks(
                state,
                &item,
                bundle.need,
                crate::method::produce::CRAFT_TICKS_MAX_DEPTH,
            )
            .saturating_add(PLACE_TICKS.saturating_mul(bundle.places.len() as Ticks))
            .saturating_add(TRANSFER_TICKS.saturating_mul(bundle.charges.len() as Ticks))
            .saturating_add(HANDOVER_WALK_TICKS);
            (price, item, bundle)
        })
        .collect();
    order.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    order
        .into_iter()
        .map(|(price, item, bundle)| {
            let bot = loads
                .iter()
                .map(|(bot, load)| (*load, *bot))
                .min()
                .map(|(_, bot)| bot)
                .expect("builders is non-empty");
            let load = loads.entry(bot).or_default();
            *load = load.saturating_add(price);
            (item, bundle, bot)
        })
        .collect()
}

/// The word a charge-insert label uses for a chest.
fn chest_role_name(role: Role) -> &'static str {
    match role {
        Role::SupplyChest => "supply",
        _ => "feed",
    }
}

// ---------------------------------------------------------------------------
// The supply link: where the ingredient comes from when it is not a bot's hands
// ---------------------------------------------------------------------------

/// How far from a stage-1 cell's furnace its offtake arm is looked for.
///
/// The arm stands on the furnace's own perimeter, so the container it drops
/// into is at most two tiles from a footprint tile and at most three from the
/// centre of a 2x2. Three, and not "everything nearby": a wider radius would
/// start finding a *neighbouring* cell's offtake and attribute it to this
/// furnace.
const OFFTAKE_RADIUS: f64 = 3.0;

/// The container a stage-1 cell already drops `item` into, and the cell's own
/// furnace that fills it -- or `None` when no such arrangement stands.
///
/// **This is the whole precondition of the supply link**, and it is asked of
/// what stands rather than of what some other method planned: the answer is
/// read out of `PlanState` through the same two predicates
/// [`crate::method::sustain`]'s own offtake check uses, so a container filled
/// by hand, an arm pointing the wrong way, or a drop onto bare ground all
/// answer `None` rather than being taken for a supply.
///
/// Three conditions, and each one has a failure it exists to exclude:
///
/// * the furnace belongs to a **cell that makes `item`** -- `cell_spec` names
///   the ore and the smelting recipe, and `standing_cells` counts only drills
///   actually standing on that ore facing a furnace, so a furnace somebody
///   fed copper into is not a copper supply;
/// * the arm **picks up from the furnace** (`delivers_into`'s pull branch), so
///   an arm delivering coal *into* it is not mistaken for an offtake;
/// * the arm **drops into a container**, so an arm dropping onto the ground is
///   not counted as a source with nothing in it.
///
/// Deterministic: `standing_cells` is in a fixed order and
/// `entities_within` is sorted by `(x, y)`, so the first answer is the same
/// answer on every run.
#[derive(Clone, Debug, PartialEq)]
pub struct SupplySource {
    /// The container the cell drops the item into. The **preferred** end of
    /// the run and the one siting is measured from.
    pub buffer: Position,
    /// The furnace that fills it -- the fallback end, and the reason the
    /// fallback exists is measured rather than hypothetical. See
    /// [`belted_link_steps`].
    pub furnace: Position,
    /// What this one furnace makes a minute, at the cell's own tempo
    /// (`3600 / ticks_per_item`, 18 for a stone furnace on a plate).
    pub per_minute: u32,
    /// Plates left in the ore under the cell's drill, at the amounts the
    /// world last reported -- [`crate::method::produce::cell_yield`] times
    /// the recipe's yield per ore. The **ledger** a belted cell is bounded
    /// by: when this is dug out the source stops, and so does the cell.
    pub yield_left: u32,
}

/// Every standing stage-1 cell that drops `item` into a container, in the
/// fixed order `standing_cells` gives them.
pub(crate) fn standing_supply_sources(state: &PlanState, item: &str) -> Vec<SupplySource> {
    let Some(spec) = cell_spec(state, item) else {
        return Vec::new();
    };
    let per_minute = u32::try_from(3600 / u64::from(spec.ticks_per_item.max(1))).unwrap_or(0);
    let per_ore = output_per_craft(&spec.recipe, &spec.item).max(1);
    let mut out = Vec::new();
    for cell in crate::method::produce::standing_cells(state, &spec) {
        for arm in state.entities_within(&cell.furnace, OFFTAKE_RADIUS) {
            if state.pickup_position(&arm).is_none() {
                continue;
            }
            if !state.delivers_into(&cell.furnace, &arm.position) {
                continue;
            }
            let Some(drop) = state.delivery_position(&arm) else {
                continue;
            };
            let Some(sink) = state.entity_at(&drop) else {
                continue;
            };
            if !is_container(state, &sink.name) {
                continue;
            }
            if !state.delivers_into(&arm.position, &sink.position) {
                continue;
            }
            let ore =
                crate::method::produce::cell_yield(state, &cell.drill, cell.facing, &spec.ore);
            out.push(SupplySource {
                buffer: sink.position,
                furnace: cell.furnace.clone(),
                per_minute,
                yield_left: ore.saturating_mul(per_ore),
            });
            break;
        }
    }
    out
}

/// One cell's belted inputs, each with the source it is belted from, in
/// [`AssemblySpec::belted_inputs`] order -- which is also the order of the
/// cell's [`Mouth`]s.
#[derive(Clone, Debug, PartialEq)]
pub struct CellSupply {
    pub inputs: Vec<(BeltedInput, SupplySource)>,
}

/// A source for every belted input of every one of `count` cells, or the
/// refusal that names the input nothing delivers.
///
/// # The rate is the ledger, and it is spent as it is assigned
///
/// A source is one furnace: [`SupplySource::per_minute`] of its plate. Each
/// cell's demand for an input ([`AssemblySpec::demand_per_minute`]) is taken
/// out of the first source with that much left, in the fixed order the
/// sources come in, so two cells asking for more than one furnace makes are
/// belted from two furnaces -- and a third, with every source spoken for,
/// is refused by name rather than belted from a chest that cannot keep up.
/// Deterministic: the sources are in `standing_cells` order and the walk is
/// first-fit.
///
/// **No fallback to a hand charge.** A belted input with no source is the
/// owner's "no chests" refused rather than worked around; the message says
/// what to compose.
pub(crate) fn assign_sources(
    state: &PlanState,
    spec: &AssemblySpec,
    count: u32,
    cell_per_minute: u32,
) -> Result<Vec<CellSupply>, PlannerError> {
    let inputs = spec.belted_inputs();
    // Every source of every belted item, with the rate still unspoken for.
    let mut pool: BTreeMap<ItemId, Vec<(SupplySource, u32)>> = BTreeMap::new();
    for input in &inputs {
        pool.entry(input.item.clone()).or_insert_with(|| {
            standing_supply_sources(state, &input.item)
                .into_iter()
                .map(|source| {
                    let left = source.per_minute;
                    (source, left)
                })
                .collect()
        });
    }
    let mut out = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    for _ in 0..count {
        let mut assigned = Vec::with_capacity(inputs.len());
        for input in &inputs {
            let demand = spec.demand_per_minute(input, cell_per_minute);
            let sources = pool.entry(input.item.clone()).or_default();
            let Some((source, left)) = sources.iter_mut().find(|(_, left)| *left >= demand) else {
                let standing = if sources.is_empty() {
                    "no stage-1 cell drops it into a container anywhere".to_string()
                } else {
                    sources
                        .iter()
                        .map(|(source, left)| {
                            format!(
                                "the chest at {} makes {}/min with {}/min not yet spoken for",
                                source.buffer, source.per_minute, left
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                };
                return Err(PlannerError::AssemblyNoStandingSource {
                    item: spec.item.clone(),
                    ingredient: input.item.clone(),
                    per_minute: demand,
                    standing,
                });
            };
            *left = left.saturating_sub(demand);
            assigned.push((input.clone(), source.clone()));
        }
        out.push(CellSupply { inputs: assigned });
    }
    Ok(out)
}

/// Does every belted input of a cell for `item` have a standing source for
/// one cell's worth?
///
/// The question `crate::method::have::Researched` asks before it emits a
/// `Goal::Producing` for a pack: a cell that would refuse for want of a
/// source is not asked for, and the packs are hand-crafted as they were
/// before cells existed. `false` for an item no cell makes.
pub fn sources_stand_for(state: &PlanState, item: &str) -> bool {
    let Some(spec) = assembly_spec(state, item) else {
        return false;
    };
    assign_sources(state, &spec, 1, spec.tempo_per_minute()).is_ok()
}

/// How much a cell will make before something it depends on runs out.
///
/// **This is what replaced `CELL_CHARGE_TICKS` for a belted cell.** A hand
/// charge was a duration standing in for a supply -- 9,000 ticks of
/// ingredients, fifteen packs, then nothing. A belted cell's supply is its
/// sources, and what bounds a source is the ore under its drill
/// ([`SupplySource::yield_left`]): the products are the fewest any input's
/// plates will make (`yield_left / per_product`), and the ticks are that many
/// of the cell's own cycles. Where a chest remains (green's gears and
/// inserters) the hand charge still bounds the cell, and the horizon is the
/// smaller of the two -- a belt cannot outrun a chest nobody refills.
///
/// Three things are sized off it, and all three used to read the constant:
/// the ledger `cellstock` draws against (`Effect::BufferGain`), the boiler
/// top-up ([`fuel_for`]) and a burner product machine's coal
/// ([`furnace_charge_coal`]) -- the last two capped at one fuel slot, which
/// is all a hand can fill.
///
/// At least one product: an ore reading of zero under a standing drill is an
/// exhausted patch, and a cell sized for nothing would be built and never
/// fuelled; one is the honest floor, not a guess at the patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupplyHorizon {
    pub products: u32,
}

impl SupplyHorizon {
    /// The cell's cycles for its products.
    pub fn ticks(self, spec: &AssemblySpec) -> Ticks {
        self.products.saturating_mul(spec.ticks_per_item)
    }
}

fn horizon_for(spec: &AssemblySpec, supply: &CellSupply) -> SupplyHorizon {
    let belted = supply
        .inputs
        .iter()
        .map(|(input, source)| source.yield_left / input.per_product.max(1))
        .min();
    let charged = spec.feeds() > 0 || spec.has_supply_chest();
    let products = match (belted, charged) {
        (Some(belted), true) => belted.min(spec.charge_products()),
        (Some(belted), false) => belted,
        (None, _) => spec.charge_products(),
    };
    SupplyHorizon {
        products: products.max(1),
    }
}

/// Is `name` a container, by the prototype rather than by the entity?
///
/// A plan-built entity may carry an empty `entity_type`, which is why the
/// prototype is asked -- the same reading `method::connect`'s own
/// `container_sides` makes, and for the same reason.
fn is_container(state: &PlanState, name: &str) -> bool {
    state
        .base()
        .globals
        .entity_prototypes
        .get(name)
        .is_some_and(|proto| proto.entity_type == "container")
}

/// Is something already dropping into the container at `at`?
///
/// The discriminator is direction, and it is exact rather than approximate:
/// a cell's own [`Role::SupplyInserter`] stands beside the supply chest and
/// **picks up from** it, so it is not an answer here; only an arm whose
/// *drop* lands in the chest is. That is what makes this idempotent across a
/// replan -- the second plan finds its own unload arm standing and lays no
/// second run.
fn already_filled_by_machine(state: &PlanState, at: &Position) -> bool {
    state
        .entities_within(at, 2.0)
        .into_iter()
        .any(|arm| state.pickup_position(&arm).is_some() && state.delivers_into(&arm.position, at))
}

/// Every [`INSERTER`] a belt run places, in emission order -- the load arm
/// first and the unload arm last, whichever bot lays the belts between.
fn inserters_placed(run: &[Step]) -> Vec<FactorioEntity> {
    let mut out = Vec::new();
    for step in run {
        match step {
            Step::Act(action) => {
                if let ActionKind::Place { entity } = &action.kind
                    && entity.name == INSERTER
                {
                    out.push((**entity).clone());
                }
            }
            Step::Owned { steps, .. } => out.extend(inserters_placed(steps)),
            _ => {}
        }
    }
    out
}

/// Every belt-shaped thing a run places, by tile.
fn belts_placed(run: &[Step]) -> BTreeSet<Pos> {
    let mut out = BTreeSet::new();
    for step in run {
        match step {
            Step::Act(action) => {
                if let ActionKind::Place { entity } = &action.kind
                    && is_belt(&entity.name)
                {
                    out.insert(Pos::from(&entity.position));
                }
            }
            Step::Owned { steps, .. } => out.extend(belts_placed(steps)),
            _ => {}
        }
    }
    out
}

/// Does an entity of this name carry items along the ground?
fn is_belt(name: &str) -> bool {
    name.ends_with("transport-belt")
        || name.ends_with("underground-belt")
        || name.ends_with("splitter")
}

/// The tile a belt at `at` pours into, read off the belt standing there.
fn belt_forward(state: &PlanState, at: &Position) -> Option<(FactorioEntity, Position)> {
    let belt = state
        .entity_at(at)
        .filter(|e| is_belt(&e.name) && Pos::from(&e.position) == Pos::from(at))?;
    let facing = Direction::from_u8(belt.direction)?;
    let step = Position::new(0., -1.).turn(facing)?;
    let forward = at.add(&step);
    Some((belt, forward))
}

/// One run laid into a mouth.
struct LaidLink {
    /// The unload arm at the mouth.
    unload_arm: Position,
    /// The machine it drops into.
    into: Position,
}

/// The tiles one ring outside a part's footprint.
fn ring_around(state: &PlanState, name: &str, at: &Position) -> Vec<Position> {
    let Some(area) = state.collision_area(name, at) else {
        return Vec::new();
    };
    const SLACK: f64 = 1. / 512.;
    #[allow(clippy::cast_possible_truncation)]
    let (x0, x1, y0, y1) = (
        (area.left_top.x() + SLACK).floor() as i32,
        (area.right_bottom.x() - SLACK).floor() as i32,
        (area.left_top.y() + SLACK).floor() as i32,
        (area.right_bottom.y() - SLACK).floor() as i32,
    );
    let mut out = Vec::new();
    for y in (y0 - 1)..=(y1 + 1) {
        for x in (x0 - 1)..=(x1 + 1) {
            if x < x0 || x > x1 || y < y0 || y > y1 {
                out.push(Position::new(f64::from(x) + 0.5, f64::from(y) + 0.5));
            }
        }
    }
    out
}

/// Is this run's end its own, and nobody else's?
///
/// # The side-load risk, checked rather than assumed
///
/// A belt faces the way its items leave, and `route_belt` gives a run's last
/// tile the direction it arrived with -- so a run ending beside another
/// run's tile can face straight into it, and the plates ride off the end
/// onto the other belt instead of stopping under the arm. Nothing in
/// `graph::route` or `method::connect` models a run's last tile against a
/// foreign belt (neither contains the word "side-load" outside the
/// underground-entry rule), so it is checked here, at the one caller that
/// lays two runs into one machine column, and in both directions:
///
/// * **out**: the tile the last belt pours into holds no belt. The tile is
///   then closed to every later run of this cell (`laid_forward`), so a
///   later route cannot lay a belt in front of an earlier run's end;
/// * **in**: no standing belt outside this run faces into any tile of it.
///
/// Either is refused by name. Both hold on the fixture in
/// `tests::two_runs_into_one_cell_do_not_pour_into_each_other`, which lays
/// iron and copper into a red cell and reads every terminal off the overlay.
///
/// Returns the tile the run pours into, for the reservation.
fn run_end_is_isolated(
    state: &PlanState,
    run: &[Step],
    unload: &FactorioEntity,
    from: &FactorioEntity,
    item: &str,
) -> Result<Position, String> {
    let terminal = state
        .pickup_position(unload)
        .ok_or_else(|| format!("the unload arm at {} has no pickup tile", unload.position))?;
    let (last, forward) = belt_forward(state, &terminal)
        .ok_or_else(|| format!("no belt stands at {terminal}, where the unload arm picks up"))?;
    if let Some(next) = state
        .entity_at(&forward)
        .filter(|e| is_belt(&e.name) && Pos::from(&e.position) == Pos::from(&forward))
    {
        return Err(format!(
            "the run's last {} at {terminal} faces a {} at {forward}: {item} would ride off the \
             end onto it instead of stopping under the arm",
            last.name, next.name
        ));
    }
    let own = belts_placed(run);
    for tile in &own {
        let at = Position::new(f64::from(tile.0) + 0.5, f64::from(tile.1) + 0.5);
        for (dx, dy) in [(0., -1.), (1., 0.), (0., 1.), (-1., 0.)] {
            let neighbour = Position::new(at.x() + dx, at.y() + dy);
            if own.contains(&Pos::from(&neighbour)) {
                continue;
            }
            if let Some((foreign, pours_into)) = belt_forward(state, &neighbour)
                && Pos::from(&pours_into) == *tile
                // What the belt carries decides, not that it stands: a chain
                // nothing loads carries nothing and cannot pour anything
                // (a fragment of the first plan's run, on a replan whose
                // bands were cut), and a chain loaded off THIS run's source
                // carries this run's own item. Only a chain loaded from
                // somewhere else is a side-load.
                && chain_loader(state, &neighbour)
                    .is_some_and(|loader| !state.delivers_into(&from.position, &loader.position))
            {
                return Err(format!(
                    "a standing {} at {neighbour} faces into this run at {at}: whatever it \
                     carries would side-load onto the {item}",
                    foreign.name
                ));
            }
        }
    }
    Ok(forward)
}

/// What loads the belt chain ending at `tail`: the first arm found, walking
/// the chain backwards tile by tile through belts that pour into one
/// another, whose drop lands on a tile of it -- or `None` for a chain
/// nothing loads.
///
/// A chain is followed only while each tile has exactly one feeder, so a
/// junction stops it (and answers `None`, which the caller reads as inert:
/// a chain this walk cannot attribute is not refused on a guess). Bounded at
/// one `connect` window's worth of tiles.
fn chain_loader(state: &PlanState, tail: &Position) -> Option<FactorioEntity> {
    let mut at = tail.clone();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bound = crate::enclosure::SEARCH_RADIUS as usize * 4;
    for _ in 0..bound {
        if let Some(loader) = state.entities_within(&at, 1.5).into_iter().find(|arm| {
            state.pickup_position(arm).is_some()
                && state
                    .delivery_position(arm)
                    .is_some_and(|drop| Pos::from(&drop) == Pos::from(&at))
        }) {
            return Some(loader);
        }
        let feeders: Vec<Position> = [(0., -1.), (1., 0.), (0., 1.), (-1., 0.)]
            .into_iter()
            .map(|(dx, dy)| Position::new(at.x() + dx, at.y() + dy))
            .filter(|n| {
                belt_forward(state, n).is_some_and(|(_, into)| Pos::from(&into) == Pos::from(&at))
            })
            .collect();
        let [only] = feeders.as_slice() else {
            return None;
        };
        at = only.clone();
    }
    None
}

/// Belt runs from each belted input's source straight into the machine that
/// eats it.
///
/// # The shape, and why this one
///
/// **Container to machine**, and the geometry decides it rather than taste:
///
/// * the source end is a stage-1 cell's plate chest, because that cell's
///   furnace has a *two-by-two* perimeter of eight tiles and already spends
///   four of them, and because the chest is a **buffer** -- the whole reason
///   the two ends may run at different rates. The furnace is the fallback
///   end, and the reason the fallback exists is measured: after `sustain`
///   began keeping the chest an exit, the run out of it once needed an
///   underground pair the force could not craft, while the furnace end
///   routed on the surface. Each end is routed on a fork first and the one
///   needing fewer undergrounds is taken, the chest first on a tie;
/// * the sink end is the machine's own mouth -- see [`Mouth`]. A 3x3 has
///   twelve perimeter tiles and `connect` would take the first free one
///   (north, east, south, west), which for the intermediate machine is a
///   tile the cell's pole does not light and for the product machine is
///   the beacon flank. So every tile of the cell that is not this mouth is
///   handed to `connect_steps_reserving` as closed: the ring round both
///   machines, the beacon flank, the lane, every other mouth, and the tile
///   in front of every run already laid. The run then ends on exactly the
///   pair the layout kept for it, and the pole lights the arm.
///
/// The [`INSERTER`] is the electric one for the same reason the rest of
/// this cell's arms are -- the cell stands on a network by construction, and
/// a burner arm here would need a coal supply of its own that nothing feeds.
/// **Every arm on a run needs a wire**, and the load arm is the one that
/// does not have one already: it stands on a stage-1 cell that is entirely
/// burner, sixteen tiles from the assembly cell's pole on seed 31337. The
/// wire is run to it by the one function that does that, and `Ok(None)` --
/// supply exists and no run of poles reaches -- is a refusal here and not a
/// silent omission: an unpowered inserter places 100%, passes every geometry
/// check, and moves nothing.
///
/// # What it does NOT do
///
/// It lays no ignition charge. A belt that has to fill delivers nothing for
/// its first several hundred ticks, and a witness whose window is shorter
/// than a run's fill time will read a dead cell; a 30-tile yellow belt fills
/// in about 1,000 ticks against the 2,400-tick witness, so the charge is not
/// kept on speculation. Measure before adding one back.
fn belted_link_steps(
    ctx: &mut ExpansionCtx,
    _spec: &AssemblySpec,
    cell: &Cell,
    supply: &CellSupply,
    laid_forward: &mut Vec<Position>,
) -> Result<(Vec<Step>, Vec<LaidLink>), PlannerError> {
    let mut steps = Vec::new();
    let mut laid = Vec::new();
    // What this cell keeps for itself, closed to every run: the ring round
    // both machines, the beacon flank, the lane, and every mouth -- each run
    // then opens exactly its own.
    let mut kept: Vec<Position> = Vec::new();
    for role in [Role::Intermediate, Role::Product] {
        if let Some(part) = cell.at(role) {
            kept.extend(ring_around(&ctx.state, part.name, &part.position));
        }
    }
    kept.extend(beacon_flank(&ctx.state, &cell.origin, cell.facing).unwrap_or_default());
    kept.extend(cell.lane.iter().cloned());
    for mouth in &cell.mouths {
        kept.push(mouth.arm.clone());
        kept.push(mouth.belt.clone());
    }

    for (mouth, (input, source)) in cell.mouths.iter().zip(&supply.inputs) {
        if mouth_is_linked(&ctx.state, cell, mouth) {
            continue;
        }
        let Some(machine) = cell.at(mouth.into) else {
            continue;
        };
        let Some(sink) = ctx.state.entity_at(&machine.position) else {
            continue;
        };
        let reserved: Vec<Position> = kept
            .iter()
            .filter(|tile| {
                Pos::from(*tile) != Pos::from(&mouth.arm)
                    && Pos::from(*tile) != Pos::from(&mouth.belt)
            })
            .cloned()
            .chain(laid_forward.iter().cloned())
            .collect();
        let item = &input.item;
        let mut why: Vec<String> = Vec::new();
        let mut linked = false;
        // The end whose run stays on the surface goes first; a refused end
        // sorts last and is still tried for its message.
        let mut ends: Vec<(&Position, usize)> = [&source.buffer, &source.furnace]
            .into_iter()
            .map(|at| {
                let cost = ctx.state.entity_at(at).map_or(usize::MAX, |from| {
                    let mut trial = ExpansionCtx::new(ctx.state.fork(), ctx.chain_actor);
                    match connect_steps_reserving(
                        &mut trial, &from, &sink, item, INSERTER, &reserved,
                    ) {
                        Ok(run) => run
                            .iter()
                            .filter(|step| {
                                matches!(step, Step::Act(action)
                                    if matches!(&action.kind, ActionKind::Place { entity }
                                        if entity.name == "underground-belt"))
                            })
                            .count(),
                        Err(_) => usize::MAX,
                    }
                });
                (at, cost)
            })
            .collect();
        ends.sort_by_key(|(_, cost)| *cost);
        for (at, _) in ends {
            let Some(from) = ctx.state.entity_at(at) else {
                continue;
            };
            match connect_steps_reserving(ctx, &from, &sink, item, INSERTER, &reserved) {
                Ok(run) => {
                    // `connect_steps_reserving` has already added this run's
                    // belts and arms to the plan overlay -- that is what
                    // makes the next run route around this one -- so every
                    // refusal from here on ends the expansion (which restores
                    // the context) rather than falling through to the next
                    // end with entities no step places.
                    let arms = inserters_placed(&run);
                    let Some(unload) = arms.last() else {
                        return Err(PlannerError::AssemblyNoRouteForSupply {
                            item: item.clone(),
                            from: at.to_string(),
                            to: machine.position.to_string(),
                            why: "the run placed no unload arm".to_string(),
                        });
                    };
                    let forward = run_end_is_isolated(&ctx.state, &run, unload, &from, item)
                        .map_err(|why| PlannerError::AssemblyNoRouteForSupply {
                            item: item.clone(),
                            from: at.to_string(),
                            to: machine.position.to_string(),
                            why,
                        })?;
                    let mut powered = Vec::new();
                    let mut claims: Vec<(Position, Condition)> = Vec::new();
                    for arm in &arms {
                        let (Some(area), Some(kw)) = (
                            ctx.state.collision_area(INSERTER, &arm.position),
                            ctx.state.consumer_draw_kw(INSERTER),
                        ) else {
                            continue;
                        };
                        match crate::method::power::ensure_powered(
                            ctx,
                            INSERTER,
                            &arm.position,
                            &area,
                            kw,
                            ANCHOR_SEARCH_RADIUS,
                            &[],
                        )? {
                            Some(powering) => {
                                powered.extend(powering.steps);
                                claims.push((arm.position.clone(), powering.powered));
                            }
                            None => {
                                return Err(PlannerError::AssemblyNoRouteForSupply {
                                    item: item.clone(),
                                    from: at.to_string(),
                                    to: machine.position.to_string(),
                                    why: format!(
                                        "the arm at {} draws {kw:.0} kW and no run of poles \
                                         this planner will build carries supply to it",
                                        arm.position
                                    ),
                                });
                            }
                        }
                    }
                    // THE CLAIM RIDES ON THE PLACEMENT: `crate::powered::audit`
                    // asks the finished network whether some action states a
                    // `Condition::Powered` about each electric consumer's
                    // tile, and `connect` states `AtPosition`, `AreaFree` and
                    // `HasItem` and nothing else.
                    let mut run = run;
                    attach_claims(&mut run, &claims);
                    laid_forward.push(forward);
                    laid.push(LaidLink {
                        unload_arm: unload.position.clone(),
                        into: machine.position.clone(),
                    });
                    steps.extend(powered);
                    steps.extend(run);
                    linked = true;
                    break;
                }
                Err(refusal) => why.push(format!("from the {} at {at}: {refusal}", from.name)),
            }
        }
        if !linked {
            return Err(PlannerError::AssemblyNoRouteForSupply {
                item: item.clone(),
                from: source.buffer.to_string(),
                to: machine.position.to_string(),
                why: why.join("; "),
            });
        }
    }
    Ok((steps, laid))
}

/// Every [`POLE`] placed in `steps`, bands and all.
fn collect_poles(steps: &[Step], out: &mut Vec<Position>) {
    for step in steps {
        match step {
            Step::Act(action) => {
                if let ActionKind::Place { entity } = &action.kind
                    && entity.name == POLE
                {
                    out.push(entity.position.clone());
                }
            }
            Step::Owned { steps, .. } => collect_poles(steps, out),
            _ => {}
        }
    }
}

/// Put each arm's `Powered` claim on its own `Place`, wherever in the run
/// (flat or banded) that placement is.
fn attach_claims(run: &mut [Step], claims: &[(Position, Condition)]) {
    for step in run {
        match step {
            Step::Act(action) => {
                let ActionKind::Place { entity } = &action.kind else {
                    continue;
                };
                if entity.name != INSERTER {
                    continue;
                }
                if let Some((_, claim)) = claims
                    .iter()
                    .find(|(at, _)| Pos::from(at) == Pos::from(&entity.position))
                {
                    action.pre.push(claim.clone());
                }
            }
            Step::Owned { steps, .. } => attach_claims(steps, claims),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// The method
// ---------------------------------------------------------------------------

/// Build enough assembly cells to produce an item at a rate.
pub struct BuildAssemblyCell {
    /// The roster the cell's bundles are dealt across -- `registry_for`'s,
    /// exactly as `Researched` carries it. Empty in `default_registry`,
    /// where the chain actor builds the whole cell, which is what this
    /// method did for every roster before the deal. See [`deal_bundles`].
    pub bots: Vec<BotId>,
}

impl BuildAssemblyCell {
    /// The bots a cell is dealt across: the registry's roster with repeats
    /// removed and the chain actor always among them, ascending -- the same
    /// rule and the same reasons as `Researched::roster`.
    fn roster_of(bots: &[BotId], chain_actor: BotId) -> Vec<BotId> {
        let mut roster: Vec<BotId> = bots.to_vec();
        roster.push(chain_actor);
        roster.sort_unstable();
        roster.dedup();
        roster
    }
}

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
    /// Ten buildings, two recipes and two chest charges have to meet in one
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
        let sink = default_sink(&ctx.state, &spec);
        Ok(build_cells(ctx, &self.bots, item, *per_minute, sink)?.steps)
    }
}

/// What [`build_cells`] planned.
pub struct CellPlan {
    pub steps: Vec<Step>,
    /// Every lab the cells feed, chain order, cell by cell -- the ones this
    /// plan places and the ones already standing on a cell it finished.
    pub labs: Vec<Position>,
    /// Per cell, the action the cell is complete at (see `CellSteps`). A
    /// research fed by the cell links from these with the lag the packs
    /// take; a cell that already stood contributes none.
    pub carriers: Vec<ActionId>,
    /// The cell's own tempo, in ticks per product.
    pub ticks_per_item: Ticks,
}

/// Build enough cells to make `item` at `per_minute`, sinking into `sink`.
///
/// [`BuildAssemblyCell::expand`]'s whole body, as a function, because
/// `crate::method::have::Researched` has to build the cell a research is fed
/// from **inline** -- a subgoal is expanded after the method returns, and the
/// research action has to name the labs the cell placed. `bots` is the
/// roster the cell's bundles are dealt across.
pub fn build_cells(
    ctx: &mut ExpansionCtx,
    bots: &[BotId],
    item: &str,
    per_minute: u32,
    sink: Sink,
) -> Result<CellPlan, PlannerError> {
    let spec = assembly_spec(&ctx.state, item).ok_or_else(|| PlannerError::NoCellProduces {
        item: item.to_string(),
    })?;
    let needed = cells_for(per_minute, spec.ticks_per_item)?;
    // What already stands counts towards the goal, or a replan after a
    // partial build doubles the factory. `AlreadySatisfied` covers only the
    // case where *all* of it stands, and it answers from the very same two
    // functions, so the two cannot disagree about how far along we are.
    let standing_labs = standing_lab_sinks(&ctx.state, &spec);
    let build = needed.saturating_sub(cells_standing(&ctx.state, &spec));
    if build == 0 {
        return Ok(CellPlan {
            steps: Vec::new(),
            labs: standing_labs,
            carriers: Vec::new(),
            ticks_per_item: spec.ticks_per_item,
        });
    }
    {
        // THE SOURCES, before anything is sited: every belted input of every
        // cell to build has to be delivered by a standing stage-1 cell with
        // rate to spare, or the goal is refused by name here and nothing is
        // placed. See `assign_sources`.
        // Each cell to build carries its share of the rate asked for.
        let cell_per_minute = per_minute.div_ceil(needed.max(1));
        let supplies = assign_sources(&ctx.state, &spec, build, cell_per_minute)?;

        // WHERE THE CELL IS SITED FROM.
        //
        // A cell is sited from an anchor, and the anchor search starts from
        // here. Started from the bot, a cell lands wherever the roster
        // happened to be standing, and a stage-1 cell already making the
        // supplied ingredient could be anywhere at all -- most often further
        // than the one `enclosure::window` a belt run is planned in, which
        // would make the link refuse for a reason that is about the roster's
        // position rather than about the ground.
        //
        // So the siting starts from the sources: the **centroid** of the
        // first cell's source chests, because a run is planned in a window
        // centred on its source and a red cell has two -- iron and copper,
        // on seed 31337 some forty tiles apart -- so a cell sited beside one
        // of them is outside the other's window. Nothing else about the
        // search changes: `supply_anchor` still adopts an existing network
        // in preference to building a plant, and still refuses by name when
        // there is no room. A cell with no belted input (none in vanilla)
        // is sited from the bot, exactly as before.
        let from = supplies
            .first()
            .filter(|supply| !supply.inputs.is_empty())
            .map(|supply| {
                let n = supply.inputs.len().to_f64().unwrap_or(1.);
                let (x, y) = supply.inputs.iter().fold((0., 0.), |(x, y), (_, source)| {
                    (x + source.buffer.x(), y + source.buffer.y())
                });
                Position::new((x / n).floor() + 0.5, (y / n).floor() + 0.5)
            })
            .or_else(|| ctx.state.bot(ctx.chain_actor).map(|b| b.position.clone()))
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
        //
        // Only the *supply* half of `power::ensure_powered` is wanted here.
        // This method does not lay poles: it chooses where the cells stand
        // *from* the anchor, so the anchor is an input to siting rather than
        // somewhere a run has to reach. `supply_anchor` is the half both share.
        let want_kw = cell_demand_kw(&ctx.state, &spec, sink) * f64::from(build);
        let (anchor, mut plant_steps_taken, mut power_links) =
            supply_anchor(ctx, &from, ANCHOR_SEARCH_RADIUS, want_kw)?;
        // THE POWER COMES TO THE SOURCES, not the cell to the power.
        //
        // A cell is sited within `CELL_SEARCH_RADIUS` of its anchor, and a
        // run into it is planned in one `enclosure::window` centred on its
        // source -- so every source chest has to be within the window's
        // half-side, less the siting ring, of the anchor, or the run refuses
        // "further apart than one search window reaches". The plant stands
        // at the shore and the ore does not: on seed 31337 the iron chest is
        // 41 tiles from where the plant's pole put the cell (measured
        // 2026-09-09, `a cell already makes iron-plate at [-5.5,-29.5] and
        // nothing can carry it to ... [35.5,-27.5]`). So when the anchor is
        // too far from any source, a wire of poles is run from the network
        // to the sources' centroid -- `ensure_powered`, the same function
        // that wires each run's load arm -- and the cell is sited off the
        // last pole of that wire. A plan whose anchor already sits among
        // its sources runs no wire and is byte-identical.
        //
        // **A cell already standing keeps its anchor.** Its site is fixed,
        // so no wire is run for it: `complete_cell` looks round the anchor,
        // and moving the anchor to the sources would walk past the parts
        // the replan is there to finish.
        let recovered = complete_cell(
            &ctx.state,
            &anchor,
            &spec,
            sink,
            &cell_machines(&ctx.state, &spec)
                .iter()
                .map(Pos::from)
                .collect(),
            None,
        )
        .is_some();
        let anchor = if !recovered
            && supplies.first().is_some_and(|supply| {
                supply.inputs.iter().any(|(_, source)| {
                    let reach = crate::enclosure::SEARCH_RADIUS - f64::from(CELL_SEARCH_RADIUS);
                    (source.buffer.x() - anchor.x()).abs() > reach
                        || (source.buffer.y() - anchor.y()).abs() > reach
                })
            }) {
            // Never AT a chest: a wire aimed at the one source of a
            // one-input cell put its poles on the chest's three free sides
            // and the run out of it refused with all four named. The
            // target is stepped away from any chest it would land beside,
            // towards the network the wire comes from.
            let mut target = from.clone();
            for (_, source) in supplies.iter().flat_map(|supply| supply.inputs.iter()) {
                let chest = &source.buffer;
                if (target.x() - chest.x()).abs() < 4. && (target.y() - chest.y()).abs() < 4. {
                    let (dx, dy) = (anchor.x() - chest.x(), anchor.y() - chest.y());
                    let step = if dx.abs() >= dy.abs() {
                        Position::new(6. * dx.signum(), 0.)
                    } else {
                        Position::new(0., 6. * dy.signum())
                    };
                    target = chest.add(&step);
                }
            }
            let from = target;
            let Some(area) = ctx.state.collision_area(spec.machine, &from) else {
                return Err(PlannerError::NoRoomForCellNearPower {
                    item: spec.item.clone(),
                    radius: CELL_SEARCH_RADIUS,
                });
            };
            match crate::method::power::ensure_powered(
                ctx,
                spec.machine,
                &from,
                &area,
                want_kw,
                ANCHOR_SEARCH_RADIUS,
                &[],
            )? {
                Some(powering) => {
                    // The wire's last pole: the one nearest the sources'
                    // centroid, which is what the cell is then sited off.
                    let mut poles: Vec<Position> = Vec::new();
                    collect_poles(&powering.steps, &mut poles);
                    let last = poles.into_iter().min_by(|a, b| {
                        calculate_distance(a, &from)
                            .total_cmp(&calculate_distance(b, &from))
                            .then(a.x.total_cmp(&b.x))
                            .then(a.y.total_cmp(&b.y))
                    });
                    plant_steps_taken.extend(powering.steps);
                    power_links.extend(powering.ids);
                    last.unwrap_or(anchor)
                }
                None => {
                    return Err(PlannerError::NoRoomForCellNearPower {
                        item: spec.item.clone(),
                        radius: CELL_SEARCH_RADIUS,
                    });
                }
            }
        } else {
            anchor
        };
        let near: Vec<Vec<Position>> = supplies
            .iter()
            .map(|supply| {
                supply
                    .inputs
                    .iter()
                    .map(|(_, source)| source.buffer.clone())
                    .collect()
            })
            .collect();
        let cells = plan_cells_near(&ctx.state, &anchor, &spec, sink, build, &near, None)?;
        // How long the cells run on what their sources will deliver -- the
        // horizon every hand-fed quantity is sized off. See `SupplyHorizon`.
        let feeds: Vec<CellFeed<'_>> = supplies
            .iter()
            .map(|supply| CellFeed {
                supply,
                horizon: horizon_for(&spec, supply),
            })
            .collect();
        let horizon_ticks = feeds
            .iter()
            .map(|feed| feed.horizon.ticks(&spec))
            .min()
            .unwrap_or(CELL_CHARGE_TICKS);
        let (coal, boilers) = fuel_for(&ctx.state, &anchor, &cells, &spec, horizon_ticks);
        let roster = BuildAssemblyCell::roster_of(bots, ctx.chain_actor);
        let CellSteps {
            steps: built,
            needs_power,
            carriers,
        } = cell_steps(ctx, &spec, &cells, coal, &boilers, &roster, &feeds)?;
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
        // The labs, chain order, cell by cell: what this plan places and
        // what already stood.
        let mut labs = standing_labs;
        for cell in &cells {
            for k in 0..cell.sink.labs() {
                if let Some(part) = cell.at(Role::Lab(k)) {
                    labs.push(part.position.clone());
                }
            }
        }
        Ok(CellPlan {
            steps,
            labs,
            carriers,
            ticks_per_item: spec.ticks_per_item,
        })
    }
}

/// The labs every complete cell for `spec` feeds, chain order: the sink of
/// each product machine's output arm when it is a lab, and every lab that
/// lab hands on to through an inserter.
///
/// Found the way [`is_drained`] finds a sink, then followed: nothing here
/// reads the layout table, so a chain somebody built by hand counts as well
/// as one this method placed.
pub fn standing_lab_sinks(state: &PlanState, spec: &AssemblySpec) -> Vec<Position> {
    let nearby = state.entities_within(&Position::new(0., 0.), CELL_SCAN_RADIUS);
    let mut out: Vec<Position> = Vec::new();
    for product in complete_cells(state, spec, None) {
        let mut from = product;
        // A lab hands on to at most one lab (one south face); bounded so a
        // ring of labs -- which the game would allow -- cannot loop this.
        for _ in 0..usize::from(u8::MAX) {
            let next = nearby
                .iter()
                .filter(|inserter| inserter.name == INSERTER)
                .filter(|inserter| state.delivers_into(&from, &inserter.position))
                .find_map(|inserter| {
                    nearby.iter().find(|lab| {
                        lab.name == LAB
                            && lab.position != from
                            && !out.contains(&lab.position)
                            && state.delivers_into(&inserter.position, &lab.position)
                    })
                });
            let Some(lab) = next else {
                break;
            };
            out.push(lab.position.clone());
            from = lab.position.clone();
        }
    }
    out
}

/// How much coal the cells' own network needs over `ticks`, and where it
/// goes.
///
/// The demand is read off a fork with every cell standing, so it is the whole
/// network's draw — the cells, and whatever else was already on it — rather
/// than the cells' own. A boiler fuelled for the cells alone would run the lab
/// beside them dry. `ticks` is the [`SupplyHorizon`]; the answer is capped at
/// one fuel slot whatever the horizon, which is all a hand can fill.
fn fuel_for(
    state: &PlanState,
    anchor: &Position,
    cells: &[Cell],
    spec: &AssemblySpec,
    ticks: Ticks,
) -> (u32, Vec<Position>) {
    let mut trial = state.fork();
    for cell in cells {
        if reserve_in(&mut trial, cell, spec).is_err() {
            return (0, Vec::new());
        }
    }
    let Some(ground) = cells.first().and_then(Cell::on_network_at) else {
        return (0, Vec::new());
    };
    let Some(area) = trial.collision_area(spec.machine, ground) else {
        return (0, Vec::new());
    };
    let demand = trial.electric_demand_kw(&area, None);
    let boilers = boilers_near(state, anchor);
    if boilers.is_empty() {
        return (0, Vec::new());
    }
    // **Split, not repeated.** The boilers of one chain share the network's
    // load, so each carries `demand / n` of it and is charged for that -- a
    // full charge each would put `n` times the coal the network burns into
    // slots that hold one stack, and `boiler_coal`'s cap would then quietly
    // truncate the difference rather than report it. With one boiler this is
    // the identical arithmetic to the single-boiler version.
    let share = demand / boilers.len() as f64;
    (boiler_coal(state, share, ticks), boilers)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::method::expand;
    use crate::method::have::registry_for;
    use crate::network::ActionNetwork;
    use factorio_bot_core::factorio::world::FactorioSurface;
    use std::sync::Arc;

    const PACK: &str = "automation-science-pack";
    const GREEN: &str = "logistic-science-pack";

    /// The shared fixture plus the two recipes the 1.1 capture never had.
    ///
    /// Ingredients and energy are the **live 2.1.17** ones, asserted against
    /// the capture in `tests/red_science_cell.rs`. They are added `enabled` so
    /// these tests are about the layout rather than about the research ladder.
    fn world() -> FactorioSurface {
        // `world_with_technologies` rather than `fixture_world`: the force's
        // technology table is what says an item is a science pack (see
        // `crate::method::have::Researched`), and several tests here read it.
        let world = crate::test_world::world_with_technologies();
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
        // The live 2.1.17 steel recipe: five iron plates, sixteen seconds,
        // **smelting**. It is the one-machine cell's whole reason to exist,
        // and it is added here rather than to the shared fixture because the
        // shared fixture is what every other method's tests read.
        let steel: factorio_bot_core::types::FactorioRecipe =
            factorio_bot_core::serde_json::from_str(
                r#"{
              "name": "steel-plate",
              "valid": true,
              "enabled": true,
              "category": "smelting",
              "ingredients": [
                { "name": "iron-plate", "ingredient_type": "item", "amount": 5 }
              ],
              "products": [
                { "name": "steel-plate", "product_type": "item", "amount": 1, "probability": 1.0 }
              ],
              "hidden": false,
              "energy": 16.0,
              "order": "c",
              "group": "intermediate-products",
              "subgroup": "raw-material"
            }"#,
            )
            .expect("the steel recipe parses");
        world
            .update_recipes(vec![steel])
            .expect("update_recipes cannot fail for a well-formed recipe");
        world
    }

    fn bare(bots: &[BotId]) -> PlanState {
        PlanState::from_world(Arc::new(world()), bots)
    }

    /// Ore per tile under every source drill in [`world_with_sources`].
    ///
    /// A number the tests can do arithmetic on: a burner drill covers four
    /// tiles, so one source holds `4 * SOURCE_ORE_PER_TILE` ore and, at one
    /// plate an ore, that many plates. The horizon tests derive their
    /// expectations from it rather than quoting a figure.
    const SOURCE_ORE_PER_TILE: u32 = 100;

    /// Where the two sources of the standard powered fixture stand: their
    /// drills, north-facing, on a tile boundary as a 2x2 is. Chosen so that
    /// both plate chests are inside one `connect` window of a cell sited off
    /// the plant's pole at `(10.5, 10.5)`, and the chests' centroid --
    /// which is where the cell is sited from -- is nearer that pole than
    /// anything else.
    const IRON_DRILL: (f64, f64) = (22., 2.);
    const COPPER_DRILL: (f64, f64) = (22., 24.);

    /// The shared world plus a **standing stage-1 cell for each plate**, each
    /// dropping its plates into a chest -- what a belted assembly cell is
    /// sourced from.
    ///
    /// Per source, in the base world (ore lives in the entity graph, and a
    /// replan sees a source through the same door): a 4x4 patch of the ore
    /// at `SOURCE_ORE_PER_TILE` a tile, a burner drill on it facing north,
    /// the stone furnace it drops into two tiles north
    /// (`produce::FURNACE_OFFSET`), a burner arm on the furnace's east face
    /// carrying the plates out, and the iron chest it drops them into. Built
    /// by hand rather than by `produce::plan_cell`, so what stands is exactly
    /// what the test says; every predicate the method reads it with --
    /// `standing_cells`, `delivers_into`, `is_container`, `cell_yield` --
    /// is the real one, and `a_world_with_no_standing_cell_offers_no_supply_source`
    /// is the control that the plain world offers nothing.
    fn world_with_sources(iron: (f64, f64), copper: (f64, f64)) -> FactorioSurface {
        world_with_sources_holding(iron, copper, SOURCE_ORE_PER_TILE)
    }

    /// [`world_with_sources`] with `per_tile` ore under each drill.
    fn world_with_sources_holding(
        iron: (f64, f64),
        copper: (f64, f64),
        per_tile: u32,
    ) -> FactorioSurface {
        use factorio_bot_core::factorio::util::add_to_rect;
        use factorio_bot_core::types::{FactorioEntity as E, Rect};
        let world = world();
        let mut entities = Vec::new();
        for (ore, (dx, dy)) in [("iron-ore", iron), ("copper-ore", copper)] {
            let drill = Position::new(dx, dy);
            let mut patch = Vec::new();
            factorio_bot_core::test_utils::spawn_ore(
                &mut patch,
                add_to_rect(&Rect::from_wh(4., 4.), &drill),
                ore,
            );
            for tile in &mut patch {
                tile.amount = Some(per_tile);
            }
            entities.extend(patch);
            entities.push(E::new_burner_mining_drill(&drill, Direction::North));
            entities.push(E::new_stone_furnace(
                &Position::new(dx, dy - 2.),
                Direction::North,
            ));
            // Picks up from the furnace to its west, drops into the chest to
            // its east: `direction` names the side an arm picks up from.
            entities.push(E::new_named_inserter(
                "burner-inserter".into(),
                &Position::new(dx + 1.5, dy - 2.5),
                Direction::West,
            ));
            entities.push(crate::test_world::iron_chest(&Position::new(
                dx + 2.5,
                dy - 2.5,
            )));
        }
        world
            .update_chunk_entities(entities)
            .expect("a fixture world accepts two stage-1 cells");
        world
    }

    fn bare_with_sources(bots: &[BotId]) -> PlanState {
        PlanState::from_world(Arc::new(world_with_sources(IRON_DRILL, COPPER_DRILL)), bots)
    }

    /// The plates one source will make before its ore is dug out.
    fn source_plates() -> u32 {
        4 * SOURCE_ORE_PER_TILE
    }

    /// What one red cell on the standard sources is sized for: the fewer of
    /// `iron / 2` and `copper / 1`, both sources being equal.
    fn red_horizon() -> u32 {
        source_plates() / 2
    }

    /// What one steel cell on the standard sources is sized for: five iron
    /// plates a steel.
    fn steel_horizon() -> u32 {
        source_plates() / 5
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
        stand_plant(&mut state, bots);
        state
    }

    /// [`powered`] on [`world_with_sources`]: a plant, and a standing source
    /// for each of red science's plates. What `plan` and every test that
    /// expands a red goal run on.
    fn powered_with_sources(bots: &[BotId]) -> PlanState {
        let mut state = bare_with_sources(bots);
        stand_plant(&mut state, bots);
        // The load arm at each source chest is some fifteen tiles from the
        // plant's pole and gets a wire of its own, and the fixture has no
        // tree a bot could reach for the poles' wood.
        for bot in bots {
            state.gain(*bot, "wood", 8);
        }
        state
    }

    fn stand_plant(state: &mut PlanState, bots: &[BotId]) {
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
                .globals
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
    }

    fn spec() -> AssemblySpec {
        assembly_spec(&bare(&[BotId(1)]), PACK).expect("red science is a two-ingredient craft")
    }

    /// Red science's spec with its intermediate widened to `feeds`
    /// ingredients, for the geometry tests that vary the feed side alone.
    ///
    /// `layout` reads the feed count off the spec now, because the spec is
    /// also what carries the product machine and its offset -- a furnace cell
    /// differs from a red one in all three at once.
    fn spec_feeds(feeds: usize) -> AssemblySpec {
        let mut spec = spec();
        let made = spec.intermediate.as_mut().expect("red science has one");
        made.ingredients = vec![("iron-plate".to_string(), 1); feeds];
        // **Every ingredient in a chest**: these are the geometry tests of
        // the chest layout, which green science still uses, and red's own
        // spec has no chest but the output one since its plates are belted.
        spec.belted.clear();
        spec
    }

    /// A cell standing in `state`, built the way the planner would build it,
    /// recipes and all.
    fn stand_a_cell(state: &mut PlanState) -> Cell {
        stand_a_cell_for(state, &spec())
    }

    fn stand_a_cell_for(state: &mut PlanState, spec: &AssemblySpec) -> Cell {
        stand_a_cell_at(state, &Position::new(10.5, 10.5), spec)
    }

    fn stand_a_cell_at(state: &mut PlanState, anchor: &Position, spec: &AssemblySpec) -> Cell {
        let cell = plan_cell(state, anchor, spec, None).expect("the fixture has room beside its plant");
        for part in &cell.parts {
            let entity = entity_for(state, part);
            state.create_entity(entity);
        }
        // And the runs into its mouths, as `connect` would leave them: an arm
        // on the arm tile picking up from a belt on the belt tile. A cell is
        // not whole without them -- `loaded_feeders` counts them.
        for entity in mouth_entities(&cell) {
            state.create_entity(entity);
        }
        state
            .set_recipe(
                &cell.at(Role::Intermediate).unwrap().position,
                &spec.intermediate.as_ref().unwrap().recipe.name,
            )
            .expect("the machine was just placed");
        state
            .set_recipe(&cell.at(Role::Product).unwrap().position, &spec.recipe.name)
            .expect("the machine was just placed");
        cell
    }

    /// The end of a run into each of the cell's mouths: the unload arm
    /// facing the belt tile (west, in the north frame -- an arm's direction
    /// is the side it picks up from) and one belt tile pouring towards it.
    fn mouth_entities(cell: &Cell) -> Vec<FactorioEntity> {
        let mut out = Vec::new();
        for mouth in &cell.mouths {
            let arm = compose(Direction::West, cell.facing).expect("a cardinal facing");
            let belt = compose(Direction::East, cell.facing).expect("a cardinal facing");
            out.push(FactorioEntity::new_named_inserter(
                INSERTER.into(),
                &mouth.arm,
                arm,
            ));
            out.push(FactorioEntity::new_transport_belt(&mouth.belt, belt));
        }
        out
    }

    fn plan(per_minute: u32) -> Result<ActionNetwork, PlannerError> {
        let bots = [BotId(1)];
        let state = powered_with_sources(&bots);
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

    // ---- room for a beacon ------------------------------------------------

    /// The reserved ground is real ground, and it is exactly as wide as the
    /// beacon's own footprint.
    ///
    /// A furnace in **any** of the three flank columns refuses the site, and
    /// a furnace well east of them does not -- the control, without which
    /// this would also pass if merely adding an entity anywhere refused.
    ///
    /// **The width is pinned elsewhere, deliberately.** The column
    /// immediately past the flank cannot be the control: at the origin this
    /// test finds it refuses too, for reasons of the fixture's own that
    /// predate this reservation (the flank is columns 2, 3 and 4, so column 5
    /// is not ground this code looks at). The claim "three columns, and three
    /// because the beacon is 3x3" is checked directly on the tile list in
    /// `a_world_without_beacons_reserves_no_ground_for_them`, and the
    /// derivation itself in
    /// `crate::method::util::tests::the_lane_follows_the_prototype_and_not_the_number_three`.
    #[test]
    fn a_cell_leaves_room_for_a_beacon_to_its_east() {
        let spec = spec();
        let origin = (6..=20)
            .flat_map(|x| {
                (6..=20).map(move |y| Position::new(f64::from(x) + 0.5, f64::from(y) + 0.5))
            })
            .find(|origin| {
                fit(
                    &powered(&[BotId(1)]),
                    origin,
                    Direction::North,
                    true,
                    &spec,
                    Sink::Labs(1),
                )
                .is_some()
            })
            .expect("the fixture has room for a cell somewhere beside its plant");

        let blocked = |dx: f64| {
            let mut state = powered(&[BotId(1)]);
            state.create_entity(FactorioEntity {
                name: "stone-furnace".into(),
                entity_type: "furnace".into(),
                position: origin.add(&Position::new(dx, 0.)),
                ..Default::default()
            });
            fit(
                &state,
                &origin,
                Direction::North,
                true,
                &spec,
                Sink::Labs(1),
            )
            .is_none()
        };

        let width = beacon_geometry(&powered(&[BotId(1)]), BEACON)
            .expect("the fixture ships a beacon")
            .lane_tiles();
        assert!(
            (width - 3.0).abs() < f64::EPSILON,
            "vanilla's beacon is 3x3; this test's columns are derived from that"
        );

        for column in 0..3 {
            let dx = BEACON_FLANK_FIRST_COLUMN + f64::from(column);
            assert!(
                blocked(dx),
                "a furnace at dx={dx} stands in the beacon flank and must refuse the site"
            );
        }
        // The control that stops this passing on "any extra entity refuses":
        // a furnace well clear of the flank leaves the site standing.
        assert!(
            !blocked(BEACON_FLANK_FIRST_COLUMN + 10.0),
            "a furnace ten tiles past the flank is nothing to do with the cell"
        );
    }

    /// A world with no beacon in it reserves nothing, so a mod that removes
    /// beacons pays no footprint for them.
    #[test]
    fn a_world_without_beacons_reserves_no_ground_for_them() {
        let origin = Position::new(10.5, 10.5);
        let with_beacons = powered(&[BotId(1)]);
        assert_eq!(
            beacon_flank(&with_beacons, &origin, Direction::North)
                .expect("north is a cardinal facing")
                .len(),
            3 * BEACON_FLANK_ROWS.len(),
            "a 3x3 beacon reserves three columns of the machines' seven rows"
        );

        let world = world();
        world.globals.entity_prototypes.remove(BEACON);
        let without = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert_eq!(
            beacon_flank(&without, &origin, Direction::North),
            Some(vec![]),
            "no beacon prototype means no reservation at all"
        );
    }

    // ---- what a cell is ---------------------------------------------------

    /// The shape rule, stated over recipes and checked on four items that are
    /// each a different *reason* to answer.
    #[test]
    fn only_a_two_ingredient_craft_with_exactly_one_makeable_half_is_a_cell() {
        let s = bare(&[BotId(1)]);
        let pack = assembly_spec(&s, PACK).expect("red science is the case this exists for");
        assert_eq!(pack.intermediate.as_ref().unwrap().item, "iron-gear-wheel");
        assert_eq!(pack.supplied, ("copper-plate".to_string(), 1));

        // Generality, not a special case: an electronic circuit is one iron
        // plate and three copper cables, and a cable is crafted from one
        // copper plate. Same shape, different numbers.
        let circuit = assembly_spec(&s, "electronic-circuit").expect("a circuit is the same shape");
        assert_eq!(circuit.intermediate.as_ref().unwrap().item, "copper-cable");
        assert_eq!(circuit.intermediate.as_ref().unwrap().per_product, 3);
        assert_eq!(
            circuit.intermediate.as_ref().unwrap().per_run,
            2,
            "a cable recipe yields two"
        );
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
            assembly_spec(&s, "lab").is_some(),
            "three ingredients now work: the supply chest charges two types"
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
        assert_eq!(green.intermediate.as_ref().unwrap().item, "transport-belt");
        assert_eq!(
            green.intermediate.as_ref().unwrap().ingredients,
            vec![
                ("iron-plate".to_string(), 1),
                ("iron-gear-wheel".to_string(), 1)
            ],
            "one feed chest per ingredient, in the recipe's own order"
        );
        assert_eq!(
            green.intermediate.as_ref().unwrap().per_run,
            2,
            "a belt recipe yields two"
        );
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
        // hand, and six runs of a belt recipe that yields two -- whose iron
        // is BELTED (a stage-1 cell smelts it) and whose gears are not.
        // So the gear chest is on feed row 1, and row 0 is a mouth.
        assert_eq!(green.charge_products(), 12);
        assert_eq!(green.intermediate_runs(), 6);
        assert!(green.is_belted("iron-plate"));
        assert!(!green.is_belted("iron-gear-wheel"));
        assert!(!green.is_belted("inserter"));
        assert_eq!(
            green.feed_charges(),
            vec![(1, "iron-gear-wheel".to_string(), 6)]
        );
        assert_eq!(green.supply_charge(), 12);
        let belted: Vec<(String, Role, Option<u8>, u32)> = green
            .belted_inputs()
            .into_iter()
            .map(|input| (input.item, input.into, input.feed_row, input.per_product))
            .collect();
        assert_eq!(
            belted,
            vec![("iron-plate".to_string(), Role::Intermediate, Some(0), 1)],
            "one iron plate a pack: a belt recipe makes two from one plate and one gear"
        );
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
            pack.intermediate.as_ref().unwrap().item,
            "iron-gear-wheel",
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
            smelting_ticks(
                &bare(&[BotId(1)]),
                &spec.intermediate.as_ref().unwrap().recipe,
                MACHINE
            ),
            60
        );
        assert_eq!(cells_for(6, spec.ticks_per_item).unwrap(), 1);
        assert_eq!(cells_for(7, spec.ticks_per_item).unwrap(), 2);
        assert_eq!(cells_for(12, spec.ticks_per_item).unwrap(), 2);
    }

    /// The charge, integer end to end and rounded at the run boundary.
    #[test]
    fn the_charge_is_integer_arithmetic_from_the_recipe() {
        // Red's real spec has no chest to charge: both plates are belted.
        let belted = spec();
        assert_eq!(
            belted.feed_charges(),
            Vec::new(),
            "the iron comes down a belt"
        );
        assert_eq!(belted.supply_charge(), 0, "and so does the copper");
        let demands: Vec<(String, u32)> = belted
            .belted_inputs()
            .iter()
            .map(|input| (input.item.clone(), belted.demand_per_minute(input, 6)))
            .collect();
        assert_eq!(
            demands,
            vec![
                ("iron-plate".to_string(), 12),
                ("copper-plate".to_string(), 6)
            ],
            "six packs a minute is two iron plates and one copper plate each"
        );
        // The arithmetic of a charge, on the same spec with its chests back.
        let spec = spec_feeds(1);
        let mut spec = spec;
        spec.intermediate.as_mut().unwrap().ingredients = vec![("iron-plate".to_string(), 2)];
        assert_eq!(spec.charge_products(), 15, "9000 ticks / 600 a pack");
        assert_eq!(
            spec.feed_charges(),
            vec![(0, "iron-plate".to_string(), 30)],
            "fifteen gears at two iron plates each, in one chest"
        );
        assert_eq!(spec.supply_charge(), 15, "one copper plate a pack");

        // The yield-of-two case, where the intermediate machine and not the
        // product machine is the bottleneck: three cables a circuit at 60
        // ticks per two-cable run is 90 ticks, against the circuit's own 60.
        let s = bare(&[BotId(1)]);
        let mut circuit = assembly_spec(&s, "electronic-circuit").unwrap();
        assert_eq!(
            circuit.ticks_per_item, 90,
            "the cable machine is the slower half"
        );
        assert_eq!(circuit.charge_products(), 100, "9000 ticks / 90 a circuit");
        assert!(
            circuit.is_belted("copper-plate"),
            "copper is smelted, so belted"
        );
        circuit.belted.clear();
        assert_eq!(
            circuit.feed_charges(),
            vec![(0, "copper-plate".to_string(), 150)],
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
        spec.intermediate.as_mut().unwrap().per_product = 1;
        spec.intermediate.as_mut().unwrap().per_run = 2;
        spec.intermediate.as_mut().unwrap().ingredients = vec![("iron-plate".to_string(), 3)];
        spec.belted.clear();
        assert_eq!(spec.charge_products(), 9);
        assert_eq!(
            spec.feed_charges(),
            vec![(0, "iron-plate".to_string(), 15)],
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
                for part in layout(&origin, facing, true, &spec_feeds(feeds), Sink::Chest)
                    .expect("a cardinal facing")
                {
                    let name = part.name;
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
            let parts = layout(
                &Position::new(10.5, 10.5),
                facing,
                true,
                &spec_feeds(feeds),
                Sink::Chest,
            )
            .unwrap();
            for (i, a) in parts.iter().enumerate() {
                for b in parts.iter().skip(i + 1) {
                    let a_box = s
                        .collision_area_facing(a.name, &a.position, a.direction)
                        .unwrap();
                    let b_box = s
                        .collision_area_facing(b.name, &b.position, b.direction)
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
    /// Eight links for a one-feed cell and ten for a two-feed one, and every
    /// one of them a chance for a rotation that is wrong by a quarter turn to
    /// place perfectly and move nothing. Asked of a fork with the cell
    /// standing, so it is the same `delivers_into` the `Condition::Feeds` on
    /// the charge inserts is checked with.
    ///
    /// The last two are the output pair, and they are the ones this test would
    /// most like to catch: `Role::OutputInserter` is the only inserter in the
    /// cell that runs *out* of a machine, and `delivers_into` reaches it
    /// through a different disjunct than every other link here.
    #[test]
    fn every_link_of_the_chain_delivers_at_every_facing() {
        for feeds in 1..=MAX_FEED {
            for facing in Direction::orthogonal() {
                let s = bare(&[BotId(1)]);
                let origin = Position::new(10.5, 10.5);
                let cell = Cell {
                    origin: origin.clone(),
                    facing,
                    parts: layout(&origin, facing, true, &spec_feeds(feeds), Sink::Chest).unwrap(),
                    standing: Vec::new(),
                    lane: lane(&origin, facing, &spec_feeds(feeds), Sink::Chest).unwrap(),
                    mouths: Vec::new(),
                    sink: Sink::Chest,
                    evacuate: Vec::new(),
                };
                let mut trial = s.fork();
                for part in &cell.parts {
                    trial.create_entity(entity_for(&s, part));
                }
                let chain = links(&cell).unwrap();
                assert_eq!(
                    chain.len(),
                    6 + 2 * feeds,
                    "two links per chest, plus the output pair"
                );
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
            .chain([
                Role::LinkInserter,
                Role::SupplyInserter,
                Role::OutputInserter,
            ]);
        for role in roles {
            let s = bare(&[BotId(1)]);
            let origin = Position::new(10.5, 10.5);
            let mut parts = layout(
                &origin,
                Direction::North,
                true,
                &spec_feeds(MAX_FEED),
                Sink::Chest,
            )
            .unwrap();
            for part in parts.iter_mut() {
                if part.role == role {
                    part.direction = compose(part.direction, Direction::South).unwrap();
                }
            }
            let cell = Cell {
                origin: origin.clone(),
                facing: Direction::North,
                parts,
                standing: Vec::new(),
                lane: lane(
                    &origin,
                    Direction::North,
                    &spec_feeds(MAX_FEED),
                    Sink::Chest,
                )
                .unwrap(),
                mouths: Vec::new(),
                sink: Sink::Chest,
                evacuate: Vec::new(),
            };
            let turned = cell.at(role).unwrap();
            assert!(
                s.is_area_free_facing(turned.name, &turned.position, turned.direction),
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
            let parts = layout(&origin, facing, true, &spec_feeds(feeds), Sink::Chest).unwrap();
            let lane = lane(&origin, facing, &spec_feeds(feeds), Sink::Chest).unwrap();
            assert_eq!(
                lane.len(),
                feeds + 2,
                "one tile behind every feed chest, the supply chest's and the output chest's"
            );
            for tile in &lane {
                for part in &parts {
                    let box_ = s
                        .collision_area_facing(part.name, &part.position, part.direction)
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
        let parts = layout(
            &Position::new(10.5, 10.5),
            Direction::North,
            true,
            &spec_feeds(MAX_FEED),
            Sink::Chest,
        )
        .unwrap();
        let lane = lane(
            &Position::new(10.5, 10.5),
            Direction::North,
            &spec_feeds(MAX_FEED),
            Sink::Chest,
        )
        .unwrap();
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

    /// Sources for a plant at `(30.5, 32.5)` with clear ground round its pole:
    /// their chests either side of it, centroid beside the pole.
    const IRON_DRILL_BY_THE_ROOM: (f64, f64) = (22., 22.);
    const COPPER_DRILL_BY_THE_ROOM: (f64, f64) = (40., 22.);

    /// A pole and a steam engine on one network, with clear ground around the
    /// pole for a cell to be sited in, on a world with a source for each plate.
    /// The generator is seven tiles south on a second pole wired to the first
    /// (a small pole reaches 7.5), which is what leaves the first pole's own
    /// supply area empty.
    fn powered_with_room_and_sources(bots: &[BotId]) -> PlanState {
        let mut state = PlanState::from_world(
            Arc::new(world_with_sources(
                IRON_DRILL_BY_THE_ROOM,
                COPPER_DRILL_BY_THE_ROOM,
            )),
            bots,
        );
        for bot in bots {
            state.gain(*bot, "wood", 8);
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
        let state = powered_with_room_and_sources(&bots);
        let cell = plan_cell(&state, &Position::new(30.5, 32.5), &spec(), None)
            .expect("there is room beside that pole");
        assert!(
            !cell.brings_pole(),
            "an existing pole covers this cell; bringing another spends a wood to duplicate it"
        );
        assert_eq!(
            cell.parts.len(),
            6,
            "two machines, the link, the output arm, the lab it drops into and the lab's \
             pole -- no chest for a belted plate, no output chest, and no pole of the \
             cell's own"
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
        // No pole of the CELL's anywhere in the plan. The runs from the
        // sources bring poles of their own for their load arms (the source
        // is a burner cell with no network), and those stand off the cell's
        // ground -- so what is asserted is that no pole lands inside the
        // cell's own layout, where its own pole would have gone.
        let own_pole = cell.origin.add(
            &Position::new(POLE_OFFSET.0, POLE_OFFSET.1)
                .turn(cell.facing)
                .unwrap(),
        );
        let placed_poles: Vec<Position> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Place { entity } if entity.name == POLE => {
                    Some(entity.position.clone())
                }
                _ => None,
            })
            .collect();
        assert!(
            !placed_poles.contains(&own_pole),
            "the cell placed the pole it did not need at {own_pole}: {placed_poles:?}"
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
        let cell = plan_cell(&powered(&[BotId(1)]), &Position::new(10.5, 10.5), &spec(), None)
            .expect("the fixture has room for a cell that carries its own pole");
        assert!(cell.brings_pole());
        assert_eq!(
            cell.parts.len(),
            7,
            "the six parts of a red cell and its pole"
        );
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
            let parts = layout(
                &Position::new(10.5, 10.5),
                facing,
                true,
                &spec_feeds(feeds),
                Sink::Chest,
            )
            .unwrap();
            let pole = parts.iter().find(|p| p.role == Role::Pole).unwrap();
            let consumers = parts
                .iter()
                .filter(|p| s.consumer_draw_kw(p.name).is_some());
            assert_eq!(
                consumers.clone().count(),
                5 + feeds,
                "two machines, one inserter per feed chest, a link, a supply and an output"
            );
            for part in consumers {
                let area = s
                    .collision_area_facing(part.name, &part.position, part.direction)
                    .unwrap();
                assert!(
                    s.pole_would_supply(POLE, &pole.position, &area),
                    "the cell's own pole does not reach its {:?} at {facing:?}",
                    part.role
                );
            }
        }
    }

    /// The other half of the claim above: what the cell's own pole does
    /// **not** reach, and the mouth it does — which the output path now
    /// stands in.
    ///
    /// [`MAX_FEED`] is two because the pole cannot light a third feed row, and
    /// that was a comment rather than a check -- so a change to
    /// [`POLE_OFFSET`] could have made it wrong in either direction with
    /// nothing failing. Both directions are pinned here, at all four facings:
    ///
    /// * the intermediate machine's third west tile, `y = -1`, is **dark**.
    ///   A third feed chest costs a second pole, which is what forecloses an
    ///   `inserter` intermediate;
    /// * the product machine's west tile at `y = 3` is **lit**, and the tile
    ///   west of it at `x = -3` is serviced by the existing lane at `x = -4`.
    ///
    /// **This is the assertion that changed, and it is worth saying how.** It
    /// used to close by requiring both tiles to be *empty* -- the measurement
    /// `31c8d579` recorded, which established the opening this cell's output
    /// path was then built into. So the same two offsets are still checked,
    /// and the check is now the opposite one: `Role::OutputInserter` and
    /// `Role::OutputChest` stand exactly there. The powered-and-serviced half
    /// is unchanged and is what makes an output path cost no second pole; the
    /// empty half was the *opening*, and it has been spent.
    ///
    /// The asymmetry it recorded therefore no longer buys a third ingredient
    /// on the product machine: that mouth is taken, and by the part that keeps
    /// the machine from halting at four crafts. See [`MAX_FEED`].
    #[test]
    fn the_pole_lights_the_output_mouth_but_no_third_feed_row() {
        let s = bare(&[BotId(1)]);
        let origin = Position::new(10.5, 10.5);
        for facing in Direction::orthogonal() {
            let parts = layout(&origin, facing, true, &spec_feeds(MAX_FEED), Sink::Chest).unwrap();
            let pole = parts.iter().find(|p| p.role == Role::Pole).unwrap();
            let at = |offset: (f64, f64)| -> Position {
                origin.add(&Position::new(offset.0, offset.1).turn(facing).unwrap())
            };
            let lit = |offset: (f64, f64)| -> bool {
                let area = s
                    .collision_area_facing(INSERTER, &at(offset), Direction::West)
                    .unwrap();
                s.pole_would_supply(POLE, &pole.position, &area)
            };
            assert!(
                !lit((-2., -1.)),
                "a third feed inserter would stand unpowered at {facing:?} -- this is why \
                 MAX_FEED is two"
            );
            assert!(
                lit(OUTPUT_INSERTER_OFFSET),
                "the product machine's output mouth is lit at {facing:?}"
            );
            // And the cell spends it: the two tiles `31c8d579` measured as
            // powered and free are the two the output path stands on.
            for (offset, role) in [
                (OUTPUT_INSERTER_OFFSET, Role::OutputInserter),
                (OUTPUT_CHEST_OFFSET, Role::OutputChest),
            ] {
                let tile = at(offset);
                assert_eq!(
                    parts
                        .iter()
                        .find(|part| part.position == tile)
                        .map(|part| part.role),
                    Some(role),
                    "{offset:?} does not carry the {role:?} at {facing:?}"
                );
            }
            // The lane still services the output chest, so a bot can empty it
            // on the same visit it refills the feed chests.
            let lane = lane(&origin, facing, &spec_feeds(MAX_FEED), Sink::Chest).unwrap();
            assert!(
                lane.contains(&at((-4., 3.))),
                "no lane tile beside the output chest at {facing:?}"
            );
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
            // With a recipe: a crafting machine with none is charged nothing
            // by `electric_demand_kw` (it can never craft), and this fixture
            // is a *load*.
            state.create_entity(FactorioEntity {
                name: "assembling-machine-3".into(),
                entity_type: "assembling-machine".into(),
                position: Position::new(31.5, y),
                recipe: Some("iron-gear-wheel".into()),
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
            plan_cell(&island, &Position::new(10.5, 10.5), &spec(), None).is_ok(),
            "750 kW on a network the plant does not reach spends none of its budget"
        );

        let mut committed = powered(&bots);
        with_a_big_load(&mut committed, true);
        assert!(
            matches!(
                plan_cell(&committed, &Position::new(10.5, 10.5), &spec(), None),
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
            "with the engine gone the cell is ten buildings on a dead wire"
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
        let s = powered_with_sources(&[BotId(1)]);
        let net = plan(6).expect("a powered fixture can build a cell");
        // **Every electric placement carries its own claim, exactly one, at
        // the ledger's figure.** The claims used to ride on the chest
        // charges; a red cell has none, so each consumer speaks for itself
        // on its `Place` -- see `place_step`.
        let mut electric_placements = 0;
        for action in net.actions() {
            let ActionKind::Place { entity } = &action.kind else {
                continue;
            };
            let Some(billed) = s.consumer_draw_kw(&entity.name) else {
                continue;
            };
            electric_placements += 1;
            let claims: Vec<f64> = action
                .pre
                .iter()
                .filter_map(|condition| match condition {
                    Condition::Powered { pos, kw, .. } if pos == &entity.position => Some(*kw),
                    _ => None,
                })
                .collect();
            assert_eq!(
                claims,
                vec![billed],
                "{} at {} claims {claims:?} kW and is billed {billed}",
                entity.name,
                entity.position
            );
        }
        assert_eq!(
            electric_placements,
            2 + 2 + 4 + 1,
            "two machines, the link and output arms, a load and an unload arm per run, and \
             the lab"
        );
    }

    // ---- what the plan says -----------------------------------------------

    /// Both machines get a recipe, nothing is charged by hand, and the
    /// product machine's `SetRecipe` asserts the whole chain that stands
    /// behind it -- the cell's own links and both belted arms.
    #[test]
    fn the_product_recipe_asserts_the_whole_chain_and_nothing_is_charged() {
        let net = plan(6).expect("a powered fixture can build a cell");
        let charges = net
            .actions()
            .filter(|a| matches!(&a.kind, ActionKind::Insert { entity, .. } if entity == CHEST))
            .count();
        assert_eq!(charges, 0, "a red cell has no chest a bot fills");
        let product: Vec<&Action> = net
            .actions()
            .filter(|a| matches!(&a.kind, ActionKind::SetRecipe { recipe, .. } if recipe == PACK))
            .collect();
        assert_eq!(product.len(), 1, "one product machine, set once");
        let feeds: Vec<(Position, Position)> = product[0]
            .pre
            .iter()
            .filter_map(|c| match c {
                Condition::Feeds { from, to } => Some((from.clone(), to.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            feeds.len(),
            4 + 2,
            "the machine-to-machine link, the output pair, and one link per belted arm: {feeds:?}"
        );
        // And the arms the runs placed to unload into the machines are each
        // named by it, dropping into the machine the mouth serves.
        let machines: Vec<Position> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Place { entity } if entity.name == MACHINE => {
                    Some(entity.position.clone())
                }
                _ => None,
            })
            .collect();
        let mut built = powered_with_sources(&[BotId(1)]);
        for action in net.actions() {
            if let ActionKind::Place { entity } = &action.kind {
                built.create_entity((**entity).clone());
            }
        }
        let unloads: Vec<(Position, Position)> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Place { entity } if entity.name == INSERTER => machines
                    .iter()
                    .find(|machine| built.delivers_into(&entity.position, machine))
                    .map(|machine| (entity.position.clone(), machine.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            unloads.len(),
            2 + 1,
            "two unload arms and the link arm drop into a machine"
        );
        for (arm, machine) in &unloads {
            assert!(
                feeds.contains(&(arm.clone(), machine.clone())),
                "the recipe does not name the link {arm} -> {machine}: {feeds:?}"
            );
        }
    }

    /// The boiler that runs the cell is topped up for the whole HORIZON, not
    /// for one slot — the coal is split across visits by `fuel_visits` so the
    /// boiler stays lit for the full research.
    ///
    /// The horizon on the standard sources is `red_horizon()` packs -- two
    /// hundred, at 600 ticks each -- which is 120,000 ticks of ~200 kW, or
    /// some 95 coal. Before 2026-09-09 `boiler_coal` capped at one slot (50),
    /// so only 50 of the 95 went in and the boiler ran out at ~63,000 ticks.
    /// Now the full 95 coal are split into `[50, 45]` across two visits, the
    /// second chained behind the first by how long 50 coal burns at 200 kW.
    #[test]
    fn the_plants_boiler_is_topped_up_for_the_horizon() {
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
        assert!(
            !coal.is_empty(),
            "the boiler must be fuelled"
        );
        for &load in &coal {
            assert!(
                load <= COAL_STACK,
                "each visit must fit in one fuel slot: got {load}"
            );
        }
        let total: u32 = coal.iter().sum();
        assert!(
            total > COAL_STACK,
            "the full horizon needs more than one slot, got {total}"
        );
        let s = bare(&[BotId(1)]);
        let horizon = SupplyHorizon {
            products: red_horizon(),
        }
        .ticks(&spec());
        assert!(
            boiler_coal(&s, 189., horizon) > COAL_STACK
                && boiler_coal(&s, 189., CELL_CHARGE_TICKS) == 8,
            "the horizon ({horizon}) needs more than one slot; the old charge (9,000) needs 8"
        );
    }

    /// **Every boiler of a chain is topped up, not the nearest one.**
    ///
    /// The correctness bug this change had to avoid, from the assemble side:
    /// `boiler_near` returned *the* boiler, which was right while
    /// `power::Plant` carried exactly one and silently wrong the moment the
    /// plant grew a chain. Coal in one boiler of four leaves three cold, the
    /// plan reads as fully powered, and the cell it was built for stalls with
    /// every entity standing.
    ///
    /// Four boilers on `power::BOILER_PITCH_TILES`, which is the chain
    /// `power::layout` actually stands up, and the charge is **split** across
    /// them rather than repeated: the network's draw is what is burnt,
    /// however many slots it is burnt out of.
    #[test]
    fn every_boiler_of_a_chain_is_topped_up_and_the_charge_is_split() {
        let bots = [BotId(1)];
        let mut state = powered_with_sources(&bots);
        // Three more boilers alongside the fixture's one, on the real pitch.
        let pitch = crate::method::power::BOILER_PITCH_TILES;
        let entity_type = state
            .base()
            .globals
            .entity_prototypes
            .get(BOILER)
            .map(|p| p.entity_type.clone())
            .unwrap_or_else(|| BOILER.to_string());
        for step in 1..4 {
            state.create_entity(FactorioEntity {
                name: BOILER.into(),
                entity_type: entity_type.clone(),
                position: Position::new(12.5 + pitch * f64::from(step), 14.5),
                ..Default::default()
            });
        }

        let found = boilers_near(&state, &Position::new(10.5, 10.5));
        assert_eq!(
            found.len(),
            4,
            "the whole chain, not the nearest boiler: found {found:?}"
        );

        let net = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a powered fixture can build a cell");
        let mut fuelled: Vec<(Position, u32)> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Insert {
                    pos,
                    entity,
                    slot: InventorySlot::Fuel,
                    count,
                    ..
                } if entity == BOILER => Some((pos.clone(), *count)),
                _ => None,
            })
            .collect();
        fuelled.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
        // Every boiler must be topped up at least once. With multi-visit
        // refuelling (`fuel_visits`) the full charge may be spread across
        // several insert actions per boiler (e.g. 50 + 45 = 95 for the
        // full horizon), so the count of actions exceeds the boiler count.
        for boiler in &found {
            assert!(
                fuelled.iter().any(|(pos, _)| pos == boiler),
                "the boiler at {boiler} is never topped up"
            );
        }
        // Split, not repeated: one boiler takes a share of the horizon,
        // and the share is split across visits so each fits in one slot.
        let per_boiler: Vec<Vec<u32>> = {
            let mut map: Vec<Vec<(Position, u32)>> = Vec::new();
            for entry in &fuelled {
                match map.iter_mut().find(|g: &&mut Vec<(Position, u32)>| g[0].0 == entry.0) {
                    Some(group) => group.push(entry.clone()),
                    None => map.push(vec![entry.clone()]),
                }
            }
            map.iter().map(|g| g.iter().map(|(_, c)| *c).collect()).collect()
        };
        assert_eq!(
            per_boiler.len(),
            4,
            "four boilers stand, got {}: {per_boiler:?}",
            per_boiler.len()
        );
        for visits in &per_boiler {
            assert!(
                visits.iter().all(|&c| c <= COAL_STACK),
                "each visit fits in one slot: {visits:?}"
            );
            let total: u32 = visits.iter().sum();
            assert!(
                total < COAL_STACK.saturating_mul(2),
                "a quarter of the horizon's coal is under two slots: {total}"
            );
        }
    }

    /// **The ledger is written when spending the output cannot pay for the
    /// cell**, and that is asked of the recipe graph rather than of a name.
    ///
    /// `transport-belt` is the case that found it: a belt cell's charge
    /// promised belts, `Withdraw` spent them on the belt run
    /// [`crate::method::sustain`] lays to fuel the cell, and
    /// `producing:transport-belt:6` -- 307 actions on `master` -- refused with
    /// `4 transport-belt in the buffer at [33.5, -12.5] does not hold there`.
    ///
    /// The three cases in one test, because the point is the *contrast*: a
    /// pack is in nothing a cell is built from, a belt is a seed of the
    /// closure itself, and an iron plate is reached through one of the cell's
    /// own buildings rather than being one.
    #[test]
    fn the_ledger_is_withheld_from_an_item_in_its_own_cells_bill() {
        use crate::method::cellstock::{cell_output_loop, output_is_spendable};
        let state = bare(&[BotId(1)]);
        let pack = assembly_spec(&state, PACK).expect("a cell for the pack");
        assert!(
            output_is_spendable(&state, &pack),
            "nothing a cell is built or charged with has {PACK} in its bill"
        );

        let mut belt = pack.clone();
        belt.item = "transport-belt".into();
        let looped = cell_output_loop(&state, &belt).expect("sustain belts the cell's fuel");
        assert_eq!(
            looped.path,
            vec!["transport-belt".to_string()],
            "a belt IS a seed, so the loop is one step long"
        );
        assert!(
            looped
                .to_string()
                .contains("transport-belt is in its own cell's construction bill"),
            "the refusal names the loop: {looped}"
        );

        // **The case the name check refused for no reason.** Steel is not a
        // science pack, and it is also in nothing a cell is built or charged
        // with -- its own charge is iron plates, and no building here is made
        // of steel.
        let steel = assembly_spec(&state, "steel-plate").expect("a furnace cell for steel");
        assert!(
            output_is_spendable(&state, &steel),
            "steel is in no cell's construction bill: {:?}",
            cell_output_loop(&state, &steel)
        );

        let mut cable = pack.clone();
        cable.item = "copper-cable".into();
        let deep = cell_output_loop(&state, &cable).expect("the cell's pole is made of cable");
        assert!(
            deep.path.len() >= 2 && deep.path.last() == Some(&"copper-cable".to_string()),
            "the path runs from a thing the cell is made of down to the cable: {deep:?}"
        );

        // And the charge is a loop by a shorter route: `iron-plate` is not a
        // building, it is what the feed chest is filled with, and drawing the
        // output to fill the cell's own chest is the same hazard.
        let mut plate = pack.clone();
        plate.item = "iron-plate".into();
        assert!(cell_output_loop(&state, &plate).is_some());
    }

    /// **What the ground supplies is a base case**, exactly as it is in
    /// [`crate::method::machine::obtain_costs`] -- and without it the check
    /// follows recipes the plan would never run.
    ///
    /// Measured on the real seed-31337 dump before this cut existed:
    ///
    /// ```text
    /// steel-plate is in its own cell's construction bill
    ///   (coal -> water -> water-barrel -> barrel -> steel-plate)
    /// ```
    ///
    /// Every edge real, and not one of them a route a bot would take: coal is
    /// mined, water comes out of the ground, and the barrelling pair is the
    /// cycle `crate::products` documents at length. A fixture cannot carry
    /// Space Age's barrel recipes, so the *shape* is reproduced instead -- one
    /// recipe making a charted resource out of the cell's own product.
    #[test]
    fn the_walk_stops_at_what_the_ground_supplies() {
        use crate::method::cellstock::{cell_output_loop, output_is_spendable};
        fn recipe(json: &str) -> factorio_bot_core::types::FactorioRecipe {
            factorio_bot_core::serde_json::from_str(json).expect("the recipe parses")
        }
        /// Coal out of steel: absurd as a recipe, and exactly the edge the
        /// barrel path opened. `coal` is a seed of the closure because
        /// `method::sustain` fuels a cell with it.
        const COAL_FROM_STEEL: &str = r#"{
              "name": "coal-from-steel", "valid": true, "enabled": true,
              "category": "crafting",
              "ingredients": [ { "name": "steel-plate", "ingredient_type": "item", "amount": 1 } ],
              "products": [ { "name": "coal", "product_type": "item", "amount": 1, "probability": 1.0 } ],
              "hidden": false, "energy": 1.0, "order": "z",
              "group": "intermediate-products", "subgroup": "raw-material"
            }"#;
        /// The same edge onto a seed the ground does *not* supply. An iron
        /// chest is crafted, so this one genuinely closes the loop.
        const CHEST_FROM_STEEL: &str = r#"{
              "name": "iron-chest-from-steel", "valid": true, "enabled": true,
              "category": "crafting",
              "ingredients": [ { "name": "steel-plate", "ingredient_type": "item", "amount": 1 } ],
              "products": [ { "name": "iron-chest", "product_type": "item", "amount": 1, "probability": 1.0 } ],
              "hidden": false, "energy": 1.0, "order": "z",
              "group": "logistics", "subgroup": "storage"
            }"#;
        let with = |json: &str| {
            let w = world();
            w.update_recipes(vec![recipe(json)])
                .expect("a fixture accepts a recipe");
            PlanState::from_world(Arc::new(w), &[BotId(1)])
        };

        let mined = with(COAL_FROM_STEEL);
        assert!(
            mined.resource_names().iter().any(|n| n == "coal"),
            "the fixture charts coal, which is what makes it a base case"
        );
        let steel = assembly_spec(&mined, "steel-plate").expect("a furnace cell for steel");
        assert!(
            output_is_spendable(&mined, &steel),
            "coal is mined here, not synthesised out of steel: {:?}",
            cell_output_loop(&mined, &steel)
        );

        let crafted = with(CHEST_FROM_STEEL);
        let steel = assembly_spec(&crafted, "steel-plate").expect("a furnace cell for steel");
        let looped = cell_output_loop(&crafted, &steel)
            .expect("nothing supplies an iron chest, so the walk expands it");
        assert_eq!(
            looped.path,
            vec!["iron-chest".to_string(), "steel-plate".to_string()],
            "and the path names the building that closed the loop"
        );
    }

    /// **A pack a standing cell can make is DRAWN out of it, not crafted in a
    /// hand.** The join this whole rung exists for, asserted end to end.
    ///
    /// Two goals in one expansion, in order, because that ordering *is* the
    /// mechanism: `run_steps` expands them against one overlay, so the second
    /// goal is asked of a world in which the first goal's cell already stands.
    /// `crate::method::cellstock::DrawFromCell` -- registered ahead of
    /// `Withdraw` and of `HandCraft` -- then claims it.
    ///
    /// The negative half is the load-bearing one: before this, **every** pack
    /// in this project's history reached a lab as `ActionKind::Craft`, and a
    /// cell standing beside the bot changed nothing at all, because the
    /// registry asked `HandCraft` first and it says yes to every crafting
    /// recipe.
    #[test]
    fn a_pack_a_standing_cell_makes_is_drawn_from_it_rather_than_hand_crafted() {
        // Steel, not the pack: a pack cell sinks into a lab now and nothing
        // draws from a lab (`a_research_fed_by_a_cell_puts_no_pack_in_anyones_hands`
        // is that join). A chest sink is what remains for an item no
        // research eats, and steel is the one this crate has.
        let bots = [BotId(1)];
        let state = powered_with_sources(&bots);
        let charge = steel_horizon();
        let net = expand(
            &[
                Goal::Producing {
                    item: STEEL.into(),
                    per_minute: 1,
                },
                Goal::Have {
                    item: STEEL.into(),
                    count: charge,
                    whose: Holder::Share(BotId(1)),
                    via: None,
                },
            ],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a powered fixture can build a cell and then draw from it");
        let drawn: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Remove {
                    entity,
                    slot: InventorySlot::Chest,
                    item,
                    count,
                    ..
                } if entity == CHEST && item == STEEL => Some(*count),
                _ => None,
            })
            .sum();
        assert_eq!(
            drawn, charge,
            "the whole horizon comes out of the cell's output chest"
        );
        let smelted = net
            .actions()
            .filter(|a| a.label.contains("smelt") && a.label.contains(STEEL))
            .count();
        assert_eq!(
            smelted, 0,
            "nothing is hand-smelted while the cell's own horizon covers the goal"
        );
    }

    /// **The cell's machine time is an EDGE, not a bot standing at the chest.**
    ///
    /// The draw used to carry `ticks_per_item * count` in its own `duration`,
    /// which is an artificial serialisation: the packs appear because the
    /// machines run, and nothing about that needs a character present. This
    /// asserts both halves -- the `Remove` costs one transfer, and a
    /// `Step::Link` from the action that charged the cell carries the whole
    /// wait as `lag`.
    ///
    /// The link is what makes the change worth making rather than a rename:
    /// the charge is emitted by *this* module and the draw by
    /// `crate::method::cellstock`, so the id crosses a method boundary through
    /// `ExpansionCtx::buffer_stock`. Assert the lag lands on a real edge, or a
    /// `buffer_stock` that silently recorded nothing would read as a plan that
    /// simply got faster.
    #[test]
    fn the_wait_for_a_cell_to_make_a_pack_is_a_lag_edge_and_not_a_bots_time() {
        let bots = [BotId(1)];
        let state = powered_with_sources(&bots);
        let charge = steel_horizon();
        let tempo = assembly_spec(&state, STEEL).unwrap().ticks_per_item;
        let net = expand(
            &[
                Goal::Producing {
                    item: STEEL.into(),
                    per_minute: 1,
                },
                Goal::Have {
                    item: STEEL.into(),
                    count: charge,
                    whose: Holder::Share(BotId(1)),
                    via: None,
                },
            ],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a powered fixture can build a cell and then draw from it");
        let draws: Vec<(crate::ids::ActionId, u32, crate::ids::Ticks)> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Remove {
                    entity,
                    slot: InventorySlot::Chest,
                    item,
                    count,
                    ..
                } if entity == CHEST && item == STEEL => Some((a.id, *count, a.duration)),
                _ => None,
            })
            .collect();
        assert!(!draws.is_empty(), "the cell is drawn from at all");
        for (id, count, duration) in draws {
            assert!(
                duration < tempo,
                "the draw costs a transfer, not the cell's machine time: \
                 {duration} ticks against a tempo of {tempo}"
            );
            let want = tempo.saturating_mul(count);
            assert!(
                net.preds(id).iter().any(|(_, lag)| *lag == want),
                "some predecessor of the draw states the {want}-tick wait for \
                 {count} packs; preds were {:?}",
                net.preds(id)
            );
        }
    }

    /// **What the sources cannot cover falls back to the hands, and says so.**
    ///
    /// The bound is the [`SupplyHorizon`]: the sources' ore, and a cell
    /// makes no more than its sources deliver. A goal for more than that is
    /// partly machine-made and partly hand-crafted. Asserting the *residual*
    /// rather than merely "some crafting happens" is what would catch a draw
    /// that over-spent the ledger -- which would plan a bot walking to an
    /// empty chest, the one failure this modelling exists to prevent.
    #[test]
    fn a_goal_larger_than_the_horizon_draws_the_horizon_and_crafts_the_rest() {
        let bots = [BotId(1)];
        let state = powered_with_sources(&bots);
        let charge = steel_horizon();
        let want = charge + 3;
        let net = expand(
            &[
                Goal::Producing {
                    item: STEEL.into(),
                    per_minute: 1,
                },
                Goal::Have {
                    item: STEEL.into(),
                    count: want,
                    whose: Holder::Share(BotId(1)),
                    via: None,
                },
            ],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a powered fixture can build a cell and then draw from it");
        let drawn: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Remove {
                    entity,
                    slot: InventorySlot::Chest,
                    item,
                    count,
                    ..
                } if entity == CHEST && item == STEEL => Some(*count),
                _ => None,
            })
            .sum();
        assert_eq!(drawn, charge, "the horizon is all the cell has");
        assert!(
            net.actions()
                .any(|a| a.label.contains(STEEL) && !a.label.contains("output chest")),
            "the residual is hand work: {:?}",
            net.actions().map(|a| a.label.clone()).collect::<Vec<_>>()
        );
    }

    /// The arithmetic on its own, including the per-visit slot bound `fuel_visits`
    /// enforces. Before 2026-09-09 this function capped at [`COAL_STACK`] (50);
    /// the caller now uses [`fuel_visits`] to split the total into per-load
    /// visits that each fit in one fuel slot.
    #[test]
    fn the_coal_bill_is_no_longer_capped_at_one_slot() {
        let s = bare(&[BotId(1)]);
        assert_eq!(boiler_coal(&s, 0., CELL_CHARGE_TICKS), 0);
        assert_eq!(boiler_coal(&s, 189., CELL_CHARGE_TICKS), 8);
        assert_eq!(boiler_coal(&s, 900., CELL_CHARGE_TICKS), 34);
        assert!(
            boiler_coal(&s, 100_000., CELL_CHARGE_TICKS) > COAL_STACK,
            "a full-load boiler needs more than one slot over 9,000 ticks: got {}",
            boiler_coal(&s, 100_000., CELL_CHARGE_TICKS)
        );
    }

    /// A world with no boiler is planned, and asks for no coal.
    #[test]
    fn a_world_whose_power_this_planner_did_not_build_asks_for_no_coal() {
        let bots = [BotId(1)];
        let mut state = bare_with_sources(&bots);
        state.gain(BotId(1), "wood", 8);
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
        let cell = plan_cell(&s, &Position::new(10.5, 10.5), &spec, None).unwrap();
        for part in &cell.parts {
            let entity = entity_for(&s, part);
            s.create_entity(entity);
        }
        s.set_recipe(
            &cell.at(Role::Intermediate).unwrap().position,
            &spec.intermediate.as_ref().unwrap().recipe.name,
        )
        .unwrap();
        assert!(
            !holds_assembling(&s, PACK, 6),
            "every building stands and the product machine is empty; it makes nothing"
        );
    }

    /// One inserter per ingredient, and each of them fed by something.
    ///
    /// The product machine's two feeders are the link arm and the copper
    /// mouth's unload arm; the arm needs the belt under its pickup, and a
    /// belt is what `loaded_feeders` accepts as a source (any entity that
    /// is not a recipe-less machine).
    #[test]
    fn a_product_machine_short_of_a_feeder_does_not_count() {
        let mut s = powered(&[BotId(1)]);
        let cell = stand_a_cell(&mut s);
        let copper = cell
            .mouths
            .iter()
            .find(|mouth| mouth.into == Role::Product)
            .expect("the product machine has a mouth");
        let link = cell.at(Role::LinkInserter).unwrap().position.clone();
        for (what, at) in [
            ("unload arm", copper.arm.clone()),
            ("belt under the arm", copper.belt.clone()),
            ("link inserter", link),
        ] {
            let mut s = s.fork();
            assert!(holds_assembling(&s, PACK, 6), "the whole cell holds first");
            s.remove_entity(&at);
            assert!(
                !holds_assembling(&s, PACK, 6),
                "a cell with no {what} is fed by fewer things than the recipe has \
                 ingredients, and makes nothing"
            );
        }
    }

    /// **The defect that made this stage's rate claim false, in the overlay.**
    ///
    /// A cell missing only its output path stands complete by every other
    /// measure: both machines placed and set to their recipes, powered with
    /// headroom, one loaded feeder per ingredient, every inserter turned the
    /// right way. It runs four crafts and halts on `full_output` forever --
    /// measured on the bench at 17,130 ticks, `finished=4`, against
    /// `finished=28` and `status=working` for the identical machine with an
    /// output inserter beside it.
    ///
    /// Both halves of the path are required, and separately: an inserter that
    /// swings into thin air empties nothing, and a chest nothing reaches is
    /// furniture.
    #[test]
    fn a_product_machine_nothing_empties_does_not_count() {
        for missing in [Role::OutputInserter, Role::Lab(0)] {
            let mut s = powered(&[BotId(1)]);
            let cell = stand_a_cell(&mut s);
            assert!(holds_assembling(&s, PACK, 6), "the whole cell holds first");
            s.remove_entity(&cell.at(missing).unwrap().position);
            assert!(
                !holds_assembling(&s, PACK, 6),
                "with no {missing:?} the pack machine fills its output slot and stops; \
                 `producing` would be claiming a rate it reaches for four crafts"
            );
        }
    }

    /// The feed and supply inserters are **not** a drain, and the output
    /// inserter is **not** a feeder.
    ///
    /// Both directions of the same mistake, and the reason neither is a
    /// separate predicate: `delivers_into` is asymmetric, so "an inserter the
    /// machine delivers into" and "an inserter that delivers into the machine"
    /// are disjoint sets for a cell whose inserters are all turned correctly.
    /// If they were not, a cell with three inserters pointing *in* would read
    /// as drained, and the whole clause would be decoration.
    #[test]
    fn the_drain_and_the_feeders_are_told_apart_by_direction_alone() {
        let mut s = powered(&[BotId(1)]);
        let cell = stand_a_cell(&mut s);
        let product = cell.at(Role::Product).unwrap().position.clone();
        let nearby = s.entities_within(&Position::new(0., 0.), CELL_SCAN_RADIUS);
        let machine = nearby
            .iter()
            .find(|e| e.position == product)
            .expect("the product machine is in range");

        assert!(is_drained(&s, &nearby, machine));
        assert_eq!(
            loaded_feeders(&s, &nearby, machine, CHAIN_DEPTH),
            2,
            "the output inserter must not be counted as a third feeder"
        );

        // Take the drain away and the feeders are unchanged: the two clauses
        // read the same cell and answer about different halves of it.
        let mut without = powered(&[BotId(1)]);
        let same = stand_a_cell(&mut without);
        without.remove_entity(&same.at(Role::OutputInserter).unwrap().position);
        let nearby = without.entities_within(&Position::new(0., 0.), CELL_SCAN_RADIUS);
        let machine = nearby
            .iter()
            .find(|e| e.position == product)
            .expect("the product machine is still in range");
        assert!(!is_drained(&without, &nearby, machine));
        assert_eq!(loaded_feeders(&without, &nearby, machine, CHAIN_DEPTH), 2);
    }

    /// **The same cell, arriving from the world instead of from this plan.**
    ///
    /// Every other test here stands its cell with `PlanState::create_entity`
    /// and `PlanState::set_recipe`, which write the overlay. A *replan* has no
    /// overlay: `PlanState::from_world` starts empty and everything standing
    /// comes back out of `FactorioSurface`'s entity graph. So the overlay tests
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
                &cell,
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
                spec.intermediate.as_ref().unwrap().recipe.name.clone(),
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
        cell: &Cell,
        parts: &[CellPart],
        recipes: &[(Position, String)],
        bots: &[BotId],
    ) -> PlanState {
        let mut standing: Vec<FactorioEntity> =
            parts.iter().map(|part| entity_for(planned, part)).collect();
        // The runs' ends, as the game would report them.
        standing.extend(mouth_entities(cell));
        for (name, position) in [
            (POLE, Position::new(10.5, 10.5)),
            ("steam-engine", Position::new(12.5, 10.5)),
            (BOILER, Position::new(12.5, 14.5)),
        ] {
            let world = world();
            let entity_type = world
                .globals
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

    /// **The drain clause at the seam**, which is the only place it counts.
    ///
    /// The overlay test above proves the predicate; this proves it against the
    /// door the mod's entity events actually use. That distinction is not
    /// theoretical here: `30b28846` was three complete cells producing science
    /// while `cells_standing` read zero, because a recipe set over RCON had no
    /// route into the entity graph, and every test that missed it stood its
    /// cell through the overlay. A new clause on the same predicate gets the
    /// same treatment or it is untested where it matters.
    ///
    /// Two claims. A whole cell the world reports still holds -- so the drain
    /// clause has not simply refused everything, which is how a clause of this
    /// shape fails silently in the *other* direction, over-building forever. A
    /// cell the world reports with no output inserter does not -- and that is
    /// the arrangement six runs stood on while `producing` claimed six packs a
    /// minute.
    #[test]
    fn a_cell_the_world_reports_with_no_drain_does_not_hold() {
        let bots = [BotId(1)];
        let spec = spec();
        let mut planned = powered(&bots);
        let cell = stand_a_cell(&mut planned);
        let recipes = cell_recipes(&cell, &spec);

        assert!(
            holds_assembling(
                &observed_world(&planned, &cell, &cell.parts, &recipes, &bots),
                PACK,
                6
            ),
            "the control: a whole cell the world reports holds, or the drain clause is \
             refusing every cell rather than the jammed ones"
        );

        for missing in [Role::OutputInserter, Role::Lab(0)] {
            let jammed: Vec<CellPart> = cell
                .parts
                .iter()
                .filter(|part| part.role != missing)
                .cloned()
                .collect();
            assert_eq!(jammed.len(), cell.parts.len() - 1);
            assert!(
                !holds_assembling(
                    &observed_world(&planned, &cell, &jammed, &recipes, &bots),
                    PACK,
                    6
                ),
                "a cell the world reports with no {missing:?} halts on full_output after \
                 four crafts; a replan must build one that does not"
            );
        }
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
        assert_eq!(
            spec.intermediate.as_ref().unwrap().ingredients.len(),
            2,
            "two ingredients: one belted, one in a chest"
        );

        let mut planned = powered(&bots);
        let cell = stand_a_cell_for(&mut planned, &spec);
        assert_eq!(
            cell.feeds(),
            1,
            "the belt machine's iron is belted; only its gears are in a chest"
        );
        let recipes = cell_recipes(&cell, &spec);

        // One cell is five packs a minute, which is what `cells_for` says.
        assert!(
            holds_assembling(
                &observed_world(&planned, &cell, &cell.parts, &recipes, &bots),
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
                &observed_world(&planned, &cell, &starved, &recipes, &bots),
                GREEN,
                5
            ),
            "a belt machine with no gears arriving makes no belts, and a cell whose \
             intermediate makes nothing makes no science"
        );
    }

    /// The plan a green goal produces, at the shape rather than the tile:
    /// the iron is belted from the standing source and the gears and the
    /// inserters -- which nothing here makes -- still arrive in chests.
    #[test]
    fn a_green_plan_belts_the_iron_and_charges_two_chests() {
        let bots = [BotId(1)];
        let state = powered_with_sources(&bots);
        let net = expand(
            &[Goal::Producing {
                item: GREEN.into(),
                per_minute: 5,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("green science plans beside an iron source");

        let placed: Vec<&str> = net
            .actions()
            .filter_map(|action| match &action.kind {
                ActionKind::Place { entity } => Some(entity.name.as_str()),
                _ => None,
            })
            .collect();
        let count = |name: &str| placed.iter().filter(|n| **n == name).count();
        assert_eq!(count(MACHINE), 2, "an intermediate and a product machine");
        assert_eq!(
            count(CHEST),
            3,
            "the gear chest, the inserter chest and the output chest -- no iron chest"
        );
        assert_eq!(
            count(INSERTER),
            4 + 2,
            "gear feed, link, supply and output, plus the iron run's load and unload arms"
        );
        assert!(
            count("transport-belt") > 0,
            "the iron comes down a belt from the standing source"
        );

        // Two charges -- one per chest a bot fills. The output chest is
        // charged with nothing, because the cell fills it.
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

    // ---- finishing what stands --------------------------------------------

    fn placed(net: &ActionNetwork, name: &str) -> Vec<Position> {
        net.actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Place { entity } if entity.name == name => {
                    Some(entity.position.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// **`run-1788608648-56109`, plan 3.** Plan 2 had put two assembling
    /// machines down and nothing else of their cell; plan 3 placed four more
    /// beside them. A cell whose machines stand is finished around them:
    /// no machine is placed or crafted, every other part is, and both
    /// recipes are set.
    #[test]
    fn a_cell_whose_machines_stand_is_finished_around_them() {
        let bots = [BotId(1)];
        let mut s = powered_with_sources(&bots);
        let cell = plan_cell(&s, &Position::new(10.5, 10.5), &spec(), None).expect("room");
        for role in [Role::Intermediate, Role::Product] {
            let part = cell.at(role).expect("a cell has two machines");
            let entity = entity_for(&s, part);
            s.create_entity(entity);
        }
        let net = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("two machines are a cell to finish");
        assert_eq!(
            placed(&net, MACHINE),
            Vec::<Position>::new(),
            "the machines stand and are not placed again"
        );
        assert!(
            !net.actions()
                .any(|a| a.label.contains("craft") && a.label.contains(MACHINE)),
            "nor crafted: {:?}",
            net.actions().map(|a| a.label.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            placed(&net, INSERTER).len(),
            inserter_count(&spec(), default_sink(&s, &spec())) as usize,
            "every inserter of the cell is placed, the runs' arms included"
        );
        assert_eq!(
            placed(&net, CHEST),
            Vec::<Position>::new(),
            "no chest anywhere: the output goes into a lab"
        );
        assert_eq!(placed(&net, LAB).len(), 1, "the lab the output drops into");
        let recipes: Vec<Position> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::SetRecipe { pos, .. } => Some(pos.clone()),
                _ => None,
            })
            .collect();
        let mut machines: Vec<Position> = [Role::Intermediate, Role::Product]
            .iter()
            .map(|role| cell.at(*role).unwrap().position.clone())
            .collect();
        let mut recipes_sorted = recipes;
        let sort = |v: &mut Vec<Position>| {
            v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        };
        sort(&mut machines);
        sort(&mut recipes_sorted);
        assert_eq!(
            recipes_sorted, machines,
            "both standing machines get their recipe"
        );
    }

    /// **`run-1788941729-70024`, tick 53,546.** The batch was cut upstream
    /// of `research automation`, so both machines were abandoned while every
    /// chest and arm of the cell stood. A cell whose chests and arms stand
    /// is finished around them: the machines go where the cell meant them,
    /// and nothing standing is placed again. Before this, only a standing
    /// MACHINE could seed a cell, and the replan sited a fresh one beside
    /// the parts.
    #[test]
    fn a_cell_whose_chests_and_arms_stand_is_finished_around_them() {
        let bots = [BotId(1)];
        let mut s = powered_with_sources(&bots);
        let cell = plan_cell(&s, &Position::new(10.5, 10.5), &spec(), None).expect("room");
        for part in cell
            .parts
            .iter()
            .filter(|part| !matches!(part.role, Role::Intermediate | Role::Product | Role::Pole))
        {
            let entity = entity_for(&s, part);
            s.create_entity(entity);
        }
        let own_arms: Vec<Position> = cell
            .parts
            .iter()
            .filter(|part| part.name == INSERTER)
            .map(|part| part.position.clone())
            .collect();
        let net = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("chests and arms are a cell to finish");
        let sort = |v: &mut Vec<Position>| {
            v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        };
        let mut machines: Vec<Position> = [Role::Intermediate, Role::Product]
            .iter()
            .map(|role| cell.at(*role).unwrap().position.clone())
            .collect();
        sort(&mut machines);
        let mut placed_machines = placed(&net, MACHINE);
        sort(&mut placed_machines);
        assert_eq!(
            placed_machines, machines,
            "the machines go where the cell meant them: finished, not re-sited"
        );
        assert_eq!(
            placed(&net, CHEST),
            Vec::<Position>::new(),
            "every chest stands and is not placed again"
        );
        for arm in &own_arms {
            assert!(
                !placed(&net, INSERTER).contains(arm),
                "the cell's own arm at {arm} stands and is not placed again"
            );
        }
    }

    /// A lone lab is not a cell. One lab of a cell standing and nothing else
    /// beside it: the plan sites a whole cell and places every lab of it,
    /// rather than growing a cell round whatever lab it finds. Two parts on
    /// their tiles are the threshold (`fit_partial`'s `least`). It was a
    /// lone chest until the output went into a lab.
    #[test]
    fn a_lone_chest_does_not_seed_a_cell() {
        let bots = [BotId(1)];
        let mut s = powered_with_sources(&bots);
        let cell = plan_cell(&s, &Position::new(10.5, 10.5), &spec(), None).expect("room");
        let chest = cell.at(Role::Lab(0)).expect("a cell has a lab");
        let entity = entity_for(&s, chest);
        s.create_entity(entity);
        let net = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a lone lab does not stop a cell being sited");
        assert_eq!(
            placed(&net, LAB).len(),
            1,
            "a whole cell's lab is placed; the lone lab seeded nothing"
        );
        assert_eq!(placed(&net, MACHINE).len(), 2);
        assert_eq!(
            chest_count(&spec()),
            1,
            "the chest a chest sink would have had"
        );
    }

    // ---- the one-machine (furnace) cell -----------------------------------

    const STEEL: &str = "steel-plate";

    /// The shape rule for a smelting recipe, and the three things about it
    /// that are not the crafting rule.
    #[test]
    fn a_smelting_recipe_no_drill_can_feed_is_a_one_machine_cell() {
        let s = bare(&[BotId(1)]);
        let steel = assembly_spec(&s, STEEL).expect("steel is a furnace cell");
        assert_eq!(
            steel.machine, FURNACE,
            "a furnace, not an assembling machine"
        );
        assert!(
            steel.intermediate.is_none(),
            "one machine and never two: steel has a single ingredient"
        );
        assert_eq!(steel.feeds(), 0, "no feed chest, so no feed side at all");
        assert_eq!(
            steel.feed_charges(),
            Vec::new(),
            "and nothing to charge it with"
        );
        // **The amount is free, and that is the whole difference from stage
        // 1.** `produce::cell_spec` demands exactly one because a drill
        // delivers one thing; an inserter carries five as happily as one.
        assert_eq!(steel.supplied, ("iron-plate".to_string(), 5));
        assert!(
            !steel.sets_recipe(),
            "a furnace picks its recipe from what is put into it"
        );
        // 16 s at a stone furnace's crafting speed of 1.
        assert_eq!(steel.ticks_per_item, 960);
        // Its iron is smelted, so it is belted: no supply chest, no charge.
        assert!(steel.is_belted("iron-plate"));
        assert!(!steel.has_supply_chest());
        assert_eq!(
            steel.supply_charge(),
            0,
            "nothing to charge a belted plate into"
        );
        let mut charged = steel.clone();
        charged.belted.clear();
        assert_eq!(
            charged.charge_products(),
            9,
            "9,000 ticks of charge at 960 each, were the iron in a chest"
        );
        assert_eq!(
            charged.supply_charge(),
            45,
            "nine plates of steel is 45 of iron"
        );
    }

    /// Stage 1 keeps what stage 1 can do, and the two methods stay disjoint.
    ///
    /// `iron-plate` and `copper-plate` are smelting recipes too. A drill
    /// standing on ore renews its own input where a chest is charged once, so
    /// where both shapes exist the drill wins -- and the rule is `cell_spec`
    /// answering for itself rather than a second copy of it here.
    #[test]
    fn a_plate_a_drill_can_feed_stays_stage_ones() {
        let s = bare(&[BotId(1)]);
        for plate in ["iron-plate", "copper-plate"] {
            assert!(
                crate::method::produce::cell_spec(&s, plate).is_some(),
                "{plate} is a stage-1 cell"
            );
            assert!(
                assembly_spec(&s, plate).is_none(),
                "{plate} must not also be a stage-2 one"
            );
        }
    }

    /// The 2x2 furnace covers the tile the supply inserter drops into and the
    /// tile the output inserter picks up from, at every facing.
    ///
    /// This is what `AssemblySpec::product_offset` exists for: the layout was
    /// written around a 3x3 machine, and a 2x2 one placed at the same offset
    /// would sit half a tile off its own build grid *and* miss both mouths.
    /// Asked through `delivers_into`, which is the same predicate
    /// `Condition::Feeds` is checked with.
    #[test]
    fn the_furnace_meets_both_of_its_inserters_at_every_facing() {
        // With its iron in a chest: steel's plates are belted since
        // 2026-09-09, so the real spec has no supply arm of the layout's.
        // The 2x2's geometry against the chest layout is what is pinned.
        let mut spec = assembly_spec(&bare(&[BotId(1)]), STEEL).unwrap();
        spec.belted.clear();
        for facing in Direction::orthogonal() {
            let origin = Position::new(10.5, 10.5);
            let s = bare(&[BotId(1)]);
            let parts =
                layout(&origin, facing, true, &spec, Sink::Chest).expect("a cardinal facing");
            assert!(
                parts.iter().all(|p| p.role != Role::Intermediate
                    && p.role != Role::LinkInserter
                    && !matches!(p.role, Role::FeedChest(_) | Role::FeedInserter(_))),
                "a one-machine cell has no intermediate half"
            );
            // The furnace is 2x2, so its centre belongs on a tile *boundary*
            // -- the grid `tile_alignment` reads off its own collision box,
            // and the reason the product offset is half a tile from the one a
            // 3x3 machine uses.
            let product = parts
                .iter()
                .find(|part| part.role == Role::Product)
                .expect("a one-machine cell has a product");
            assert_eq!(product.name, FURNACE);
            let (ax, ay) = tile_alignment(&s, FURNACE);
            assert_eq!(product.position.x().rem_euclid(1.), ax, "at {facing:?}");
            assert_eq!(product.position.y().rem_euclid(1.), ay, "at {facing:?}");
            let mut trial = s.fork();
            for part in &parts {
                trial.create_entity(entity_for(&s, part));
            }
            let s = trial;
            let lane = lane(&origin, facing, &spec, Sink::Chest).unwrap();
            let cell = Cell {
                origin,
                facing,
                parts,
                standing: Vec::new(),
                lane,
                mouths: Vec::new(),
                sink: Sink::Chest,
                evacuate: Vec::new(),
            };
            for (from, to) in links(&cell).expect("a one-machine cell has links") {
                assert!(
                    s.delivers_into(&from, &to),
                    "{from} -> {to} at facing {facing:?}"
                );
            }
        }
    }

    /// A furnace cell is emitted with **no `SetRecipe` at all** and with coal
    /// in the furnace.
    ///
    /// Both halves are failures that place 100 % and produce nothing: a
    /// `SetRecipe` on a furnace is an action the game refuses, and an
    /// unfuelled burner is a machine that never starts. The coal is
    /// hand-loaded because no coal flows through a steel furnace -- its input
    /// is iron plates and its output steel.
    #[test]
    fn a_furnace_cell_sets_no_recipe_and_gets_its_own_coal() {
        let bots = vec![BotId(1)];
        let s = powered_with_sources(&bots);
        let spec = assembly_spec(&s, STEEL).unwrap();
        let net = expand(
            &[Goal::Producing {
                item: STEEL.to_string(),
                per_minute: 1,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a furnace cell plans on a powered world");
        assert!(
            !net.actions()
                .any(|a| matches!(a.kind, ActionKind::SetRecipe { .. })),
            "nothing in a furnace cell takes a recipe"
        );
        assert_eq!(
            placed(&net, MACHINE),
            Vec::<Position>::new(),
            "no assembling machine anywhere: the cell's machine is a furnace"
        );
        assert_eq!(
            placed(&net, INSERTER).len(),
            1 + 2,
            "the output arm, and the iron run's load and unload arms; no link, no supply arm"
        );
        assert_eq!(
            placed(&net, CHEST).len(),
            1,
            "the output chest; no supply, no feed"
        );
        // The furnace's coal is sized off the horizon -- eighty steel at 960
        // ticks off four hundred iron plates, five a plate -- and capped at
        // the one slot a hand fills. Found by its count rather than by being
        // the only furnace: the plan may stand hand-fed furnaces of its own.
        let horizon = SupplyHorizon {
            products: source_plates() / spec.supplied.1,
        }
        .ticks(&spec);
        let charge = furnace_charge_coal(&s, &spec, horizon);
        assert!(
            charge > fuel_for_duration(CELL_CHARGE_TICKS, COAL_BURN_TICKS),
            "the horizon is longer than the old charge: {charge}"
        );
        let fuelled: Vec<Position> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Insert {
                    pos,
                    slot: InventorySlot::Fuel,
                    item,
                    count,
                    entity,
                } if item == "coal" && *count == charge && entity == FURNACE => Some(pos.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            fuelled.len(),
            1,
            "exactly one furnace is fuelled for the horizon"
        );
        assert!(
            placed(&net, FURNACE).contains(&fuelled[0]),
            "and it is one this plan placed"
        );
        assert!(
            !net.actions().any(|a| matches!(
                &a.kind,
                ActionKind::Insert { item, .. } if item == "iron-plate"
            )),
            "no iron plate is poured anywhere by hand: it comes down the belt"
        );
        assert!(
            !placed(&net, "transport-belt").is_empty(),
            "the run from the iron source"
        );
    }

    /// Sources for the tests that build the plant on the fixture's lake at
    /// `(40, 40)`: either side of it, so the chests' centroid is at the
    /// water and a cell sited off the plant's pole is inside both runs'
    /// windows.
    const IRON_DRILL_BY_THE_LAKE: (f64, f64) = (30., 26.);
    const COPPER_DRILL_BY_THE_LAKE: (f64, f64) = (50., 26.);

    /// A world that already holds a whole plant and a lab gets a cell beside
    /// them and none of them again.
    #[test]
    fn a_standing_plant_and_lab_are_built_neither_again() {
        use crate::method::power::{BOILER, ENGINE, PUMP};
        let bots = [BotId(1)];
        let mut s = PlanState::from_world(
            Arc::new(world_with_sources(
                IRON_DRILL_BY_THE_LAKE,
                COPPER_DRILL_BY_THE_LAKE,
            )),
            &bots,
        );
        s.gain(BotId(1), "wood", 8);
        let plant = crate::method::power::plan_plant(&s, &Position::new(40., 40.))
            .expect("the fixture has a lake");
        for part in &plant.parts {
            let entity = crate::method::power::entity_for(&s, part);
            s.create_entity(entity);
        }
        let lab = crate::method::util::free_area_near_where(&s, &plant.pole, "lab", |candidate| {
            s.collision_area("lab", candidate)
                .is_some_and(|area| s.electric_supply_kw(&area) >= 60.)
        })
        .expect("powered ground beside the plant");
        s.create_entity(FactorioEntity {
            name: "lab".into(),
            entity_type: "lab".into(),
            position: lab,
            ..Default::default()
        });
        let net = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a powered world gets a cell");
        for name in [PUMP, BOILER, ENGINE] {
            assert_eq!(
                placed(&net, name),
                Vec::<Position>::new(),
                "{name} stands already"
            );
        }
        // The standing lab is a hand-fed lab somebody sited on its own; the
        // cell's output goes into a lab of the CELL's, on the product
        // machine's south face, and nothing carries packs to a lab elsewhere.
        assert_eq!(
            placed(&net, LAB).len(),
            1,
            "the cell's own lab, and no second one beside the standing hand-fed lab"
        );
        assert_eq!(
            placed(&net, MACHINE).len(),
            2,
            "one cell, beside the standing plant"
        );
        let product = placed(&net, MACHINE)[0].clone();
        assert!(
            calculate_distance(&product, &plant.pole) < 30.,
            "sited off the standing plant's pole, not somewhere of its own"
        );
    }

    /// The cell's power is the half-built plant, finished: with a pump, its
    /// pipes and a boiler standing and no engine, the plan places exactly
    /// the engine and its pole.
    #[test]
    fn a_half_built_plant_is_finished_for_the_cell() {
        use crate::method::power::{BOILER, ENGINE, PIPE, POLE as PLANT_POLE, PUMP};
        let bots = [BotId(1)];
        let mut s = PlanState::from_world(
            Arc::new(world_with_sources(
                IRON_DRILL_BY_THE_LAKE,
                COPPER_DRILL_BY_THE_LAKE,
            )),
            &bots,
        );
        s.gain(BotId(1), "wood", 8);
        let plant = crate::method::power::plan_plant(&s, &Position::new(40., 40.))
            .expect("the fixture has a lake");
        for part in plant
            .parts
            .iter()
            .filter(|part| [PUMP, PIPE, BOILER].contains(&part.name))
        {
            let entity = crate::method::power::entity_for(&s, part);
            s.create_entity(entity);
        }
        let net = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a half-built plant is finished for the cell");
        for name in [PUMP, BOILER, PIPE] {
            assert_eq!(
                placed(&net, name),
                Vec::<Position>::new(),
                "{name} stands already"
            );
        }
        assert_eq!(
            placed(&net, ENGINE),
            vec![plant.engine.clone()],
            "exactly the engine"
        );
        assert!(
            placed(&net, PLANT_POLE).contains(&plant.pole),
            "and the pole that carries it"
        );
    }

    // ---- determinism ------------------------------------------------------

    /// The same state plans the same cell, down to the tile and the facing.
    #[test]
    fn the_same_state_sites_the_same_cell_twice() {
        let a = plan_cell(&powered(&[BotId(1)]), &Position::new(10.5, 10.5), &spec(), None).unwrap();
        let b = plan_cell(&powered(&[BotId(1)]), &Position::new(10.5, 10.5), &spec(), None).unwrap();
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
            None,
        )
        .expect("the fixture has room for two");
        assert_eq!(cells.len(), 2);
        assert_ne!(cells[0].origin, cells[1].origin);
        let s = bare(&[BotId(1)]);
        for a in &cells[0].parts {
            for b in &cells[1].parts {
                let a_box = s
                    .collision_area_facing(a.name, &a.position, a.direction)
                    .unwrap();
                let b_box = s
                    .collision_area_facing(b.name, &b.position, b.direction)
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

    /// **The gate on everything above, and what keeps every baseline still.**
    ///
    /// The supply link is reached only when a stage-1 cell for the supplied
    /// item is already standing, and on a world where none is, this answers
    /// `None` -- so the cell is sited from the bot exactly as before and no
    /// belt run is laid. That is why `researched:automation`,
    /// `producing:automation-science-pack:6` and the other five baselines are
    /// byte-identical across this change: not because the link is cheap, but
    /// because nothing reaches it.
    ///
    /// Asserted about the *shared fixture*, which is the world those
    /// baselines' unit-test siblings run on.
    #[test]
    fn a_world_with_no_standing_cell_offers_no_supply_source() {
        let state = bare(&[BotId(1)]);
        let spec = assembly_spec(&state, PACK).expect("red science is a cell shape");
        assert_eq!(
            spec.supplied.0.as_str(),
            "copper-plate",
            "red science's smelted ingredient"
        );
        assert!(
            standing_supply_sources(&state, &spec.supplied.0).is_empty(),
            "no drill, no furnace, no offtake -- nothing to link to"
        );
    }

    /// **Direction is the discriminator, and it has to be.** A run's unload
    /// arm stands on the mouth's arm tile and *drops into* the machine; an
    /// arm on the same tile turned round would *pick up* from it. Read the
    /// wrong way round, a freshly-planned cell would look already-fed and the
    /// run would never be laid -- the silent half of the failure this whole
    /// change exists to end.
    #[test]
    fn a_mouth_is_linked_only_by_an_arm_that_drops_into_the_machine() {
        let bots = [BotId(1)];
        let mut state = powered(&bots);
        let spec = assembly_spec(&state, PACK).expect("red science is a cell shape");
        let cell = plan_cell(&state, &Position::new(10.5, 10.5), &spec, None).expect("room");
        for part in &cell.parts {
            let entity = entity_for(&state, part);
            state.create_entity(entity);
        }
        assert_eq!(
            cell.mouths.len(),
            2,
            "iron into the gear machine, copper into the pack one"
        );
        for mouth in &cell.mouths {
            assert!(
                !mouth_is_linked(&state, &cell, mouth),
                "nothing stands at {}: the mouth is open",
                mouth.arm
            );
        }
        let mut linked = state.fork();
        for entity in mouth_entities(&cell) {
            linked.create_entity(entity);
        }
        for mouth in &cell.mouths {
            assert!(
                mouth_is_linked(&linked, &cell, mouth),
                "an arm at {} dropping into the machine is the run, standing",
                mouth.arm
            );
        }
        let mut turned = state.fork();
        for mut entity in mouth_entities(&cell) {
            if entity.name == INSERTER {
                entity = FactorioEntity::new_named_inserter(
                    INSERTER.into(),
                    &entity.position,
                    compose(Direction::East, cell.facing).unwrap(),
                );
            }
            turned.create_entity(entity);
        }
        for mouth in &cell.mouths {
            assert!(
                !mouth_is_linked(&turned, &cell, mouth),
                "an arm at {} picking up FROM the machine is not a run into it",
                mouth.arm
            );
        }
    }

    // ---- no chests in the line --------------------------------------------

    /// **A red cell has no chest but the output one.** Both of its plates
    /// are smelted, so both are belted: the layout stands two machines, the
    /// link, the output pair and (when it brings one) a pole, and keeps a
    /// mouth on the west face of each machine for the run that feeds it.
    #[test]
    fn a_red_cell_has_no_chest_but_the_output_one() {
        let spec = spec();
        assert_eq!(
            spec.belted,
            ["copper-plate", "iron-plate"]
                .into_iter()
                .map(String::from)
                .collect::<BTreeSet<_>>()
        );
        let state = bare(&[BotId(1)]);
        assert_eq!(
            default_sink(&state, &spec),
            Sink::Labs(1),
            "a science pack's cell sinks into a lab, and one lab is the default chain"
        );
        let roles: BTreeSet<Role> = layout_table(&spec, Sink::Labs(1))
            .into_iter()
            .map(|(r, _, _)| r)
            .collect();
        assert_eq!(
            roles,
            [
                Role::Intermediate,
                Role::Product,
                Role::LinkInserter,
                Role::OutputInserter,
                Role::Lab(0),
                Role::LabPole(0),
            ]
            .into_iter()
            .collect(),
            "no chest at all: the output arm drops into a lab"
        );
        let origin = Position::new(10.5, 10.5);
        let mouths = mouths(&origin, Direction::North, &spec).unwrap();
        let at = |x: f64, y: f64| origin.add(&Position::new(x, y));
        assert_eq!(
            mouths
                .iter()
                .map(|m| (m.item.as_str(), m.into, m.arm.clone(), m.belt.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("iron-plate", Role::Intermediate, at(-2., 0.), at(-3., 0.)),
                ("copper-plate", Role::Product, at(-2., 4.), at(-3., 4.)),
            ],
            "the rows a chest used to stand on, so the pole lights the arms as it lit the chests"
        );
        assert_eq!(
            lane(&origin, Direction::North, &spec, Sink::Labs(1)).unwrap(),
            Vec::<Position>::new(),
            "nothing is filled or emptied by hand, so no bot needs to stand anywhere"
        );
        assert_eq!(
            lane(&origin, Direction::North, &spec, Sink::Chest).unwrap(),
            vec![at(-4., 3.)],
            "with a chest sink a bot stands behind the output chest and nowhere else"
        );
    }

    /// **A cell asked for with no standing source is refused by name**, and
    /// the refusal names the ingredient and the rate, not the ground.
    #[test]
    fn a_cell_with_no_standing_source_is_refused_by_name() {
        let bots = [BotId(1)];
        let state = powered(&bots);
        let refusal = expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect_err("no source stands, so no cell can be belted");
        match refusal {
            PlannerError::AssemblyNoStandingSource {
                item,
                ingredient,
                per_minute,
                ..
            } => {
                assert_eq!(item, PACK);
                assert_eq!(
                    ingredient, "iron-plate",
                    "the first belted input, in mouth order"
                );
                assert_eq!(per_minute, 12, "two plates a pack at six a minute");
            }
            other => panic!("refused for the wrong reason: {other}"),
        }
        // And the same goal plans the moment the sources stand: the refusal
        // is about the sources and nothing else.
        expand(
            &[Goal::Producing {
                item: PACK.into(),
                per_minute: 6,
            }],
            &powered_with_sources(&bots),
            &registry_for(&bots),
            BotId(1),
        )
        .expect("with both sources standing the cell plans");
    }

    /// **A source's rate is spent as it is assigned, and a cell it cannot
    /// carry is refused rather than belted from a chest that cannot keep
    /// up.** One furnace makes eighteen plates a minute; two red cells eat
    /// twenty-four of iron.
    #[test]
    fn a_source_with_no_rate_left_is_not_belted_twice() {
        let state = bare_with_sources(&[BotId(1)]);
        let spec = spec();
        let one =
            assign_sources(&state, &spec, 1, 6).expect("one cell has a source for each plate");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].inputs.len(), 2);
        let refusal = assign_sources(&state, &spec, 2, 6).expect_err("two cells want 24 iron/min");
        match refusal {
            PlannerError::AssemblyNoStandingSource {
                ingredient,
                per_minute,
                standing,
                ..
            } => {
                assert_eq!(ingredient, "iron-plate");
                assert_eq!(per_minute, 12);
                assert!(
                    standing.contains("not yet spoken for"),
                    "the refusal says what the one source had left: {standing}"
                );
            }
            other => panic!("refused for the wrong reason: {other}"),
        }
    }

    /// **The ledger is the sources' ore, not a constant.** Halve the ore
    /// under the source drills and the `BufferGain` the cell writes halves
    /// with it; under `CELL_CHARGE_TICKS` it read fifteen whatever stood.
    #[test]
    fn the_ledger_is_the_sources_ore_and_not_a_constant() {
        let bots = [BotId(1)];
        let ledger = |state: &PlanState| -> u32 {
            let net = expand(
                &[Goal::Producing {
                    item: STEEL.into(),
                    per_minute: 1,
                }],
                state,
                &registry_for(&bots),
                BotId(1),
            )
            .expect("a powered fixture with sources builds a cell");
            net.actions()
                .flat_map(|a| a.eff.iter())
                .filter_map(|effect| match effect {
                    Effect::BufferGain { item, count, .. } if item == STEEL => Some(*count),
                    _ => None,
                })
                .sum()
        };
        let full = ledger(&powered_with_sources(&bots));
        assert_eq!(
            full,
            steel_horizon(),
            "four hundred iron plates at five a steel"
        );

        let mut halved = PlanState::from_world(
            Arc::new(world_with_sources_holding(
                IRON_DRILL,
                COPPER_DRILL,
                SOURCE_ORE_PER_TILE / 2,
            )),
            &bots,
        );
        stand_plant(&mut halved, &bots);
        halved.gain(BotId(1), "wood", 8);
        assert_eq!(
            ledger(&halved),
            steel_horizon() / 2,
            "half the ore is half the steel"
        );
    }

    /// **A research is fed by the cell, and no pack is ever in anyone's
    /// hands.** `logistics` researches with red packs alone and is not the
    /// technology that unlocks the assembling machine, so with both plate
    /// sources standing the cell is built inline with a lab as its sink and
    /// the research waits on the cell: no pack is crafted, no pack is
    /// inserted, exactly one lab is placed, and the research action names
    /// that lab and is linked from the cell with the packs' lag.
    ///
    /// `automation` -- the prerequisite, and the unlocker -- is marked
    /// researched first, so the plan is one research and not the bootstrap's
    /// hand-fed lab beside it.
    #[test]
    fn a_research_fed_by_a_cell_puts_no_pack_in_anyones_hands() {
        let bots = [BotId(1)];
        let mut state = powered_with_sources(&bots);
        state.set_researched("automation");
        let tech = state
            .technology("logistics")
            .expect("the fixture has logistics");
        let net = expand(
            &[Goal::Researched("logistics".into())],
            &state,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a research beside standing sources is fed by a cell");
        let labels: Vec<String> = net.actions().map(|a| a.label.clone()).collect();
        assert!(
            !net.actions()
                .any(|a| matches!(&a.kind, ActionKind::Craft { item, .. } if item == PACK)),
            "no pack is hand-crafted: {labels:?}"
        );
        assert!(
            !net.actions()
                .any(|a| matches!(&a.kind, ActionKind::Insert { item, .. } if item == PACK)),
            "no pack is carried to a lab: {labels:?}"
        );
        let labs = placed(&net, LAB);
        assert_eq!(labs.len(), 1, "one lab, the cell's: {labels:?}");
        let research = net
            .actions()
            .find(|a| matches!(&a.kind, ActionKind::Research { tech } if tech == "logistics"))
            .expect("the research is planned");
        assert!(
            research.pre.iter().any(|c| matches!(c, Condition::EntityAt { pos, name } if name == LAB && labs.contains(pos))),
            "the research names the cell's lab: {:?}",
            research.pre
        );
        let want = spec()
            .ticks_per_item
            .saturating_mul(u32::try_from(tech.research_unit_count).unwrap());
        assert!(
            net.preds(research.id).iter().any(|(_, lag)| *lag == want),
            "the research waits {want} ticks on the cell for its packs; preds were {:?}",
            net.preds(research.id)
        );
        assert_eq!(
            labs_fed_by(6, PACK, &tech),
            1,
            "six packs a minute against four a lab feeds one lab, not two"
        );
    }

    /// **The side-load risk, pinned.** Two runs end on one machine column,
    /// four rows apart, and `route_belt` gives each run's last tile the
    /// direction it arrived with. If either last tile faced a tile the
    /// other run stood on, plates would ride off the end onto the wrong
    /// belt; if either run's tile were faced by a foreign belt, that belt
    /// would side-load onto it. Both are read off the built world here,
    /// for every belt the plan lays, and both arms are checked to stand on
    /// their mouths and be lit by the cell's own pole.
    #[test]
    fn two_runs_into_one_cell_do_not_pour_into_each_other() {
        let bots = [BotId(1)];
        let state = powered_with_sources(&bots);
        let net = plan(6).expect("a powered fixture with sources builds a cell");
        let mut built = state.fork();
        let mut belts: Vec<Position> = Vec::new();
        let mut arms: Vec<FactorioEntity> = Vec::new();
        for action in net.actions() {
            if let ActionKind::Place { entity } = &action.kind {
                built.create_entity((**entity).clone());
                if is_belt(&entity.name) {
                    belts.push(entity.position.clone());
                }
                if entity.name == INSERTER {
                    arms.push((**entity).clone());
                }
            }
        }
        assert!(
            belts.len() >= 2,
            "two runs were laid: {} belts",
            belts.len()
        );
        // Every belt pours into a belt of its own run, an arm's tile, or
        // nothing -- never a belt that then leads somewhere else. A run's
        // tiles are found by following the chain from each mouth backwards
        // is more than this needs: it is enough that NO belt's forward tile
        // is a belt facing away from it that is not simply the next tile of
        // a straight chain -- i.e. every belt faced by another belt is faced
        // from exactly one side, its rear.
        let machine_positions: Vec<Position> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Place { entity } if entity.name == MACHINE => {
                    Some(entity.position.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(machine_positions.len(), 2);
        let mut fed_from: BTreeMap<Pos, Vec<Pos>> = BTreeMap::new();
        for at in &belts {
            let (_, forward) = belt_forward(&built, at).expect("a placed belt stands");
            if let Some(next) = built
                .entity_at(&forward)
                .filter(|e| is_belt(&e.name) && Pos::from(&e.position) == Pos::from(&forward))
            {
                fed_from
                    .entry(Pos::from(&next.position))
                    .or_default()
                    .push(Pos::from(at));
            }
        }
        // One feeder per belt. A single perpendicular feeder is a CURVE --
        // the game turns the belt and both lanes carry -- and only a second
        // feeder makes a side-load. Two runs pouring into one tile, or a
        // run's end pouring onto the other run, both read as two feeders.
        for (tile, feeders) in &fed_from {
            assert_eq!(
                feeders.len(),
                1,
                "the belt at {tile:?} is fed from {feeders:?}: a side-load"
            );
        }
        // Each unload arm -- an arm picking up from a belt and dropping into
        // a machine of the cell; the link arm picks from a machine -- stands
        // on a mouth, and the belt it picks from pours into nothing but the
        // arm's own tile or thin air.
        let unloads: Vec<&FactorioEntity> = arms
            .iter()
            .filter(|arm| {
                built
                    .pickup_position(arm)
                    .and_then(|at| built.entity_at(&at))
                    .is_some_and(|under| is_belt(&under.name))
                    && machine_positions
                        .iter()
                        .any(|machine| built.delivers_into(&arm.position, machine))
            })
            .collect();
        assert_eq!(
            unloads.len(),
            2,
            "one unload arm per belted plate: {unloads:?}"
        );
        for arm in unloads {
            let terminal = built
                .pickup_position(arm)
                .expect("an arm picks up from somewhere");
            let (_, forward) = belt_forward(&built, &terminal).expect("a belt under the arm");
            assert!(
                built.entity_at(&forward).is_none_or(|e| !is_belt(&e.name)),
                "the run's last belt at {terminal} pours onto a belt at {forward}"
            );
            let area = built.collision_area(INSERTER, &arm.position).unwrap();
            assert!(
                built.electric_supply_kw(&area) > 0.,
                "the unload arm at {} stands outside every pole's reach",
                arm.position
            );
        }
    }
}
