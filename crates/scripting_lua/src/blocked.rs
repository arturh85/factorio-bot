//! What the entity graph's `blocked_tree` holds over one rectangle of ground,
//! in a shape a script can read.
//!
//! # Why this exists
//!
//! [`EntityGraph`](factorio_bot_core::graph::entity_graph::EntityGraph) keeps
//! two structures over the same ground and only one of them was reachable from
//! Lua. `world.find_entities_in_radius` reads `entity_tree`, a **whitelist** of
//! entity types that models a factory: furnaces, inserters, belts, containers
//! and the two big rocks. Trees, small rocks, cliffs, units and water never
//! enter it. `blocked_tree` is the other one -- every collision box the model
//! has been told about -- and until now nothing outside `crates/core` and
//! `crates/planner` could enumerate it at all.
//!
//! That gap has cost two sessions. A build refused with *"cannot build
//! burner-mining-drill at tile (-16,-14): occupied by a tree, cliff, rock or
//! unit"*, the game was asked (`rcon.*`) and the model was asked (`world.*`),
//! and neither showed a tree at that tile at any point. A box was in the tree
//! and no source the planner had could name it. Two attempts to explain the
//! mechanism were written and both were retracted.
//!
//! This module is the instrument, not the explanation. It reports what the
//! tree actually holds so a script can diff it against
//! `rcon.find_entities_filtered` over the same rectangle.
//!
//! # Three things it is careful about
//!
//! **A box is anonymous and stays anonymous.** `blocked_tree`'s payload is a
//! bare `is_minable` flag and no name, which is exactly why the refusal above
//! could recite four things it had not read. [`BlockedBox`] carries the flag
//! and says *"source unknown"*; it never invents a name. This follows
//! `crates/planner`'s `Occupant::Terrain { minable }`, whose wording was fixed
//! for the same reason.
//!
//! **Absent is not empty.** An empty [`BlockedBoxReport::boxes`] over
//! [`BlockedCoverage::Charted`] ground means there is nothing there. The same
//! empty list over [`BlockedCoverage::Unknown`] means nobody has looked, and
//! reading the second as the first is the mistake that produced the retracted
//! tree hypothesis. See [`BlockedCoverage`] for how the difference is
//! established.
//!
//! **It never hands Lua a `nil`.** `Option::None` reaches Lua as mlua's null
//! sentinel, which is light userdata and therefore *truthy*, so
//! `local t = r.boxes or {}` does not substitute a default and the next
//! `pairs(t)` raises. Every field of the report is always present and `boxes`
//! is always a list.

use factorio_bot_core::graph::entity_graph::EntityGraph;
use factorio_bot_core::schemars::JsonSchema;
use factorio_bot_core::serde::{Deserialize, Serialize};
use factorio_bot_core::types::{Pos, Position, Rect};
use std::collections::BTreeSet;

/// The largest rectangle [`blocked_boxes_report`] will answer for, in whole
/// tiles.
///
/// 128x128, i.e. sixteen chunks. The coverage test walks the rectangle tile by
/// tile, so an unbounded rectangle would let one line of user Lua walk the
/// whole ±5120 map -- 104 million tiles -- inside a lock. A diagnostic asks
/// about a footprint and its surroundings; anything larger is a different
/// tool.
pub const MAX_TILES: u64 = 128 * 128;

/// Whether the model has been told about the ground the question is about.
///
/// Reaches Lua as one of the four strings `"charted"`, `"partial"`,
/// `"unknown"` and `"outside_model"`. Written out here because the generated
/// `types.lua` renders a unit-variant enum as `any` and would otherwise carry
/// none of them.
///
/// The house rule this follows is `EntityGraph::resource_fingerprint`'s and
/// `runMatch.ts`': **equal means equal, different means unknown.** An answer of
/// "no boxes" is only worth anything when the ground behind it was actually
/// ingested, and the model's record of what it has ingested is `tile_tree` --
/// the mod's `on_chunk_generated` writes out *every* tile of every chunk the
/// engine generates, exactly once (`tile_chunks` in `control.lua`), and
/// `EntityGraph::add_tiles` files all of them. So a tile that is in
/// `tile_tree` is ground the model has seen, and a tile that is not is ground
/// nobody has described to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "factorio_bot_core::schemars")]
#[serde(crate = "factorio_bot_core::serde", rename_all = "snake_case")]
pub enum BlockedCoverage {
    /// Every tile of the rectangle has been written out to the model, so the
    /// box list is complete for this ground and an empty list means clear.
    Charted,
    /// Some tiles have been written out and some have not. The boxes are real,
    /// but their absence over the uncharted part means nothing.
    Partial,
    /// No tile of the rectangle has ever been written out. The box list is
    /// empty because nobody has looked, **not** because the ground is clear.
    /// Never fold this into "no boxes".
    Unknown,
    /// The rectangle is not entirely inside the region `blocked_tree` covers
    /// at all -- the quad tree is built over a fixed ±5120 extent -- so a query
    /// for it comes back short by construction, and the missing part would
    /// read as open ground. The same check `PlanState::enclosure_grid` and
    /// `enclosure::Escape::Unknown` already make, kept separate from
    /// [`Self::Unknown`] because the causes are different: one is ground
    /// nobody charted, the other is ground this index cannot hold.
    OutsideModel,
}

