//! Stage 1 of the starter factory: a machine that makes things.
//!
//! A burner mining drill standing on ore, dropping straight into a stone
//! furnace two tiles ahead of it. Nine iron plates and ten stone; no research,
//! no electricity, no wood, no inserter, no belt, and no change to
//! `crates/executor` or `mods/BotBridge`. It is the first machine-to-machine
//! link this project has ever planned — all 329 entities placed across 21
//! archived runs are stone furnaces, loaded by hand.
//!
//! # What this method claims, and what it deliberately does not
//!
//! [`Goal::Producing`] names a *rate*, and a rate is a durative claim that this
//! crate cannot check: `PlanState` models presence, never process. So the
//! question is split, and the split is the design.
//!
//! * **The planner answers structure.** Do enough drills stand, on the right
//!   ore, facing the right way, each delivering into a furnace? That is
//!   [`Condition::Feeds`] plus [`PlanState::covers_resource`], and it is what
//!   [`cells_standing`] counts and what [`holds_producing`] answers.
//! * **The supervisor answers duration.** `supervisor.witness` dispatches no
//!   actions at all, waits, and asserts a terminal machine's output inventory
//!   rose anyway. With every bot idle, any increase is machine-made by
//!   construction.
//!
//! **Neither half is sufficient, and this file is only the first.** Say plainly
//! what a structurally satisfied cell can still be: a drill whose fuel slot has
//! run out (22 minutes per fuelling and nothing reads a fuel level), a furnace
//! whose output has backed up because nobody empties it, or a patch mined out
//! from under a drill that never claimed it. `PlanState` models none of the
//! three and no test in this file can catch any of them. What it *does* make
//! impossible is the failure this project has paid for twice — a machine that
//! is merely *placed*: a drill one tile off the patch, or facing away from its
//! furnace, is refused here, and both of those place 100 %, pass every geometry
//! check, and produce nothing.
//!
//! # One shape of cell, not a layout solver
//!
//! The precedent is `crate::method::power`, which builds a six-building power
//! plant as a rigid body of derived offsets, sited by one search and rotated
//! about a tile centre. A cell is the same object with two parts: the drill's
//! site is the only state-dependent choice, and the furnace follows from it by
//! arithmetic. Ten independent first-fit searches would be ten chances for the
//! connection — which is the entire content of the design — to come apart, with
//! no repair step, because you cannot slide a furnace one tile without
//! invalidating the drop point that put it there.

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ItemId, Ticks};
use crate::method::have::{
    COAL_BURN_TICKS, Demand, PLACE_TICKS, TRANSFER_TICKS, attach_unlock, demand,
};
use crate::method::util::{
    CRAFTING_CATEGORY, RecipeGate, SMELTING_CATEGORY, ingredients_of, mining_ticks,
    nearest_resource_tile, output_per_craft, recipe_for, recipe_gate, recipe_ticks,
    seconds_to_ticks, smelting_ticks, tile_alignment_facing,
};
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::num_traits::{FromPrimitive, ToPrimitive};
use factorio_bot_core::types::{Direction, FactorioEntity, FactorioRecipe, Pos, Position};
use std::collections::BTreeSet;

/// The two machines a stage-1 cell is made of.
pub const DRILL: &str = "burner-mining-drill";
pub const FURNACE: &str = "stone-furnace";

/// Where the furnace stands relative to the drill, in the drill's north frame.
///
/// Two tiles ahead: the drill covers the two tile rows behind its own centre
/// and the furnace covers the two ahead of them, which is the vanilla starter
/// pair. The offset is turned by the cell's facing, and because the drill's
/// centre is on a tile **corner** (a 2x2 entity has an even extent on both
/// axes) a quarter turn about it takes corners to corners — so the furnace
/// stays on its own build grid at all four facings. That is
/// [`tests::every_facing_puts_both_machines_on_their_own_grid`], not a comment.
const FURNACE_OFFSET: (f64, f64) = (0., -2.);

/// How far from the anchor tile a cell site is looked for, in tiles.
///
/// The same 12 `crate::method::util::free_area_near` uses, and for the same
/// reason: a cell has to sit at a patch *edge* — its drill on the ore and its
/// furnace off it — so the search has to be able to walk out of the middle of a
/// patch to reach one. Twelve rings is 625 candidate tiles times four facings,
/// and only the candidates that pass both free-area checks pay for anything
/// more than that.
const CELL_SEARCH_RADIUS: i32 = 12;

/// How far around a drill a candidate furnace is looked for when *counting*
/// cells that already stand, in tiles.
///
/// Not a policy: the furnace is two tiles away by construction and the widest
/// delivery offset in the table reaches 1.85, so four tiles covers every cell
/// this planner can build with room to spare. It exists only to keep
/// [`cells_standing`] from reading the whole map per drill.
const CELL_PAIR_RADIUS: f64 = 4.;

/// How long one coal keeps a **burner mining drill** running, in ticks.
///
/// Coal carries 4 MJ and a burner drill draws 150 kW, so one coal sustains
/// 4 MJ / 150 kW = 26.67 s = 1600 ticks. The sibling of
/// [`COAL_BURN_TICKS`], which is the same arithmetic at a stone furnace's
/// 90 kW, and written down here for the same reason: **the mod does not send
/// `energy_usage`**, so neither number can be read from the world. Sending it
/// is the follow-up that deletes both constants and derives fuel from energy.
///
/// Rounding down over-fuels very slightly, which is the safe direction.
const DRILL_BURN_TICKS: Ticks = 1600;

/// How long a cell is fuelled to run unattended, in ticks — ten minutes.
///
/// **A flat budget, and deliberately not derived from the plan's duration**,
/// exactly as `power::PLANT_COAL` is not: a `Producing` goal names a rate and
/// no end, so there is no duration to derive one from, and this crate has no
/// wall clock.
///
/// Ten minutes is bounded above and below by two real numbers. Below: a cell
/// makes 15 plates a minute, so ten minutes is ~150 plates against the ~37 coal
/// this budget costs — the cell pays back its own 9 iron plates in 36 seconds
/// and its fuel many times over. Above: a burner machine has **one** fuel slot,
/// which holds one stack of 50 coal, so the drill's 23 and the furnace's 14 both
/// fit in the slot the executor puts them in. A budget past ~35 minutes would
/// silently ask the game to accept more coal than a slot holds.
///
/// **After it runs out, nothing detects it.** No fuel level is read anywhere in
/// this stack, so the cell simply stops and the structural predicate keeps
/// holding. Only the supervisor's witness catches it, and only at the next
/// witness milestone.
const CELL_FUELLED_TICKS: Ticks = 36_000;

/// The most cells one goal may ask for.
///
/// A bound on work, not a claim about what a map could hold. Siting a cell
/// walks a patch, so a rate asking for millions of them is a hang rather than a
/// refusal; this turns it into a refusal. Twelve cells is 180 iron plates a
/// minute, an order of magnitude past anything the ladder has ever consumed.
const MAX_CELLS: u32 = 12;

/// Minutes are what a rate is stated in and ticks are what everything else is
/// measured in.
const TICKS_PER_MINUTE: u64 = 3600;

/// What a cell for one item is made of, resolved from the world's own recipes.
#[derive(Clone, Debug, PartialEq)]
pub struct CellSpec {
    /// What the cell produces, e.g. `iron-plate`.
    pub item: ItemId,
    /// The single ore its drill mines, e.g. `iron-ore`.
    pub ore: ItemId,
    /// The smelting recipe the furnace runs.
    pub recipe: FactorioRecipe,
    /// Ticks per item for the whole cell: the **slower** of its two halves.
    ///
    /// Integer, and the whole reason [`Goal::Producing`] carries a `u32`
    /// rather than an `f64`. The cell count is
    /// `ceil(per_minute * ticks_per_item / 3600)`, which is exact integer
    /// arithmetic end to end — no float touches it, so it cannot land on a
    /// different side of a ceiling on a different run.
    pub ticks_per_item: Ticks,
}

/// Can a stage-1 cell make `item`, and out of what?
///
/// Three conditions, all read from the world rather than named here:
///
/// * the recipe is a **smelting** recipe, because a stone furnace runs no
///   other category;
/// * it takes exactly **one** ingredient, of amount one, because a burner
///   drill delivers one kind of thing and no inserter tops the furnace up;
/// * that ingredient is something the map carries as a **resource**, because a
///   drill has to stand on it.
///
/// In vanilla 2.1 that is the four plates and nothing else, which is exactly
/// stage 1's scope. `None` is not "impossible" — it is "no *machine* this
/// planner can build makes it", and `Goal::Have` still reaches it by hand.
pub fn cell_spec(state: &PlanState, item: &str) -> Option<CellSpec> {
    let recipe = recipe_for(state, item)?;
    if recipe.category != SMELTING_CATEGORY {
        return None;
    }
    let ingredients = ingredients_of(&recipe);
    let [(ore, amount)] = ingredients.as_slice() else {
        return None;
    };
    if *amount != 1 {
        return None;
    }
    // A resource, not merely an item: a drill stands on ground, and
    // `entity_type` is the game's own classification — the same field
    // `PlanState::stands_on_resources` reads from the other side.
    if state
        .base()
        .entity_prototypes
        .get(ore.as_str())
        .map(|proto| proto.entity_type.clone())
        .as_deref()
        != Some("resource")
    {
        return None;
    }
    let ticks_per_item =
        drill_ticks_per_item(state, ore)?.max(furnace_ticks_per_item(state, &recipe, item));
    if ticks_per_item == 0 {
        return None;
    }
    Some(CellSpec {
        item: item.to_string(),
        ore: ore.clone(),
        recipe,
        ticks_per_item,
    })
}

/// Ticks a burner drill takes to produce one ore.
///
/// `mining_time / mining_speed` seconds, the same division
/// `crate::method::util::mining_ticks` does for a character — the prototype's
/// `mining_time` is the numerator, never the answer. Vanilla: iron ore's 1 s
/// over the drill's 0.25 is 4 s, i.e. 240 ticks, i.e. 15 a minute.
fn drill_ticks_per_item(state: &PlanState, ore: &str) -> Option<Ticks> {
    let mining_time = state
        .base()
        .entity_prototypes
        .get(ore)
        .and_then(|p| p.mining_time)
        .unwrap_or(1.0);
    let speed = state
        .base()
        .entity_prototypes
        .get(DRILL)
        .and_then(|p| p.mining_speed)
        // A zero speed is a divide by zero, not a slow drill. No prototype
        // for the drill at all means this world cannot build a cell, which is
        // a refusal rather than a guessed rate.
        .filter(|speed| *speed > 0.)?;
    Some(seconds_to_ticks(mining_time / speed))
}

