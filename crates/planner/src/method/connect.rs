//! Connecting two entities with a belt, and the inserters at each end.

use crate::action::ActionKind;
use crate::enclosure;
use crate::ids::ItemId;
use crate::state::PlanState;
use factorio_bot_core::graph::enclosure::GRID;
use factorio_bot_core::graph::route::{RouteError, TileKind, route_belt};
use factorio_bot_core::types::{Direction, FactorioEntity, Position};

/// The `direction` an inserter must carry to move an item from `from` to `to`.
///
/// **It names the side it picks up from.** See the test; this is the one
/// place in the planner that knows it, so that no caller has to.
pub fn inserter_facing(from: &Position, to: &Position) -> Option<Direction> {
    let dx = to.x() - from.x();
    let dy = to.y() - from.y();
    if dx.abs() > f64::EPSILON && dy.abs() > f64::EPSILON {
        return None;
    }
    if dx.abs() > f64::EPSILON {
        // Moving east means picking up from the west.
        return Some(if dx > 0.0 {
            Direction::West
        } else {
            Direction::East
        });
    }
    if dy.abs() > f64::EPSILON {
        // Screen coordinates: +y is south. Moving south picks up from north.
        return Some(if dy > 0.0 {
            Direction::North
        } else {
            Direction::South
        });
    }
    None
}

/// Why a connection could not be made. **Every variant is returned before
/// anything is placed** -- a half-built belt run is worse than no belt run,
/// because the items sit on it and the bot that would carry them is gone.
#[derive(Debug, Clone)]
pub enum ConnectRefusal {
    /// No inserter tile, no belt endpoint next to it, or no belt route
    /// between the two endpoints could be found. `blocked` names whichever
    /// positions stood in the way -- the occupied neighbours that stopped a
    /// tile search, or the obstacles `route_belt` itself reports.
    NoRoute { blocked: Vec<Position> },
    /// An underground span longer than the belt prototype allows.
    ///
    /// Structurally unreachable while `connect_steps` always calls
    /// `route_belt` with `max_underground: None` (RULING 2 of the task-4
    /// brief) -- with `None`, `route_belt` never attempts an underground
    /// move at all, so it can never report one being too long either. Kept
    /// so this type matches `RouteError` one-to-one rather than silently
    /// dropping a variant a future change to that call might need.
    SpanTooLong { needed: u32, max: u8 },
    /// An inserter's own machine and its belt tile were not aligned on one
    /// axis -- an inserter only ever moves items between two tiles directly
    /// opposite each other, and this pair is not that.
    NotCardinal,
}

