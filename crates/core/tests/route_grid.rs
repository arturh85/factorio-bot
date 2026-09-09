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
fn a_sealed_destination_names_its_own_walls_and_never_a_span() {
    // A wall seven cells wide across the whole window, so no surface route
    // exists and the straight line from `from` to `to` crosses a run wider
    // than any pair -- exactly the grid that used to come back as
    // `SpanTooLong { needed: 8 }`. And the destination ringed on all four
    // sides, which is what actually stops the route: no jump can land on a
    // cell it cannot leave, and no span fixes that.
    //
    // Measured in `run-1788936524-99544`: the science cell's supply chest
    // sat in the one-tile gap between the steam engine and the boiler, and
    // the refusal said "an underground span of 7 tiles" about the sink's
    // own four neighbours. Three notes read that as a wall round the SOURCE.
    let mut grid = open_grid();
    for x in 15..=21 {
        for y in 0..GRID {
            block(&mut grid, x, y);
        }
    }
    let to = (30, 10);
    for (x, y) in [(30, 9), (31, 10), (30, 11), (29, 10)] {
        block(&mut grid, x, y);
    }

    let err = route_belt(&grid, (0.0, 0.0), (10, 10), to, Some(5))
        .expect_err("a destination with every side taken has no route");

    match err {
        RouteError::NoPath { blocked } => {
            let mut named: Vec<(i64, i64)> = blocked
                .iter()
                .map(|p| (p.x().floor() as i64, p.y().floor() as i64))
                .collect();
            named.sort_unstable();
            assert_eq!(
                named,
                vec![(29, 10), (30, 9), (30, 11), (31, 10)],
                "the refusal names the four walls round the destination, not the frontier \
                 the search died on: {blocked:?}"
            );
        }
        other => panic!(
            "expected NoPath naming the sink's walls (the sink is sealed; the seven-wide \
             wall is not what stopped the route), got {other:?}"
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

/// An exit half emits onto the tile in front of it. With the wall crossed
/// eastward and the destination two rows down, the search must surface,
/// take ONE straight tile east, and only then turn -- never surface and
/// turn on the exit tile, which would leave an `output` half facing the
/// next turn and emitting onto ground no belt stands on. The launch gate
/// cannot catch this (no second jump is involved); only the exit rule does.
#[test]
fn the_tile_after_an_exit_is_straight_ahead_of_it() {
    let mut grid = open_grid();
    for y in 0..GRID {
        block(&mut grid, 12, y);
        // Every launch tile west of the wall is on row 10, so every exit
        // is at (13, 10) and the destination three rows down cannot be
        // the exit itself.
        if y != 10 {
            for x in 8..=11 {
                block(&mut grid, x, y);
            }
        }
    }
    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (13, 13), Some(5))
        .expect("the wall is crossed and the destination reached below it");
    let tiles = shape(&route);
    let exit = tiles
        .iter()
        .position(|t| t.2 == TileKind::UndergroundExit)
        .expect("surfaces");
    let entry = tiles
        .iter()
        .position(|t| t.2 == TileKind::UndergroundEntry)
        .expect("dives");
    assert_eq!(
        tiles[exit].3, tiles[entry].3,
        "the exit half faces the tunnel's direction: {tiles:?}"
    );
    let after = tiles
        .get(exit + 1)
        .expect("the exit is not the last tile here");
    assert_eq!(
        (after.0, after.1),
        (tiles[exit].0 + 1, tiles[exit].1),
        "the tile after the exit is straight ahead of it (east): {tiles:?}"
    );
}

/// The exit cell itself may not sit on an existing same-axis tunnel even
/// when nothing beneath the span is tunnelled: a two-column wall at
/// x = 12..=13 with every row tunnelled east-west at x = 14 only. Every
/// jump that clears the wall surfaces at 14, so no route exists; allowing
/// the exit there would pair the new output with whatever runs beneath.
#[test]
fn a_jump_never_surfaces_on_an_existing_tunnel_of_the_same_axis() {
    let mut grid = open_grid();
    for y in 0..GRID {
        block(&mut grid, 12, y);
        block(&mut grid, 13, y);
    }
    let mut tunnels = vec![0u8; GRID * GRID];
    for y in 0..GRID {
        tunnels[y * GRID + 14] |= TUNNEL_EW;
    }
    let err = route_belt_with_tunnels(&grid, &tunnels, (0.0, 0.0), (10, 10), (16, 10), Some(5))
        .expect_err("the only exit cells that clear the wall are tunnelled");
    assert!(matches!(err, RouteError::NoPath { .. }), "{err:?}");
}

/// A pair is a last resort, not a shortcut: one blocked cell on the line is
/// walked round (two extra tiles and some corners), never tunnelled under,
/// because a pair costs the iron of some sixteen belts and reserves the
/// ground beneath it. Priced like walking, the search would dive here.
#[test]
fn a_single_obstacle_is_walked_round_not_tunnelled_under() {
    let mut grid = open_grid();
    block(&mut grid, 12, 10);
    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), Some(5))
        .expect("open ground on either side");
    assert!(
        route.tiles.iter().all(|t| t.kind == TileKind::Belt),
        "a one-tile obstacle is a detour, not a pair: {:?}",
        shape(&route)
    );
}