/// Ticks a stone furnace takes to produce one `item`.
///
/// Divided by the recipe's own yield, because a recipe producing two per run
/// takes half as long per item. Vanilla iron plate: 3.2 s at crafting speed 1,
/// yield 1, i.e. 192 ticks, i.e. 18.75 a minute — so the **drill** is the
/// bottleneck of a stage-1 cell and its 240 ticks is what
/// [`CellSpec::ticks_per_item`] carries.
fn furnace_ticks_per_item(state: &PlanState, recipe: &FactorioRecipe, item: &str) -> Ticks {
    smelting_ticks(state, recipe, FURNACE).div_ceil(output_per_craft(recipe, item).max(1))
}

/// How many cells `per_minute` of an item at `ticks_per_item` needs.
///
/// `ceil(per_minute * ticks_per_item / 3600)`, in `u64` so a large rate
/// refuses rather than wrapping to something cheap.
pub fn cells_for(per_minute: u32, ticks_per_item: Ticks) -> Result<u32, PlannerError> {
    let cells = (u64::from(per_minute) * u64::from(ticks_per_item)).div_ceil(TICKS_PER_MINUTE);
    if cells > u64::from(MAX_CELLS) {
        return Err(PlannerError::TooManyCells {
            cells,
            max: MAX_CELLS,
        });
    }
    // The cast is safe: `cells <= MAX_CELLS`, which is a `u32`.
    Ok(cells as u32)
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// A cell, sited and checked, ready to be turned into steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// Where the drill stands, and the direction it faces.
    pub drill: Position,
    pub facing: Direction,
    /// The furnace it drops into — the cell's terminal machine, and the one a
    /// `supervisor.witness` reads.
    pub furnace: Position,
}

/// Where the furnace of a cell whose drill stands at `drill` facing `facing`
/// goes.
fn furnace_site(drill: &Position, facing: Direction) -> Option<Position> {
    Some(drill.add(&Position::new(FURNACE_OFFSET.0, FURNACE_OFFSET.1).turn(facing)?))
}

/// Would a stone furnace standing at `site` take ground a stage-1 `ore` cell
/// needs?
///
/// # Why a hand-smelt has to ask
///
/// A cell is a drill standing **on** the ore with its furnace two tiles ahead
/// standing **off** it, so the only ground a cell's furnace can occupy is the
/// ring of non-ore tiles immediately outside the patch — the same ring
/// [`crate::method::util::free_area_near`] settles on when a hand-smelt sites
/// its furnace from `nearest_resource_tile`. The two want the identical tiles,
/// and the smelt gets there first: it is expanded inline, the cell arrives as a
/// subgoal, and the furnace it built is still standing on the next plan.
///
/// Measured on `workspace/runs/run-1788497495-79997`'s world: the iron patch
/// packs 15 cells clean, 7 with the run's 44 standing furnaces, and 7 again
/// after a single further plan's 13 hand-smelt furnaces are sited on the clean
/// world. Roughly **0.6 cell sites per hand-smelt furnace**, and the run halted
/// on `NoRoomForCell` after six epochs of it with zero drills ever placed.
///
/// Asked of the *state as it stands*, which is what makes it cheap and honest:
/// a site is refused only when a cell really does fit there **now**, so ground
/// no cell could use is never withheld and a patch with no cell sites left
/// withholds nothing at all.
pub fn is_cell_furnace_ground(state: &PlanState, ore: &str, site: &Position) -> bool {
    Direction::orthogonal().into_iter().any(|facing| {
        // Invert `furnace_site`: the drill this site would be the furnace of.
        let Some(offset) = Position::new(FURNACE_OFFSET.0, FURNACE_OFFSET.1).turn(facing) else {
            return false;
        };
        let drill = site.add(&Position::new(-offset.x(), -offset.y()));
        fit(state, &drill, facing, ore).is_some()
    })
}

/// Cell sites a hand-smelt has to leave a patch.
///
/// **Not [`MAX_CELLS`].** That is a bound on *work* — "an order of magnitude
/// past anything the ladder has ever consumed", by its own docstring — and
/// reserving that many would withhold ground on patches in no danger whatever;
/// on this map the copper patch packs 12 to 16, so a reserve of `MAX_CELLS`
/// fires on `researched:automation` and moves red.
///
/// This is the other number: the most cells anything on the ladder has
/// actually asked one patch for. `producing:logistic-science-pack:6` sites
/// **four** iron cells and two copper ones, in every one of the six epochs of
/// `workspace/runs/run-1788497495-79997`. Six is that with margin, and it sits
/// under the nine `researched:automation` leaves standing at its tightest —
/// measured, not assumed — so red is untouched.
///
/// What it buys, paving that run's own map one furnace at a time from where
/// its last bot stood: the iron patch's cell count decays
/// `15, 11, 10, 8, 6, 4, 3, 2, 1` over sixty furnaces without the reserve, and
/// `15, 11, 10, 8, 6, 5, 5, 5, 5` with it. A **floor of five**, against the
/// four iron cells green asks for — and against the zero the live run reached.
const CELL_SITES_RESERVED: u32 = 6;

/// Can the patch near `from` still site [`CELL_SITES_RESERVED`] cells for
/// `item`?
///
/// While it can, a hand-smelt standing its furnace on a cell site costs the
/// planner nothing and sites exactly where it always did. Once it cannot,
/// every remaining site is one a `Producing` goal is likely to need, and
/// [`is_cell_furnace_ground`] starts withholding them.
///
/// Packed on a fork, so this counts sites that can **co-exist** rather than
/// sites that each fit on their own — two cells a tile apart do not both fit,
/// and counting them separately would report room that is not there. It stops
/// at the reserve because the only question is which side of it the patch is
/// on, never how far past.
pub fn cell_room_to_spare(state: &PlanState, from: &Position, item: &str) -> bool {
    let Some(spec) = cell_spec(state, item) else {
        // Nothing a cell can make, so nothing a cell can be crowded out of.
        return true;
    };
    let mut trial = state.fork();
    for _ in 0..CELL_SITES_RESERVED {
        let Ok(cell) = plan_cell(&trial, from, &spec) else {
            return false;
        };
        for entity in parts(&trial, &cell) {
            trial.create_entity(entity);
        }
    }
    true
}

/// The drill and the furnace `cell` is made of, in build order.
///
/// The drill first, so a furnace can never be standing where the drill has to
/// go: the two footprints are disjoint by construction
/// ([`tests::a_cells_two_machines_never_overlap`]) but the order is what makes
/// the reservation in [`cell_steps`] mean anything.
pub fn parts(state: &PlanState, cell: &Cell) -> Vec<FactorioEntity> {
    vec![
        machine(state, DRILL, &cell.drill, cell.facing),
        machine(state, FURNACE, &cell.furnace, Direction::North),
    ]
}

/// The `FactorioEntity` one part of a cell places.
///
/// `entity_type` is read from the prototype rather than guessed — a burner
/// mining drill's type is `mining-drill` and a stone furnace's is `furnace`,
/// neither of which is its name, and `EntityGraph::add`'s whitelist,
/// `PlanState::stands_on_resources` and `EntityGraph`'s own miner-ore lookup
/// are all keyed on the type.
fn machine(
    state: &PlanState,
    name: &str,
    position: &Position,
    facing: Direction,
) -> FactorioEntity {
    let entity_type = state
        .base()
        .entity_prototypes
        .get(name)
        .map(|proto| proto.entity_type.clone())
        .unwrap_or_else(|| name.to_string());
    FactorioEntity {
        name: name.to_string(),
        entity_type,
        position: position.clone(),
        direction: Direction::to_u8(&facing).unwrap_or(0),
        ..Default::default()
    }
}

/// Does a whole cell fit with its drill at `drill` facing `facing`?
///
/// Four questions, and the last one is the one that makes this more than a
/// geometry check:
///
/// 1. the drill's footprint covers a tile that still holds `ore` — a drill one
///    tile off the patch mines nothing — and no tile it covers has already
///    been promised to a mining action
///    ([`PlanState::covers_claimed_resource`]). Ore is the one thing
///    `is_area_free_facing` lets a drill stand on, so without the second half
///    a cell will happily be sited on the very tiles the plan is about to send
///    a bot to hand-mine, and the bot arrives to `expected iron-ore ..., found
///    burner-mining-drill`;
/// 2. the ground under the drill is otherwise clear (ore does not block a
///    drill; everything else still does, and `stands_on_resources` is narrow on
///    purpose);
/// 3. the ground under the furnace is clear, **including of ore** — which is
///    what pushes a cell to a patch edge rather than into the middle of one;
/// 4. with both machines standing, the drill's drop point really does land in
///    the furnace. Asked of a fork with the pair placed, so it is the same
///    predicate [`Condition::Feeds`] will be checked with rather than a
///    restatement of it.
fn fit(state: &PlanState, drill: &Position, facing: Direction, ore: &str) -> Option<Cell> {
    let area = state.collision_area_facing(DRILL, drill, facing)?;
    if !state.covers_resource(&area, ore) {
        return None;
    }
    if state.covers_claimed_resource(&area) {
        return None;
    }
    if !state.is_area_free_facing(DRILL, drill, facing) {
        return None;
    }
    let furnace = furnace_site(drill, facing)?;
    if !state.is_area_free_facing(FURNACE, &furnace, Direction::North) {
        return None;
    }
    let cell = Cell {
        drill: drill.clone(),
        facing,
        furnace,
    };
    let mut trial = state.fork();
    for entity in parts(state, &cell) {
        trial.create_entity(entity);
    }
    if !trial.delivers_into(&cell.drill, &cell.furnace) {
        return None;
    }
    Some(cell)
}

// ---------------------------------------------------------------------------
// Siting
// ---------------------------------------------------------------------------

