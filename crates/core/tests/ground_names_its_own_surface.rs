//! **Ground says which surface it came from, and the parser routes it there.**
//!
//! `tiles` was the last position-keyed writeout that could not name a surface.
//! Its header was `x1,y1;x2,y2: name:0,name:1,...` -- two positional fields and
//! no third -- so `OutputParser` filled `FactorioTile::surface` with `None` and
//! every tile landed in the default surface's `tile_tree`, keyed by position
//! alone. The single thing standing between that and silent aliasing was the
//! mod's Nauvis guard in `on_chunk_generated`, which drops a chunk from any
//! other surface before `writeout_tiles` is ever called.
//!
//! Since 2026-09-07 the header carries a third `;`-separated field
//! (`ground_header` in `mods/BotBridge/control.lua`) and the parser routes on
//! it through the same `route` the four `FactorioEntity`-shaped writeouts use.
//!
//! ## Two things these tests are built to avoid
//!
//! **Vacuity.** The equivalent deletion test was found on 2026-09-06 to pass
//! for the wrong reason: it paired "Nauvis kept its chest" with "platform-4 is
//! empty there", and platform-4 had never been occupied at that tile, so a
//! deletion that routed correctly and then did nothing satisfied both halves.
//! Every "the other surface does not have it" assertion here is therefore
//! paired with a positive one from the same computation, and **the other
//! surface genuinely holds a different tile at the very same coordinates** --
//! so an absence is an absence of *that name*, not an absence of ingest.
//!
//! **Silence on an older sender.** A two-field header must keep parsing and
//! must yield `None`, which is what every archived server log and both world
//! dumps (`workspace/scripts/map.json`, `map-31337-explored.json`) contain.
//! `None` is "nobody said", never "an unknown surface".

use factorio_bot_core::factorio::world::{FactorioSurface, FactorioWorld};
use factorio_bot_core::process::output_parser::OutputParser;
use factorio_bot_core::types::{Position, Rect, SurfaceId};
use std::sync::Arc;

/// A `tiles` writeout body. `surface` absent is an older mod.
///
/// Positions are derived by the parser from the header's `left_top` plus the
/// index, 32 to a row, so `tiles[0]` lands on the chunk corner and `tiles[1]`
/// one tile east of it.
fn tiles_line(surface: Option<&str>, tiles: &[(&str, bool)]) -> String {
    let head = match surface {
        Some(s) => format!("0,0;32,32;{s}: "),
        None => "0,0;32,32: ".to_string(),
    };
    let body: Vec<String> = tiles
        .iter()
        .map(|(name, collides)| format!("{name}:{}", u8::from(*collides)))
        .collect();
    format!("{head}{}", body.join(","))
}

/// The names the surface's tile tree holds at `(0, 0)`.
fn names_at_origin(surface: &FactorioSurface) -> Vec<String> {
    let bounds = Rect::new(&Position::new(0.1, 0.1), &Position::new(0.9, 0.9));
    surface
        .entity_graph
        .tiles_within(&bounds)
        .into_iter()
        .map(|t| t.name)
        .collect()
}

fn world() -> Arc<FactorioWorld> {
    Arc::new(FactorioWorld::nauvis_only(Arc::new(FactorioSurface::new())))
}

/// **The aliasing case, which is the reason `FactorioWorld` exists.**
///
/// Water at (0, 0) on Nauvis and grass at (0, 0) on Vulcanus are two tiles.
/// Both surfaces are occupied at that coordinate, so "Vulcanus does not hold
/// water" is a statement about routing and not about an empty graph.
#[test]
fn a_tile_at_the_same_coordinate_on_two_surfaces_does_not_collide() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(10, "tiles", &tiles_line(Some("nauvis"), &[("water", true)]))
        .expect("the nauvis tiles line must parse");
    parser
        .parse(
            10,
            "tiles",
            &tiles_line(Some("vulcanus"), &[("grass-1", false)]),
        )
        .expect("the vulcanus tiles line must parse");

    let nauvis = world.nauvis().expect("nauvis must be held");
    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must have been created by the routing");

    assert_eq!(
        names_at_origin(&nauvis),
        vec!["water".to_string()],
        "the nauvis tile must land on nauvis, and nothing else may"
    );
    assert_eq!(
        names_at_origin(&vulcanus),
        vec!["grass-1".to_string()],
        "and the vulcanus tile on vulcanus, at the same coordinate"
    );
}