/// One collision box the model holds, and the one bit it knows about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "factorio_bot_core::schemars")]
#[serde(crate = "factorio_bot_core::serde", rename_all = "snake_case")]
pub struct BlockedBox {
    /// The rectangle, with its edges snapped back to Factorio's own
    /// 1/256-of-a-tile position grid -- see
    /// `EntityGraph::blocking_boxes_within`, which does the snapping. The tree
    /// stores `f32`, so without it the *centre* of a recovered box reads a
    /// hair below the integer it should be and floors into the neighbouring
    /// tile.
    pub area: Rect,
    /// `FactorioEntity::is_minable`, which is exactly "the entity's type is
    /// `tree` or `simple-entity`". `true` is a tree or a rock. `false` is
    /// **anything else with a collision box the entity tree does not hold** --
    /// this does not know which, and does not guess.
    pub minable: bool,
    /// The same fact as [`Self::minable`], written out for a reader.
    ///
    /// Deliberately says *source unknown* rather than naming an obstacle. The
    /// tree has no name to give, so a name here would be invented -- which is
    /// how a tile holding nothing but ore came to be reported as holding a
    /// tree.
    pub description: String,
}

impl BlockedBox {
    fn new(area: Rect, minable: bool) -> Self {
        let description = if minable {
            "a box, minable, source unknown"
        } else {
            "a box, not minable, source unknown"
        }
        .to_owned();
        Self {
            area,
            minable,
            description,
        }
    }
}

/// What `blocked_tree` holds over one rectangle, and how much that answer is
/// worth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "factorio_bot_core::schemars")]
#[serde(crate = "factorio_bot_core::serde", rename_all = "snake_case")]
pub struct BlockedBoxReport {
    /// The ground actually answered about: the caller's rectangle widened out
    /// to whole tiles (`floor` on the top-left, `ceil` on the bottom-right).
    ///
    /// Widened rather than clipped because the coverage test is per tile and a
    /// half-covered tile is either charted or not; there is no half. Reported
    /// back so a caller diffing against `rcon.find_entities_filtered` asks the
    /// game about the same ground.
    pub area: Rect,
    /// See [`BlockedCoverage`]. Read this **before** reading
    /// [`Self::boxes`].
    pub coverage: BlockedCoverage,
    /// How many whole tiles [`Self::area`] covers.
    pub tiles_in_area: u64,
    /// How many of them the model has been told about. Equal to
    /// [`Self::tiles_in_area`] exactly when [`Self::coverage`] is
    /// [`BlockedCoverage::Charted`].
    pub tiles_charted: u64,
    /// Every box overlapping [`Self::area`], ordered by
    /// `(left, top, right, bottom, minable)` so two calls over one tree state
    /// agree.
    ///
    /// **Not deduplicated.** The tree can hold the same rectangle more than
    /// once -- a chunk's entities reach `EntityGraph::add` from both
    /// `on_chunk_generated` and the mod's `initial_discovery` replay -- and a
    /// diagnostic that quietly collapsed those would hide exactly the kind of
    /// bookkeeping fault this instrument exists to find.
    ///
    /// Always a list. Empty means "no boxes" only under
    /// [`BlockedCoverage::Charted`].
    pub boxes: Vec<BlockedBox>,
}

/// Why a rectangle could not be answered about at all.
///
/// A refusal, not an empty report: an empty report over a rectangle nobody
/// could measure is the failure mode this whole module is written against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockedQueryError {
    /// The rectangle has no area -- zero or negative extent on an axis, which
    /// is usually a swapped corner.
    Empty { width: i64, height: i64 },
    /// The rectangle covers more tiles than [`MAX_TILES`].
    TooLarge { tiles: u64 },
}