/// Find somewhere to put one cell for `ore` within reach of `from`, or say why
/// not.
///
/// Deterministic by construction: the anchor is the tile
/// `nearest_resource_tile` orders first (distance, then `x`, then `y`), the
/// candidates come out of a ring search in a fixed order, and the four facings
/// are tried north, east, south, west. Nothing here reads a quad tree's own
/// order.
pub fn plan_cell(
    state: &PlanState,
    from: &Position,
    spec: &CellSpec,
) -> Result<Cell, PlannerError> {
    let anchor = nearest_resource_tile(state, &spec.ore, from, 1).ok_or_else(|| {
        PlannerError::NoPatchForCell {
            item: spec.item.clone(),
            ore: spec.ore.clone(),
        }
    })?;
    let base = Pos::from(&anchor);
    for radius in 0..=CELL_SEARCH_RADIUS {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                // Only the ring at exactly this radius; inner ones were done.
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                for facing in Direction::orthogonal() {
                    // The drill's own build grid, which is a function of its
                    // facing: a 2x2 entity is even on both axes so this is the
                    // integer grid at every cardinal, but reading it rather
                    // than assuming it is what keeps this correct the day the
                    // cell's first machine is not square.
                    let (offset_x, offset_y) = tile_alignment_facing(state, DRILL, facing);
                    let candidate = Position::new(
                        f64::from(base.0 + dx) + offset_x,
                        f64::from(base.1 + dy) + offset_y,
                    );
                    if let Some(cell) = fit(state, &candidate, facing, &spec.ore) {
                        return Ok(cell);
                    }
                }
            }
        }
    }
    Err(PlannerError::NoRoomForCell {
        ore: spec.ore.clone(),
        radius: CELL_SEARCH_RADIUS,
    })
}

/// Site `count` cells, each clear of the ones before it.
///
/// Each cell is reserved on a fork as it is chosen, so the next search sees it
/// standing there — the same discipline `power::plant_steps` uses inside one
/// plant, applied across a plan's cells. Without it every cell in a plan lands
/// on the site of the first.
pub fn plan_cells(
    state: &PlanState,
    from: &Position,
    spec: &CellSpec,
    count: u32,
) -> Result<Vec<Cell>, PlannerError> {
    let mut trial = state.fork();
    let mut out = Vec::new();
    for _ in 0..count {
        let cell = plan_cell(&trial, from, spec)?;
        for entity in parts(&trial, &cell) {
            trial.create_entity(entity);
        }
        out.push(cell);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// What already stands
// ---------------------------------------------------------------------------

/// How many complete cells for `spec` already stand in this state.
///
/// A cell counts when a **burner** drill stands on a tile of the cell's own
/// ore and delivers into a stone furnace. All three clauses are load-bearing:
///
/// * *burner*, and not an electric drill, even though
///   [`crate::state::PlanState::delivery_position`] knows about both. An
///   electric drill with no network produces nothing at all, and this stage
///   builds no power and checks none — crediting a rate to a machine whose
///   supply nobody has looked at is the "coverage is not capacity" failure in a
///   new coat. Stage 2 counts them, together with the `Powered` check that
///   makes it honest.
/// * *on its own ore*, because a drill beside the patch is a drill.
/// * *delivers into*, because a drill facing away from its furnace places
///   100 % and moves nothing.
///
/// Order-independent by construction: it counts, and it dedupes drills through
/// a `BTreeSet` of tiles, so the unstable order `resource_patches` may return
/// equal-sized patches in cannot reach the answer.
pub fn cells_standing(state: &PlanState, spec: &CellSpec) -> u32 {
    let mut counted: BTreeSet<Pos> = BTreeSet::new();
    let mut count = 0u32;
    for patch in state.resource_patches(&spec.ore) {
        let centre = Position::new(
            (patch.rect.left_top.x() + patch.rect.right_bottom.x()) / 2.,
            (patch.rect.left_top.y() + patch.rect.right_bottom.y()) / 2.,
        );
        // The patch's own half-diagonal, plus the furthest a cell's drill can
        // sit from a tile of it. `calculate_distance` is Euclidean, which is
        // what `entities_within` measures with.
        let reach = (patch.rect.width() / 2.).hypot(patch.rect.height() / 2.)
            + f64::from(CELL_SEARCH_RADIUS)
            + CELL_PAIR_RADIUS;
        for drill in state.entities_within(&centre, reach) {
            if drill.name != DRILL {
                continue;
            }
            if !counted.insert(Pos::from(&drill.position)) {
                continue;
            }
            let Some(area) = Direction::from_u8(drill.direction)
                .and_then(|facing| state.collision_area_facing(DRILL, &drill.position, facing))
            else {
                continue;
            };
            if !state.covers_resource(&area, &spec.ore) {
                continue;
            }
            let fed = state
                .entities_within(&drill.position, CELL_PAIR_RADIUS)
                .into_iter()
                .any(|target| {
                    target.name == FURNACE && state.delivers_into(&drill.position, &target.position)
                });
            if fed {
                count += 1;
            }
        }
    }
    count
}

/// Does `Goal::Producing { item, per_minute }` hold, structurally, right now?
///
/// The answer `crate::method::have::holds` gives for that goal, and the reason
/// it stopped being `None`. It is `Some(_)` and not `None` because the entity
/// overlay really can answer a question about entities — but it answers a
/// **narrower question than the goal's name suggests**, which is why
/// [`Goal::Producing`]'s own doc comment says the narrow thing in its first
/// line.
///
/// `false` for an item no cell can make: the arrangement does not exist, which
/// is a fact, not an absence of one.
pub fn holds_producing(state: &PlanState, item: &str, per_minute: u32) -> bool {
    let Some(spec) = cell_spec(state, item) else {
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
/// **Build order, and nothing rides on it** — which is worth writing down,
/// because it looks as though something should. `burner-mining-drill` costs
/// *one stone furnace* among its ingredients, so a bill that asked for the
/// furnaces first looks like one whose furnaces `HandCraft` then eats to make
/// the drills. It does not: `run_steps` **reserves each `Goal::Have`'s whole
/// stated count** as soon as that subgoal is satisfied, so the drill's own
/// craft sees the placement furnaces as spoken for and makes another. Reversing
/// these two lines changes no plan, which a mutation confirmed rather than an
/// argument, and
/// [`tests::the_plan_crafts_a_furnace_for_the_drill_as_well_as_one_to_place`]
/// is a test of that reservation and not of this order.
///
/// `Goal::Have` and not `Goal::Produced`, which is where the 2026-09-01 design
/// named the wrong goal: `Produced` deliberately ignores inventory — that is
/// its entire reason to exist — so a bill written with it re-crafts machines
/// the bot is already carrying, every replan.
///
/// `Holder::Share`, for the same reason `power::bill` uses it: one bot places
/// these, so one bot has to be holding them, and `Anyone` sizes its shortfall
/// against the sum across the roster.
fn bill(count: u32, coal: u32) -> Vec<(&'static str, u32)> {
    vec![(DRILL, count), (FURNACE, count), ("coal", coal)]
}

/// How much coal one machine of `burn_ticks` per coal takes to run for
/// `duration` ticks.
fn fuel_for_duration(duration: Ticks, burn_ticks: Ticks) -> u32 {
    duration.div_ceil(burn_ticks.max(1)).max(1)
}

/// How much coal one machine of `burn_ticks` per coal takes to run
/// [`CELL_FUELLED_TICKS`].
fn fuel_for(burn_ticks: Ticks) -> u32 {
    fuel_for_duration(CELL_FUELLED_TICKS, burn_ticks)
}

/// The steps that build `cells`.
///
/// Every site is **reserved in `ctx.state` as its `Place` is emitted**, the
/// same discipline `power::plant_steps` uses: `expand` returns its whole step
/// list before `run_steps` executes any of it, so a site left unreserved would
/// be chosen twice by two subtrees of the same plan.
///
/// No `Step::Link` is emitted and none is needed. Each insert carries a
/// `Condition::EntityAt` for the machine it loads, which is world-scoped, so
/// `ActionNetwork::infer_edges` draws the edge from the placement that created
/// it. The `Condition::Feeds` on the furnace's fuel insert names *both*
/// machines through the two `EntityAt`s beside it, so by the time the scheduler
/// checks it both placements have run — which is the whole reason it goes on
/// that action rather than on a placement, where it could not yet be true.
fn cell_steps(ctx: &mut ExpansionCtx, spec: &CellSpec, cells: &[Cell]) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    let drill_coal = fuel_for(DRILL_BURN_TICKS);
    let furnace_coal = fuel_for(COAL_BURN_TICKS);
    let count = cells.len() as u32;

    // A smelting recipe the force has not unlocked will not run in a furnace,
    // and a trigger technology lands *after* the craft that fires it settles —
    // measured at 16 ticks on a real run. So the condition is stated as well as
    // the subgoal, which is what `await_research` reads off the action.
    let mut research_pre: Vec<Condition> = Vec::new();
    match recipe_gate(&ctx.state, &spec.recipe) {
        RecipeGate::NeedsResearch(tech) => {
            steps.push(Step::Subgoal(Goal::Researched(tech.clone())));
            research_pre.push(Condition::Researched(tech));
        }
        RecipeGate::PlannedResearch(tech) => research_pre.push(Condition::Researched(tech)),
        RecipeGate::Open | RecipeGate::Unobtainable => {}
    }

    for (item, amount) in bill(count, (drill_coal + furnace_coal).saturating_mul(count)) {
        steps.push(Step::Subgoal(Goal::Have {
            item: item.into(),
            count: amount,
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

    for cell in cells {
        for entity in parts(&ctx.state, cell) {
            let name = entity.name.clone();
            let position = entity.position.clone();
            // The annulus's inner bound, exactly as the furnace, the lab and
            // the plant use it: standing *on* the tile a building is going for
            // satisfies a plain disc and then has the game refuse the build
            // with `player_blocks_placement`.
            let min_radius = ctx.state.placement_clearance(&name).unwrap_or(0.0);
            let id = ctx.ids.next();
            steps.push(Step::Act(Box::new(Action {
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
            })));
            ctx.state.create_entity(entity);
        }

        for (machine_name, position, coal, feeds) in [
            (DRILL, cell.drill.clone(), drill_coal, false),
            (FURNACE, cell.furnace.clone(), furnace_coal, true),
        ] {
            let id = ctx.ids.next();
            let mut pre = vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: position.clone(),
                    radius: reach,
                    min_radius: 0.0,
                },
                Condition::EntityAt {
                    pos: position.clone(),
                    name: machine_name.into(),
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: "coal".into(),
                    count: coal,
                },
            ];
            if feeds {
                // Both ends named, so this action cannot be scheduled before
                // either machine stands, and cannot be scheduled at all unless
                // the drill really delivers into the furnace. Fuelling a
                // furnace nothing feeds is the placed-but-dead machine this
                // whole stage exists to make impossible.
                pre.push(Condition::EntityAt {
                    pos: cell.drill.clone(),
                    name: DRILL.into(),
                });
                pre.push(Condition::Feeds {
                    from: cell.drill.clone(),
                    to: cell.furnace.clone(),
                });
                pre.extend(research_pre.iter().cloned());
            }
            steps.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::Insert {
                    pos: position.clone(),
                    entity: machine_name.into(),
                    slot: InventorySlot::Fuel,
                    item: "coal".into(),
                    count: coal,
                },
                pre,
                eff: vec![Effect::LoseItem {
                    who: Actor::Role,
                    item: "coal".into(),
                    count: coal,
                }],
                duration: TRANSFER_TICKS,
                pinned: None,
                label: format!("fuel the {} with {} coal", machine_name, coal),
            })));
        }
    }
    steps
}

// ---------------------------------------------------------------------------
// The method
// ---------------------------------------------------------------------------

/// Build enough cells to produce an item at a rate.
pub struct BuildCell;

impl Method for BuildCell {
    fn name(&self) -> &'static str {
        "build-cell"
    }

    /// Claims [`Goal::Producing`] and nothing else, and only when a cell for
    /// the item exists as a *shape*.
    ///
    /// Deliberately no siting here. `applicable` answers "can I satisfy this
    /// goal at all", and a goal this method understands but cannot place is a
    /// **refusal with a reason** — `NoPatchForCell`, `NoRoomForCell` — which is
    /// strictly better than the `NoApplicableMethod` a false answer here would
    /// produce. The supervisor already turns either into a `stuck` milestone
    /// carrying the planner's own code.
    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        match goal {
            Goal::Producing { item, per_minute } => {
                *per_minute > 0 && cell_spec(state, item).is_some()
            }
            _ => false,
        }
    }

    /// One bot builds one plan's worth of cells.
    ///
    /// Three items — the drills, the furnaces and the coal — have to meet in
    /// one inventory before any of them can be placed, which is the same
    /// reasoning `HandCraft` and `Researched` use: each is a separate sub-chain
    /// and a placement holds all of them at once.
    fn converges(&self, _goal: &Goal, _state: &PlanState) -> bool {
        true
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Producing { item, per_minute } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let spec = cell_spec(&ctx.state, item)
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
        // Sited from the ore rather than from the bot, which never advances
        // during expansion -- otherwise every chain walks patch, origin, patch.
        let from = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.position.clone())
            .unwrap_or_default();
        let cells = plan_cells(&ctx.state, &from, &spec, build)?;
        Ok(cell_steps(ctx, &spec, &cells))
    }
}

