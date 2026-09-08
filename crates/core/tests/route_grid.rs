use factorio_bot_core::graph::enclosure::{GRID, cell_index};
use factorio_bot_core::graph::route::{
    RouteError, TUNNEL_EW, TUNNEL_NS, TileKind, route_belt, route_belt_with_tunnels,
};
use factorio_bot_core::types::Direction;

/// An empty grid: every cell free.
fn open_grid() -> Vec<bool> {
    vec![false; GRID * GRID]
}

/// Block one cell.
fn block(grid: &mut [bool], x: usize, y: usize) {
    grid[cell_index(x, y)] = true;
}

#[test]
fn a_clear_line_is_straight_and_faces_the_destination() {
    let grid = open_grid();
    let route =
        route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), None).expect("an empty grid has a route");

    assert_eq!(route.tiles.len(), 5, "five tiles inclusive of both ends");
    assert!(
        route.tiles.iter().all(|t| t.direction == Direction::East),
        "a straight eastward run faces east the whole way: {:?}",
        route.tiles.iter().map(|t| t.direction).collect::<Vec<_>>()
    );
    assert!(
        route.tiles.iter().all(|t| t.kind == TileKind::Belt),
        "no undergrounds are needed on open ground"
    );
}

#[test]
fn a_blocked_line_goes_around_and_says_so_in_its_directions() {
    let mut grid = open_grid();
    block(&mut grid, 12, 10);

    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), None)
        .expect("one blocked cell has a way around it");

    assert!(
        route
            .tiles
            .iter()
            .all(|t| t.position.x() != 12.5 || t.position.y() != 10.5),
        "the route must not cross the blocked cell"
    );
    assert!(
        route.tiles.iter().any(|t| t.direction != Direction::East),
        "going around means at least one tile turns"
    );
}

#[test]
fn a_walled_destination_is_refused_by_name() {
    let mut grid = open_grid();
    // A closed ring around (12, 10), leaving the destination cell itself
    // free -- a walled destination, not a solid one.
    for (x, y) in [
        (11, 9),
        (12, 9),
        (13, 9),
        (11, 10),
        (13, 10),
        (11, 11),
        (12, 11),
        (13, 11),
    ] {
        block(&mut grid, x, y);
    }

    let err = route_belt(&grid, (0.0, 0.0), (10, 10), (12, 10), None)
        .expect_err("a destination behind a wall has no surface route");

    match err {
        RouteError::NoPath { blocked } => assert!(
            !blocked.is_empty(),
            "a refusal must name the tiles that stopped it"
        ),
        other => panic!("expected NoPath, got {other:?}"),
    }
}

#[test]
fn a_directly_blocked_destination_is_refused() {
    let mut grid = open_grid();
    block(&mut grid, 14, 10);

    let err = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), None)
        .expect_err("a destination that is itself an obstacle has no route");

    match err {
        RouteError::NoPath { blocked } => assert!(
            !blocked.is_empty(),
            "a refusal must name the tiles that stopped it"
        ),
        other => panic!("expected NoPath, got {other:?}"),
    }
}

#[test]
fn equal_length_routes_prefer_the_straight_one_over_a_zigzag() {
    let mut grid = open_grid();
    // (10,10) to (14,14): every eight-edge monotone path is equally short.
    // Blocking both corners a single-turn "L" could pivot on -- (14,10) for
    // east-then-south, (10,14) for south-then-east -- removes every 1-turn
    // option, so the cheapest remaining shape is a two-turn jog, tied in
    // length with a worse three-turn alternative. Without the turn penalty
    // this repo's own search picks the three-turn one (verified by hand:
    // setting TURN_PENALTY to 0 turns this same fixture into 3 turns), so
    // this is a real discriminator, not a fixture that happens to pass
    // either way.
    for (x, y) in [(14, 10), (10, 14)] {
        block(&mut grid, x, y);
    }

    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 14), None)
        .expect("blocking both L-corners still leaves a jogged route");

    assert_eq!(
        route.tiles.len(),
        9,
        "still the shortest possible length, eight edges"
    );
    let turns = route
        .tiles
        .windows(2)
        .filter(|w| w[0].direction != w[1].direction)
        .count();
    assert_eq!(
        turns,
        2,
        "with both straight-L corners blocked, the cheapest equal-length \
         route jogs once each way (2 turns); the turn penalty is what rules \
         out the equally-short 3-turn alternative: {:?}",
        route.tiles.iter().map(|t| t.direction).collect::<Vec<_>>()
    );
}

