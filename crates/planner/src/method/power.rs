//! Building the power a lab needs.
//!
//! An offshore pump on a shoreline, three pipes, a boiler, a steam engine and
//! one small electric pole: 900 kW, at the water, for about 45 iron plates.
//! This is the subsystem `2026-09-02-research-needs-power.md` named as stage 2
//! and `2026-09-02-building-power.md` designed but could not build, because
//! the planner could not see water. It can since `9ca7229a`.
//!
//! # Why the plant is at the water and the coal is carried
//!
//! Water is the one input that cannot be moved. Coal is five items in an
//! inventory. Siting the plant at the coal and running pipe to the lake costs
//! `pipe-to-ground` at 15 iron plates per 10 tiles — a 60-tile separation
//! roughly *doubles* rung 7's whole iron bill, which is about 98 plates — and
//! scatters blocking entities across the ground the bots mine along. Siting it
//! at the water costs one walk. The full arithmetic is in
//! `docs/superpowers/notes/2026-09-02-building-power.md` §5.
//!
//! The lab follows the plant rather than the other way round: it has to stand
//! inside the pole's supply area anyway, and `Researched::lab_site` already
//! searches around the supplying pole rather than around the bot.
//!
//! # Solar is excluded structurally
//!
//! `solar-energy` needs `logistic-science-pack`, which needs
//! `automation-science-pack`, which is what the lab this plant powers is going
//! to consume. Research needs power; solar power needs research. So the plant
//! is a boiler and an engine, and `crate::state`'s `generation_kw` has no
//! `solar-panel` arm on purpose.

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
pub const PIPE_COUNT: u32 = 3;

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

/// How far from the acting bot the plant may be sited, in tiles.
///
/// The same bound `PlanState::electric_supply_kw` and `Researched`'s lab
/// search already use, and for a sharper reason here: everything the plant
/// needs is carried to it — the pump, the pipes, the boiler, the engine, the
/// pole, five coal, the lab and ten science packs — and everything it is made
/// of is mined somewhere else.
const PLANT_SITE_RADIUS: f64 = 64.;

/// How far the search looks when it wants to *report* a distance it will then
/// refuse, in tiles.
///
/// A refusal that says "the nearest water is 210 tiles away" is worth more
/// than one that says "no water within 64", and this is what buys the number.
/// It is deliberately not the siting radius: reading tiles is linear in the
/// area, a fully charted map carries ~410,000 water tiles since `fa8dabf3`,
/// and this pass runs only on the path that is about to fail anyway.
const PLANT_REPORT_RADIUS: f64 = 128.;