// ---------------------------------------------------------------------------
// Stage 1 for a one-shot count, not a rate
// ---------------------------------------------------------------------------

/// How many levels of a recipe's own ingredients [`craft_ticks`] follows
/// before it gives up and prices the item as unaffordable.
///
/// Vanilla's deepest chain this planner ever prices is three levels down from
/// a burner mining drill: drill -> iron-plate/iron-gear-wheel/stone-furnace ->
/// iron-ore/iron-plate/stone. Set well past that, the same backstop-not-a-limit
/// role `crate::method::MAX_EXPANSION_DEPTH` plays for the driver's own
/// recursion -- a guard against a cyclic (modded) recipe table, not a number
/// this crate's own data ever reaches.
pub(crate) const CRAFT_TICKS_MAX_DEPTH: u32 = 8;

/// Character-speed ticks to acquire `count` of `item` starting from nothing --
/// no inventory, no furnace already standing, nothing another share already
/// produced. Used **only** to compare [`PlaceDrill`] against the hand path it
/// would replace, in [`cell_setup_bot_ticks`]; it is never what `HandCraft` or
/// `Smelt` actually charge, and a real plan can come out cheaper than this
/// says (a spare furnace already in a bot's hands, most obviously). That
/// asymmetry only ever strengthens a "hand-mine" verdict this function
/// reached and can never flip a "build" verdict the wrong way, which is the
/// direction that matters for a gate whose failure mode to avoid is proposing
/// a drill for a handful of plates.
///
/// Recurses into a recipe's own ingredients, one call per ingredient --
/// unlike `crate::method::have`'s own `solo_ticks`, which deliberately stops
/// at one level. The two exist for opposite reasons: `solo_ticks` under-states a
/// *split*'s alternative on purpose, so under-splitting is its safe failure;
/// this prices a one-off structure against a per-unit cost, where
/// under-pricing the structure is the *unsafe* direction. The recipe graph
/// this recurses over is a DAG in every world the game produces -- the same
/// fact `Researched`'s own doc relies on for technologies -- so `depth` is a
/// backstop, not a limit that is ever expected to bind.
///
/// A recipe category this function does not know how to run by hand (nothing
/// in vanilla reaches this) is priced at `Ticks::MAX / 4`, not `0`:
/// unpriceable must never look free, or an item this function cannot cost
/// would make building look cheap by omission.
pub(crate) fn craft_ticks(state: &PlanState, item: &str, count: u32, depth: u32) -> Ticks {
    if count == 0 {
        return 0;
    }
    // Checked before the recipe, mirroring `solo_ticks`: a raw resource is
    // priced by mining it regardless of `depth`, since it never recurses.
    //
    // `has_resource_patches`, not `!resource_patches(..).is_empty()`: this is
    // a predicate over every item the recursion meets, and most of them are
    // legitimately not ore. The query form warns on each miss and dumps the
    // world's whole resource list beside it, which cost ~22 spurious warning
    // pairs per plan.
    if state.has_resource_patches(item) {
        return mining_ticks(state, item).saturating_mul(count);
    }
    let Some(recipe) = recipe_for(state, item) else {
        // Neither minable nor recipe-bearing: a free item, exactly as
        // `solo_ticks` reads the same absence.
        return 0;
    };
    if depth == 0 {
        return Ticks::MAX / 4;
    }
    let per = output_per_craft(&recipe, item).max(1);
    let runs = count.div_ceil(per);
    let own = match recipe.category.as_str() {
        SMELTING_CATEGORY => smelting_ticks(state, &recipe, FURNACE).saturating_mul(runs),
        CRAFTING_CATEGORY => recipe_ticks(&recipe).saturating_mul(runs),
        _ => return Ticks::MAX / 4,
    };
    ingredients_of(&recipe)
        .into_iter()
        .fold(own, |sum, (ingredient, amount)| {
            sum.saturating_add(craft_ticks(
                state,
                &ingredient,
                amount.saturating_mul(runs),
                depth - 1,
            ))
        })
}

/// Bot-busy ticks [`crate::method::have::Smelt`] actually spends
/// hand-supplying `need` of `spec.item`, mirroring `smelt_steps`'s own
/// arithmetic for the ore, the coal and the furnace's one-time overhead.
///
/// **The furnace's own smelting time is deliberately excluded.** `smelt_steps`
/// says so in place: "the bot is free to do other work across this lag". That
/// makes it wall-clock, not bot-time, and bot-time is what
/// [`PlaceDrill::applicable`] compares -- the same reason
/// [`cell_setup_bot_ticks`] excludes the drill's own mining time below.
fn hand_smelt_bot_ticks(state: &PlanState, spec: &CellSpec, need: u32) -> Ticks {
    let per_craft = output_per_craft(&spec.recipe, &spec.item).max(1);
    let runs = need.div_ceil(per_craft);
    let coal = recipe_ticks(&spec.recipe)
        .saturating_mul(runs)
        .div_ceil(COAL_BURN_TICKS)
        .max(1);
    // Mine the ore, mine the coal, place one furnace, load it twice (ore,
    // fuel) and take the result once -- `smelt_steps`'s whole action list
    // minus the wait between the last load and the take.
    mining_ticks(state, &spec.ore)
        .saturating_mul(runs)
        .saturating_add(mining_ticks(state, "coal").saturating_mul(coal))
        .saturating_add(PLACE_TICKS)
        .saturating_add(TRANSFER_TICKS.saturating_mul(3))
}

/// Bot-busy ticks [`PlaceDrill`] spends building one cell and fuelling it to
/// run unattended for `need` items plus one cycle of headroom -- the same
/// margin `smelt_steps` gives its own wait, for the same reason: a fuel load
/// sized to land exactly on the last item is right only if nothing is early.
///
/// **The drill's own mining time is excluded**, for the same reason
/// [`hand_smelt_bot_ticks`] excludes the furnace's smelting time: once fuelled
/// the bot walks away, and the drill mines unattended.
fn cell_setup_bot_ticks(state: &PlanState, spec: &CellSpec, need: u32) -> Ticks {
    let duration = spec.ticks_per_item.saturating_mul(need.saturating_add(1));
    let coal = fuel_for_duration(duration, DRILL_BURN_TICKS)
        .saturating_add(fuel_for_duration(duration, COAL_BURN_TICKS));
    // The drill, its own placement furnace, and the coal for both -- `bill`'s
    // three items, priced from raw materials since none of them can be
    // assumed already in hand. Place both machines, fuel both, and take the
    // result once.
    craft_ticks(state, DRILL, 1, CRAFT_TICKS_MAX_DEPTH)
        .saturating_add(craft_ticks(state, FURNACE, 1, CRAFT_TICKS_MAX_DEPTH))
        .saturating_add(craft_ticks(state, "coal", coal, CRAFT_TICKS_MAX_DEPTH))
        .saturating_add(PLACE_TICKS.saturating_mul(2))
        .saturating_add(TRANSFER_TICKS.saturating_mul(3))
}