/// The pocket `run-1788926478-07032` refused out of, transcribed cell for
/// cell from the grid `method::connect` handed the search at the moment of
/// the replan (the window's cells within 20 x 20 of the haul, shifted by
/// `(-20, +50)` so the start at `(10, 8)` is the record's `[30.5, -41.5]`
/// and `(5, 9)` its `[25.5, -40.5]`). A two-wide corridor runs south along
/// a one-tile belt column, sealed on every other side by the cell's own
/// coal ring, with the destination on the column's far side. The only way
/// out is a jump west across the column, and a jump is launched straight,
/// so a run heading south must hook round to face west first. The hook
/// that turns north lands its entry half on a tile the run already stands
/// on; the hook that turns south is exactly as cheap and legal. The search
/// used to return the first and then refuse, naming the doubled tile --
/// empty ground -- as "blocked".
///
/// Transcribed rather than sketched because a sketch of the same corridor
/// routed cleanly on the old search: which hook wins is a tie broken by cell
/// order, and only the real grid breaks it the wrong way.
fn sealed_corridor() -> Vec<bool> {
    let mut grid = open_grid();
    for (x, y) in [
        (2, 0),
        (3, 0),
        (4, 0),
        (5, 0),
        (6, 0),
        (14, 0),
        (15, 0),
        (16, 0),
        (2, 1),
        (3, 1),
        (4, 1),
        (5, 1),
        (6, 1),
        (7, 1),
        (8, 1),
        (9, 1),
        (10, 1),
        (11, 1),
        (12, 1),
        (14, 1),
        (15, 1),
        (16, 1),
        (4, 2),
        (5, 2),
        (6, 2),
        (7, 2),
        (8, 2),
        (12, 2),
        (14, 2),
        (15, 2),
        (16, 2),
        (4, 3),
        (5, 3),
        (6, 3),
        (7, 3),
        (8, 3),
        (9, 3),
        (12, 3),
        (14, 3),
        (15, 3),
        (16, 3),
        (4, 4),
        (5, 4),
        (6, 4),
        (7, 4),
        (8, 4),
        (9, 4),
        (10, 4),
        (11, 4),
        (12, 4),
        (14, 4),
        (15, 4),
        (16, 4),
        (4, 5),
        (5, 5),
        (6, 5),
        (7, 5),
        (9, 5),
        (14, 5),
        (15, 5),
        (16, 5),
        (4, 6),
        (5, 6),
        (6, 6),
        (7, 6),
        (9, 6),
        (10, 6),
        (11, 6),
        (12, 6),
        (14, 6),
        (15, 6),
        (16, 6),
        (4, 7),
        (5, 7),
        (6, 7),
        (7, 7),
        (9, 7),
        (12, 7),
        (14, 7),
        (15, 7),
        (16, 7),
        (5, 8),
        (7, 8),
        (9, 8),
        (10, 8),
        (11, 8),
        (12, 8),
        (13, 8),
        (14, 8),
        (15, 8),
        (16, 8),
        (6, 9),
        (7, 9),
        (8, 9),
        (9, 9),
        (11, 9),
        (12, 9),
        (15, 9),
        (16, 9),
        (7, 10),
        (8, 10),
        (9, 10),
        (12, 10),
        (13, 10),
        (14, 10),
        (15, 10),
        (16, 10),
        (7, 11),
        (8, 11),
        (9, 11),
        (16, 11),
        (9, 12),
        (16, 12),
        (9, 13),
        (16, 13),
        (9, 14),
        (16, 14),
        (9, 15),
        (11, 15),
        (16, 15),
        (9, 16),
        (16, 16),
        (9, 17),
        (10, 17),
        (11, 17),
        (12, 17),
        (13, 17),
        (14, 17),
        (15, 17),
        (16, 17),
        (9, 18),
        (11, 18),
        (9, 19),
        (11, 19),
        (12, 19),
        (15, 19),
    ] {
        block(&mut grid, x, y);
    }
    grid
}