#[test]
fn a_wall_is_crossed_underground_when_the_surface_cannot_go_round() {
    let mut grid = open_grid();
    // A full-height wall at x = 12: no surface route exists at all.
    for y in 0..GRID {
        block(&mut grid, 12, y);
    }

    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), Some(5))
        .expect("an underground pair crosses a one-tile wall");

    assert_eq!(
        route
            .tiles
            .iter()
            .filter(|t| t.kind == TileKind::UndergroundEntry)
            .count(),
        1,
        "exactly one entry"
    );
    assert_eq!(
        route
            .tiles
            .iter()
            .filter(|t| t.kind == TileKind::UndergroundExit)
            .count(),
        1,
        "exactly one exit"
    );
    assert!(
        route.tiles.iter().all(|t| t.position.x() != 12.5),
        "nothing is placed inside the wall"
    );
}

#[test]
fn a_wall_wider_than_the_prototype_allows_is_refused_by_span() {
    let mut grid = open_grid();
    for x in 12..=20 {
        for y in 0..GRID {
            block(&mut grid, x, y);
        }
    }

    let err = route_belt(&grid, (0.0, 0.0), (10, 10), (22, 10), Some(5))
        .expect_err("a nine-tile wall is wider than a reach of five");

    match err {
        RouteError::SpanTooLong { needed, max } => {
            assert_eq!(max, 5, "the prototype's own number, untranslated");
            assert_eq!(
                needed, 10,
                "the wall is nine columns wide (x = 12..=20), so the shortest pair that \
                 crosses it is ten apart -- the same unit as `max`"
            );
        }
        other => panic!("expected SpanTooLong, got {other:?}"),
    }
}

#[test]
fn undergrounds_are_not_used_when_the_surface_is_open() {
    let grid = open_grid();
    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), Some(5))
        .expect("open ground routes on the surface");
    assert!(
        route.tiles.iter().all(|t| t.kind == TileKind::Belt),
        "an underground pair costs 2 belts' worth of iron for nothing here"
    );
}

#[test]
fn a_wall_exactly_as_wide_as_the_prototype_allows_still_succeeds() {
    let mut grid = open_grid();
    // Four blocked columns: the widest wall a `max_underground_distance` of
    // five can still cross (entry at x = 11, exit at x = 16 -- five apart,
    // four hidden tiles in between). This is the basic `underground-belt`
    // on this install, whose prototype reads 5 and whose in-game reach is
    // "four tiles": the prototype counts entry-to-exit, the game counts what
    // is hidden, and this test pins which one the parameter is.
    for x in 12..=15 {
        for y in 0..GRID {
            block(&mut grid, x, y);
        }
    }

    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (17, 10), Some(5))
        .expect("a four-tile wall is exactly what a reach of five can cross");

    assert_eq!(
        route
            .tiles
            .iter()
            .filter(|t| t.kind == TileKind::UndergroundEntry)
            .count(),
        1
    );
    assert_eq!(
        route
            .tiles
            .iter()
            .filter(|t| t.kind == TileKind::UndergroundExit)
            .count(),
        1
    );
    assert!(
        (12..=15).all(|x| route.tiles.iter().all(|t| t.position.x() != x as f64 + 0.5)),
        "nothing is placed inside the wall"
    );
}