/// Build one stage-1 cell to satisfy a one-shot [`Goal::Have`] or
/// [`Goal::Produced`], instead of mining and hand-smelting `count` units by
/// hand.
///
/// # Why a new method, and not a branch of `BuildCell`
///
/// [`BuildCell`] answers a *rate* and never removes what it makes: a
/// `supervisor.witness` reads the standing structure, not an inventory, and
/// the whole point of stage 1's design is that nobody ever empties the
/// furnace it builds. A one-shot goal is the opposite question -- it needs
/// `count` of `item` **in a bot's hands**, once, and then the cell is done.
/// Sharing `BuildCell::expand` would mean teaching one function two unrelated
/// endings; this method instead reuses the geometry (`cell_spec`,
/// `plan_cells`, `parts`) and adds the one step `BuildCell` deliberately never
/// takes: a `Remove` off the furnace's own output, sized to `need` and gated
/// on the same `Condition::Feeds`-checked pair `BuildCell` places.
///
/// # What happens to the cell afterward
///
/// It is left standing, running -- nothing in this crate ever demolishes
/// anything. That is the same answer `BuildCell` already gives for a
/// `Producing` goal whose rate a replan later drops, and it is the right one
/// here too: the cell is fuelled for `need` items and then it idles, which
/// costs nothing further and is available for free if a later goal asks for
/// more of the same plate -- `AlreadySatisfied`/`cells_standing`-style reuse
/// of a structure this crate already does for `Producing`. A future goal
/// wanting the coal back, or the ground under it, is out of scope for the
/// same reason demolition is out of scope for `BuildCell`.
///
/// # Why quantity gates it, and what the gate actually compares
///
/// A burner mining drill is *slower* per unit than a character's own hands --
/// 240 ticks an ore against 120 at vanilla's numbers -- so building one never
/// wins on raw wall-clock ticks for this stage; see
/// [`cell_setup_bot_ticks`] and [`hand_smelt_bot_ticks`], which `applicable`
/// compares directly, in the same unit, with no float and no invented
/// constant. What a cell wins on is **bot attention**: once it is fuelled the
/// bot walks away and the drill mines unattended, while hand-smelting keeps
/// the bot mining every single unit itself. So the comparison is *bot-busy*
/// ticks, not wall-clock ticks -- the same quantity this crate's own
/// `solo_ticks`/`worth_converging` pair already compares for the analogous
/// mining-vs-splitting question, and the one `SplitAcrossBots` frees a bot
/// for.
///
/// # Registered ahead of `Smelt`
///
/// `Smelt` (and `SharedSmelt`) claim any smelting-category `Have`/`Produced`
/// regardless of `count`, so this has to be asked first for the comparison to
/// mean anything -- asked after `Smelt` had already claimed the goal, it would
/// never run at all. See `registry_for`'s own ordering comment for the one
/// case this method does **not** reach: a top-level `Holder::Anyone` goal is
/// scattered by `SplitAcrossBots` before either method sees it, and each
/// resulting share is sized against the roster rather than against one bot's
/// hands -- narrowing a share below this gate's crossover is a known
/// limitation, not something this method can see from here.
pub struct PlaceDrill;

impl Method for PlaceDrill {
    fn name(&self) -> &'static str {
        "place-drill"
    }

    /// The drill, the furnace and their coal all have to land in one bot's
    /// hands before any of them can be placed -- the same reasoning
    /// `BuildCell` gives for the identical bill.
    fn converges(&self, _goal: &Goal, _state: &PlanState) -> bool {
        true
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Some(Demand { item, need, .. }) = demand(goal, state) else {
            return false;
        };
        if need == 0 {
            return false;
        }
        let Some(spec) = cell_spec(state, item) else {
            return false;
        };
        // A structural refusal -- no ore patch reachable, no room for the
        // pair -- is `expand`'s business, exactly as `BuildCell` leaves it to
        // its own `expand`: answering it here would pay for the same siting
        // search twice.
        cell_setup_bot_ticks(state, &spec, need) < hand_smelt_bot_ticks(state, &spec, need)
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Some(Demand {
            item,
            need,
            unlocks,
            ..
        }) = demand(goal, &ctx.state)
        else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let spec = cell_spec(&ctx.state, item)
            .ok_or_else(|| PlannerError::NoCellProduces { item: item.clone() })?;

        let mut steps: Vec<Step> = Vec::new();

        // A smelting recipe the force has not unlocked will not run in a
        // furnace, exactly as `cell_steps` gates the recurring form of this
        // build.
        let mut research_pre: Vec<Condition> = Vec::new();
        match recipe_gate(&ctx.state, &spec.recipe) {
            RecipeGate::NeedsResearch(tech) => {
                steps.push(Step::Subgoal(Goal::Researched(tech.clone())));
                research_pre.push(Condition::Researched(tech));
            }
            RecipeGate::PlannedResearch(tech) => research_pre.push(Condition::Researched(tech)),
            RecipeGate::Open | RecipeGate::Unobtainable => {}
        }

        let duration = spec.ticks_per_item.saturating_mul(need.saturating_add(1));
        let drill_coal = fuel_for_duration(duration, DRILL_BURN_TICKS);
        let furnace_coal = fuel_for_duration(duration, COAL_BURN_TICKS);

        for (bill_item, amount) in bill(1, drill_coal.saturating_add(furnace_coal)) {
            steps.push(Step::Subgoal(Goal::Have {
                item: bill_item.into(),
                count: amount,
                whose: Holder::Share(ctx.chain_actor),
            }));
        }

        let from = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.position.clone())
            .unwrap_or_default();
        let cells = plan_cells(&ctx.state, &from, &spec, 1)?;
        let cell = cells
            .into_iter()
            .next()
            .ok_or_else(|| PlannerError::NoPatchForCell {
                item: spec.item.clone(),
                ore: spec.ore.clone(),
            })?;

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

        for entity in parts(&ctx.state, &cell) {
            let name = entity.name.clone();
            let position = entity.position.clone();
            let min_radius = ctx.state.placement_clearance(&name).unwrap_or(0.0);
            let id = ctx.ids.next();
            steps.push(Step::Act(Box::new(Action {
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
            })));
            ctx.state.create_entity(entity);
        }

        let mut fuel_ids: Vec<crate::ids::ActionId> = Vec::new();
        for (machine_name, position, coal, feeds) in [
            (DRILL, cell.drill.clone(), drill_coal, false),
            (FURNACE, cell.furnace.clone(), furnace_coal, true),
        ] {
            let id = ctx.ids.next();
            fuel_ids.push(id);
            let mut pre = vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: position.clone(),
                    radius: reach,
                    min_radius: 0.0,
                },
                Condition::EntityAt {
                    pos: position.clone(),
                    name: machine_name.into(),
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: "coal".into(),
                    count: coal,
                },
            ];
            if feeds {
                pre.push(Condition::EntityAt {
                    pos: cell.drill.clone(),
                    name: DRILL.into(),
                });
                pre.push(Condition::Feeds {
                    from: cell.drill.clone(),
                    to: cell.furnace.clone(),
                });
                pre.extend(research_pre.iter().cloned());
            }
            steps.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::Insert {
                    pos: position.clone(),
                    entity: machine_name.into(),
                    slot: InventorySlot::Fuel,
                    item: "coal".into(),
                    count: coal,
                },
                pre,
                eff: vec![Effect::LoseItem {
                    who: Actor::Role,
                    item: "coal".into(),
                    count: coal,
                }],
                duration: TRANSFER_TICKS,
                pinned: None,
                label: format!("fuel the {} with {} coal", machine_name, coal),
            })));
        }

        // The one step `BuildCell` never takes: pull `need` of the item back
        // out of the furnace and into the acting bot's hands, which is what
        // turns a standing structure into a satisfied `Have`/`Produced`.
        let take_id = ctx.ids.next();
        let mut take_pre = vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: cell.furnace.clone(),
                radius: reach,
                min_radius: 0.0,
            },
            Condition::EntityAt {
                pos: cell.furnace.clone(),
                name: FURNACE.into(),
            },
        ];
        take_pre.extend(research_pre.iter().cloned());
        steps.push(Step::Act(Box::new(Action {
            id: take_id,
            kind: ActionKind::Remove {
                pos: cell.furnace.clone(),
                entity: FURNACE.into(),
                slot: InventorySlot::FurnaceResult,
                item: item.clone(),
                count: need,
            },
            pre: take_pre,
            eff: vec![Effect::GainItem {
                who: Actor::Role,
                item: item.clone(),
                count: need,
            }],
            duration: TRANSFER_TICKS,
            pinned: None,
            label: format!("take {} {} from the cell", need, item),
        })));

        // Production cannot start before either machine is fuelled, and the
        // scheduler needs to be told: an `Insert`'s effect satisfies no
        // condition of the `Remove` above, so nothing here is inferred. One
        // cycle of headroom, the same margin `smelt_steps` gives its own wait
        // and for the same reason -- a removal timed to land exactly on the
        // last item is right only if nothing about it runs long.
        for id in fuel_ids {
            steps.push(Step::Link {
                from: id,
                to: take_id,
                lag: duration,
            });
        }

        attach_unlock(&mut steps, item, unlocks);
        Ok(steps)
    }
}

