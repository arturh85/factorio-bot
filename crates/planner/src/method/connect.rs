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
pub fn connect_steps(
    state: &PlanState,
    from: &Position,
    to: &Position,
    item: &ItemId,
) -> Result<Vec<ActionKind>, ConnectRefusal> {
    let _ = item;

    let (area, origin) = enclosure::window(from);
    let blocked = enclosure::rasterize(
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

    let src_inserter_cell = first_free_neighbor(&blocked, origin, from_cell)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
    let belt_start_cell = first_free_neighbor(&blocked, origin, src_inserter_cell)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
    let dst_inserter_cell = first_free_neighbor(&blocked, origin, to_cell)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;
    let belt_end_cell = first_free_neighbor(&blocked, origin, dst_inserter_cell)
        .map_err(|blocked| ConnectRefusal::NoRoute { blocked })?;

    // The two inserter tiles must not become part of the belt path itself:
    // each is about to hold an inserter, not a belt, and a route that looped
    // back through one would ask for two entities on the same tile.
    let mut route_blocked = blocked.clone();
    route_blocked[enclosure::cell_index(src_inserter_cell.0, src_inserter_cell.1)] = true;
    route_blocked[enclosure::cell_index(dst_inserter_cell.0, dst_inserter_cell.1)] = true;

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
    let route = route_belt(&route_blocked, origin, belt_start_cell, belt_end_cell, None).map_err(
        |error| match error {
            RouteError::NoPath { blocked } => ConnectRefusal::NoRoute { blocked },
            RouteError::SpanTooLong { needed, max } => ConnectRefusal::SpanTooLong { needed, max },
        },
    )?;

    let belt_start_pos = enclosure::cell_to_position(origin, belt_start_cell);
    let belt_end_pos = enclosure::cell_to_position(origin, belt_end_cell);
    let src_inserter_pos = enclosure::cell_to_position(origin, src_inserter_cell);
    let dst_inserter_pos = enclosure::cell_to_position(origin, dst_inserter_cell);

    // Load: moves items from the source machine onto the belt.
    let load_facing = inserter_facing(from, &belt_start_pos).ok_or(ConnectRefusal::NotCardinal)?;
    // Unload: moves items off the belt into the destination machine.
    let unload_facing =
        inserter_facing(&belt_end_pos, to).ok_or(ConnectRefusal::NotCardinal)?;

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

        let placed: Vec<&str> = steps
            .iter()
            .filter_map(|k| match k {
                ActionKind::Place { entity } => Some(entity.name.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(
            placed.iter().filter(|n| **n == "inserter").count(),
            2,
            "one to load the belt and one to unload it: {placed:?}"
        );
        assert!(
            placed.iter().any(|n| *n == "transport-belt"),
            "and belt between them: {placed:?}"
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
}