#[test]
fn a_run_that_would_meet_itself_is_repaired_rather_than_refused() {
    let grid = sealed_corridor();
    let route = route_belt(&grid, (0.0, 0.0), (10, 8), (5, 9), Some(5))
        .expect("the column is one tile wide and a jump crosses it");

    let mut cells: Vec<(i64, i64)> = route
        .tiles
        .iter()
        .map(|t| (t.position.x().floor() as i64, t.position.y().floor() as i64))
        .collect();
    let laid = cells.len();
    cells.sort_unstable();
    cells.dedup();
    assert_eq!(
        cells.len(),
        laid,
        "a tile holds one entity, so no cell may appear twice: {:?}",
        route
            .tiles
            .iter()
            .map(|t| format!("{} {:?} {:?}", t.position, t.direction, t.kind))
            .collect::<Vec<_>>()
    );
    assert!(
        route
            .tiles
            .iter()
            .any(|t| t.kind == TileKind::UndergroundEntry),
        "the pocket is sealed on the surface, so the route tunnels"
    );
    for tile in route.tiles.iter().skip(1) {
        let (x, y) = (
            tile.position.x().floor() as usize,
            tile.position.y().floor() as usize,
        );
        assert!(
            !grid[cell_index(x, y)],
            "the route stands on free ground only: {tile:?}"
        );
    }
}

#[test]
fn a_refusal_names_the_map_and_never_the_route_itself() {
    // The same corridor with the ground west of the column walled off
    // full-height, except one cell on the direct line -- so the widest run
    // on that line is under the reach and the search falls through to
    // `NoPath` rather than `SpanTooLong`. Nothing routes: a jump can surface
    // in the gap and may then only step straight, into the wall. The tiles
    // named are obstacles on the grid, never the search's own cells.
    let mut grid = sealed_corridor();
    for x in 4..=8 {
        for y in 0..GRID {
            if (x, y) != (6, 8) {
                block(&mut grid, x, y);
            }
        }
    }
    let err = route_belt(&grid, (0.0, 0.0), (10, 8), (2, 8), Some(5))
        .expect_err("the wall west of the column has no way through");
    match err {
        RouteError::SpanTooLong { needed, max } => {
            panic!("the gap keeps the direct line's widest run under the reach: {needed} > {max}")
        }
        RouteError::NoPath { blocked } => {
            assert!(!blocked.is_empty(), "the frontier touched the wall");
            for tile in &blocked {
                let (x, y) = (tile.x().floor() as usize, tile.y().floor() as usize);
                assert!(
                    grid[cell_index(x, y)],
                    "{tile} is named as blocked and is free ground"
                );
            }
        }
    }
}