impl std::fmt::Display for BlockedQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty { width, height } => write!(
                f,
                "the rectangle covers no whole tile ({width} x {height}); left_top must be above \
                 and left of right_bottom"
            ),
            Self::TooLarge { tiles } => write!(
                f,
                "the rectangle covers {tiles} tiles, more than the {MAX_TILES} this query walks \
                 tile by tile; ask about a smaller area"
            ),
        }
    }
}

/// Read `blocked_tree` over `area`, together with whether the model has been
/// told about that ground at all.
///
/// Read-only in every sense: it takes read guards, mutates nothing, and does
/// not bump the graph's generation counter.
pub fn blocked_boxes_report(
    graph: &EntityGraph,
    area: &Rect,
) -> Result<BlockedBoxReport, BlockedQueryError> {
    let x0 = area.left_top.x().floor() as i64;
    let y0 = area.left_top.y().floor() as i64;
    let x1 = area.right_bottom.x().ceil() as i64;
    let y1 = area.right_bottom.y().ceil() as i64;
    let width = x1 - x0;
    let height = y1 - y0;
    if width <= 0 || height <= 0 {
        return Err(BlockedQueryError::Empty { width, height });
    }
    let tiles_in_area = width as u64 * height as u64;
    if tiles_in_area > MAX_TILES {
        return Err(BlockedQueryError::TooLarge {
            tiles: tiles_in_area,
        });
    }
    let span = Rect::new(
        &Position::new(x0 as f64, y0 as f64),
        &Position::new(x1 as f64, y1 as f64),
    );

    // Is this ground inside the region the index can hold at all? Read before
    // anything else, because a "no boxes" from a query that ran off the tree's
    // own extent is not an answer about the ground.
    let outside_model = {
        let tree = graph.blocked_tree();
        let bounds = tree.bounding_box();
        let modelled = (
            f64::from(bounds.origin.x),
            f64::from(bounds.origin.y),
            f64::from(bounds.origin.x + bounds.size.width),
            f64::from(bounds.origin.y + bounds.size.height),
        );
        span.left_top.x() < modelled.0
            || span.left_top.y() < modelled.1
            || span.right_bottom.x() > modelled.2
            || span.right_bottom.y() > modelled.3
    };

    // What the model has been *told about*, distinct tile by distinct tile.
    // `tiles_within` clips to the query exactly, so a tile abutting an edge
    // does not count as inside it.
    let charted: BTreeSet<Pos> = graph
        .tiles_within(&span)
        .into_iter()
        .map(|tile| Pos::from(&tile.position))
        .filter(|pos| {
            i64::from(pos.0) >= x0
                && i64::from(pos.0) < x1
                && i64::from(pos.1) >= y0
                && i64::from(pos.1) < y1
        })
        .collect();
    let tiles_charted = charted.len() as u64;

    let coverage = if outside_model {
        BlockedCoverage::OutsideModel
    } else if tiles_charted == 0 {
        BlockedCoverage::Unknown
    } else if tiles_charted < tiles_in_area {
        BlockedCoverage::Partial
    } else {
        BlockedCoverage::Charted
    };

    // `blocking_boxes_within_minable` is a *narrowing* pass: the quad tree's
    // own predicate admits boxes that merely come close, and is half-open, so
    // it reports a box abutting the left or top edge and not its mirror image
    // on the right or bottom. Re-tested strictly here for the reason
    // `tiles_within` does the same: this is a positive question ("where are
    // the boxes?"), and over-reporting would put an obstacle one tile outside
    // every rectangle anybody asks about.
    let mut boxes: Vec<BlockedBox> = graph
        .blocking_boxes_within_minable(&span)
        .into_iter()
        .filter(|(rect, _)| overlaps(&span, rect))
        .map(|(rect, minable)| BlockedBox::new(rect, minable))
        .collect();
    boxes.sort_by(|a, b| {
        a.area
            .left_top
            .x()
            .total_cmp(&b.area.left_top.x())
            .then(a.area.left_top.y().total_cmp(&b.area.left_top.y()))
            .then(a.area.right_bottom.x().total_cmp(&b.area.right_bottom.x()))
            .then(a.area.right_bottom.y().total_cmp(&b.area.right_bottom.y()))
            .then(a.minable.cmp(&b.minable))
    });

    Ok(BlockedBoxReport {
        area: span,
        coverage,
        tiles_in_area,
        tiles_charted,
        boxes,
    })
}

