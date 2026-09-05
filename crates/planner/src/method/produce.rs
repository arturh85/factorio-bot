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
use crate::ids::{ActionId, ItemId, Ticks};
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
use factorio_bot_core::types::{Direction, FactorioEntity, FactorioRecipe, Pos, Position, Rect};
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
        // Ground a cell would run one load on, not ground a cell could stand
        // on: a site whose drill would go dry inside a load is not one a
        // `Producing` goal will ever want, so a hand-smelt may have it.
        fit(state, &drill, facing, ore, load_ore(state, ore)).is_some()
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
    let ore = rate_cell_ore(&spec);
    for _ in 0..CELL_SITES_RESERVED {
        let Ok(cell) = plan_cell(&trial, from, &spec, ore) else {
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
///    restatement of it;
/// 5. the tiles under the drill hold at least `ore` between them
///    ([`cell_yield`]). A drill mines what is under it and nothing else, and
///    the tile of a patch nearest a bot is its thin edge: on seed `31337`
///    the iron patch's rim holds 3–10 ore a tile against 200+ three tiles
///    in. The live run `run-1788552801-73005` sited a cell on that rim,
///    fuelled it for 66 ore, and got `take 64 iron-plate ... removed 40`
///    with the drill reporting no mining target and coal still in it;
///    its neighbours stopped at 36 and 37. The plan had sized the take
///    from the coal and never asked the ground.
fn fit(
    state: &PlanState,
    drill: &Position,
    facing: Direction,
    ore: &str,
    want: u32,
) -> Option<Cell> {
    let area = state.collision_area_facing(DRILL, drill, facing)?;
    if !state.covers_resource(&area, ore) {
        return None;
    }
    if state.covers_claimed_resource(&area) {
        return None;
    }
    if cell_yield(state, drill, facing, ore) < want {
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

/// Ore a drill standing at `drill` facing `facing` can still reach: the sum of
/// what is left on every tile under its footprint.
///
/// **The mining area is taken to be the footprint.** A burner mining drill's
/// `resource_searching_radius` is 0.99 in vanilla -- exactly the 2x2 it stands
/// on -- and the prototype capture this crate reads carries no such field, so
/// the footprint is an assumption stated here rather than a number read from
/// the world. An electric drill (radius 2.49, a 5x5 area) would be
/// under-counted; no stage-1 cell uses one.
///
/// `resource_available`, not the tile's reported amount: what this plan's own
/// mining and its earlier cells' takes have already spoken for is gone. The
/// reported amount itself is what the mod sent when the chunk was written
/// out, refreshed only when the entity is delivered again -- a drill from an
/// earlier plan may have eaten some of it since, and nothing here can tell.
/// That is the one way this can still over-count, and it is why a site is
/// asked for more than the take (see [`site_ore`]).
pub fn cell_yield(state: &PlanState, drill: &Position, facing: Direction, ore: &str) -> u32 {
    let Some(area) = state.collision_area_facing(DRILL, drill, facing) else {
        return 0;
    };
    footprint_tiles(&area)
        .iter()
        .map(|tile| state.resource_available(&Position::from(tile), ore))
        .fold(0u32, u32::saturating_add)
}

/// The tiles a collision box covers, the way `PlanState` counts them: a box
/// that merely touches a tile edge (within 1/512) does not cover that tile.
/// Restated here because `PlanState::tiles_under` is private to `state.rs`.
fn footprint_tiles(area: &Rect) -> Vec<Pos> {
    const SLACK: f64 = 1. / 512.;
    let x0 = (area.left_top.x() + SLACK).floor() as i32;
    let x1 = (area.right_bottom.x() - SLACK).floor() as i32;
    let y0 = (area.left_top.y() + SLACK).floor() as i32;
    let y1 = (area.right_bottom.y() - SLACK).floor() as i32;
    let mut out = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            out.push(Pos(x, y));
        }
    }
    out
}

/// Ore the drill has to dig for `items` of the cell's product.
///
/// `cell_spec` admits only a recipe taking one of its ore per run, so this is
/// `items` over the recipe's yield per run -- one to one in vanilla.
fn ore_for(spec: &CellSpec, items: u32) -> u32 {
    items.div_ceil(output_per_craft(&spec.recipe, &spec.item).max(1))
}

/// Ore one load of coal ([`CELL_FUELLED_TICKS`]) lets a cell dig.
///
/// The ground a *rate* cell is sited on, and the floor for a one-shot cell's
/// site too: a `Producing` goal refuels for ever, so the least a site has to
/// hold is one load's worth, and `cell_room_to_spare` reserves sites on that
/// definition. Vanilla iron: 150.
fn rate_cell_ore(spec: &CellSpec) -> u32 {
    let cycles = CELL_FUELLED_TICKS / spec.ticks_per_item.max(1);
    ore_for(spec, cycles)
}

/// [`rate_cell_ore`] for whatever cell makes `item`, or zero when nothing
/// does -- for callers that hold an ore name rather than a spec.
fn load_ore(state: &PlanState, ore: &str) -> u32 {
    let item = state
        .base()
        .recipes
        .iter()
        .find(|entry| {
            entry.value().category == SMELTING_CATEGORY
                && matches!(ingredients_of(entry.value()).as_slice(), [(i, 1)] if i == ore)
        })
        .map(|entry| entry.key().clone());
    item.and_then(|item| cell_spec(state, &item))
        .map(|spec| rate_cell_ore(&spec))
        .unwrap_or(0)
}

/// Ground a one-shot cell for `items` is sited on: the ore it will take plus
/// one cycle of headroom and an eighth again, and never less than a load.
///
/// The eighth is the allowance for what [`cell_yield`] cannot see -- a tile's
/// reported amount is as old as the last time its chunk was delivered. The
/// load floor is what makes a cell worth draining later: a site that covers
/// exactly this fragment is a site the next fragment finds empty.
fn site_ore(spec: &CellSpec, items: u32) -> u32 {
    let ore = ore_for(spec, items.saturating_add(1));
    ore.saturating_add(ore / 8).max(rate_cell_ore(spec))
}

// ---------------------------------------------------------------------------
// Siting
// ---------------------------------------------------------------------------

/// Find somewhere to put one cell for `ore` within reach of `from` whose drill
/// can reach at least `want` ore, or say why not.
///
/// The ring walks outward from the tile nearest `from`, so the answer is the
/// **nearest site that covers `want`**, not the richest: a 2x2 on the rim
/// with 40 ore under it is skipped for one three tiles in with 900, and a
/// site is never traded for a farther, richer one once it covers what was
/// asked. `NoRoomForCell` now also means "nothing within the radius holds
/// that much", which the error does not distinguish; a caller reading it
/// should know both refusals share the name.
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
    want: u32,
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
                    if let Some(cell) = fit(state, &candidate, facing, &spec.ore, want) {
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
    want: u32,
) -> Result<Vec<Cell>, PlannerError> {
    let mut trial = state.fork();
    let mut out = Vec::new();
    for _ in 0..count {
        let cell = plan_cell(&trial, from, spec, want)?;
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

/// One burner to load: which machine, where it stands, how much coal, how
/// long a coal lasts in it, and -- when the caller wants the plan to say so
/// -- what it makes with that coal before it stops.
#[derive(Clone, Copy)]
struct Burner<'a> {
    machine: &'a str,
    position: &'a Position,
    coal: u32,
    burn_ticks: Ticks,
    /// `(product, ticks per product)`, or `None` to leave the label plain.
    runs_out: Option<(&'a str, Ticks)>,
}

/// Load `coal` into one machine's fuel slot, in as many visits as that slot
/// allows, and return the ids in order.
///
/// **A burner's fuel inventory is one slot holding exactly one stack**, so a
/// bill sized from how long the job runs is a bill the machine cannot accept:
/// `fuel the burner-mining-drill with 113 coal` puts 50 in and 63 nowhere.
/// [`crate::method::have::fuel_visits`] divides it; this emits one `Insert`
/// per visit and chains visit `j + 1` behind visit `j` by how long visit `j`
/// burns, which is when the slot next has room.
///
/// **The first id is the one a caller links production to.** The machine
/// starts when it is first fuelled and runs across the refuels — they keep it
/// running rather than starting it — so a take that waited on the last visit
/// would be waiting for a stack of coal that has not been *burned* yet. The
/// refuel visits are bot errands on the critical path of nothing.
///
/// # The last visit's label says when the machine stops
///
/// A burner runs exactly as long as the coal in it and then it stops, and a
/// plan that does not say so reads as though the machine keeps going. Ten
/// drills on `run-1788640611-64852` were each fuelled once — 6, 6, 6, 7, 8,
/// 8, 8, 8, 10 and 11 coal — and each produced exactly what that load buys
/// (39, 39, 39, 46, 53, 53, 53, 53, 66 and 73 ore) before ending `no_fuel`.
/// That was the plan working as designed, but nothing in the plan, the
/// record or the action list said a drill was expected to deliver 53 ore and
/// stop, so the run's `no_fuel` statuses read as a fault. `runs_out` names
/// the product and the cell's ticks per item; the final visit's label then
/// carries the burn this machine has been bought and what it makes in it.
fn fuel_steps(
    ctx: &mut ExpansionCtx,
    burner: &Burner<'_>,
    reach: f64,
    extra_pre: &[Condition],
) -> (Vec<Step>, Vec<crate::ids::ActionId>) {
    let &Burner {
        machine,
        position,
        coal,
        burn_ticks,
        runs_out,
    } = burner;
    let visits = crate::method::have::fuel_visits(&ctx.state, "coal", coal);
    let burn = coal.saturating_mul(burn_ticks);
    let mut steps: Vec<Step> = Vec::with_capacity(visits.len() * 2);
    let mut ids: Vec<crate::ids::ActionId> = Vec::with_capacity(visits.len());
    let last = visits.len().saturating_sub(1);
    for (visit, &load) in visits.iter().enumerate() {
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
                name: machine.into(),
            },
            Condition::HasItem {
                who: Actor::Role,
                item: "coal".into(),
                count: load,
            },
        ];
        pre.extend(extra_pre.iter().cloned());
        steps.push(Step::Act(Box::new(Action {
            id,
            kind: ActionKind::Insert {
                pos: position.clone(),
                entity: machine.into(),
                slot: InventorySlot::Fuel,
                item: "coal".into(),
                count: load,
            },
            pre,
            eff: vec![Effect::LoseItem {
                who: Actor::Role,
                item: "coal".into(),
                count: load,
            }],
            duration: TRANSFER_TICKS,
            pinned: None,
            label: match runs_out {
                Some((product, ticks_per_item)) if visit == last && ticks_per_item > 0 => {
                    format!(
                        "fuel the {} with {} coal ({} ticks, {} {}, then it stops)",
                        machine,
                        load,
                        burn,
                        burn / ticks_per_item,
                        product,
                    )
                }
                _ => format!("fuel the {} with {} coal", machine, load),
            },
        })));
        ids.push(id);
    }
    for (pair, &load) in ids.windows(2).zip(visits.iter()) {
        steps.push(Step::Link {
            from: pair[0],
            to: pair[1],
            lag: load.saturating_mul(burn_ticks),
        });
    }
    (steps, ids)
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

    let reach = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.reach_distance)
        .unwrap_or(10.0);

    for cell in cells {
        place_steps(ctx, spec, cell, &mut steps);
        let fuel_ids = fuel_both(
            ctx,
            spec,
            cell,
            (drill_coal, furnace_coal),
            &research_pre,
            reach,
            &mut steps,
        );
        // A rate cell promises nothing when it is stood; it is fuelled for a
        // load and refuelled for ever, and its output is a rate a
        // `supervisor.witness` reads. It is still entered in the ledger, timed
        // from its drill's first fuel visit, so a later one-shot fragment can
        // draw on it -- see `cell_ledger`.
        if let Some(started_by) = fuel_ids.first().copied() {
            promise(ctx, spec, cell, started_by, 0);
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
        let cells = plan_cells(&ctx.state, &from, &spec, build, rate_cell_ore(&spec))?;
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
        return raw_ticks(state, item, count);
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

/// Ticks a bot spends **with its hands** making `count` of `item`: every
/// crafting-category recipe on the way down, and nothing for the ore or the
/// smelting. Recurses exactly as [`craft_ticks`] does and shares its `depth`
/// backstop and its "unpriceable is never free" rule; it differs only in what
/// a raw material and a furnace recipe cost, which here is zero.
///
/// **This is the price a deal balances, and [`craft_ticks`] is not.** A
/// plate's ore and smelt are real work, but in a fleet plan they are a
/// cell's work or a furnace's lag, not the bot's timeline the deal is
/// balancing -- and from raw they dominate: a lab is 17,232 ticks from raw
/// and about 1,300 in the hands, a pack 1,266 from raw and 330 in the hands.
/// Priced from raw, a lab was "worth" thirteen packs and
/// `have::deal_by_load` gave the two lab builders 20 and 21 of 75 packs and
/// the third bot 34, whose chain then waited 7,166 ticks on its drill for
/// the plates those packs need and crafted alone until tick 55,554 while
/// the builders had finished by 49,427 (`workspace/scripts/map.json`, green,
/// four bots). In the hands a lab is four packs, and the deal comes out
/// where the timelines do.
///
/// `craft_ticks` keeps its own callers: [`cell_setup_bot_ticks`] compares a
/// drill against hand-mining, where the ore *is* the bot's time, and
/// `have::labs_worth_building` deliberately over-prices a lab to err toward
/// building fewer.
pub(crate) fn hand_ticks(state: &PlanState, item: &str, count: u32, depth: u32) -> Ticks {
    if count == 0 || state.has_resource_patches(item) {
        return 0;
    }
    let Some(recipe) = recipe_for(state, item) else {
        return 0;
    };
    if depth == 0 {
        return Ticks::MAX / 4;
    }
    let per = output_per_craft(&recipe, item).max(1);
    let runs = count.div_ceil(per);
    let own = match recipe.category.as_str() {
        SMELTING_CATEGORY => 0,
        CRAFTING_CATEGORY => recipe_ticks(&recipe).saturating_mul(runs),
        _ => return Ticks::MAX / 4,
    };
    ingredients_of(&recipe)
        .into_iter()
        .fold(own, |sum, (ingredient, amount)| {
            sum.saturating_add(hand_ticks(
                state,
                &ingredient,
                amount.saturating_mul(runs),
                depth - 1,
            ))
        })
}

/// Character ticks to get `count` of a raw resource the cheaper of the two
/// ways a plan has: off a tile by hand, or off a rock.
///
/// `crate::method::have::Chop` sits ahead of `Mine` in the registry and takes
/// the rock whenever its `chop_beats_mining` says the swings pay, so a price
/// that only knew the tile charged a cell's coal at 120 ticks apiece when the
/// plan pays 15 -- one swing at a `huge-rock` is 360 ticks for 24 coal *and*
/// 24 stone -- and the gate refused cells the plan could afford. The two
/// world-record replays read on 2026-09-04 take every gram of early stone and
/// coal from rocks; this is the price at which they do.
///
/// The rule is `chop_beats_mining`'s own, restated rather than shared because
/// that function answers yes-or-no and this one needs the number: the
/// **worst** ticks-per-item deal among the standing sources, taken only when
/// they can supply `count` at all, and never more than the tile costs. Like
/// it, this reads no distance -- a rock across the map is priced like one
/// beside the bot -- which flatters the rock in exactly the way `Chop`'s own
/// choice already does.
fn raw_ticks(state: &PlanState, item: &str, count: u32) -> Ticks {
    let hand = mining_ticks(state, item).saturating_mul(count);
    let mut supply: u32 = 0;
    let mut worst: Option<(Ticks, u32)> = None;
    for (entity, _position, yields) in state.minable_sources(item) {
        if yields == 0 {
            continue;
        }
        supply = supply.saturating_add(yields);
        let candidate = (mining_ticks(state, &entity), yields);
        let worse = match worst {
            None => true,
            Some(best) => {
                u64::from(candidate.0) * u64::from(best.1)
                    > u64::from(best.0) * u64::from(candidate.1)
            }
        };
        if worse {
            worst = Some(candidate);
        }
    }
    let Some((swing, yields)) = worst else {
        return hand;
    };
    if supply < count {
        return hand;
    }
    hand.min(swing.saturating_mul(count.div_ceil(yields)))
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
    raw_ticks(state, &spec.ore, runs)
        .saturating_add(raw_ticks(state, "coal", coal))
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
///
/// # Priced from raw even for a drill the holder already carries, and that
/// # is load-bearing -- measured 2026-09-05
///
/// A freeplay character starts holding one burner drill and one stone
/// furnace, and this prices both as nine plates mined and smelted from
/// nothing. That is a lie about the bill -- the true fixed cost of a starter
/// cell is two placements, three transfers and the coal -- and it was tried
/// as a fix: price each of [`bill`]'s items net of `PlanState::available`
/// for the goal's holder. Every starter drill then went on ore in the first
/// minute, as the brief wanted, and the plans got worse on two of three
/// goals over the reference dump with four bots:
///
/// | goal | as it is | priced net of stock |
/// | --- | ---: | ---: |
/// | `researched:automation` | 25,886 | 32,693 (utilisation 42% -> 28%) |
/// | `producing:automation-science-pack:6` | 39,118 | 35,351 |
/// | `producing:logistic-science-pack:6` | 97,232 | **127,801** |
///
/// The mechanism is not the price. This gate compares *bot-busy* ticks, and
/// a cell wins that comparison for any share once its drill is free -- but a
/// burner drill is 240 ticks a plate against 120 by hand, so a share served
/// by one drill finishes *later* unless the bot has other work to fill the
/// wait. On `researched:automation` it has none: every bot's share went on
/// its own drill and every bot stood idle beside it. On green the wait moved
/// bot 4's 78-plate share and, through the hand-smelt furnace it shares as a
/// slot (`crate::method::have::Smelt`), bot 1's whole cell ladder behind it.
/// The wrong price was keeping single-drill cells off the critical path by
/// accident. What would make the true price safe is the count rung --
/// enough drills per share that the cell is faster than hands on the clock,
/// which the world-record replays reach with forty of them -- and until that
/// exists this stays as it is, deliberately, with the number that says why.
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

    /// Claimed for **any** count once this plan has a live cell for the item
    /// -- see [`cell_ledger`] for what "live" is and [`Drain`] for how the
    /// count is then served. Only a goal with no live cell behind it is priced,
    /// and that comparison is what it costs to *open* the first cell against
    /// hand-smelting the goal.
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
        let drain = Drain::new(state, &spec);
        // A structural refusal -- no ore patch reachable, no room for the
        // pair -- is `expand`'s business, exactly as `BuildCell` leaves it to
        // its own `expand`: answering it here would pay for the same siting
        // search twice.
        !drain.eligible.is_empty()
            || cell_setup_bot_ticks(state, &spec, need) < hand_smelt_bot_ticks(state, &spec, need)
    }

    /// Serve the count from the plan's live cells, open another while they
    /// are all backlogged, and price only what no cell can serve.
    ///
    /// In order:
    ///
    /// 1. every live cell under the drain bound is drawn on, least backlog
    ///    first, up to what is left under its drill ([`drain_steps`]);
    /// 2. if that did not cover the count and the remainder pays for a cell
    ///    of its own ([`cell_setup_bot_ticks`] against
    ///    [`hand_smelt_bot_ticks`]), a fresh cell is opened for it
    ///    ([`open_cell_steps`]). A patch with no site
    ///    left is not a failure here: the backlogged cells are drawn on
    ///    instead, bound or no bound -- a wait is still cheaper in bot time
    ///    than a hand -- and with nothing live to draw on the remainder is
    ///    hand-smelted in place through `Smelt`, because re-stating the goal
    ///    would only bring it back here to the same refusal;
    /// 3. whatever is still uncovered goes back out as the same goal for the
    ///    methods behind this one. `Withdraw`'s construction: the takes'
    ///    effects have landed by the time the subgoal is expanded, so a `Have`
    ///    is re-stated with its own count and `shortfall` recomputes, while a
    ///    `Produced` -- which ignores inventory by design -- carries the
    ///    remainder and the unlock. It terminates because every pass either
    ///    empties the cells it drew on or is refused by `applicable`, which
    ///    reads the same [`Drain`].
    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Some(Demand {
            item,
            need,
            whose,
            unlocks,
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

        let reach = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.reach_distance)
            .unwrap_or(10.0);
        let per = output_per_craft(&spec.recipe, &spec.item).max(1);

        let drain = Drain::new(&ctx.state, &spec);
        let mut left = need;
        let mut last_take: Option<ActionId> = None;
        let mut drained: BTreeSet<Pos> = BTreeSet::new();
        for cell in &drain.eligible {
            if left == 0 {
                break;
            }
            let count = cell.room.saturating_mul(per).min(left);
            let (drawn, last) = drain_steps(ctx, &spec, cell, count, &research_pre, reach);
            steps.extend(drawn);
            last_take = last;
            drained.insert(Pos::from(&cell.cell.drill));
            left = left.saturating_sub(count);
        }

        if left > 0 {
            let pays = cell_setup_bot_ticks(&ctx.state, &spec, left)
                < hand_smelt_bot_ticks(&ctx.state, &spec, left);
            if pays {
                match open_cell_steps(ctx, &spec, left, &research_pre, reach) {
                    Ok((built, last)) => {
                        steps.extend(built);
                        last_take = last;
                        left = 0;
                    }
                    Err(
                        PlannerError::NoRoomForCell { .. } | PlannerError::NoPatchForCell { .. },
                    ) => {
                        // No site for another cell. The backlogged cells
                        // still hold ore, and a wait costs no bot time.
                        for cell in &drain.live {
                            if left == 0 || drained.contains(&Pos::from(&cell.cell.drill)) {
                                continue;
                            }
                            let count = cell.room.saturating_mul(per).min(left);
                            let (drawn, last) =
                                drain_steps(ctx, &spec, cell, count, &research_pre, reach);
                            steps.extend(drawn);
                            last_take = last;
                            left = left.saturating_sub(count);
                        }
                        if left > 0 {
                            // Nothing live to draw on either. `applicable`
                            // would say yes again to the same goal for the
                            // same reason, so the hand path is taken here
                            // rather than re-stated. The count is stated so
                            // that `Smelt` sees `left`: the takes above have
                            // not landed yet, and a `Have`'s shortfall is
                            // against what is held.
                            let hand = match goal {
                                Goal::Produced { .. } => Goal::Produced {
                                    item: item.clone(),
                                    count: left,
                                    whose: whose.clone(),
                                    unlocks: unlocks.map(str::to_owned),
                                },
                                Goal::Have { count, .. } => Goal::Have {
                                    item: item.clone(),
                                    count: count.saturating_sub(need.saturating_sub(left)),
                                    whose: whose.clone(),
                                },
                                _ => unreachable!("`demand` admits only Have and Produced"),
                            };
                            steps.extend(crate::method::have::Smelt.expand(&hand, ctx)?);
                            return Ok(steps);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        if left > 0 {
            steps.push(Step::Subgoal(match goal {
                Goal::Produced { .. } => Goal::Produced {
                    item: item.clone(),
                    count: left,
                    whose: whose.clone(),
                    unlocks: unlocks.map(str::to_owned),
                },
                _ => Goal::Have {
                    item: item.clone(),
                    count: match goal {
                        Goal::Have { count, .. } => *count,
                        _ => left,
                    },
                    whose: whose.clone(),
                },
            }));
            return Ok(steps);
        }

        // The unlock rides on the *last* take, because that is the one that
        // completes `need`; `attach_unlock` takes the first producing action
        // it finds, so it is handed only the tail of the step list.
        let last_take_at = steps
            .iter()
            .rposition(|s| matches!(s, Step::Act(a) if Some(a.id) == last_take))
            .unwrap_or(0);
        attach_unlock(&mut steps[last_take_at..], item, unlocks);
        Ok(steps)
    }
}

/// One cell this plan built and may still draw on.
#[derive(Clone, Debug)]
struct LiveCell {
    cell: Cell,
    /// Ore left under the drill after everything this plan has already
    /// promised out of it -- [`cell_yield`] read after the earlier takes'
    /// own `Effect::ConsumeResource` landed.
    room: u32,
    /// Ticks of production this plan has already promised out of the cell.
    queued: Ticks,
    /// The action the cell's production is timed from: the first fuel visit
    /// to its drill, from `PlanState::machine_queue`. `None` for a cell
    /// standing from an earlier plan, which this plan has no action for --
    /// its production is timed from the top-up this plan brings it instead.
    started_by: Option<ActionId>,
    /// Coal already in the drill's and the furnace's fuel slots, as last
    /// read (`PlanState::fuelled`); zero when nobody asked. A top-up is
    /// sized net of this.
    drill_fuel: u32,
    furnace_fuel: u32,
}

/// Cells this plan has stood for `spec` and can still draw on, least backlog
/// first and then by `(x, y)` of the drill.
///
/// # This plan's cells, and the ones an earlier plan left standing
///
/// A cell is known to be this plan's by the queue entry [`promise`] leaves on
/// its furnace (`PlanState::queue_machine`): nothing else queues a furnace a
/// drill feeds -- `adoptable_furnaces` refuses fed furnaces before it ever
/// reads the queue -- so the entry is both the bookkeeping and the mark. Both
/// `PlaceDrill`'s one-shot cells and `BuildCell`'s rate cells are here: a
/// rate cell promises nothing when it is stood, and a later fragment draws
/// on it exactly as on a one-shot cell.
///
/// A cell standing from an **earlier plan** -- a drill on the ore feeding a
/// furnace, with no queue entry at all -- is here too, since 2026-09-05, with
/// `started_by: None` and nothing queued. It used to be left out on the
/// grounds that its output is timed from a fuelling this plan cannot name an
/// action for. Measured on `run-1788552801-73005`, that timing did not
/// exist to be missed: every one of the run's five drills had burned its
/// ten coal (~16,000 ticks) and stood `no_fuel` at every replan that
/// followed, over 77 and 166 ore, while plans 5 and 6 each hand-mined 44 and
/// 30 iron ore and stood *another* cell. What a standing cell has now is a
/// furnace slot `Withdraw` already empties, and what it can do next is
/// exactly what [`drain_steps`] does to a live cell: bring coal, take
/// plates. So it is offered as one, timed from this plan's own top-up, and
/// its slots' coal (`PlanState::fuelled`, when the game was asked) is
/// credited against the top-up rather than counted on as a rate: a machine
/// that is still running only makes the take early, and the top-up is what
/// makes the take *certain*.
///
/// Its feed is the ground under the drill and nothing else -- a cell has no
/// chests, so the "never drain an input" rule the stage-2 chest list keeps
/// (`BUFFER_ENTITIES`) has nothing here to protect.
///
/// # Not before its own placement has been simulated
///
/// A cell is promised at expand time -- its machines are written into the
/// overlay and its furnace queued so that nothing else is sited or smelted
/// there -- but its **bill** (the drill's nine plates, its furnace's stone)
/// expands afterwards, as subgoals, and those plates are a `Have` of the very
/// item the cell makes. Offered the cell then, they would drain it: a take
/// whose furnace is placed by a drill crafted from the take's own plates,
/// which the scheduler rejects as `ChainOwnerInfeasible` (measured, not
/// imagined: `has 6 iron-plate` on the fixture's fifty-plate plan). So a cell
/// is live only once a tile under its drill is **claimed** -- the mark the
/// drill's placement leaves when it is simulated ([`place_steps`]), which is
/// after the bill, and a mark nothing else can have left there because
/// [`fit`] never sites a drill on a claimed tile.
///
/// # Bounded by ground
///
/// `room` is what is left under the drill, so a cell whose ore this plan has
/// already spoken for is not offered again -- that is the exhaustion the live
/// run hit (`take 64 ... removed 40`), moved from the game into the model.
fn cell_ledger(state: &PlanState, spec: &CellSpec) -> Vec<LiveCell> {
    let mut out: Vec<LiveCell> = Vec::new();
    for (drill, facing, furnace) in drill_fed_pairs(state, spec) {
        let Some(area) = state.collision_area_facing(DRILL, &drill.position, facing) else {
            continue;
        };
        let claimed = footprint_tiles(&area)
            .iter()
            .any(|tile| state.is_resource_claimed(&Position::from(tile)));
        let (queued, started_by) = match state.machine_queue(&furnace) {
            Some(queue) => {
                if queue.item != spec.item {
                    continue;
                }
                // This plan's cell, live only once its placement has been
                // simulated -- the claim is the mark; see above.
                if !claimed {
                    continue;
                }
                (queue.queued, Some(queue.release))
            }
            None => {
                // Standing from an earlier plan. A claim under it with no
                // queue on its furnace is a hand's, and a tile a hand has
                // spoken for is not offered twice; a furnace a hand-smelt
                // has committed is that smelt's.
                if claimed || state.machine_committed(&furnace) {
                    continue;
                }
                if !state.covers_resource(&area, &spec.ore) {
                    continue;
                }
                (0, None)
            }
        };
        let room = cell_yield(state, &drill.position, facing, &spec.ore);
        if room == 0 {
            continue;
        }
        out.push(LiveCell {
            cell: Cell {
                drill: drill.position.clone(),
                facing,
                furnace: furnace.clone(),
            },
            room,
            queued,
            started_by,
            drill_fuel: state.fuelled(&drill.position, "coal"),
            furnace_fuel: state.fuelled(&furnace, "coal"),
        });
    }
    out.sort_by(|a, b| {
        a.queued
            .cmp(&b.queued)
            .then(a.cell.drill.x.total_cmp(&b.cell.drill.x))
            .then(a.cell.drill.y.total_cmp(&b.cell.drill.y))
    });
    out
}

/// How freely a fragment may wait on a cell this plan already stood.
///
/// # Why this is a choice at all, and why it cannot be made here
///
/// [`Drain`]'s bound weighs a *wall-clock* wait against a *bot-time* cost, and
/// which of those is scarce is a property of the whole plan, not of the
/// fragment asking. Measured over `workspace/scripts/map.json` on 2026-09-05,
/// raising the bound moves a goal's makespan in whichever direction its
/// roster has slack, and the baseline plan's own utilisation predicts the
/// sign every time:
///
/// | goal, roster | utilisation | conservative | parallel |
/// | --- | ---: | ---: | ---: |
/// | `researched:automation`, 4 | 39.6% | **21,776** | 21,776 (one cell, unchanged) |
/// | `producing:automation-science-pack:6`, 4 | 63.3% | **26,990** | 26,990 (one cell, unchanged) |
/// | `producing:logistic-science-pack:6`, 4 | 72.0% | 52,819 | **49,051** |
/// | `producing:logistic-science-pack:6`, 8 | 51.8% | **49,229** | 55,093 |
///
/// A goal with an idle roster is short of wall time, so a fragment that waits
/// on a cell lengthens the plan; a goal with a busy one is short of bot time,
/// so a fragment that hand-mines lengthens it. **No constant serves both**,
/// and a flat multiplier tried on the same sweep bought green's four-bot
/// number by giving up 5,927 ticks on `researched:automation` -- a goal whose
/// plan has since been measured live at 6:05 with the execution 147 ticks
/// over it. That is a measured record to give up, not a stale one.
///
/// So the policy is not decided here. [`crate::plan_best`] builds the plan
/// under each policy and keeps the shorter schedule, which is the only
/// arbiter that knows the utilisation -- it is reading a finished schedule
/// rather than guessing at expansion time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum DrainPolicy {
    /// A fragment waits at most one cell-build's worth of backlog. What every
    /// plan did before 2026-09-05, and still the right answer whenever the
    /// roster has slack.
    #[default]
    Conservative,
    /// A fragment waits up to one cell-build's worth of backlog **per cell
    /// already standing**. A plan that has stood six cells is a plan whose
    /// roster is busy -- the count is the plan's own signal that bot time,
    /// not latency, is the scarce thing -- so waiting on the least-loaded of
    /// the six beats mining a fragment's ore by hand and paying a seventh
    /// drill's nine plates for it.
    Parallel,
}

/// What the plan's live cells can do for a fragment.
///
/// # The count lever, and what a fragment is allowed to see
///
/// Two world-record replays read on 2026-09-04 put their first six minutes
/// into 40–70 burner drills and let everything after draw on them; our best
/// run had none. A method sees one fragment of a goal at a time -- green's
/// plates arrive four and six at a time -- so *count* cannot be decided here
/// from demand. What a fragment can see is each live cell's **backlog**, and
/// that decides one thing: a cell with more queued than a cell takes to
/// stand ([`bound`](Self::bound)) is a cell this fragment would wait on
/// longer than a hand would take, so it is not offered, and the fragment
/// goes by hand unless it pays for a cell of its own.
///
/// # The cap that used to override the bound, and why it is gone
///
/// Until 2026-09-05, once the plan had stood as many cells as match the
/// roster's hand-mining rate (two a bot -- eight on four bots) every cell
/// was offered *whatever its backlog*, on the argument that past that
/// point the hands are the slower supply. Two things were wrong with it,
/// and the first hid the second. The count took every furnace near the
/// patch with a queue for the item, and a hand-smelt's furnace carries the
/// same `queue_machine` entry -- so on `producing:logistic-science-pack:6`
/// it read 8 with **five** drills standing and three hand-smelt furnaces,
/// and from then on every fragment of bot 1's -- the drills' own three-,
/// six- and twelve-plate bills among them -- was queued behind cells
/// carrying 8,640 to 30,000 ticks: `take 12 iron-plate` waited 21,830 ticks
/// behind bot 4's 78 science plates for what a hand makes in 1,500, and the
/// drill it was for stood at tick 79,000. That is the serial ladder of
/// cells the run record showed. Counting only drill-fed furnaces, the cap
/// still trips honestly at eight on green and does the same thing a little
/// later: 143,745 ticks with it, **97,232** without, on
/// `workspace/scripts/map.json` over four bots (the solo rung-1 fixture
/// moves 31,482 -> 29,260 the same way). The argument was about
/// throughput; a fragment asks about latency, and a cell past the bound is
/// a wait a hand beats however many cells stand. The one place a
/// backlogged cell is still drawn on is `PlaceDrill::expand`'s no-room
/// path, where nothing else can serve.
///
/// **Opening a cell on that backlog was measured and rejected.** With "open
/// another while every live cell is past the bound, up to two cells a bot",
/// `producing:automation-science-pack:6` on `workspace/scripts/map.json` went
/// from 43,871 ticks to 78,240 with eight drills: each cell's nine-plate bill
/// was hand-smelted, serially, by the one bot that owns every chain, and each
/// take then waited its turn behind the cell's queue. A cell is cheap in bot
/// time and slow in wall time -- 240 ticks a plate against four hands at 120
/// -- and a plan whose roster is 15–35 % busy is short of wall time, not bot
/// time. Count ahead of demand is a *ladder* decision (`producing:iron-plate`
/// before the goal that needs the plates): `BuildCell` stands the cells and
/// every fragment after them drains, which is what [`cell_ledger`] admitting
/// rate cells is for.
struct Drain {
    /// Every live cell, least backlog first.
    live: Vec<LiveCell>,
    /// The live cells a fragment may draw on now: all of them once the
    /// roster is matched, otherwise those under the bound.
    eligible: Vec<LiveCell>,
}

impl Drain {
    fn new(state: &PlanState, spec: &CellSpec) -> Self {
        let live = cell_ledger(state, spec);
        let bound = Self::bound(state, spec, live.len());
        // Under the bound, and only under it -- see the type's doc for the
        // cap this replaced.
        let eligible: Vec<LiveCell> = live.iter().filter(|c| c.queued < bound).cloned().collect();
        Drain { live, eligible }
    }

    /// Backlog past which a fragment would wait longer than a cell takes to
    /// stand: [`cell_setup_bot_ticks`] for one item, which is the cell's
    /// fixed cost with next to no coal in it. Vanilla, with rocks to hand:
    /// about 4,000 ticks, sixteen plates.
    ///
    /// Under [`DrainPolicy::Parallel`] that fixed cost is multiplied by the
    /// number of cells already standing, so a plan that has stood six of them
    /// lets a fragment wait six cell-builds' worth of backlog rather than
    /// one. See [`DrainPolicy`] for why the choice cannot be made here.
    fn bound(state: &PlanState, spec: &CellSpec, live: usize) -> Ticks {
        let fixed = cell_setup_bot_ticks(state, spec, 1);
        match state.drain_policy() {
            DrainPolicy::Conservative => fixed,
            DrainPolicy::Parallel => {
                fixed.saturating_mul(u32::try_from(live.max(1)).unwrap_or(u32::MAX))
            }
        }
    }
}

/// Every burner drill near a patch of the cell's ore that delivers into a
/// stone furnace: `(drill, facing, furnace)`, each drill once, in the order
/// the patches and the entity query return them.
///
/// Extracted from [`cell_ledger`] on 2026-09-05 for a cell *count* that
/// has since been deleted with the cap it served (see [`Drain`]); kept as
/// the one place the pair is defined. It does **not** ask whether the drill
/// stands on ore -- the ledger asks that of each pair itself.
fn drill_fed_pairs(
    state: &PlanState,
    spec: &CellSpec,
) -> Vec<(FactorioEntity, Direction, Position)> {
    let mut seen: BTreeSet<Pos> = BTreeSet::new();
    let mut out = Vec::new();
    for patch in state.resource_patches(&spec.ore) {
        let centre = Position::new(
            (patch.rect.left_top.x() + patch.rect.right_bottom.x()) / 2.,
            (patch.rect.left_top.y() + patch.rect.right_bottom.y()) / 2.,
        );
        let reach = (patch.rect.width() / 2.).hypot(patch.rect.height() / 2.)
            + f64::from(CELL_SEARCH_RADIUS)
            + CELL_PAIR_RADIUS;
        for drill in state.entities_within(&centre, reach) {
            if drill.name != DRILL || !seen.insert(Pos::from(&drill.position)) {
                continue;
            }
            let Some(facing) = Direction::from_u8(drill.direction) else {
                continue;
            };
            let Some(furnace) = state
                .entities_within(&drill.position, CELL_PAIR_RADIUS)
                .into_iter()
                .find(|target| {
                    target.name == FURNACE && state.delivers_into(&drill.position, &target.position)
                })
            else {
                continue;
            };
            out.push((drill, facing, furnace.position.clone()));
        }
    }
    out
}

/// Record on the furnace what this plan has now promised out of `cell`:
/// `count` more items, timed from `started_by`, the first fuel visit to the
/// cell's drill.
///
/// `PlanState::queue_machine` is the ledger: its `item` names the product,
/// its `release` the action the machine's work is timed from, its `queued`
/// the ticks of work waiting in it -- the same three things a hand-smelt
/// records when it loads a furnace, read with the same meanings. A drill-fed
/// furnace is never a candidate for a hand-smelt (`adoptable_furnaces`
/// filters fed furnaces before it reads the queue), so the entry is seen by
/// [`cell_ledger`] and by nothing else. `started_by` is passed back unchanged
/// on every later promise, since `queue_machine` overwrites it.
fn promise(ctx: &mut ExpansionCtx, spec: &CellSpec, cell: &Cell, started_by: ActionId, count: u32) {
    ctx.state.queue_machine(
        &cell.furnace,
        &spec.item,
        started_by,
        None,
        spec.ticks_per_item.saturating_mul(count),
    );
}

/// Pull `count` of the cell's product out of its furnace, a stack at a time,
/// each take eating its ore off the tiles under the drill.
///
/// # One take per stack, not one take per goal
///
/// A stone furnace's output is a **single slot holding exactly one stack**,
/// so `take 141 iron-plate` was never physically possible -- not at plan
/// time, not at dispatch, not at any moment in between. It came back `tried
/// to remove 141 iron-plate but removed 100`, and the larger cost was the
/// 9,600 ticks *before* that: a furnace whose output slot is full reports
/// `full_output` and **stops smelting**, with a bot idle beside it and its
/// input backing up. Sizing this take from demand asks the game for something
/// a slot cannot hold; sizing it from `slot_capacity` asks for a stack at a
/// time and empties the slot often enough that the machine never stalls.
///
/// Each take's lag is `already` plus the time to produce everything taken
/// *so far* plus one cycle of headroom -- the same margin `smelt_steps` gives
/// its own wait, and for the same reason: a removal timed to land exactly on
/// the last item is right only if nothing about it runs long. The caller
/// links the action the cell's production is timed from to every take with
/// that take's lag.
///
/// `None` from `slot_capacity` means the world has no prototype for the item
/// (fixtures only) and is deliberately *not* a guessed cap: it falls back to
/// one take of `count`.
/// See `docs/superpowers/specs/2026-09-04-world-model-divergence-design.md`.
///
/// # The ore is spent here, on the take
///
/// Each take carries an `Effect::ConsumeResource` for the ore it stands for,
/// laid over the drill's tiles in tile order. That closes the gap
/// `PlanState::covers_resource` documents -- "a drill neither claims nor
/// consumes what it stands on" -- from this side: a later [`cell_ledger`]
/// reads the room that is left, and a later cell is never sited on ore this
/// one has spoken for. Nothing checks the effect at dispatch
/// (`Condition::ResourceAvailable` is not on a take), so a stale amount costs
/// a short take, which the executor already reports, and never a refused
/// action.
fn take_steps(
    ctx: &mut ExpansionCtx,
    spec: &CellSpec,
    cell: &Cell,
    count: u32,
    already: Ticks,
    research_pre: &[Condition],
    reach: f64,
) -> (Vec<Step>, Vec<(ActionId, Ticks)>) {
    let item = &spec.item;
    let cap = ctx
        .state
        .slot_capacity(InventorySlot::FurnaceResult, item)
        .unwrap_or(count)
        .max(1);
    let mut tiles: Vec<(Pos, u32)> = ctx
        .state
        .collision_area_facing(DRILL, &cell.drill, cell.facing)
        .map(|area| footprint_tiles(&area))
        .unwrap_or_default()
        .into_iter()
        .map(|tile| {
            let left = ctx
                .state
                .resource_available(&Position::from(&tile), &spec.ore);
            (tile, left)
        })
        .collect();
    let mut steps: Vec<Step> = Vec::new();
    let mut takes: Vec<(ActionId, Ticks)> = Vec::new();
    let mut taken = 0u32;
    loop {
        let take = cap.min(count.saturating_sub(taken));
        taken = taken.saturating_add(take);
        let id = ctx.ids.next();
        let mut pre = vec![
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
        pre.extend(research_pre.iter().cloned());
        let mut eff = vec![Effect::GainItem {
            who: Actor::Role,
            item: item.clone(),
            count: take,
        }];
        let mut ore = ore_for(spec, take);
        for (tile, left) in tiles.iter_mut() {
            if ore == 0 {
                break;
            }
            let dig = (*left).min(ore);
            if dig == 0 {
                continue;
            }
            *left -= dig;
            ore -= dig;
            eff.push(Effect::ConsumeResource {
                pos: Position::from(&*tile),
                item: spec.ore.clone(),
                count: dig,
            });
        }
        steps.push(Step::Act(Box::new(Action {
            id,
            kind: ActionKind::Remove {
                pos: cell.furnace.clone(),
                entity: FURNACE.into(),
                slot: InventorySlot::FurnaceResult,
                item: item.clone(),
                count: take,
            },
            pre,
            eff,
            duration: TRANSFER_TICKS,
            pinned: None,
            label: format!("take {} {} from the cell", take, item),
        })));
        takes.push((
            id,
            already.saturating_add(spec.ticks_per_item.saturating_mul(taken.saturating_add(1))),
        ));
        if taken >= count {
            break;
        }
    }
    // One slot, emptied in order. The staggered lags already imply it, but
    // the ordering is physical rather than a consequence of the arithmetic,
    // so it is stated: stack `n + 1` is not in the slot until stack `n` has
    // been carried away.
    for pair in takes.windows(2) {
        steps.push(Step::Link {
            from: pair[0].0,
            to: pair[1].0,
            lag: 0,
        });
    }
    (steps, takes)
}

/// Fuel `cell`'s two machines for `count` more items and take them.
///
/// A cell is fuelled for exactly what has been asked of it, one cycle over
/// (see [`open_cell_steps`]), so a further `count` is a further top-up: the
/// coal for `count` cycles in each machine, at least one apiece, brought by
/// the taking bot on the same walk. Each take is then timed twice -- from the
/// cell's start by everything promised before it plus its own share, and
/// from the top-up by its own share alone, since the drill may have stood
/// idle until the coal came -- and the scheduler holds the later of the two.
/// Both are conservative: a drill still running on its headroom makes the
/// take late, never early.
///
/// # A standing cell is topped up net of what it holds, and timed from that
///
/// For a cell from an earlier plan (`started_by: None`) there is no start to
/// time from, so the top-up's first visit to the drill *is* the start, and
/// every take is timed from it. The top-up is what the job burns less the
/// coal last read in each slot -- and never less than one apiece, because
/// the visit is the anchor the take needs and a slot that already holds a
/// stack still has room for one more. Offline, or for a machine nobody
/// asked about, the credit is zero and the cell is fuelled in full.
fn drain_steps(
    ctx: &mut ExpansionCtx,
    spec: &CellSpec,
    cell: &LiveCell,
    count: u32,
    research_pre: &[Condition],
    reach: f64,
) -> (Vec<Step>, Option<ActionId>) {
    let mut steps: Vec<Step> = Vec::new();
    if count == 0 {
        return (steps, None);
    }
    let duration = spec.ticks_per_item.saturating_mul(count);
    let drill_coal = fuel_for_duration(duration, DRILL_BURN_TICKS)
        .saturating_sub(cell.drill_fuel)
        .max(1);
    let furnace_coal = fuel_for_duration(duration, COAL_BURN_TICKS)
        .saturating_sub(cell.furnace_fuel)
        .max(1);
    steps.push(Step::Subgoal(Goal::Have {
        item: "coal".into(),
        count: drill_coal.saturating_add(furnace_coal),
        whose: Holder::Share(ctx.chain_actor),
    }));
    let fuel_ids = fuel_both(
        ctx,
        spec,
        &cell.cell,
        (drill_coal, furnace_coal),
        research_pre,
        reach,
        &mut steps,
    );
    let Some(started_by) = cell.started_by.or_else(|| fuel_ids.first().copied()) else {
        // `fuel_both` always emits a visit per machine, so this is a
        // fixture with no coal prototype at all; nothing to time from and
        // nothing to promise.
        return (steps, None);
    };
    let (takes, timed) = take_steps(
        ctx,
        spec,
        &cell.cell,
        count,
        cell.queued,
        research_pre,
        reach,
    );
    steps.extend(takes);
    for (take, lag) in &timed {
        if cell.started_by.is_some() {
            steps.push(Step::Link {
                from: started_by,
                to: *take,
                lag: *lag,
            });
        }
        for fuel in &fuel_ids {
            steps.push(Step::Link {
                from: *fuel,
                to: *take,
                lag: lag.saturating_sub(cell.queued),
            });
        }
    }
    let last = timed.last().map(|(id, _)| *id);
    if last.is_some() {
        promise(ctx, spec, &cell.cell, started_by, count);
    }
    (steps, last)
}

/// Place a cell's two machines, drill first, and write them into the overlay.
///
/// The drill first, so a furnace can never be standing where the drill has
/// to go: the two footprints are disjoint by construction
/// ([`tests::a_cells_two_machines_never_overlap`]) but the order is what
/// makes the reservation mean anything.
///
/// # The drill's placement claims its ground
///
/// The drill's `Place` carries an `Effect::ConsumeResource` of **zero** for
/// each ore tile under it. Zero, because placing a drill digs nothing; an
/// effect at all, because `PlanState::consume_resource` claims the tile on
/// the way through, and that claim is what makes the ground a *machine's*
/// rather than a hand's: `Mine`'s selectors skip it, a second cell is never
/// sited over it, and -- the reason it was added -- [`cell_ledger`] reads it
/// as "this cell's placement has been simulated", which is the one thing that
/// separates a cell that stands from a cell whose own bill is still being
/// gathered. `PlanState::covers_resource` documents the gap this closes.
fn place_steps(ctx: &mut ExpansionCtx, spec: &CellSpec, cell: &Cell, steps: &mut Vec<Step>) {
    let build = ctx
        .state
        .bot(ctx.chain_actor)
        .map(|b| b.build_distance)
        .unwrap_or(10.0);
    for entity in parts(&ctx.state, cell) {
        let name = entity.name.clone();
        let position = entity.position.clone();
        let min_radius = ctx.state.placement_clearance(&name).unwrap_or(0.0);
        let id = ctx.ids.next();
        let mut eff = vec![
            Effect::LoseItem {
                who: Actor::Role,
                item: name.clone(),
                count: 1,
            },
            Effect::CreateEntity(Box::new(entity.clone())),
        ];
        if name == DRILL
            && let Some(area) = ctx
                .state
                .collision_area_facing(DRILL, &cell.drill, cell.facing)
        {
            for tile in footprint_tiles(&area) {
                let pos = Position::from(&tile);
                if ctx.state.resource_available(&pos, &spec.ore) > 0 {
                    eff.push(Effect::ConsumeResource {
                        pos,
                        item: spec.ore.clone(),
                        count: 0,
                    });
                }
            }
        }
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
            eff,
            duration: PLACE_TICKS,
            pinned: None,
            label: format!("place {} at {}", name, position),
        })));
        ctx.state.create_entity(entity);
    }
}

/// The first fuel visit of each of a cell's machines, drill first, which is
/// when that machine starts running and so what a take is timed from. A job
/// longer than a stack of coal will burn adds refuel visits behind it --
/// `fuel_steps` chains those by burn time, and nothing waits on them.
fn fuel_both(
    ctx: &mut ExpansionCtx,
    spec: &CellSpec,
    cell: &Cell,
    coal: (u32, u32),
    research_pre: &[Condition],
    reach: f64,
    steps: &mut Vec<Step>,
) -> Vec<ActionId> {
    let (drill_coal, furnace_coal) = coal;
    let mut fuel_ids: Vec<ActionId> = Vec::new();
    for (burner, feeds) in [
        (
            Burner {
                machine: DRILL,
                position: &cell.drill,
                coal: drill_coal,
                burn_ticks: DRILL_BURN_TICKS,
                runs_out: Some((spec.item.as_str(), spec.ticks_per_item)),
            },
            false,
        ),
        (
            Burner {
                machine: FURNACE,
                position: &cell.furnace,
                coal: furnace_coal,
                burn_ticks: COAL_BURN_TICKS,
                runs_out: None,
            },
            true,
        ),
    ] {
        let mut extra_pre: Vec<Condition> = Vec::new();
        if feeds {
            extra_pre.push(Condition::EntityAt {
                pos: cell.drill.clone(),
                name: DRILL.into(),
            });
            extra_pre.push(Condition::Feeds {
                from: cell.drill.clone(),
                to: cell.furnace.clone(),
            });
            extra_pre.extend(research_pre.iter().cloned());
        }
        let (fuel, visits) = fuel_steps(ctx, &burner, reach, &extra_pre);
        steps.extend(fuel);
        fuel_ids.extend(visits.first().copied());
    }
    fuel_ids
}

/// Stand a fresh cell for `need` of the product: its bill, its two
/// placements, one cycle of headroom's worth of coal in each machine, and
/// the takes that bring `need` back out.
///
/// Sited on ground that holds [`site_ore`] -- the take, the headroom, an
/// allowance for a stale amount, and never less than a load -- from where the
/// acting bot stands.
///
/// # One cell per share, and that was measured on 2026-09-05
///
/// A share of any size stands one cell and waits on it: `have 78 iron-plate`
/// is one drill with 18,720 ticks queued. Splitting a share across
/// `ceil(need / c)`-plate cells built by the rest of the roster in
/// [`Step::Owned`] blocks -- each builder bringing the drill, the furnace
/// and the coal, the taker only taking -- was built and measured on
/// `workspace/scripts/map.json` over four bots, and lost on every goal:
/// `researched:automation` 25,886 -> 27,836, `producing:automation-science-pack:6`
/// 39,118 -> 48,559, `producing:logistic-science-pack:6` 97,232 -> 122,718.
/// Each builder's own share needs its starter drill, so a cell built for
/// someone else costs a crafted drill whose nine plates queue on the
/// hand-smelt bank; the taker still waits for the slower of its cells; and
/// the plan's real serialisation was never this wait -- it was every plate
/// consumer on a chain being ordered after every earlier plate producer
/// (`ActionNetwork::infer_edges`, narrowed the same day) and the drain cap
/// tripping on hand-smelt furnaces (see [`Drain`]). With those two fixed,
/// green's makespan fell by a quarter with one cell per share. The count
/// lever the world-record replays show is real, and it is a *rate* decision
/// -- `Producing` standing its ore cells up front -- not a per-fragment one.
fn open_cell_steps(
    ctx: &mut ExpansionCtx,
    spec: &CellSpec,
    need: u32,
    research_pre: &[Condition],
    reach: f64,
) -> Result<(Vec<Step>, Option<ActionId>), PlannerError> {
    let mut steps: Vec<Step> = Vec::new();
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
    let cells = plan_cells(&ctx.state, &from, spec, 1, site_ore(spec, need))?;
    let cell = cells
        .into_iter()
        .next()
        .ok_or_else(|| PlannerError::NoPatchForCell {
            item: spec.item.clone(),
            ore: spec.ore.clone(),
        })?;

    place_steps(ctx, spec, &cell, &mut steps);
    let fuel_ids = fuel_both(
        ctx,
        spec,
        &cell,
        (drill_coal, furnace_coal),
        research_pre,
        reach,
        &mut steps,
    );

    // The one step `BuildCell` never takes: pull `need` of the item back out
    // of the furnace and into the acting bot's hands, which is what turns a
    // standing structure into a satisfied `Have`/`Produced`.
    let (takes, timed) = take_steps(ctx, spec, &cell, need, 0, research_pre, reach);
    steps.extend(takes);
    // Production cannot start before either machine is fuelled, and the
    // scheduler needs to be told: an `Insert`'s effect satisfies no condition
    // of the `Remove` above, so nothing here is inferred.
    for fuel in &fuel_ids {
        for (take, lag) in &timed {
            steps.push(Step::Link {
                from: *fuel,
                to: *take,
                lag: *lag,
            });
        }
    }
    let last = timed.last().map(|(id, _)| *id);
    if let (Some(_), Some(started_by)) = (last, fuel_ids.first().copied()) {
        promise(ctx, spec, &cell, started_by, need);
    }
    Ok((steps, last))
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
        let cell =
            plan_cell(s, &Position::new(0., 0.), &spec, 1).expect("the fixture has iron ore");
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
        let cell =
            plan_cell(&s, &Position::new(0., 0.), &spec, 1).expect("the fixture has iron ore");
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
            fit(
                &s,
                &Position::new(-32., 40.),
                Direction::North,
                "iron-ore",
                1
            )
            .is_none(),
            "the ground east of the patch is clear, and a drill on it mines nothing"
        );
        assert!(
            fit(
                &s,
                &Position::new(-35., 35.),
                Direction::North,
                "iron-ore",
                1
            )
            .is_some(),
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
        let cells = plan_cells(&s, &Position::new(0., 0.), &spec, 3, 1).expect("room for three");
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
                plan_cell(&s, &Position::new(0., 0.), &spec, 1),
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
    ///
    /// **21 actions and 5,773 ticks since 2026-09-04**, when `Chop` was
    /// registered ahead of `Mine`. This is the largest single movement any
    /// pinned figure in the crate has taken -- 48% off the makespan -- and the
    /// reason is in the summary at the top: **mine 37 coal**. Thirty-seven
    /// units of hand mining is 4,440 ticks, and the fixture's `rock-huge`
    /// hands over twenty-four coal *and* twenty-four stone for 360. The stone
    /// that comes with it is what removes the other actions: the furnaces this
    /// plan crafts no longer need their stone dug for separately. See
    /// `have::Chop`.
    ///
    /// **5,773 -> 5,800 later the same day**, when a chop learned to stand
    /// beside the rock rather than on it: the bot now walks to the rock's
    /// placement clearance and starts its next walk from there. 27 ticks for
    /// a stand-point the game will actually grant, against one it refused
    /// before dispatch in every batch of the first live run to chop a rock.
    #[test]
    fn the_whole_of_stage_one_costs_this_much() {
        let bots = [BotId(1)];
        let s = state(&bots);
        let net = plan(15).expect("the fixture can build a cell");
        // 21 -> 20 on 2026-09-05: `expand` rehearses, and the one coal the
        // furnace's first fuel load asked for is priced over the plan's whole
        // coal and comes off the rock instead of a tile (`have::chop_beats_mining`).
        assert_eq!(net.len(), 20, "actions in a one-cell plan");
        let sched = crate::schedule::schedule(&net, &s, &bots).expect("it schedules");
        // 5,800 -> 5,810 when the fuel load started carrying the smelting lag
        // (`have::every_fuel_load_gates_the_take_by_the_whole_smelting_time`).
        // Moved by the lookahead scheduling key (51c7f695): a bound over the bot's other ready work replaces (end, id) as the primary key, and the plan overlaps the longer smelt under the shorter one.
        // 5466 -> 3989 on 2026-09-05: `infer_edges` no longer orders every plate consumer on the chain after every earlier plate producer; the stated supply edge (`run_steps`) is the only one, and the drill's and the furnace's bills overlap.
        // 3989 -> 3732 later on 2026-09-05: the furnace's first coal comes off the rock the drill's coal is swung for anyway, so the tile it was dug from is never walked to (`expand` rehearses; `have::chop_beats_mining`).
        // 3732 -> 3895 on 2026-09-05: the walk model stopped crediting a bot for `radius` tiles it never saved (`schedule::travel_ticks`) and the speed constant came down from the prototype's 0.15 to the measured 0.14. Same plan, priced honestly; see `WALK_TILES_PER_TICK`.
        // 3895 -> 3810 on 2026-09-05: a walk stops on the outer ring of the
        // action's reach (`rcon::approach_aim`), so an approach is charged only
        // as far as the bot actually goes.
        assert_eq!(sched.makespan, 3810, "ticks for one bot to build one cell");
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
    /// * `craft_ticks(stone-furnace, 1)` = craft (30) + **one swing at the
    ///   fixture's `rock-huge`** for its 5 stone (360) = 390. It was 630 --
    ///   five stone hand-mined at 120 each -- until 2026-09-04, when
    ///   [`raw_ticks`] started pricing stone and coal the way `Chop` supplies
    ///   them: the rock is 360 ticks for 24 stone, so anything up to 24 stone
    ///   costs one swing, and 5 stone is that swing rather than 600 ticks of
    ///   digging. A world with no rock still prices at 630.
    ///
    /// So one drill -- which needs its *own* stone-furnace as an ingredient,
    /// per its recipe -- costs `120 + 312*3 + 654*3 + 390 = 3408` ticks, and
    /// the cell's own, separate placement furnace costs another flat `390`:
    /// `3798` for the pair, the number [`the_hand_and_cell_costs_cross_over_near_fifty_plates`]
    /// builds on.
    #[test]
    fn craft_ticks_prices_a_drill_and_a_furnace_from_raw_materials() {
        let s = state(&[BotId(1)]);
        assert_eq!(
            craft_ticks(&s, "stone-furnace", 1, CRAFT_TICKS_MAX_DEPTH),
            390,
            "30 (craft) + 360 (one swing at a rock for the 5 stone)"
        );
        assert_eq!(
            craft_ticks(&s, DRILL, 1, CRAFT_TICKS_MAX_DEPTH),
            3408,
            "120 (assemble) + 3*312 (plate) + 3*654 (gear wheel) + 390 (the drill's own furnace)"
        );
    }

    /// The two bot-busy costs [`PlaceDrill::applicable`] compares, at the
    /// quantity that started this investigation (fifty iron plates, the
    /// `steam-power` trigger) and at one small enough that nobody wants a
    /// drill built for it.
    ///
    /// Hand-smelting fifty: mine 50 ore (`120 * 50 = 6000`), get the coal a
    /// furnace burns smelting them (`recipe_ticks(iron-plate) * 50 = 9600`
    /// ticks of energy, `div_ceil`d by `COAL_BURN_TICKS = 2666` is 4 coal --
    /// **one swing at the fixture's `rock-huge`, 360**, where it was 480 of
    /// digging until [`raw_ticks`] learned what `Chop` pays), one placement
    /// (`30`) and three transfers (`3 * 10 = 30`): `6000 + 360 + 30 + 30 =
    /// 6420`.
    ///
    /// Building a cell for fifty: the drill and furnace from
    /// [`craft_ticks_prices_a_drill_and_a_furnace_from_raw_materials`]
    /// (`3408 + 390 = 3798`), two placements (`60`) and three transfers
    /// (`30`) -- `3888` fixed -- plus coal for 51 cycles at the cell's own
    /// 240-tick rate (`51 * 240 = 12240`; `div_ceil(1600) = 8` for the
    /// drill, `div_ceil(2666) = 5` for the furnace, 13 coal, one swing:
    /// `360`): `3888 + 360 = 4248`. That is below hand-smelting's `6420`, so
    /// a cell wins fifty plates. The margin was 612 ticks when stone and coal
    /// were priced as dug (`5928` against `6540`); with rocks priced as the
    /// plan actually swings at them the crossover sits near thirty-two
    /// plates, and fifty wins by 2,172.
    ///
    /// Five: hand-smelting stays cheap (`120*5 + 120*1(coal: one coal is
    /// cheaper dug than swung for) + 30 + 30 = 780`) while a cell's fixed
    /// cost barely moves with the quantity (`3888 + 240 (2 coal, dug) =
    /// 4128`), so hand-mining wins by a wide margin -- nobody builds a drill
    /// for five plates.
    #[test]
    fn the_hand_and_cell_costs_cross_over_near_fifty_plates() {
        let s = state(&[BotId(1)]);
        let spec = iron();

        assert_eq!(hand_smelt_bot_ticks(&s, &spec, 50), 6420);
        assert_eq!(cell_setup_bot_ticks(&s, &spec, 50), 4248);
        assert!(
            cell_setup_bot_ticks(&s, &spec, 50) < hand_smelt_bot_ticks(&s, &spec, 50),
            "fifty plates must be cheaper in bot-time to build than to hand-smelt"
        );

        assert_eq!(hand_smelt_bot_ticks(&s, &spec, 5), 780);
        assert_eq!(cell_setup_bot_ticks(&s, &spec, 5), 4128);
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
        assert_eq!(
            net.actions()
                .filter(|a| a.label.ends_with("from the cell"))
                .count(),
            1,
            "fifty fits in one stack, so it is still exactly one take -- \
             the split must not touch a plan that was already legal"
        );
    }

    /// The fixture's iron patch with a thin rim: every tile whose `x` is
    /// within three of the patch's near (east) edge holds `rim` ore, the rest
    /// keep the fixture's default. The near edge is the one
    /// `nearest_resource_tile` anchors on from the origin, exactly as seed
    /// `31337`'s rim is the one the live run drilled dry.
    fn world_with_a_thin_rim(rim: u32) -> factorio_bot_core::factorio::world::FactorioWorld {
        use factorio_bot_core::factorio::util::add_to_rect;
        use factorio_bot_core::types::Rect;
        let world = fixture_world();
        let mut ore: Vec<FactorioEntity> = Vec::new();
        factorio_bot_core::test_utils::spawn_ore(
            &mut ore,
            add_to_rect(&Rect::from_wh(10., 10.), &Position::new(-40., 40.)),
            "iron-ore",
        );
        let east = ore.iter().map(|e| e.position.x()).fold(f64::MIN, f64::max);
        for entity in &mut ore {
            entity.amount = Some(if entity.position.x() > east - 3. {
                rim
            } else {
                crate::state::DEFAULT_RESOURCE_PER_TILE
            });
        }
        world
            .update_chunk_entities(ore)
            .expect("the amounts are delivered");
        world
    }

    /// The live failure, in the model: a drill sited on the thin rim of a
    /// patch runs dry inside the take it was fuelled for. A site now has to
    /// hold what the cell is asked for, and the ring walk finds the nearest
    /// one that does -- three tiles further in, on the same patch, rather
    /// than nowhere.
    #[test]
    fn a_cell_is_sited_where_the_ground_covers_what_it_is_asked_for() {
        let s = PlanState::from_world(Arc::new(world_with_a_thin_rim(10)), &[BotId(1)]);
        let spec = iron();
        let from = Position::new(0., 0.);

        let anywhere = plan_cell(&s, &from, &spec, 1).expect("a drill fits on the rim");
        assert!(
            cell_yield(&s, &anywhere.drill, anywhere.facing, "iron-ore") < 150,
            "asked for nothing, the nearest site is on the rim and would run dry"
        );

        let covered = plan_cell(&s, &from, &spec, 150).expect("the patch holds 150 three tiles in");
        assert!(
            cell_yield(&s, &covered.drill, covered.facing, "iron-ore") >= 150,
            "asked for a load, the site holds a load: {covered:?}"
        );
        assert!(
            covered.drill.x() < anywhere.drill.x(),
            "and it is further into the patch, not somewhere else: {:?} against {:?}",
            covered.drill,
            anywhere.drill
        );
    }

    /// A take spends the ore it stands for, on the tiles under the drill --
    /// the gap `PlanState::covers_resource` documents, closed from the take's
    /// side -- and the drill's placement claims that ground.
    #[test]
    fn a_take_spends_the_ore_under_the_drill_and_the_placement_claims_it() {
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
        .expect("fifty plates plan");
        let drill = net
            .actions()
            .find_map(|a| match &a.kind {
                ActionKind::Place { entity } if entity.name == DRILL => Some(entity.clone()),
                _ => None,
            })
            .expect("a drill is placed");
        let facing = Direction::from_u8(drill.direction).expect("a cardinal");
        let area = s
            .collision_area_facing(DRILL, &drill.position, facing)
            .expect("the fixture has a drill prototype");
        let under: BTreeSet<Pos> = footprint_tiles(&area).into_iter().collect();

        let spent: u32 = net
            .actions()
            .filter(|a| a.label.ends_with("from the cell"))
            .flat_map(|a| a.eff.iter())
            .filter_map(|e| match e {
                Effect::ConsumeResource { pos, item, count } if item == "iron-ore" => {
                    assert!(
                        under.contains(&Pos::from(pos)),
                        "ore spent at {pos}, which is not under the drill at {}",
                        drill.position
                    );
                    Some(*count)
                }
                _ => None,
            })
            .sum();
        assert_eq!(
            spent, 50,
            "fifty plates are fifty ore off the drill's own tiles"
        );

        let claims = net
            .actions()
            .filter(|a| a.label.starts_with("place burner-mining-drill"))
            .flat_map(|a| a.eff.iter())
            .filter(|e| matches!(e, Effect::ConsumeResource { count: 0, .. }))
            .count();
        assert!(
            claims > 0,
            "the placement claims the ground under the drill"
        );
    }

    /// A rate cell stood earlier in the plan is drawn on by a later one-shot
    /// fragment -- no second drill, no hand-smelt, one take timed after the
    /// cell's first fuel -- which is the whole mechanism a ladder with
    /// `producing:iron-plate` on its first rung relies on.
    #[test]
    fn a_later_fragment_drains_a_cell_the_plan_already_stood() {
        let bots = vec![BotId(1)];
        let s = state(&bots);
        let net = expand(
            &[
                Goal::Producing {
                    item: "iron-plate".into(),
                    per_minute: 15,
                },
                Goal::Have {
                    item: "iron-plate".into(),
                    count: 5,
                    whose: Holder::Share(BotId(1)),
                },
            ],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("a cell and a fragment plan");
        assert_eq!(
            net.actions()
                .filter(|a| a.label.starts_with("place burner-mining-drill"))
                .count(),
            1,
            "the fragment draws on the rate cell instead of standing its own"
        );
        let take = net
            .actions()
            .find(|a| a.label == "take 5 iron-plate from the cell")
            .expect("the five come out of the cell");
        assert!(
            net.actions().all(|a| a.label != "insert 5 iron-ore"),
            "and are not hand-smelted"
        );
        let fuel = net
            .actions()
            .find(|a| a.label.starts_with("fuel the burner-mining-drill"))
            .expect("the drill is fuelled");
        assert!(
            net.preds(take.id)
                .iter()
                .any(|(from, lag)| *from == fuel.id && *lag >= 5 * 240),
            "the take is timed from the drill's first fuel by at least its own five cycles: {:?}",
            net.preds(take.id)
        );
        let plan = schedule(&net, &s, &bots).expect("it schedules");
        assert!(plan.makespan > 0);
    }

    /// A burner drill runs exactly as long as the coal in it and then it
    /// stops, and the plan says so on the visit that buys the last of it.
    ///
    /// This is the disclosure `run-1788640611-64852` had no way to make: ten
    /// drills, one fuel visit each, every one of them ending `no_fuel` after
    /// delivering precisely what its load bought (6 coal -> 39 ore, 8 -> 53,
    /// 11 -> 73). The plan was right and unreadable. The numbers in the label
    /// are checked against each other here rather than hard-coded, so the
    /// label cannot drift away from the arithmetic that sizes the load.
    #[test]
    fn the_drills_last_fuel_visit_says_when_it_runs_out() {
        let bots = vec![BotId(1)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 50,
                whose: Holder::Share(BotId(1)),
            }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .expect("fifty plates stand a cell");
        let label = net
            .actions()
            .map(|a| a.label.clone())
            .find(|l| l.starts_with("fuel the burner-mining-drill"))
            .expect("the drill is fuelled");
        let (head, tail) = label
            .split_once(" coal (")
            .expect("the last visit discloses what the load buys: {label}");
        let coal: u32 = head
            .rsplit(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .expect("the load is a number");
        let fields: Vec<&str> = tail.trim_end_matches(')').split(", ").collect();
        assert_eq!(fields.len(), 3, "ticks, yield, verdict: {label}");
        assert_eq!(fields[2], "then it stops", "{label}");
        let burn: u32 = fields[0]
            .trim_end_matches(" ticks")
            .parse()
            .expect("the burn is a number");
        assert_eq!(
            burn,
            coal * DRILL_BURN_TICKS,
            "the disclosed burn is the load's own: {label}"
        );
        let (made, item) = fields[1].split_once(' ').expect("a count and an item");
        assert_eq!(item, "iron-plate", "{label}");
        assert_eq!(
            made.parse::<u32>().expect("the yield is a number"),
            burn / iron().ticks_per_item,
            "the disclosed yield is what the cell makes in that burn: {label}"
        );
    }

    /// Stand `count` cells the way `run_steps` would -- placements simulated,
    /// so the ground under each drill is claimed and the ledger reads them as
    /// live -- each carrying `queued` items of backlog.
    fn stand_cells(spec: &CellSpec, bots: &[BotId], count: u32, queued: u32) -> ExpansionCtx {
        let mut ctx = ExpansionCtx::new(state(bots), BotId(1));
        ctx.state.gain(BotId(1), DRILL, count);
        ctx.state.gain(BotId(1), FURNACE, count);
        let cells = plan_cells(&ctx.state, &Position::new(0., 0.), spec, count, 1)
            .expect("the fixture sites the cells");
        for cell in &cells {
            let mut steps = Vec::new();
            place_steps(&mut ctx, spec, cell, &mut steps);
            for step in &steps {
                if let Step::Act(action) = step {
                    for effect in &action.eff {
                        effect
                            .apply(&mut ctx.state, BotId(1))
                            .expect("a placement's effects apply");
                    }
                }
            }
            let started_by = ctx.ids.next();
            promise(&mut ctx, spec, cell, started_by, queued);
        }
        ctx
    }

    /// [`DrainPolicy::Parallel`] scales the bound by the cells that stand, so
    /// two cells carrying a backlog one cell-build long -- refused outright
    /// under the conservative policy -- are both offered.
    ///
    /// The backlog is computed from `cell_setup_bot_ticks` rather than written
    /// down, so the test straddles the bound whatever the fixture's recipe
    /// costs; a hard-coded count would silently stop testing anything the day
    /// the fixture's prices moved.
    #[test]
    fn the_parallel_policy_scales_the_bound_by_the_cells_that_stand() {
        let bots = vec![BotId(1)];
        let spec = iron();
        let fixed = cell_setup_bot_ticks(&state(&bots), &spec, 1);
        // Just past one cell-build's backlog and well under two.
        let queued = fixed.div_ceil(spec.ticks_per_item) + 1;
        let ctx = stand_cells(&spec, &bots, 2, queued);

        let conservative = Drain::new(&ctx.state, &spec);
        assert_eq!(conservative.live.len(), 2, "both cells are live");
        assert!(
            conservative.eligible.is_empty(),
            "one cell-build of backlog is past the conservative bound: {:?} against {fixed}",
            conservative
                .live
                .iter()
                .map(|c| c.queued)
                .collect::<Vec<_>>()
        );

        let parallel = ctx.state.fork().with_drain_policy(DrainPolicy::Parallel);
        assert_eq!(
            Drain::new(&parallel, &spec).eligible.len(),
            2,
            "two cells standing buy two cell-builds' worth of patience"
        );

        // And a single cell is left exactly where it was: the policy can only
        // widen the bound by the parallelism the plan actually has, which is
        // why `researched:automation` -- one cell, 39.6% busy -- is untouched
        // by it.
        let solo = stand_cells(&spec, &bots, 1, queued)
            .state
            .fork()
            .with_drain_policy(DrainPolicy::Parallel);
        assert!(
            Drain::new(&solo, &spec).eligible.is_empty(),
            "one standing cell buys no extra patience under either policy"
        );
    }

    /// [`crate::plan_best`] never returns a plan longer than the conservative
    /// policy's, because the conservative policy is one of the two it builds
    /// and a tie keeps the earlier one.
    #[test]
    fn plan_best_is_never_worse_than_the_policy_every_plan_used_to_have() {
        let bots = vec![BotId(1), BotId(2), BotId(3), BotId(4)];
        let s = state(&bots);
        let goals = vec![Goal::Producing {
            item: "iron-plate".into(),
            per_minute: 15,
        }];
        let registry = registry_for(&bots);
        let actor = BotId(1);
        let conservative = {
            let under = s.fork().with_drain_policy(DrainPolicy::Conservative);
            let net = expand(&goals, &under, &registry, actor).expect("it expands");
            schedule(&net, &under, &bots).expect("it schedules")
        };
        let (_, best) =
            crate::plan_best(&goals, &s, &registry, actor, &bots).expect("a policy plans");
        assert!(
            best.makespan <= conservative.makespan,
            "plan_best returned {} against the conservative {}",
            best.makespan,
            conservative.makespan
        );
    }

    /// A cell past the drain bound is not offered, however many cells stand.
    /// The cap that used to override the bound -- "two a bot, then every
    /// cell whatever its backlog" -- is gone; see [`Drain`] for the measured
    /// reason. On a solo roster the old cap was two, so two backlogged
    /// cells are exactly the case it used to open up.
    #[test]
    fn a_backlogged_cell_is_never_offered_however_many_stand() {
        let bots = vec![BotId(1)];
        let spec = iron();
        // Stand both cells the way `run_steps` would: the placements'
        // effects applied, so the drills' ground is claimed and the ledger
        // reads the cells as live.
        let stand = |queued: u32| -> ExpansionCtx {
            let mut ctx = ExpansionCtx::new(state(&bots), BotId(1));
            ctx.state.gain(BotId(1), DRILL, 2);
            ctx.state.gain(BotId(1), FURNACE, 2);
            let cells = plan_cells(&ctx.state, &Position::new(0., 0.), &spec, 2, 1)
                .expect("the fixture sites two cells");
            for cell in &cells {
                let mut steps = Vec::new();
                place_steps(&mut ctx, &spec, cell, &mut steps);
                for step in &steps {
                    if let Step::Act(action) = step {
                        for effect in &action.eff {
                            effect
                                .apply(&mut ctx.state, BotId(1))
                                .expect("a placement's effects apply");
                        }
                    }
                }
                let started_by = ctx.ids.next();
                promise(&mut ctx, &spec, cell, started_by, queued);
            }
            ctx
        };
        // Fifty plates queued: 12,000 ticks, three times the bound.
        let ctx = stand(50);
        let drain = Drain::new(&ctx.state, &spec);
        assert_eq!(drain.live.len(), 2, "both cells are live");
        assert!(
            drain.eligible.is_empty(),
            "neither is offered past the bound: {:?}",
            drain.eligible.iter().map(|c| c.queued).collect::<Vec<_>>()
        );

        // The same two cells with next to nothing queued are both offered.
        let fresh = stand(1);
        assert_eq!(Drain::new(&fresh.state, &spec).eligible.len(), 2);
    }

    /// Four bots each asking for a share of plates stand their cells on
    /// more than one bot, and no cell's bill -- the drill's own gears and
    /// plates -- is served out of another cell's take. That take would be
    /// queued behind the other share (12,000 ticks here), which is the
    /// serial ladder `run-1788569499-05724` showed live: bot 1 waiting on
    /// `take 12 iron-plate from the cell` for the drill of its next cell
    /// while the roster stood idle.
    #[test]
    fn four_bots_stand_their_own_cells_and_no_bill_waits_on_a_take() {
        let bots = vec![BotId(1), BotId(2), BotId(3), BotId(4)];
        let s = state(&bots);
        let goals: Vec<Goal> = bots
            .iter()
            .map(|bot| Goal::Have {
                item: "iron-plate".into(),
                count: 50,
                whose: Holder::Share(*bot),
            })
            .collect();
        let net = expand(&goals, &s, &registry_for(&bots), BotId(1)).expect("four shares plan");
        let plan = schedule(&net, &s, &bots).expect("it schedules");

        let placers: BTreeSet<BotId> = bots
            .iter()
            .copied()
            .filter(|bot| {
                plan.steps_for(*bot).iter().any(|step| {
                    matches!(&step.what, crate::schedule::StepKind::Act { label, .. }
                        if label.starts_with("place burner-mining-drill"))
                })
            })
            .collect();
        assert!(
            placers.len() >= 2,
            "cells are stood on more than one bot: {placers:?}"
        );

        let is_take = |id: ActionId| {
            net.action(id)
                .is_some_and(|a| a.label.contains("from the cell"))
        };
        for craft in net
            .actions()
            .filter(|a| a.label.starts_with("craft 1 burner-mining-drill"))
        {
            let mut seen: BTreeSet<ActionId> = BTreeSet::new();
            let mut stack: Vec<ActionId> =
                net.preds(craft.id).into_iter().map(|(id, _)| id).collect();
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                assert!(
                    !is_take(id),
                    "{} is downstream of {}",
                    craft.label,
                    net.action(id).map(|a| a.label.as_str()).unwrap_or("?")
                );
                stack.extend(net.preds(id).into_iter().map(|(id, _)| id));
            }
        }
    }

    // ---- cells an earlier plan left standing --------------------------------

    /// The fixture's iron patch with every tile holding `amount` ore.
    fn world_with_ore_amount(amount: u32) -> factorio_bot_core::factorio::world::FactorioWorld {
        use factorio_bot_core::factorio::util::add_to_rect;
        use factorio_bot_core::types::Rect;
        let world = fixture_world();
        let mut ore: Vec<FactorioEntity> = Vec::new();
        factorio_bot_core::test_utils::spawn_ore(
            &mut ore,
            add_to_rect(&Rect::from_wh(10., 10.), &Position::new(-40., 40.)),
            "iron-ore",
        );
        for entity in &mut ore {
            entity.amount = Some(amount);
        }
        world
            .update_chunk_entities(ore)
            .expect("the amounts are delivered");
        world
    }

    /// Put a cell an *earlier plan* built into `world` itself -- the entity
    /// graph, not an overlay -- so that a state built from it finds the two
    /// machines standing with no queue entry, no claim and, when `fuel` says
    /// so, coal in their slots as the game would have reported it.
    fn leave_a_cell_standing(
        world: &factorio_bot_core::factorio::world::FactorioWorld,
        fuel: Option<(u32, u32)>,
    ) -> Cell {
        let s = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let spec = iron();
        let cell =
            plan_cell(&s, &Position::new(0., 0.), &spec, 1).expect("the fixture has iron ore");
        for mut entity in parts(&s, &cell) {
            let facing = Direction::from_u8(entity.direction).expect("a cardinal");
            entity.bounding_box = s
                .collision_area_facing(&entity.name, &entity.position, facing)
                .expect("the fixture has both prototypes");
            world
                .on_some_entity_created(entity)
                .expect("the machine stands");
        }
        if let Some((drill_coal, furnace_coal)) = fuel {
            let coal = |count: u32| {
                Box::new(Some(vec![
                    factorio_bot_core::types::InventoryItemWithQuality {
                        name: "coal".into(),
                        count,
                        quality: "normal".into(),
                    },
                ]))
            };
            world.observe_inventories(vec![
                factorio_bot_core::types::InventoryResponse {
                    name: DRILL.into(),
                    position: cell.drill.clone(),
                    output_inventory: Box::new(None),
                    fuel_inventory: coal(drill_coal),
                },
                factorio_bot_core::types::InventoryResponse {
                    name: FURNACE.into(),
                    position: cell.furnace.clone(),
                    output_inventory: Box::new(None),
                    fuel_inventory: coal(furnace_coal),
                },
            ]);
        }
        cell
    }

    fn fifty_plates(s: &PlanState) -> ActionNetwork {
        expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 50,
                whose: Holder::Anyone,
            }],
            s,
            &crate::method::have::default_registry(),
            BotId(1),
        )
        .expect("fifty plates plan")
    }

    fn fuel_loads(net: &ActionNetwork, machine: &str) -> Vec<(ActionId, u32)> {
        net.actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Insert {
                    slot: InventorySlot::Fuel,
                    entity,
                    count,
                    ..
                } if entity == machine => Some((a.id, *count)),
                _ => None,
            })
            .collect()
    }

    /// The replan case `run-1788552801-73005` paid for five times: a cell
    /// from an earlier plan stands on ore, fuel burned out, and the plan
    /// stood another one beside it. Now the standing cell is topped up and
    /// drained -- no second drill, no hand-smelt -- and the take is timed
    /// from this plan's own fuel visit, since there is no earlier one.
    #[test]
    fn a_cell_left_standing_by_an_earlier_plan_is_refuelled_and_drained_not_rebuilt() {
        let bots = vec![BotId(1)];
        let world = fixture_world();
        leave_a_cell_standing(&world, None);
        let s = PlanState::from_world(Arc::new(world), &bots);
        assert_eq!(
            cell_ledger(&s, &iron()).len(),
            1,
            "the standing cell is in the ledger with nothing queued on it"
        );

        let net = fifty_plates(&s);
        assert_eq!(
            net.actions()
                .filter(|a| a.label.starts_with("place burner-mining-drill"))
                .count(),
            0,
            "no drill is placed: the one standing is used"
        );
        assert!(
            net.actions()
                .all(|a| !a.label.starts_with("insert") || !a.label.contains("iron-ore")),
            "and nothing is hand-smelted"
        );
        let take = net
            .actions()
            .find(|a| a.label == "take 50 iron-plate from the cell")
            .expect("the fifty come out of the standing cell");
        let drill_fuel = fuel_loads(&net, DRILL);
        assert_eq!(drill_fuel.len(), 1, "one top-up visit to the drill");
        assert!(
            net.preds(take.id)
                .iter()
                .any(|(from, lag)| *from == drill_fuel[0].0 && *lag >= 50 * 240),
            "the take is timed from this plan's top-up by at least its own fifty cycles: {:?}",
            net.preds(take.id)
        );
        assert!(
            schedule(&net, &s, &bots).is_ok(),
            "an adopted cell's plan must schedule, not merely construct"
        );
    }

    /// A standing cell whose ground is gone is not a source. Same two
    /// machines, same facing, zero ore under the drill -- the state the live
    /// run's first cell was in at every replan after tick 135,300.
    #[test]
    fn a_standing_cell_over_exhausted_ground_is_not_offered() {
        let world = world_with_ore_amount(0);
        let cell = leave_a_cell_standing(&world, None);
        let s = PlanState::from_world(Arc::new(world), &[BotId(1)]);
        assert!(
            s.delivers_into(&cell.drill, &cell.furnace),
            "the pair still feeds -- it is only the ore that is gone"
        );
        assert!(
            cell_ledger(&s, &iron()).is_empty(),
            "a dry cell is not in the ledger"
        );
    }

    /// What a standing cell is offered for is bounded by the ore under its
    /// drill, and what it cannot cover is made some other way.
    #[test]
    fn a_standing_cell_is_offered_for_no_more_than_the_ground_under_it() {
        let bots = vec![BotId(1)];
        let world = world_with_ore_amount(3);
        leave_a_cell_standing(&world, None);
        let s = PlanState::from_world(Arc::new(world), &bots);
        let ledger = cell_ledger(&s, &iron());
        assert_eq!(ledger.len(), 1);
        assert_eq!(
            ledger[0].room, 6,
            "a cell sits on the patch's edge, so two of the drill's four tiles \
             are on ore: two tiles of three"
        );

        let net = fifty_plates(&s);
        let from_cell: u32 = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Remove { count, .. } if a.label.ends_with("from the cell") => {
                    Some(*count)
                }
                _ => None,
            })
            .sum();
        assert_eq!(from_cell, 6, "six plates is all the ground holds");
        assert!(
            net.actions()
                .any(|a| a.label.starts_with("insert") && a.label.contains("iron-ore")),
            "the other forty-four are hand-smelted, on a patch too thin for a new cell"
        );
    }

    /// Coal the game reported in a standing cell's slots is credited against
    /// the top-up, never counted on as production: the visit still happens
    /// (it is what the take is timed from) but brings only what the job
    /// burns beyond what is already there.
    #[test]
    fn a_standing_cell_is_topped_up_net_of_the_coal_it_holds() {
        let bots = vec![BotId(1)];
        let world = fixture_world();
        leave_a_cell_standing(&world, Some((5, 2)));
        let s = PlanState::from_world(Arc::new(world), &bots);
        let ledger = cell_ledger(&s, &iron());
        assert_eq!((ledger[0].drill_fuel, ledger[0].furnace_fuel), (5, 2));

        let net = fifty_plates(&s);
        let duration = 50 * 240;
        let drill_full = fuel_for_duration(duration, DRILL_BURN_TICKS);
        let furnace_full = fuel_for_duration(duration, COAL_BURN_TICKS);
        assert_eq!(
            fuel_loads(&net, DRILL).iter().map(|(_, n)| *n).sum::<u32>(),
            drill_full - 5,
            "the drill gets what fifty cycles burn less the five it holds"
        );
        assert_eq!(
            fuel_loads(&net, FURNACE)
                .iter()
                .map(|(_, n)| *n)
                .sum::<u32>(),
            furnace_full - 2,
            "and the furnace less its two"
        );

        // With more in the slot than the job burns, one coal still goes in:
        // the visit is the anchor the take is timed from.
        let world = fixture_world();
        leave_a_cell_standing(&world, Some((50, 50)));
        let s = PlanState::from_world(Arc::new(world), &bots);
        let net = fifty_plates(&s);
        assert_eq!(
            fuel_loads(&net, DRILL).iter().map(|(_, n)| *n).sum::<u32>(),
            1,
            "a full slot is still visited once, with one coal"
        );
    }

    /// A stone furnace's output is one slot holding one stack, so a goal
    /// larger than a stack is a **sequence of visits**, not one visit.
    ///
    /// This is the live failure: `take 141 iron-plate` came back
    /// `tried to remove 141 but removed 100`, and the 9,600 ticks before that
    /// were the furnace sitting in `full_output` -- stopped, not merely full
    /// -- because the plan would not come and empty it. 150 plates now leave
    /// as 100 and then 50, and the first take is timed to when the first
    /// hundred exist rather than to the end of the whole job.
    #[test]
    fn a_count_over_one_stack_leaves_the_cell_a_stack_at_a_time() {
        let bots = vec![BotId(1)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 150,
                whose: Holder::Anyone,
            }],
            &s,
            &crate::method::have::default_registry(),
            BotId(1),
        )
        .expect("a hundred and fifty plates plan");

        let takes: Vec<String> = net
            .actions()
            .filter(|a| a.label.ends_with("from the cell"))
            .map(|a| a.label.clone())
            .collect();
        assert_eq!(
            takes,
            vec![
                "take 100 iron-plate from the cell".to_string(),
                "take 50 iron-plate from the cell".to_string(),
            ],
            "one stack, then the remainder -- and never a take above the cap"
        );
        assert!(
            net.actions().all(|a| !matches!(
                &a.kind,
                ActionKind::Remove { slot, item, count, .. }
                    if *slot == InventorySlot::FurnaceResult
                        && s.slot_capacity(*slot, item).is_some_and(|cap| *count > cap)
            )),
            "no removal in the plan asks a slot for more than it holds"
        );
        assert!(
            schedule(&net, &s, &bots).is_ok(),
            "the split plan must schedule, not merely construct"
        );
    }

    /// **A fuel slot holds one stack, so a long job is refuelled rather than
    /// over-loaded.**
    ///
    /// `fuel the burner-mining-drill with 113 coal` put 50 coal in and 63
    /// nowhere, and the plan had no way to say the drill needed a second
    /// visit. It says so now: stack-sized loads, chained by how long each one
    /// burns, and nothing downstream waits on them -- the machine started at
    /// the first.
    #[test]
    fn a_cell_that_outlasts_a_stack_of_coal_is_refuelled_rather_than_overloaded() {
        let bots = vec![BotId(1)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 600,
                whose: Holder::Anyone,
            }],
            &s,
            &crate::method::have::default_registry(),
            BotId(1),
        )
        .expect("six hundred plates plan");

        let cap = s
            .slot_capacity(InventorySlot::Fuel, "coal")
            .expect("the fixture carries coal");
        let fuel: Vec<u32> = net
            .actions()
            .filter_map(|a| match &a.kind {
                ActionKind::Insert {
                    slot: InventorySlot::Fuel,
                    count,
                    ..
                } => Some(*count),
                _ => None,
            })
            .collect();
        assert!(
            fuel.iter().all(|count| *count <= cap),
            "a fuel slot takes {cap} and the plan asks {fuel:?}"
        );
        assert!(
            fuel.contains(&cap),
            "and this goal really is long enough to fill one, in {fuel:?} -- \
             a test that never reaches the cap proves nothing about it"
        );
        assert!(
            schedule(&net, &s, &bots).is_ok(),
            "the refuelled plan must schedule, not merely construct"
        );
    }

    /// The takes are timed from the **first** fuel visit, not the last.
    ///
    /// A machine starts when it is first fuelled and runs across the refuels.
    /// Timing a take from the last visit would wait for a stack of coal that
    /// has not been burned yet -- an off-by-a-whole-job error that would look
    /// like a slower plan rather than like a bug.
    #[test]
    fn a_refuel_visit_is_on_the_critical_path_of_nothing() {
        let bots = vec![BotId(1)];
        let s = state(&bots);
        let mut ctx = crate::method::ExpansionCtx::new(s.fork(), BotId(1));
        let (steps, ids) = fuel_steps(
            &mut ctx,
            &Burner {
                machine: DRILL,
                position: &Position::new(0., 0.),
                coal: 113,
                burn_ticks: DRILL_BURN_TICKS,
                runs_out: None,
            },
            10.,
            &[],
        );
        assert_eq!(ids.len(), 3, "fifty, fifty and thirteen");
        let links: Vec<(usize, Ticks)> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Link { from, to, lag } => ids.contains(to).then_some((
                    ids.iter().position(|id| id == from).unwrap_or(usize::MAX),
                    *lag,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            links,
            vec![(0, 50 * DRILL_BURN_TICKS), (1, 50 * DRILL_BURN_TICKS)],
            "each visit follows the one before it by exactly how long that one \
             burns -- which is when the slot next has room"
        );
    }
}