#[test]
fn span_too_long_is_withheld_when_undergrounds_cannot_be_the_reason() {
    // The destination cell itself is the obstacle. No span, however long,
    // can fix that -- an underground's exit must land on a free cell, so
    // enabling undergrounds must not turn this into a fabricated
    // `SpanTooLong`; it is still `NoPath`.
    let mut grid = open_grid();
    block(&mut grid, 14, 10);

    let err = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), Some(5))
        .expect_err("a destination that is itself blocked has no route, underground or not");

    match err {
        RouteError::NoPath { blocked } => assert!(
            !blocked.is_empty(),
            "a refusal must name the tiles that stopped it"
        ),
        other => panic!(
            "expected NoPath (the obstacle is the destination cell itself, not a span the \
             prototype can't cross), got {other:?}"
        ),
    }
}

#[test]
fn adjacent_jumps_cannot_share_a_tile_as_both_exit_and_entry() {
    // Two three-tile walls separated by a single free column at x = 15.
    // Neither wall alone needs more than a span of four, but there is no
    // room for a real surface tile between the two undergrounds it would
    // take to cross both: the only free column between the walls is the
    // one tile wide. Before the chaining gate existed, the search happily
    // launched a second jump directly from the first jump's exit tile,
    // silently overwriting that tile's `UndergroundExit` role with
    // `UndergroundEntry` on reconstruction -- entry and exit counts came out
    // unequal, i.e. an entry with no matching exit. Both walls span the
    // grid's full height, so the only way through is some underground
    // arrangement; there being an actual route at all already exercises the
    // one-real-tile-between-jumps requirement, and the entry/exit count
    // check below is what the missing gate broke.
    //
    // The gap is TWO columns (x = 15, 16), not one. An exit half emits onto
    // the tile in front of it, so the tile after an exit must be a belt (or
    // the next entry) in the same direction; with a one-column gap the only
    // way on was to surface at x = 15 and turn along the wall, which the
    // game cannot do and which the search used to accept. Now exit at 15
    // feeds the entry at 16 head-on, which the game does accept.
    let mut grid = open_grid();
    for x in [12, 13, 14, 17, 18, 19] {
        for y in 0..GRID {
            block(&mut grid, x, y);
        }
    }

    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (21, 10), Some(5))
        .expect("a route exists: exit at 15, entry at 16, no shared tile");

    let entries = route
        .tiles
        .iter()
        .filter(|t| t.kind == TileKind::UndergroundEntry)
        .count();
    let exits = route
        .tiles
        .iter()
        .filter(|t| t.kind == TileKind::UndergroundExit)
        .count();
    assert_eq!(
        entries,
        exits,
        "every underground entry must have a matching exit -- got {entries} entries and \
         {exits} exits: {:?}",
        route
            .tiles
            .iter()
            .map(|t| (t.position.x(), t.position.y(), t.kind))
            .collect::<Vec<_>>()
    );
    assert_eq!(entries, 2, "both walls are crossed underground");
    for x in [12, 13, 14, 17, 18, 19] {
        assert!(
            route.tiles.iter().all(|t| t.position.x() != x as f64 + 0.5),
            "nothing is placed inside either wall"
        );
    }
}

/// Every tile of a route as `(x cell, y cell, kind, direction)`.
fn shape(
    route: &factorio_bot_core::graph::route::Route,
) -> Vec<(usize, usize, TileKind, Direction)> {
    route
        .tiles
        .iter()
        .map(|t| {
            (
                t.position.x() as usize,
                t.position.y() as usize,
                t.kind,
                t.direction,
            )
        })
        .collect()
}