/// The four cardinal offsets, checked in this fixed order everywhere a tile is
/// chosen below: North, East, South, West. Fixed so that the tile a caller
/// gets for a given obstacle layout is always the same tile, never an
/// artefact of iteration order.
const NESW: [(i64, i64); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// The cell `at` falls in, in the window whose origin corner is `origin` --
/// or `None` when it lies outside that window's `GRID x GRID` cells
/// altogether, which is what it means to ask `connect_steps` to join two
/// points too far apart for one `enclosure::window` (centred on `from`) to
/// model both of.
fn cell_of(origin: (f64, f64), at: &Position) -> Option<(usize, usize)> {
    let x = (at.x() - origin.0) / enclosure::CELL;
    let y = (at.y() - origin.1) / enclosure::CELL;
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let (x, y) = (x as usize, y as usize);
    (x < GRID && y < GRID).then_some((x, y))
}

/// The first free cardinal neighbour of `cell`, in fixed North/East/South/
/// West order.
///
/// `Err` names every neighbour that stopped it -- in range and blocked. An
/// off-grid neighbour is skipped rather than named: it has no position this
/// window can state.
fn first_free_neighbor(
    blocked: &[bool],
    origin: (f64, f64),
    cell: (usize, usize),
) -> Result<(usize, usize), Vec<Position>> {
    let mut stopped = Vec::new();
    for (dx, dy) in NESW {
        let x = cell.0 as i64 + dx;
        let y = cell.1 as i64 + dy;
        if x < 0 || y < 0 || x >= GRID as i64 || y >= GRID as i64 {
            continue;
        }
        let (x, y) = (x as usize, y as usize);
        if blocked[enclosure::cell_index(x, y)] {
            stopped.push(enclosure::cell_to_position(origin, (x, y)));
        } else {
            return Ok((x, y));
        }
    }
    Err(stopped)
}

/// Connect `from` to `to` with a belt run and the inserter at each end that
/// loads and unloads it.
///
/// # The geometry (RULING 1 of the task-4 brief)
///
/// `from` and `to` are the two **machines'** own positions -- occupied tiles,
/// so neither an inserter nor a belt can stand there. Four tiles are derived
/// instead, each the first free cardinal neighbour (North, East, South, West)
/// of the one before it: an inserter adjacent to `from`, a belt tile adjacent
/// to that inserter, a belt tile adjacent to the destination inserter, and an
/// inserter adjacent to `to`. Only the two belt tiles are routed between; the
/// built line reads `machine | inserter | belt ... belt | inserter |
/// machine`. Deterministic by construction: the same obstacle layout always
/// yields the same four tiles.
///
/// `item` decides nothing about the geometry above; it is the caller's label
/// for what the belt is expected to carry.
///
/// # Scope: only the base world is an obstacle
///
/// Obstacles come from `state.base().entity_graph.blocking_boxes_within`
/// alone -- the world as the game (or a fixture) reported it, never this
/// plan's own `added` overlay. A caller chaining two connections in the same
/// plan must not treat the first call's output as ground truth for the
/// second: this function cannot see a machine, inserter or belt that an
/// earlier action in the *same plan* placed, so nothing here stops the two
/// from overlapping. Widening this to see `added` too is future work, not a
/// guarantee this function already makes.
pub fn connect_steps(
    state: &PlanState,
    from: &Position,
    to: &Position,
    item: &ItemId,
) -> Result<Vec<ActionKind>, ConnectRefusal> {
    let _ = item;

    let (area, origin) = enclosure::window(from);
    // `mut`: the four tile selections below claim their cell onto this same
    // grid the moment each is chosen -- see the comment there for why.
    let mut blocked = enclosure::rasterize(
        state
            .base()
            .entity_graph
            .blocking_boxes_within(&area)
            .into_iter(),
        origin,
        (0.0, 0.0),
    );

    let from_cell = cell_of(origin, from).ok_or_else(|| ConnectRefusal::NoRoute {
        blocked: vec![from.clone()],
    })?;
    let to_cell = cell_of(origin, to).ok_or_else(|| ConnectRefusal::NoRoute {
        blocked: vec![to.clone()],
    })?;

    // Each of the next four tiles is claimed onto `blocked` as soon as it is
    // chosen, so every later search sees what every earlier one already
    // took. Without this, each of the four is "the first free neighbour of
    // X" against the *same static* grid, and none of the four knows what the
    // other three claimed -- in tight geometry two of them could pick the
    // same empty tile, and this function would go on to emit two `Place`
    // actions for it (a belt tile from one machine's route and an inserter
    // from the other's). Claiming turns that collision into a refusal
    // instead: the tile is no longer "free" for whichever search asks next,
    // so it is forced to keep looking, and if nothing is left it refuses via
    // `first_free_neighbor`'s `Err` exactly as an ordinary blocked tile
    // would.
    let src_inserter_cell = first_free_neighbor(&blocked, origin, from_cell)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
    blocked[enclosure::cell_index(src_inserter_cell.0, src_inserter_cell.1)] = true;

    let belt_start_cell = first_free_neighbor(&blocked, origin, src_inserter_cell)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
    blocked[enclosure::cell_index(belt_start_cell.0, belt_start_cell.1)] = true;

    let dst_inserter_cell = first_free_neighbor(&blocked, origin, to_cell)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
    blocked[enclosure::cell_index(dst_inserter_cell.0, dst_inserter_cell.1)] = true;

    let belt_end_cell = first_free_neighbor(&blocked, origin, dst_inserter_cell)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
    // `belt_end_cell` itself is not claimed: nothing further is derived from
    // it, so there is nothing left to protect it from. `route_belt` below
    // does not need it excluded either -- see the comment at that call.

    // `blocked` now also carries `src_inserter_cell` and `belt_start_cell`
    // and `dst_inserter_cell` as obstacles, which is exactly what the route
    // search needs on top of the base obstacles: each of those three is
    // about to hold an inserter or already IS the route's own start, not a
    // *new* belt tile, and a route that looped back through one would ask
    // for two entities on the same tile. `route_belt` never blocks its own
    // `from`/`to` cells regardless of what the grid says there (it seeds
    // both endpoints' initial states unconditionally and only consults
    // `blocked` for cells it steps *into*), so `belt_start_cell` reading
    // blocked here does not stop the search from leaving it, and
    // `belt_end_cell` reading free does not stop the search from reaching
    // it.
    //
    // RULING 2: undergrounds are never emitted on this branch, always and
    // deliberately -- do not "fix" this by threading a real
    // `max_underground` through. A Factorio underground-belt pair needs one
    // half `input` and one half `output`; `FactorioEntity` has no field to
    // record which, and the mod's `rcon_place_entity(player_id, item_name,
    // position, direction)` has no argument for it either. Emitting two
    // identical halves would place both perfectly and move nothing -- this
    // project's defining failure. A route that would need to go underground
    // must refuse instead, which passing `None` here guarantees: `route_belt`
    // never attempts an underground move without a `max_underground`.
    let route =
        route_belt(&blocked, origin, belt_start_cell, belt_end_cell, None).map_err(|error| {
            match error {
                RouteError::NoPath { blocked } => ConnectRefusal::NoRoute { blocked },
                RouteError::SpanTooLong { needed, max } => {
                    ConnectRefusal::SpanTooLong { needed, max }
                }
            }
        })?;

    let belt_start_pos = enclosure::cell_to_position(origin, belt_start_cell);
    let belt_end_pos = enclosure::cell_to_position(origin, belt_end_cell);
    let src_inserter_pos = enclosure::cell_to_position(origin, src_inserter_cell);
    let dst_inserter_pos = enclosure::cell_to_position(origin, dst_inserter_cell);

    // Load: moves items from the source machine onto the belt.
    let load_facing = inserter_facing(from, &belt_start_pos).ok_or(ConnectRefusal::NotCardinal)?;
    // Unload: moves items off the belt into the destination machine.
    let unload_facing = inserter_facing(&belt_end_pos, to).ok_or(ConnectRefusal::NotCardinal)?;

    let mut steps = Vec::with_capacity(route.tiles.len() + 2);
    steps.push(ActionKind::Place {
        entity: Box::new(FactorioEntity::new_inserter(&src_inserter_pos, load_facing)),
    });
    for tile in &route.tiles {
        let entity = match tile.kind {
            TileKind::Belt => FactorioEntity::new_transport_belt(&tile.position, tile.direction),
            // See the comment at the `route_belt` call above: with
            // `max_underground: None` the search can never take the branch
            // that produces these, so this arm can never run.
            TileKind::UndergroundEntry | TileKind::UndergroundExit => unreachable!(
                "connect_steps always calls route_belt with max_underground: None, \
                 so it can never emit an underground route tile"
            ),
        };
        steps.push(ActionKind::Place {
            entity: Box::new(entity),
        });
    }
    steps.push(ActionKind::Place {
        entity: Box::new(FactorioEntity::new_inserter(
            &dst_inserter_pos,
            unload_facing,
        )),
    });

    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::num_traits::ToPrimitive;
    use factorio_bot_core::types::{Direction, Position};

    /// **The whole reason this function exists.** An inserter's `direction`
    /// names the side it PICKS UP from, not the side it drops into --
    /// established empirically here (chest / burner-inserter / chest, then
    /// machine / inserter / chest). `direction = 12` ("west") is what moves
    /// items *west to east*. Getting it backwards produces a layout that
    /// places 100% correctly, passes every geometry check, and does
    /// absolutely nothing.
    #[test]
    fn an_inserter_faces_the_side_it_picks_up_from() {
        let west = Position::new(0.5, 0.5);
        let east = Position::new(2.5, 0.5);
        assert_eq!(
            inserter_facing(&west, &east),
            Some(Direction::West),
            "moving items west -> east means facing WEST, the pickup side"
        );
        assert_eq!(
            inserter_facing(&east, &west),
            Some(Direction::East),
            "and the reverse faces east"
        );
    }

    #[test]
    fn a_diagonal_has_no_inserter_facing() {
        assert_eq!(
            inserter_facing(&Position::new(0.5, 0.5), &Position::new(2.5, 2.5)),
            None,
            "an inserter is cardinal; a diagonal is a caller bug, not a default"
        );
    }

    /// **Defends the sign convention.** Screen coordinates here use +y for
    /// south, not north. A future edit that inverts the north/south directions
    /// would pass the horizontal tests but fail this one.
    #[test]
    fn an_inserter_picks_up_from_north_when_moving_south_in_screen_coordinates() {
        let north = Position::new(0.5, 0.5);
        let south = Position::new(0.5, 2.5);
        assert_eq!(
            inserter_facing(&north, &south),
            Some(Direction::North),
            "moving items north -> south (positive y) means facing NORTH, the pickup side"
        );
        assert_eq!(
            inserter_facing(&south, &north),
            Some(Direction::South),
            "and the reverse faces south"
        );
    }

    /// The bill is the deliverable: a caller has to be able to refuse the
    /// whole plan before a single belt is placed, which is what the
    /// preconditions on these actions are for.
    ///
    /// **Asserts position AND direction, not just "an inserter exists at
    /// each end".** A name-only count cannot catch a regression that swaps
    /// the two `inserter_facing` calls, or feeds either the wrong pair of
    /// positions: that would still place one inserter at each end and pass a
    /// count-only check, while producing exactly the layout that places
    /// perfectly and moves nothing -- this project's defining failure.
    ///
    /// The expected numbers are traced by hand against
    /// `open_world_with_two_machines`'s exact geometry: both furnaces block
    /// only the single cell they are centred on (see that fixture's own doc
    /// comment), so North is free at every step and both machines' chains go
    /// straight north. Load inserter: north of `(0.5, 0.5)` is `(0.5,
    /// -0.5)`, and it moves items from the machine (south of it, `dy < 0`
    /// picks up from the south) -- direction `South`. Unload inserter: north
    /// of `(6.5, 0.5)` is `(6.5, -0.5)`, and it moves items into the machine
    /// (south of it too, but as the *destination* this time, so the belt is
    /// north and `dy > 0` picks up from the north) -- direction `North`.
    #[test]
    fn a_connection_places_belts_and_an_inserter_at_each_end() {
        let state = crate::test_world::open_world_with_two_machines();
        let steps = connect_steps(
            &state,
            &Position::new(0.5, 0.5),
            &Position::new(6.5, 0.5),
            &"iron-ore".into(),
        )
        .expect("open ground between two machines connects");

        let inserters: Vec<(Position, u8)> = steps
            .iter()
            .filter_map(|k| match k {
                ActionKind::Place { entity } if entity.name == "inserter" => {
                    Some((entity.position.clone(), entity.direction))
                }
                _ => None,
            })
            .collect();

        assert_eq!(
            inserters,
            vec![
                (
                    Position::new(0.5, -0.5),
                    Direction::South.to_u8().expect("South is representable")
                ),
                (
                    Position::new(6.5, -0.5),
                    Direction::North.to_u8().expect("North is representable")
                ),
            ],
            "load inserter south of its machine facing South (picks up from \
             the machine), unload inserter north of its machine facing North \
             (picks up from the belt): {inserters:?}"
        );
        assert!(
            steps.iter().any(|k| matches!(
                k,
                ActionKind::Place { entity } if entity.name == "transport-belt"
            )),
            "and belt between them"
        );
    }

    #[test]
    fn a_walled_destination_refuses_and_places_nothing() {
        let state = crate::test_world::two_machines_behind_a_wall();
        let refusal = connect_steps(
            &state,
            &Position::new(0.5, 0.5),
            &Position::new(6.5, 0.5),
            &"iron-ore".into(),
        )
        .expect_err("a walled destination has no route");
        assert!(matches!(refusal, ConnectRefusal::NoRoute { .. }));
    }

    /// **The IMPORTANT-2 regression test.** Each of the four derived tiles is
    /// found by a search that does not know what the other three claimed --
    /// only the two inserter cells are excluded before the belt is routed,
    /// and the route search does not test its own start cell either. In
    /// tight geometry two of the four searches can land on the very same
    /// tile, and without a distinctness check `connect_steps` would emit two
    /// `Place` actions for it: a belt from one machine's route and an
    /// inserter from the other's.
    ///
    /// `two_machines_sharing_a_neighbour` engineers exactly that: the second
    /// furnace's only unwalled cardinal neighbour is the very cell the first
    /// furnace's own belt already claims. With the four selections aware of
    /// each other, the second furnace's inserter search finds that cell
    /// already taken, has nowhere else to go, and refuses -- it must not
    /// silently accept the shared tile instead.
    #[test]
    fn machines_close_enough_to_share_a_derived_tile_refuse_instead_of_colliding() {
        let state = crate::test_world::two_machines_sharing_a_neighbour();
        let refusal = connect_steps(
            &state,
            &Position::new(0.5, 0.5),
            &Position::new(0.5, -2.5),
            &"iron-ore".into(),
        )
        .expect_err(
            "every neighbour of the second furnace is either walled or \
             already claimed by the first furnace's own belt",
        );
        assert!(matches!(refusal, ConnectRefusal::NoRoute { .. }));
    }
}