/// Strict overlap on all four sides: touching is not overlapping, in either
/// direction. The same predicate `EntityGraph::overlaps_bounds` applies, which
/// is private to `crates/core`.
fn overlaps(bounds: &Rect, rect: &Rect) -> bool {
    rect.left_top.x() < bounds.right_bottom.x()
        && bounds.left_top.x() < rect.right_bottom.x()
        && rect.left_top.y() < bounds.right_bottom.y()
        && bounds.left_top.y() < rect.right_bottom.y()
}

#[cfg(test)]
mod tests {
    use super::*;
    use factorio_bot_core::types::{FactorioEntity, FactorioTile};

    fn graph() -> EntityGraph {
        EntityGraph::new(
            std::sync::Arc::new(factorio_bot_core::dashmap::DashMap::new()),
            std::sync::Arc::new(factorio_bot_core::dashmap::DashMap::new()),
        )
    }

    /// Every tile of one patch, so coverage can be established without
    /// depending on what happens to be standing on it.
    ///
    /// `water` names the tiles that are `player_collidable` and therefore also
    /// land in `blocked_tree`; every other tile is plain grass. Written as one
    /// pass rather than a chart plus an overwrite because `tile_tree` is built
    /// with `allow_duplicates = false`, so a second tile at a position already
    /// filed is refused -- the game writes each chunk's tiles exactly once and
    /// the tree is configured for that.
    fn chart_with_water(
        graph: &EntityGraph,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        water: &[(i32, i32)],
    ) {
        let mut tiles = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let wet = water.contains(&(x, y));
                tiles.push(FactorioTile {
                    name: if wet { "water" } else { "grass-1" }.to_owned(),
                    player_collidable: wet,
                    position: Position::new(f64::from(x), f64::from(y)),
                    color: None,
                    surface: None,
                });
            }
        }
        graph.add_tiles(tiles, None).expect("tiles");
    }

    /// [`chart_with_water`] over dry ground.
    fn chart(graph: &EntityGraph, x0: i32, y0: i32, x1: i32, y1: i32) {
        chart_with_water(graph, x0, y0, x1, y1, &[]);
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect::new(&Position::new(x0, y0), &Position::new(x1, y1))
    }

    /// **The one that separates "clear" from "nobody looked".**
    ///
    /// Both answers are an empty box list, and reading the second as the first
    /// is the mistake behind the retracted tree hypothesis. Asserted as a pair
    /// in one test on purpose: the two calls differ only by whether the ground
    /// was ever written out, so a coverage that collapsed them would have to
    /// break one of these lines.
    #[test]
    fn empty_over_charted_ground_is_not_empty_over_uncharted_ground() {
        let graph = graph();
        chart(&graph, 0, 0, 8, 8);

        let looked = blocked_boxes_report(&graph, &rect(1., 1., 4., 4.)).expect("a report");
        assert_eq!(looked.coverage, BlockedCoverage::Charted);
        assert_eq!(looked.tiles_charted, 9);
        assert_eq!(looked.tiles_in_area, 9);
        assert!(looked.boxes.is_empty(), "{:?}", looked.boxes);

        let never = blocked_boxes_report(&graph, &rect(100., 100., 103., 103.)).expect("a report");
        assert_eq!(
            never.coverage,
            BlockedCoverage::Unknown,
            "ground nobody described read as charted"
        );
        assert_eq!(never.tiles_charted, 0);
        assert!(never.boxes.is_empty());
    }

    /// Half-charted ground is neither, and says so.
    #[test]
    fn partly_charted_ground_is_reported_as_partial() {
        let graph = graph();
        chart(&graph, 0, 0, 2, 4);
        let report = blocked_boxes_report(&graph, &rect(0., 0., 4., 4.)).expect("a report");
        assert_eq!(report.coverage, BlockedCoverage::Partial);
        assert_eq!(report.tiles_charted, 8);
        assert_eq!(report.tiles_in_area, 16);
    }

    /// Ground the index cannot hold is its own answer, never "clear".
    #[test]
    fn ground_outside_the_index_is_reported_as_outside_the_model() {
        let graph = graph();
        let report =
            blocked_boxes_report(&graph, &rect(-6000., -6000., -5990., -5990.)).expect("a report");
        assert_eq!(report.coverage, BlockedCoverage::OutsideModel);
    }

    /// A box comes back with the flag the tree stores and **no name**.
    ///
    /// A tree is used as the subject precisely because it is the thing the bad
    /// refusal named: the report must not say "tree" even when it is one,
    /// because it did not read that and the next box might be a cliff.
    #[test]
    fn a_box_reads_as_a_box_and_never_as_a_tree() {
        let graph = graph();
        chart(&graph, 0, 0, 8, 8);
        graph
            .add(
                vec![FactorioEntity::new_tree(&Position::new(2.5, 2.5))],
                None,
            )
            .expect("a tree");

        let report = blocked_boxes_report(&graph, &rect(0., 0., 8., 8.)).expect("a report");
        assert_eq!(report.boxes.len(), 1, "{:?}", report.boxes);
        let found = &report.boxes[0];
        assert!(found.minable, "a tree is minable and the tree said not");
        assert_eq!(found.description, "a box, minable, source unknown");
        let rendered = format!("{found:?}");
        assert!(
            !rendered.contains("tree"),
            "the report named the obstacle it never read: {rendered}"
        );
    }

    /// Water is in the same tree and carries the *other* value of the one bit.
    ///
    /// Without this the minable flag could be hard-coded `true` and every test
    /// above would still pass.
    #[test]
    fn a_water_tile_is_a_box_that_is_not_minable() {
        let graph = graph();
        chart_with_water(&graph, 0, 0, 4, 4, &[(1, 1)]);
        let report = blocked_boxes_report(&graph, &rect(1., 1., 2., 2.)).expect("a report");
        assert_eq!(report.coverage, BlockedCoverage::Charted);
        assert_eq!(report.boxes.len(), 1, "{:?}", report.boxes);
        assert!(!report.boxes[0].minable);
        assert_eq!(
            report.boxes[0].description,
            "a box, not minable, source unknown"
        );
    }

    /// A box merely abutting the query does not come back.
    ///
    /// The quad tree's own predicate is half-open, so without the strict
    /// re-test the row of boxes immediately left of the rectangle reads as
    /// inside it -- and an obstacle reported one tile off is worse than none,
    /// because it looks like an answer.
    #[test]
    fn a_box_touching_the_edge_is_not_inside_the_rectangle() {
        let graph = graph();
        // Two water tiles: one occupying exactly [0,1] x [1,2] and one [4,5] x
        // [1,2]. Both edges are covered because the tree's own predicate is
        // asymmetric -- it admits the box abutting the query's *left* edge and
        // rejects its mirror image on the right, so a test using only the
        // right-hand one passes with the strict re-test deleted.
        chart_with_water(&graph, 0, 0, 8, 8, &[(0, 1), (4, 1)]);

        let touching = blocked_boxes_report(&graph, &rect(1., 1., 4., 4.)).expect("a report");
        assert!(
            touching.boxes.is_empty(),
            "a box merely abutting an edge was reported as inside: {:?}",
            touching.boxes
        );
        let overlapping = blocked_boxes_report(&graph, &rect(0., 1., 5., 4.)).expect("a report");
        assert_eq!(
            overlapping.boxes.len(),
            2,
            "the same boxes were missed when they genuinely overlap: {:?}",
            overlapping.boxes
        );
    }

    /// A swapped corner is refused rather than answered with an empty list.
    #[test]
    fn a_rectangle_with_no_area_is_refused() {
        let graph = graph();
        let err = blocked_boxes_report(&graph, &rect(4., 4., 1., 1.)).expect_err("a refusal");
        assert!(matches!(err, BlockedQueryError::Empty { .. }), "{err:?}");
    }

    /// The walk is bounded, and the bound is a refusal rather than a hang.
    #[test]
    fn a_rectangle_larger_than_the_cap_is_refused() {
        let graph = graph();
        let err = blocked_boxes_report(&graph, &rect(0., 0., 200., 200.)).expect_err("a refusal");
        assert!(matches!(err, BlockedQueryError::TooLarge { .. }), "{err:?}");
        assert!(
            blocked_boxes_report(&graph, &rect(0., 0., 128., 128.)).is_ok(),
            "the cap itself was refused"
        );
    }

    /// The rectangle answered about is the one reported, widened to whole
    /// tiles, so a caller can hand the same ground to the game.
    #[test]
    fn the_area_reported_is_the_ground_answered_about() {
        let graph = graph();
        chart(&graph, -4, -4, 4, 4);
        let report = blocked_boxes_report(&graph, &rect(-1.3, 0.2, 1.4, 2.9)).expect("a report");
        assert_eq!(report.area, rect(-2., 0., 2., 3.));
        assert_eq!(report.tiles_in_area, 12);
        assert_eq!(report.tiles_charted, 12);
    }
}