/// The same fact read through a **consequential** query rather than a name
/// list: `is_water_at` is what sites an offshore pump.
///
/// Vulcanus holds a tile at (0, 0) -- it is simply not water -- so a `false`
/// here means the water stayed on Nauvis, not that Vulcanus is empty.
#[test]
fn water_charted_on_one_surface_is_not_water_on_the_other() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(10, "tiles", &tiles_line(Some("nauvis"), &[("water", true)]))
        .expect("the nauvis tiles line must parse");
    parser
        .parse(
            10,
            "tiles",
            &tiles_line(Some("vulcanus"), &[("grass-1", false)]),
        )
        .expect("the vulcanus tiles line must parse");

    let nauvis = world.nauvis().expect("nauvis must be held");
    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must have been created by the routing");
    let at = Position::new(0.5, 0.5);

    assert!(
        nauvis.entity_graph.is_water_at(&at),
        "nauvis was told about water here"
    );
    assert!(
        !vulcanus.entity_graph.is_water_at(&at),
        "vulcanus was told about grass here, so the lake must not have followed"
    );
    assert_eq!(
        names_at_origin(&vulcanus),
        vec!["grass-1".to_string()],
        "and vulcanus is occupied at that tile, so the line above is about \
         routing and not about an empty graph"
    );
}

/// Two lines for one surface land in **one** graph.
///
/// A `surface_or_create` handing back a fresh surface per call would pass every
/// aliasing test above while dropping all but the last chunk of a real planet --
/// a routing bug that looks exactly like a routing fix. The same reason
/// `two_records_on_one_surface_land_in_one_graph` exists for entities.
#[test]
fn two_ground_lines_for_one_surface_land_in_one_graph() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(
            10,
            "tiles",
            &tiles_line(Some("vulcanus"), &[("grass-1", false)]),
        )
        .expect("the first vulcanus line must parse");
    // The next chunk east, so the two lines occupy different tiles: the tile
    // tree refuses a second box at a position it already holds.
    parser
        .parse(11, "tiles", "32,0;64,32;vulcanus: water:1")
        .expect("the second vulcanus line must parse");

    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must have been created by the routing");
    let bounds = Rect::new(&Position::new(0.1, 0.1), &Position::new(32.9, 0.9));
    let names: Vec<String> = vulcanus
        .entity_graph
        .tiles_within(&bounds)
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(
        names,
        vec!["grass-1".to_string(), "water".to_string()],
        "both lines must be in the same vulcanus graph -- the second must not \
         have gone to a freshly created second copy"
    );
    assert_eq!(
        world.surface_ids(),
        vec![SurfaceId::nauvis(), SurfaceId::from("vulcanus")],
        "and two lines for one surface must create exactly one surface"
    );
}

/// **An older sender is read as an older sender.**
///
/// A two-field header is what every archived server log and both world dumps
/// carry. It must still parse, must land on the default surface, and must leave
/// `FactorioTile::surface` at `None` -- "nobody said", never a surface named
/// nothing and never Nauvis by assertion.
#[test]
fn a_header_without_a_surface_is_absent_not_unknown() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(10, "tiles", &tiles_line(None, &[("water", true)]))
        .expect("a two-field header must still parse");

    let nauvis = world.nauvis().expect("nauvis must be held");
    let bounds = Rect::new(&Position::new(0.1, 0.1), &Position::new(0.9, 0.9));
    let tiles = nauvis.entity_graph.tiles_within(&bounds);
    assert_eq!(tiles.len(), 1, "the tile must have landed somewhere");
    assert_eq!(tiles[0].name, "water", "and it must be the tile we sent");
    assert_eq!(
        tiles[0].surface, None,
        "the sender did not say, so the tile must not claim one"
    );
    assert_eq!(
        world.surface_ids(),
        vec![SurfaceId::nauvis()],
        "and a nameless line must not conjure a surface"
    );
}

/// The tile carries the name the sender gave it, which is what a later reader
/// -- a dump, a keyframe, a census -- sees.
///
/// Paired with the assertion above: absent stays absent, present is the exact
/// name and not a normalisation of it.
#[test]
fn a_routed_tile_remembers_which_surface_named_it() {
    let world = world();
    let mut parser = OutputParser::with_game_world(world.clone());

    parser
        .parse(
            10,
            "tiles",
            &tiles_line(Some("vulcanus"), &[("grass-1", false)]),
        )
        .expect("the vulcanus tiles line must parse");

    let vulcanus = world
        .surface(&SurfaceId::from("vulcanus"))
        .expect("vulcanus must have been created by the routing");
    let bounds = Rect::new(&Position::new(0.1, 0.1), &Position::new(0.9, 0.9));
    let tiles = vulcanus.entity_graph.tiles_within(&bounds);
    assert_eq!(tiles.len(), 1, "the tile must have landed on vulcanus");
    assert_eq!(
        tiles[0].surface,
        Some(SurfaceId::from("vulcanus")),
        "and it must carry the surface the header named"
    );
}
