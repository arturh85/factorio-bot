use factorio_bot_core::graph::enclosure::{GRID, cell_index};
use factorio_bot_core::graph::route::{RouteError, TileKind, route_belt};
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

    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), Some(4))
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

    let err = route_belt(&grid, (0.0, 0.0), (10, 10), (22, 10), Some(4))
        .expect_err("a nine-tile wall is wider than a span of four");

    match err {
        RouteError::SpanTooLong { max, .. } => assert_eq!(max, 4),
        other => panic!("expected SpanTooLong, got {other:?}"),
    }
}

#[test]
fn undergrounds_are_not_used_when_the_surface_is_open() {
    let grid = open_grid();
    let route = route_belt(&grid, (0.0, 0.0), (10, 10), (14, 10), Some(4))
        .expect("open ground routes on the surface");
    assert!(
        route.tiles.iter().all(|t| t.kind == TileKind::Belt),
        "an underground pair costs 2 belts' worth of iron for nothing here"
    );
}