/// How far apart a cell's own two machines are, for the tests below and for
/// anyone reading a plan.
#[cfg(test)]
fn gap_between(state: &PlanState, cell: &Cell) -> f64 {
    let drill = state
        .collision_area_facing(DRILL, &cell.drill, cell.facing)
        .expect("the fixture has a drill prototype");
    let furnace = state
        .collision_area_facing(FURNACE, &cell.furnace, Direction::North)
        .expect("the fixture has a furnace prototype");
    let gap_x = (furnace.left_top.x() - drill.right_bottom.x())
        .max(drill.left_top.x() - furnace.right_bottom.x());
    let gap_y = (furnace.left_top.y() - drill.right_bottom.y())
        .max(drill.left_top.y() - furnace.right_bottom.y());
    gap_x.max(gap_y)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::method::GoalSite;
    use crate::method::expand;
    use crate::method::have::registry_for;
    use crate::network::ActionNetwork;
    use crate::schedule::schedule;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn state(bots: &[BotId]) -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), bots)
    }

    fn iron() -> CellSpec {
        cell_spec(&state(&[BotId(1)]), "iron-plate").expect("iron plate smelts from one ore")
    }

    fn plan(per_minute: u32) -> Result<ActionNetwork, PlannerError> {
        let bots = [BotId(1)];
        let s = state(&bots);
        expand(
            &[Goal::Producing {
                item: "iron-plate".into(),
                per_minute,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
    }

    /// A cell that stands in `s`, built the way the planner would build it.
    fn stand_a_cell(s: &mut PlanState) -> Cell {
        let spec = iron();
        let cell = plan_cell(s, &Position::new(0., 0.), &spec).expect("the fixture has iron ore");
        for entity in parts(s, &cell) {
            s.create_entity(entity);
        }
        cell
    }

    // ---- what a cell is ---------------------------------------------------

    #[test]
    fn a_cell_is_a_burner_drill_dropping_into_a_stone_furnace() {
        let spec = iron();
        assert_eq!(spec.ore, "iron-ore");
        // The drill is the bottleneck: 240 ticks an ore against the furnace's
        // 192 a plate. A cell is 15 plates a minute, not 18.75.
        assert_eq!(spec.ticks_per_item, 240, "3600 / 240 = 15 a minute");
    }

    #[test]
    fn only_what_smelts_from_one_ore_can_be_produced_by_a_stage_one_cell() {
        let s = state(&[BotId(1)]);
        for item in ["iron-plate", "copper-plate"] {
            assert!(cell_spec(&s, item).is_some(), "{item} smelts from one ore");
        }
        for item in ["iron-gear-wheel", "stone-furnace", "burner-mining-drill"] {
            assert!(
                cell_spec(&s, item).is_none(),
                "{item} is crafted, not smelted; a stone furnace will not make it"
            );
        }
        assert!(
            cell_spec(&s, "not-a-thing").is_none(),
            "an item with no recipe at all is not producible"
        );
    }

    #[test]
    fn the_cell_count_is_integer_arithmetic_end_to_end() {
        // The boundary D2' was about: a target that is exactly one machine's
        // output must be one machine, and one more item must be two.
        assert_eq!(cells_for(15, 240).unwrap(), 1);
        assert_eq!(cells_for(16, 240).unwrap(), 2);
        assert_eq!(cells_for(30, 240).unwrap(), 2);
        assert_eq!(cells_for(31, 240).unwrap(), 3);
        assert_eq!(cells_for(0, 240).unwrap(), 0);
    }

    #[test]
    fn a_rate_no_plan_could_build_is_refused_rather_than_attempted() {
        assert!(matches!(
            cells_for(u32::MAX, 240),
            Err(PlannerError::TooManyCells { .. })
        ));
    }

    // ---- geometry ---------------------------------------------------------

    #[test]
    fn every_facing_puts_both_machines_on_their_own_grid() {
        // The rigid-body claim: a quarter turn about the drill's tile corner
        // takes corners to corners, so the furnace is legal at every facing.
        // A wrong offset moves the furnace half a tile and the game refuses
        // the placement -- after the bot has walked there.
        let s = state(&[BotId(1)]);
        for facing in Direction::orthogonal() {
            let drill = Position::new(10., 10.);
            let furnace = furnace_site(&drill, facing).expect("a cardinal facing");
            for (name, pos, dir) in [
                (DRILL, &drill, facing),
                (FURNACE, &furnace, Direction::North),
            ] {
                let (offset_x, offset_y) = tile_alignment_facing(&s, name, dir);
                assert!(
                    (pos.x() - offset_x).fract().abs() < 1. / 512.
                        && (pos.y() - offset_y).fract().abs() < 1. / 512.,
                    "{name} at {pos} facing {facing:?} is off its own build grid"
                );
            }
        }
    }

    #[test]
    fn the_drill_drops_into_the_furnace_at_every_facing() {
        // The claim the whole of stage 1 rests on, and the one the spec named
        // as the cheapest thing a run can settle. It is checked here at all
        // four facings because a rotation that is wrong by one quarter turn
        // still places both machines perfectly.
        for facing in Direction::orthogonal() {
            let mut s = state(&[BotId(1)]);
            let drill = Position::new(10., 10.);
            let furnace = furnace_site(&drill, facing).unwrap();
            let cell = Cell {
                drill: drill.clone(),
                facing,
                furnace: furnace.clone(),
            };
            for entity in parts(&s, &cell) {
                s.create_entity(entity);
            }
            assert!(
                s.delivers_into(&drill, &furnace),
                "a drill facing {facing:?} must drop into the furnace ahead of it"
            );
            assert!(
                !s.delivers_into(&furnace, &drill),
                "and a furnace delivers into nothing: it is not in the table"
            );
        }
    }

    #[test]
    fn the_games_own_drop_position_beats_the_table() {
        // Every entity this planner *places* is built from a name, a position
        // and a direction, so the table is what answers for it. An entity a
        // live save already carries reports its own `drop_position`, which the
        // mod fills in and `EntityGraph::add` even corrects for a pumpjack --
        // and that answer must win, or the planner would overrule the game
        // about the game's own geometry.
        let mut s = state(&[BotId(1)]);
        let drill = Position::new(10., 10.);
        let behind = Position::new(10., 12.);
        let mut entity = machine(&s, DRILL, &drill, Direction::North);
        // Reported as dropping *south*, which is where the table says it
        // cannot.
        entity.drop_position = Some(Position::new(9.65, 11.3));
        s.create_entity(entity);
        s.create_entity(machine(&s, FURNACE, &behind, Direction::North));
        assert!(
            s.delivers_into(&drill, &behind),
            "the reported drop position is the game's answer and the table is only the fallback"
        );
        let ahead = furnace_site(&drill, Direction::North).unwrap();
        s.create_entity(machine(&s, FURNACE, &ahead, Direction::North));
        assert!(
            !s.delivers_into(&drill, &ahead),
            "and where the table would have pointed is not fed at all"
        );
    }

    #[test]
    fn an_electric_drill_is_not_a_stage_one_cell_however_well_it_is_placed() {
        // Stage 1 builds no power and checks none, so an electric drill's
        // rate is a rate nobody has looked at the supply for -- the
        // "coverage is not capacity" failure in a new coat. It is excluded by
        // *name*, and this pins that the exclusion is not accidentally doing
        // its work through the geometry: the pair below really does feed.
        let spec = iron();
        let mut s = state(&[BotId(1)]);
        let drill = Position::new(-34.5, 40.5);
        let furnace = Position::new(-34., 38.);
        s.create_entity(machine(
            &s,
            "electric-mining-drill",
            &drill,
            Direction::North,
        ));
        s.create_entity(machine(&s, FURNACE, &furnace, Direction::North));
        assert!(
            s.delivers_into(&drill, &furnace),
            "the electric drill really does drop into this furnace"
        );
        let area = s
            .collision_area_facing("electric-mining-drill", &drill, Direction::North)
            .unwrap();
        assert!(
            s.covers_resource(&area, "iron-ore"),
            "and it really does stand on the iron patch"
        );
        assert_eq!(
            cells_standing(&s, &spec),
            0,
            "and it is still not a stage-1 cell: nothing here has looked at its power"
        );
    }

    #[test]
    fn a_drill_facing_away_from_its_furnace_feeds_nothing() {
        // The inserter-direction trap, in its stage-1 form. Both machines
        // place; nothing moves.
        let mut s = state(&[BotId(1)]);
        let drill = Position::new(10., 10.);
        let furnace = furnace_site(&drill, Direction::North).unwrap();
        for entity in [
            machine(&s, DRILL, &drill, Direction::South),
            machine(&s, FURNACE, &furnace, Direction::North),
        ] {
            s.create_entity(entity);
        }
        assert!(
            !s.delivers_into(&drill, &furnace),
            "a drill turned around drops behind itself"
        );
    }

    #[test]
    fn a_cells_two_machines_never_overlap_and_leave_no_walkable_lane_between_them() {
        // 0.6015625 between the boxes -- the very gap that produced 18 of 18
        // walk stalls, eight of them at 1/256 of a tile, against a character
        // 0.3984375 wide. It is stated here as a measurement so that nobody
        // later mistakes the space between a drill and its furnace for a
        // servicing lane: the cell is serviced from its flanks.
        let s = state(&[BotId(1)]);
        for facing in Direction::orthogonal() {
            let drill = Position::new(10., 10.);
            let cell = Cell {
                drill: drill.clone(),
                facing,
                furnace: furnace_site(&drill, facing).unwrap(),
            };
            let gap = gap_between(&s, &cell);
            assert!(gap > 0., "the two machines must not overlap at {facing:?}");
            assert!(
                (gap - 0.6015625).abs() < 1. / 4096.,
                "expected the 0.6015625 gap, got {gap} at {facing:?}"
            );
            let character = s
                .base()
                .entity_prototypes
                .get("character")
                .map(|p| p.collision_box.width())
                .unwrap_or(0.3984375);
            assert!(
                gap > character,
                "a character is {character} wide and the gap is {gap}: if this ever \
                 becomes a lane, say so on purpose rather than by arithmetic"
            );
        }
    }

    // ---- siting -----------------------------------------------------------

    #[test]
    fn a_cell_sits_at_a_patch_edge_with_its_drill_on_the_ore() {
        let s = state(&[BotId(1)]);
        let spec = iron();
        let cell = plan_cell(&s, &Position::new(0., 0.), &spec).expect("the fixture has iron ore");
        let drill_area = s
            .collision_area_facing(DRILL, &cell.drill, cell.facing)
            .unwrap();
        assert!(
            s.covers_resource(&drill_area, "iron-ore"),
            "the drill must stand on the ore it mines"
        );
        let furnace_area = s.collision_area(FURNACE, &cell.furnace).unwrap();
        assert!(
            !s.covers_resource(&furnace_area, "iron-ore"),
            "and the furnace must not, or nothing could ever have placed it"
        );
    }

    #[test]
    fn a_site_off_the_ore_does_not_fit_however_clear_the_ground_is() {
        // `fit` is what `plan_cell`'s ring search consults, and its ore check
        // is otherwise unexercised: the search starts *at* an ore tile, so the
        // first site it tries is on the patch whether or not anything checks.
        // Asked directly, off the patch, it must refuse.
        //
        // Far enough east that the *furnace* is clear of the patch too:
        // an earlier version of this test sat one tile off the edge, where
        // `fit` refused the site because the furnace two tiles ahead of it
        // stood on ore -- the right answer for the wrong reason, and a
        // mutation deleting the ore check left it passing.
        let s = state(&[BotId(1)]);
        assert!(
            fit(&s, &Position::new(-32., 40.), Direction::North, "iron-ore").is_none(),
            "the ground east of the patch is clear, and a drill on it mines nothing"
        );
        assert!(
            fit(&s, &Position::new(-35., 35.), Direction::North, "iron-ore").is_some(),
            "and the patch edge two tiles west of it does fit, so the refusal \
             above is about the ore and not about the ground"
        );
    }

    #[test]
    fn a_site_whose_furnace_would_not_be_fed_does_not_fit() {
        // The trial-fork delivery check in `fit`. It is a belt-and-braces
        // check against the layout constants, so it cannot go red while they
        // are right -- what makes it worth having is that a wrong constant is
        // caught *here*, at siting, rather than by a plan that schedules,
        // places both machines perfectly and then does nothing.
        //
        // The pair below is the layout with the furnace **one tile too far**,
        // which is exactly what a wrong `FURNACE_OFFSET` produces. One tile
        // too *near* is not the test: the drill's drop tile is covered by that
        // furnace as well, and the two collision boxes overlap, so the site is
        // refused for being occupied rather than for being unfed. The slack is
        // one tile in one direction only, and that asymmetry is the reason to
        // check the delivery instead of the distance.
        let mut s = state(&[BotId(1)]);
        let drill = Position::new(-35., 35.);
        let too_far = Position::new(-35., 32.);
        for entity in [
            machine(&s, DRILL, &drill, Direction::North),
            machine(&s, FURNACE, &too_far, Direction::North),
        ] {
            s.create_entity(entity);
        }
        assert!(
            !s.delivers_into(&drill, &too_far),
            "a furnace three tiles ahead catches nothing, and `fit` refuses \
             the site rather than planning a dead cell"
        );
    }

    #[test]
    fn two_cells_never_land_on_one_site() {
        let s = state(&[BotId(1)]);
        let spec = iron();
        let cells = plan_cells(&s, &Position::new(0., 0.), &spec, 3).expect("room for three");
        let sites: BTreeSet<Pos> = cells
            .iter()
            .flat_map(|c| [Pos::from(&c.drill), Pos::from(&c.furnace)])
            .collect();
        assert_eq!(sites.len(), 6, "three cells are six distinct machines");
    }

    /// A world with every prototype and recipe the fixture has, and no ore at
    /// all — the shape of an attached snapshot, or of a run that has charted
    /// no chunks yet.
    fn world_without_ore() -> factorio_bot_core::factorio::world::FactorioWorld {
        let world = factorio_bot_core::factorio::world::FactorioWorld::new();
        world
            .update_entity_prototypes(
                factorio_bot_core::test_utils::fixture_entity_prototypes()
                    .iter()
                    .map(|v| v.clone())
                    .collect(),
            )
            .unwrap();
        world
            .update_recipes(
                factorio_bot_core::test_utils::fixture_recipes()
                    .iter()
                    .map(|v| v.clone())
                    .collect(),
            )
            .unwrap();
        world
    }

    #[test]
    fn a_map_with_no_patch_of_the_ore_is_a_refusal_that_names_it() {
        let s = PlanState::from_world(Arc::new(world_without_ore()), &[BotId(1)]);
        let spec = cell_spec(&s, "iron-plate").expect("the recipe shape is still describable");
        assert!(
            matches!(
                plan_cell(&s, &Position::new(0., 0.), &spec),
                Err(PlannerError::NoPatchForCell { .. })
            ),
            "a map with no ore refuses by name rather than siting a drill on nothing"
        );
    }

    // ---- counting what stands ---------------------------------------------

    #[test]
    fn a_complete_cell_counts_and_half_of_one_does_not() {
        let spec = iron();
        let mut s = state(&[BotId(1)]);
        assert_eq!(cells_standing(&s, &spec), 0, "an empty map has no cells");
        let cell = stand_a_cell(&mut s);
        assert_eq!(cells_standing(&s, &spec), 1);

        // Take the furnace away: the drill still stands, on the same ore,
        // facing the same way -- and the cell stops counting.
        let mut half = s.fork();
        half.remove_entity(&cell.furnace);
        assert_eq!(
            cells_standing(&half, &spec),
            0,
            "a drill with nothing to drop into is not a cell"
        );
    }

    #[test]
    fn a_furnace_the_drill_does_not_feed_is_not_a_cell() {
        // The whole point, stated as a test: two machines that merely stand
        // there. Same drill, same furnace, same ore, same distance -- and the
        // drill turned around.
        let spec = iron();
        let mut s = state(&[BotId(1)]);
        let cell = stand_a_cell(&mut s);
        let mut turned = s.fork();
        turned.remove_entity(&cell.drill);
        turned.create_entity(machine(&turned, DRILL, &cell.drill, cell.facing.opposite()));
        assert_eq!(
            cells_standing(&turned, &spec),
            0,
            "placed is not producing: a drill facing away moves nothing"
        );
    }

    #[test]
    fn a_drill_beside_the_patch_is_not_a_cell() {
        let spec = iron();
        let s = state(&[BotId(1)]);
        // Two tiles east of the iron patch's edge -- close enough that
        // `cells_standing` really looks at it, which is the whole point: an
        // earlier version of this test put the pair at the origin, where the
        // patch-anchored search never reached it, and it passed for the wrong
        // reason.
        let drill = Position::new(-33., 40.);
        let furnace = furnace_site(&drill, Direction::North).unwrap();
        let mut off = s.fork();
        for entity in [
            machine(&s, DRILL, &drill, Direction::North),
            machine(&s, FURNACE, &furnace, Direction::North),
        ] {
            off.create_entity(entity);
        }
        assert!(
            off.delivers_into(&drill, &furnace),
            "the pair really does feed -- it is only the ore that is missing"
        );
        assert_eq!(
            cells_standing(&off, &spec),
            0,
            "a drill on bare ground mines nothing"
        );
    }

    // ---- the goal ---------------------------------------------------------

    #[test]
    fn a_production_goal_is_answerable_and_answers_no_before_anything_is_built() {
        let s = state(&[BotId(1)]);
        assert!(
            !holds_producing(&s, "iron-plate", 15),
            "nothing stands, so nothing is producing"
        );
        assert!(
            !holds_producing(&s, "iron-gear-wheel", 15),
            "and an item no cell can make is not being produced either"
        );
    }

    #[test]
    fn a_standing_cell_holds_its_own_rate_and_not_a_larger_one() {
        let mut s = state(&[BotId(1)]);
        stand_a_cell(&mut s);
        assert!(holds_producing(&s, "iron-plate", 15), "one cell is 15/min");
        assert!(
            !holds_producing(&s, "iron-plate", 16),
            "and 16 needs a second one"
        );
    }

    // ---- planning ---------------------------------------------------------

    #[test]
    fn a_production_plan_places_a_drill_and_a_furnace_and_fuels_both() {
        let net = plan(15).expect("the fixture can build a cell");
        let placed: Vec<String> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Place { entity } => Some(entity.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            placed.iter().filter(|n| *n == DRILL).count(),
            1,
            "one drill, in {placed:?}"
        );
        assert!(
            placed.iter().filter(|n| *n == FURNACE).count() >= 1,
            "at least the cell's own furnace, in {placed:?}"
        );
        let fuelled: Vec<&str> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Insert {
                    entity,
                    slot: InventorySlot::Fuel,
                    ..
                } => Some(entity.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            fuelled.contains(&DRILL) && fuelled.contains(&FURNACE),
            "both machines are burners and both need coal, got {fuelled:?}"
        );
    }

    #[test]
    fn the_plan_crafts_a_furnace_for_the_drill_as_well_as_one_to_place() {
        // The bill's ordering trap: a burner drill costs a stone furnace, so a
        // plan that crafts one furnace has none left to place.
        let net = plan(15).expect("the fixture can build a cell");
        let crafted: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Craft { item, count, .. } if item == FURNACE => Some(*count),
                _ => None,
            })
            .sum();
        assert!(
            crafted >= 2,
            "one furnace goes into the drill and one is placed, got {crafted}"
        );
    }

    #[test]
    fn the_plan_states_that_the_drill_feeds_the_furnace() {
        let net = plan(15).expect("the fixture can build a cell");
        assert!(
            net.actions()
                .any(|a| a.pre.iter().any(|c| matches!(c, Condition::Feeds { .. }))),
            "a plan that never states the link is a plan that never checks it"
        );
    }

    #[test]
    fn a_bigger_rate_builds_more_cells() {
        let one = plan(15).expect("one cell");
        let two = plan(16).expect("two cells");
        let drills = |net: &ActionNetwork| {
            net.actions()
                .filter(|a| matches!(&a.kind, ActionKind::Place { entity } if entity.name == DRILL))
                .count()
        };
        assert_eq!(drills(&one), 1);
        assert_eq!(drills(&two), 2);
    }

    #[test]
    fn a_half_built_factory_is_finished_rather_than_doubled() {
        // The case `AlreadySatisfied` does not cover, and the one a replanning
        // supervisor meets every iteration: some of the goal stands. Two cells
        // are wanted, one is there, so exactly one more is built.
        let bots = [BotId(1)];
        let mut s = state(&bots);
        stand_a_cell(&mut s);
        let net = expand(
            &[Goal::Producing {
                item: "iron-plate".into(),
                per_minute: 30,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("two cells fit at this patch");
        let drills = net
            .actions()
            .filter(|a| matches!(&a.kind, ActionKind::Place { entity } if entity.name == DRILL))
            .count();
        assert_eq!(drills, 1, "one cell stands and the goal wants two");
    }

    #[test]
    fn a_cell_that_already_stands_is_not_built_again() {
        let bots = [BotId(1)];
        let mut s = state(&bots);
        stand_a_cell(&mut s);
        let net = expand(
            &[Goal::Producing {
                item: "iron-plate".into(),
                per_minute: 15,
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a goal that already holds is satisfiable");
        assert!(
            net.is_empty(),
            "one cell already stands and one is all the goal asked for"
        );
    }

    /// `holds` and the empty plan must agree wherever `holds` has an opinion —
    /// the invariant `scripts/supervisor.lua` relies on to tell
    /// `already_satisfied` from a silently empty plan.
    #[test]
    fn an_empty_production_plan_and_a_held_production_goal_agree() {
        let bots = [BotId(1)];
        let mut s = state(&bots);
        stand_a_cell(&mut s);
        for per_minute in [1u32, 15, 16, 30] {
            let goal = Goal::Producing {
                item: "iron-plate".into(),
                per_minute,
            };
            let net = expand(
                std::slice::from_ref(&goal),
                &s,
                &registry_for(&bots),
                BotId(1),
            )
            .expect("iron plate is producible in the fixture world");
            assert_eq!(
                net.is_empty(),
                holds_producing(&s, "iron-plate", per_minute),
                "an empty plan and a held goal must be the same thing for {per_minute}"
            );
        }
    }

    #[test]
    fn an_item_no_cell_can_make_is_refused_by_name() {
        let bots = [BotId(1)];
        let s = state(&bots);
        // Nothing claims it: `BuildCell` is not applicable, so the driver's own
        // "no method" refusal is what a caller sees, and it names the goal.
        assert!(matches!(
            expand(
                &[Goal::Producing {
                    item: "iron-gear-wheel".into(),
                    per_minute: 15,
                }],
                &s,
                &registry_for(&bots),
                BotId(1),
            ),
            Err(PlannerError::NoApplicableMethod { .. })
        ));
    }

    #[test]
    fn a_production_plan_schedules_end_to_end() {
        // The `Feeds` precondition is checked by the scheduler against the
        // state as it advances, exactly as `Powered` is, so a plan that
        // schedules is a plan in which the drill really does deliver into the
        // furnace *at the moment the furnace is fuelled*. A cell whose
        // geometry were wrong would not merely place badly -- it would fail to
        // schedule at all, here, before any bot walked.
        let bots = [BotId(1)];
        let s = state(&bots);
        let net = plan(15).expect("the fixture can build a cell");
        let plan = crate::schedule::schedule(&net, &s, &bots).expect("the cell plan schedules");
        assert!(plan.makespan > 0);
        let again = crate::schedule::schedule(&net, &s, &bots).expect("and again");
        assert_eq!(plan.makespan, again.makespan, "a makespan is a function");
    }

    /// The whole of stage 1, pinned end to end.
    ///
    /// One bot, one cell, 15 iron plates a minute: hand-mine and hand-smelt
    /// nine iron plates, craft three gears and a spare furnace, craft the
    /// drill (which eats a furnace of its own), mine 37 coal, place the pair
    /// at the iron patch's edge and fuel both. 24 actions and 11,025 ticks --
    /// about 3.1 minutes of game time, against a cell that then makes 150
    /// plates in its first ten.
    ///
    /// Pinned rather than described, because every number here is a
    /// consequence of a choice somewhere else: the fuel budget, the bill's
    /// ordering, and the siting. A change that moves any of them should say so
    /// out loud.
    ///
    /// **10,288 until the cell stopped being allowed to bury its own ore.**
    /// The two iron mines used to be `[-34.5, 35.5]` and `[-35.5, 35.5]`,
    /// which are two of the four tiles under the drill this same plan sites at
    /// `[-35, 35]` -- the `run-1788455754-92581` failure in miniature. With
    /// `PlanState::resource_tile_blocked` reading the plan's own entities they
    /// are `[-34.5, 36.5]` and `[-36.5, 35.5]`, one tile further out each, and
    /// the two hand-smelting furnaces sited from those tiles follow them. The
    /// action count does not move -- the same work, 27 ticks more walking.
    ///
    /// **27 actions and 10,315 ticks until in-plan furnace reuse.** The two
    /// hand-smelts are one furnace now, because `have::patch_furnace_budget`
    /// is one per bot per ore patch and this roster is one bot: three actions
    /// go (a stone mine, a craft, a placement) and the second smelt waits on
    /// the first instead, which costs 710 ticks.
    ///
    /// **This is the one fixture in the crate where reuse loses on time**, and
    /// it is pinned that way deliberately rather than tuned away. Two smelts
    /// is the smallest case there is, so a furnace's bill is at its least
    /// amortised here; the same change on the rung-1 solo plan
    /// (`have::the_single_bot_rung_one_plan_is_untouched`) is 113 actions ->
    /// 92 and 46,446 ticks -> 41,835. A bound that got both right would have
    /// to price bot idle time, which this crate cannot do -- `bank_size` says
    /// so at length and for the same reason.
    #[test]
    fn the_whole_of_stage_one_costs_this_much() {
        let bots = [BotId(1)];
        let s = state(&bots);
        let net = plan(15).expect("the fixture can build a cell");
        assert_eq!(net.len(), 24, "actions in a one-cell plan");
        let sched = crate::schedule::schedule(&net, &s, &bots).expect("it schedules");
        assert_eq!(
            sched.makespan, 11_025,
            "ticks for one bot to build one cell"
        );
    }

    #[test]
    fn the_same_inputs_give_the_same_plan_twice() {
        // The crate's defining constraint, asserted rather than assumed.
        let first = plan(30).expect("two cells");
        let second = plan(30).expect("two cells");
        let render = |net: &ActionNetwork| {
            net.actions()
                .map(|a| format!("{:?} {} {:?}", a.id, a.label, a.pre))
                .collect::<Vec<_>>()
        };
        assert_eq!(render(&first), render(&second));
    }

    // ---- PlaceDrill: a one-shot count, not a rate --------------------------

    /// The recursive materials cost `cell_setup_bot_ticks` prices a drill and
    /// a furnace at, worked out from the fixture's own vanilla numbers so the
    /// test is a check on the arithmetic rather than a restatement of it.
    ///
    /// A burner mining drill: 3 iron-plate + 3 iron-gear-wheel + 1
    /// stone-furnace, assembled in 2.0 s = 120 ticks.
    /// * `craft_ticks(iron-plate, K)` = smelt (192 * K) + mine the ore
    ///   (120 * K) = 312 * K.
    /// * `craft_ticks(iron-gear-wheel, K)` = craft (30 * K) +
    ///   `craft_ticks(iron-plate, 2K)` = 30K + 624K = 654 * K.
    /// * `craft_ticks(stone-furnace, K)` = craft (30 * K) + mine 5 stone a
    ///   furnace (5 * 120 * K) = 630 * K.
    ///
    /// So one drill -- which needs its *own* stone-furnace as an ingredient,
    /// per its recipe -- costs `120 + 312*3 + 654*3 + 630 = 3648` ticks, and
    /// the cell's own, separate placement furnace costs another flat `630`:
    /// `4278` for the pair, the number [`the_hand_and_cell_costs_cross_over_near_fifty_plates`]
    /// builds on.
    #[test]
    fn craft_ticks_prices_a_drill_and_a_furnace_from_raw_materials() {
        let s = state(&[BotId(1)]);
        assert_eq!(
            craft_ticks(&s, "stone-furnace", 1, CRAFT_TICKS_MAX_DEPTH),
            630
        );
        assert_eq!(
            craft_ticks(&s, DRILL, 1, CRAFT_TICKS_MAX_DEPTH),
            3648,
            "120 (assemble) + 3*312 (plate) + 3*654 (gear wheel) + 630 (the drill's own furnace)"
        );
    }

    /// The two bot-busy costs [`PlaceDrill::applicable`] compares, at the
    /// quantity that started this investigation (fifty iron plates, the
    /// `steam-power` trigger) and at one small enough that nobody wants a
    /// drill built for it.
    ///
    /// Hand-smelting fifty: mine 50 ore (`120 * 50 = 6000`), mine the coal a
    /// furnace burns smelting them (`recipe_ticks(iron-plate) * 50 = 9600`
    /// ticks of energy, `div_ceil`d by `COAL_BURN_TICKS = 2666` is 4 coal,
    /// `120 * 4 = 480`), one placement (`30`) and three transfers (`3 * 10 =
    /// 30`): `6000 + 480 + 30 + 30 = 6540`.
    ///
    /// Building a cell for fifty: the drill and furnace from
    /// [`craft_ticks_prices_a_drill_and_a_furnace_from_raw_materials`]
    /// (`3648 + 630 = 4278`), two placements (`60`) and three transfers
    /// (`30`) -- `4368` fixed -- plus coal for 51 cycles at the cell's own
    /// 240-tick rate (`51 * 240 = 12240`; `div_ceil(1600) = 8` for the
    /// drill, `div_ceil(2666) = 5` for the furnace, `120 * 13 = 1560`):
    /// `4368 + 1560 = 5928`. That is below hand-smelting's `6540`, so a cell
    /// wins fifty plates -- by a margin of 612 ticks, not a landslide, which
    /// is what makes fifty a real crossover and not an arbitrary example.
    ///
    /// Five: hand-smelting stays cheap (`120*5 + 120*1(coal) + 30 + 30 =
    /// 780`) while a cell's fixed cost barely moves with the quantity
    /// (`4368 + 120*2(coal for 6 cycles) = 4608`), so hand-mining wins by a
    /// wide margin -- nobody builds a drill for five plates.
    #[test]
    fn the_hand_and_cell_costs_cross_over_near_fifty_plates() {
        let s = state(&[BotId(1)]);
        let spec = iron();

        assert_eq!(hand_smelt_bot_ticks(&s, &spec, 50), 6540);
        assert_eq!(cell_setup_bot_ticks(&s, &spec, 50), 5928);
        assert!(
            cell_setup_bot_ticks(&s, &spec, 50) < hand_smelt_bot_ticks(&s, &spec, 50),
            "fifty plates must be cheaper in bot-time to build than to hand-smelt"
        );

        assert_eq!(hand_smelt_bot_ticks(&s, &spec, 5), 780);
        assert_eq!(cell_setup_bot_ticks(&s, &spec, 5), 4608);
        assert!(
            cell_setup_bot_ticks(&s, &spec, 5) > hand_smelt_bot_ticks(&s, &spec, 5),
            "five plates must stay cheaper to hand-smelt than to build a cell for"
        );
    }

    /// The method registry itself, not just the two cost functions in
    /// isolation: a small shortfall still goes to `Smelt`, and the
    /// `steam-power` trigger's fifty goes to `PlaceDrill` -- the concrete
    /// finding this whole method exists to fix.
    #[test]
    fn the_registry_prefers_hand_smelting_below_the_crossover_and_a_cell_above_it() {
        let bots = vec![BotId(1)];
        let s = state(&bots);
        let reg = registry_for(&bots);
        let site = GoalSite {
            top_level: false,
            in_chain: true,
            converging: false,
        };
        let small = Goal::Have {
            item: "iron-plate".into(),
            count: 5,
            whose: Holder::Share(BotId(1)),
        };
        let trigger_sized = Goal::Have {
            item: "iron-plate".into(),
            count: 50,
            whose: Holder::Share(BotId(1)),
        };
        assert_eq!(
            reg.find(&small, &s, site).map(|m| m.name()),
            Some("smelt"),
            "five plates: hand-smelting wins"
        );
        assert_eq!(
            reg.find(&trigger_sized, &s, site).map(|m| m.name()),
            Some("place-drill"),
            "fifty plates, the steam-power trigger's own count: a cell wins"
        );
    }

    /// The actual plan `PlaceDrill` emits for a one-shot count: a drill and a
    /// furnace, fuelled, and -- the one step `BuildCell` never takes -- the
    /// fifty plates pulled back out of the furnace into the bot's own hands.
    #[test]
    fn a_one_shot_plan_places_a_cell_and_takes_the_count_back_out() {
        let bots = vec![BotId(1)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 50,
                whose: Holder::Anyone,
            }],
            &s,
            &crate::method::have::default_registry(),
            BotId(1),
        )
        .expect("fifty plates plans");

        net.actions()
            .find(|a| a.label == "take 50 iron-plate from the cell")
            .expect("the count is pulled back out of the cell");
        assert!(
            net.actions()
                .any(|a| a.label.starts_with("place burner-mining-drill")),
            "a drill is placed"
        );
        assert!(
            net.actions()
                .any(|a| a.label.starts_with("place stone-furnace")),
            "and a furnace"
        );
        // The final fifty plates are never hand-inserted into a furnace --
        // the drill feeds it unattended. A bot starting with empty hands
        // still hand-smelts the nine plates a fresh drill's own recipe
        // needs (see `craft_ticks_prices_a_drill_and_a_furnace_from_raw_materials`),
        // which is the bootstrap this design accepts, not a defect: it is a
        // one-time cost paid once per cell, not once per unit of the goal.
        assert!(
            net.actions().all(|a| a.label != "insert 50 iron-ore"),
            "the final fifty are drilled, not hand-inserted"
        );
        assert!(
            schedule(&net, &s, &bots).is_ok(),
            "the plan this method emits must be schedulable, not merely constructible"
        );
    }
}