/// How far around the nearest water tile a shoreline is looked for, in tiles.
///
/// The nearest water tile is very unlikely to be a *buildable* shoreline: it
/// may be a one-tile inlet, or the plant may not fit behind it. This is how
/// much of that lake's edge gets tried before the whole lake is given up on.
const SHORE_SEARCH_RADIUS: i32 = 10;

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
    /// In build order: pump, pipe, pipe, boiler, pipe, engine, pole.
    pub parts: Vec<PlantPart>,
    /// Where the coal goes.
    pub boiler: Position,
    /// The generator whose 900 kW the pole carries.
    pub engine: Position,
    /// The pole that carries it, and the anchor the lab is then sited around.
    ///
    /// **The lab follows the plant, not the bot.** `Researched::lab_site`
    /// searches for supply within 64 tiles of whatever origin it is given, and
    /// the plant itself may be up to 64 tiles from the bot — so re-asking from
    /// the bot's position could put a plant just built out of the lab's reach.
    /// Asking from the pole finds it at distance zero.
    pub pole: Position,
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
fn layout(pump: &Position, facing: Direction) -> Option<Vec<PlantPart>> {
    let lateral = turned((1., 0.), facing)?;

    let joint_pump = pump.add(&turned(PUMP_OUTPUT, facing)?);
    let joint_boiler = joint_pump.add(&lateral);

    // Steam away from the water: the boiler faces opposite the pump.
    let boiler_facing = compose(Direction::South, facing)?;
    let boiler = subtract(&joint_boiler, &turned(BOILER_WATER[1], boiler_facing)?);

    let joint_engine = boiler.add(&turned(BOILER_STEAM, boiler_facing)?);
    let engine_facing = facing;
    let engine = subtract(&joint_engine, &turned(ENGINE_STEAM[1], engine_facing)?);

    Some(vec![
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
        PlantPart {
            name: ENGINE,
            position: engine,
            direction: engine_facing,
        },
    ])
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

/// Find somewhere to build a plant within reach of `from`, or say why not.
///
/// Deterministic by construction: the water tile is the one
/// `EntityGraph::nearest_water_tile` orders first, the shoreline candidates
/// come out of a ring search in a fixed order, and the four facings are tried
/// north, east, south, west. Nothing here reads a quad tree's own order.
pub fn plan_plant(state: &PlanState, from: &Position) -> Result<Plant, PlannerError> {
    // The narrow search first, and the wide one **only** when it fails.
    // `nearest_water_tile` is linear in the tiles inside its radius, and since
    // `fa8dabf3` a fully charted map carries ~410,000 water tiles: reading a
    // 256-by-256 box on every expansion to buy a sentence for the failing case
    // would charge every successful plan for it.
    let Some(water) = state.nearest_water_tile(from, PLANT_SITE_RADIUS) else {
        return Err(match state.nearest_water_tile(from, PLANT_REPORT_RADIUS) {
            Some(distant) => PlannerError::PowerPlantTooFarFromWater {
                distance: calculate_distance(&tile_centre(&Pos::from(&distant.position)), from),
                limit: PLANT_SITE_RADIUS,
            },
            None => PlannerError::PowerPlantNeedsWater {
                radius: PLANT_REPORT_RADIUS,
            },
        });
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
                    if let Some(plant) = fit(state, &tile_centre(&tile), facing) {
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
fn fit(state: &PlanState, pump: &Position, facing: Direction) -> Option<Plant> {
    let parts = layout(pump, facing)?;
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
    let engine_part = parts.iter().find(|part| part.name == ENGINE)?;
    let engine = engine_part.position.clone();

    let mut trial = state.fork();
    for part in &parts {
        trial.create_entity(entity_for(&trial, part));
    }
    let engine_area = trial.collision_area_facing(ENGINE, &engine, engine_part.direction)?;
    let pole = free_area_near_where(&trial, &engine, POLE, |candidate| {
        trial.pole_would_supply(POLE, candidate, &engine_area)
    })?;

    let mut parts = parts;
    parts.push(PlantPart {
        name: POLE,
        position: pole.clone(),
        direction: Direction::North,
    });
    Some(Plant {
        parts,
        boiler,
        engine,
        pole,
    })
}

/// The `FactorioEntity` a part places.
///
/// `entity_type` is read from the prototype rather than guessed: a steam
/// engine's type is `generator` and a small electric pole's is `electric-pole`,
/// neither of which is its name, and `EntityGraph::add`'s whitelist is keyed on
/// the pair.
fn entity_for(state: &PlanState, part: &PlantPart) -> FactorioEntity {
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
fn bill() -> Vec<(&'static str, u32)> {
    vec![
        (PUMP, 1),
        (PIPE, PIPE_COUNT),
        (BOILER, 1),
        (ENGINE, 1),
        (POLE, 1),
        ("coal", PLANT_COAL),
    ]
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

    for (item, count) in bill() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
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
            let parts = layout(&Position::new(0.5, 0.5), facing).expect("a cardinal layout");
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
            for part in layout(&Position::new(10.5, 10.5), facing).expect("cardinal") {
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
            let parts = layout(&Position::new(10.5, 10.5), facing).expect("cardinal");
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

    #[test]
    fn water_beyond_the_siting_radius_is_refused_with_the_distance() {
        let s = state();
        // The fixture's lake is ~54 tiles from the origin; from far enough
        // away it is still *visible* (the report radius is wider) but too far
        // to site against, and the refusal carries the number.
        let err = plan_plant(&s, &Position::new(-40., 40.))
            .expect_err("80 tiles of separation is past the limit");
        let PlannerError::PowerPlantTooFarFromWater { distance, limit } = err else {
            panic!("expected PowerPlantTooFarFromWater, got {err:?}");
        };
        assert_eq!(limit, PLANT_SITE_RADIUS);
        assert!(
            distance > PLANT_SITE_RADIUS,
            "the refusal has to name a distance that actually exceeds the limit, got {distance}"
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
}