/// A one-column gap between two full-height walls is NOT a route: the exit
/// at x = 15 would have to turn, and an output half emits straight ahead.
#[test]
fn a_route_cannot_turn_on_the_tile_after_an_exit() {
    let mut grid = open_grid();
    for x in [12, 13, 14, 16, 17, 18] {
        for y in 0..GRID {
            block(&mut grid, x, y);
        }
    }
    let err = route_belt(&grid, (0.0, 0.0), (10, 10), (20, 10), Some(5))
        .expect_err("surfacing at x = 15 leaves nowhere straight to go");
    assert!(
        matches!(err, RouteError::NoPath { .. }),
        "no wall here is wider than the reach, so this is NoPath, not SpanTooLong: {err:?}"
    );
}

/// The belt before an entry must feed it from behind. The start is walled
/// into the column x = 10 (walls at x = 9 and x = 12), so the only way to
/// the wall is a northward run along x = 10; crossing it eastward means a
/// corner belt at (10, 10) facing east, ONE straight tile at (11, 10), and
/// the dive from there. The search must never dive straight out of the
/// northward-facing state at (10, 10) with the entry at (11, 10) fed from
/// its side.
#[test]
fn a_jump_is_launched_only_from_a_tile_already_facing_its_way() {
    let mut grid = open_grid();
    for y in 0..GRID {
        block(&mut grid, 9, y);
        block(&mut grid, 12, y);
    }
    let route = route_belt(&grid, (0.0, 0.0), (10, 14), (14, 10), Some(5))
        .expect("north along x = 10, corner, one straight tile, then east under the wall");
    let tiles = shape(&route);
    let entry = tiles
        .iter()
        .position(|t| t.2 == TileKind::UndergroundEntry)
        .expect("the wall is crossed underground");
    assert!(entry > 0, "the entry is not the start tile");
    let before = tiles[entry - 1];
    assert_eq!(
        (before.0, before.1),
        (tiles[entry].0 - 1, tiles[entry].1),
        "the tile before the entry is directly behind it: {tiles:?}"
    );
    assert_eq!(
        before.3,
        Direction::East,
        "and already faces the tunnel's direction, so the entry is fed from behind: {tiles:?}"
    );
    assert_eq!(tiles[entry].3, Direction::East);
    let exit = tiles
        .iter()
        .position(|t| t.2 == TileKind::UndergroundExit)
        .expect("and surfaces");
    assert_eq!(
        tiles[exit].3,
        Direction::East,
        "the exit half carries the tunnel's direction, not the next turn's"
    );
}

/// An existing east-west tunnel under x = 12..=16 on row 10: a new east-west
/// jump along that row would pair with it, so it is refused; the search
/// goes round instead when it can, and refuses when it cannot.
#[test]
fn a_jump_never_runs_along_an_existing_tunnel_of_the_same_axis() {
    let mut grid = open_grid();
    for y in 0..GRID {
        block(&mut grid, 13, y);
    }
    let mut tunnels = vec![0u8; GRID * GRID];
    // Every row is tunnelled east-west across the wall: no east-west jump
    // anywhere on the grid may cross it.
    for y in 0..GRID {
        for x in 12..=16 {
            tunnels[y * GRID + x] |= TUNNEL_EW;
        }
    }
    let err = route_belt_with_tunnels(&grid, &tunnels, (0.0, 0.0), (10, 10), (18, 10), Some(5))
        .expect_err("the wall is crossable only along an axis already tunnelled");
    assert!(matches!(err, RouteError::NoPath { .. }), "{err:?}");

    // The same wall with a NORTH-SOUTH tunnel under it is crossed freely:
    // the game lets pairs cross at right angles.
    let mut across = vec![0u8; GRID * GRID];
    for y in 0..GRID {
        for x in 12..=16 {
            across[y * GRID + x] |= TUNNEL_NS;
        }
    }
    let route = route_belt_with_tunnels(&grid, &across, (0.0, 0.0), (10, 10), (18, 10), Some(5))
        .expect("a perpendicular tunnel is not in the way");
    assert_eq!(
        route
            .tiles
            .iter()
            .filter(|t| t.kind != TileKind::Belt)
            .count(),
        2,
        "one pair crosses the wall"
    );
}
