//! "Would placing this seal in the character placing it?" -- asked before
//! the placement, from where the character actually stands.
//!
//! # The gap this closes
//!
//! `crates/planner`'s `enclosure::check` asks whether a candidate cell would
//! wall in a *bystander*: every bot at its plan-time position, before the
//! cell is built. It cannot ask about the bot that will build the cell,
//! because that bot's stand-point does not exist at plan time -- the plan
//! names the annulus a placement can be made from, and the actuator
//! (`approach_annulus`, `crates/core/src/factorio/rcon.rs`) picks the point
//! in it from the direction the bot happens to arrive from, against ground
//! only the game can see.
//!
//! Run `run-1788552801-73005` is what falls through that gap. Bot 1 arrived
//! from the east to place `assembling-machine-1 [31.5, -4.5]`, so the
//! actuator stood it 2.9 tiles east of the site -- on the two-by-two patch
//! between an older cell's chest column and its pole, whose every exit ran
//! across the site. The bot placed the assembler four ticks after arriving
//! and did not move again for the remaining 40 000 ticks of the run. The
//! planner's check had run, and passed: at plan time bot 1 was fifty tiles
//! away.
//!
//! # Why this layer
//!
//! This is the one place that has all three facts at once: where the
//! character *is* (the world's last reading of it, a few ticks old at most),
//! what is about to be built (the footprint from the prototype), and what is
//! already standing (`entity_graph`). The check is the same fill detection
//! uses after the fact (`crates/core::graph::enclosure`), run with the
//! footprint added as a hypothetical obstacle; the step-aside target is the
//! nearest tile the character can reach now that stays connected to open
//! ground once the footprint stands, filtered to tiles the placement can
//! still be made from.
//!
//! # The second gap, closed here for the same reason
//!
//! The same three facts answer a simpler question first: **is the character
//! standing inside the footprint it is about to build?** The plan says it
//! must not -- every `Place` carries an `AtPosition` annulus whose inner
//! radius is the placement's clearance -- but the annulus is judged against
//! where the *model* left the bot after its last walk, and the walker stops
//! anywhere within a small box of its waypoint. `run-1788569499-05724`: bot 1
//! walked to the pipe at `[43.5, -5.5]` and stopped at `(42.24, -6.77)`;
//! the plan's next placement was a steam engine facing east at
//! `[40.5, -5.5]`, whose turned box reaches to `x = 42.85`. The model had the
//! bot 3.0 tiles from the engine's centre, past the 2.94-tile clearance, so
//! no walk was emitted; the game had it 0.55 tiles inside the box and
//! refused the build. The mod is meant to answer that case with
//! `§player_blocks_placement§` and did not (its own check used the
//! north-frame box -- fixed alongside this); the refusal went into the ledger
//! as a fact about the ground and the next plan moved the whole plant.
//!
//! Asked here, the answer is a one- or two-tile walk to the nearest tile
//! outside the box that keeps the site within reach, before any RPC is
//! spent. The mod's own answer remains the backstop for a world whose
//! position for the character is stale.
//!
//! # What it deliberately does not do
//!
//! * It does not refuse to build next to an already-enclosed bot. A bot that
//!   is sealed in before the placement is not this placement's doing; the
//!   enclosure ledger is the place that condition is named, and refusing
//!   every build near it would stall the run on a bot that is already
//!   stalled.
//! * It does not treat "could not tell" as "would seal": an escape search
//!   that runs off the modelled map answers `Unknown`, and a placement is
//!   allowed to go ahead on that answer, because holding every build near
//!   the map's edge on an unmodelled window is a worse trade than the rare
//!   enclosure it might prevent -- detection still names one afterwards.

use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::factorio::world::{FactorioWorld, StepAsideReason};
use factorio_bot_core::graph::enclosure::{
    Escape, blocks_character, character_half_box, escape_from, escape_with, step_aside_target,
};
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::tracing::{info, warn};
use factorio_bot_core::types::{Direction, FactorioEntity, PlayerId, Position, Rect};

/// How close to the chosen tile the step-aside walk has to land. The same
/// number as `run.rs`'s `EVACUATE_RADIUS`, for the same reason: the tile was
/// proven safe, and the game's own pathing does not promise to land a
/// character on an exact float.
pub const STEP_ASIDE_RADIUS: f64 = 0.5;

/// Kept back from the character's building reach when choosing where to
/// stand, so that the walk landing [`STEP_ASIDE_RADIUS`] off the tile centre
/// cannot put the site out of reach.
const REACH_MARGIN: f64 = 1.0;

/// What the check decided.
#[derive(Debug, Clone, PartialEq)]
pub enum PrePlace {
    /// Build from where the character stands.
    Proceed,
    /// Walk the character to `to` first, then build.
    StepAside {
        from: Position,
        to: Position,
        /// Which of the two conditions asked for the walk.
        reason: StepAsideReason,
        /// How many tiles the character would have been left with; `0.0`
        /// for [`StepAsideReason::Footprint`], where the placement would
        /// stand on the character's own tile.
        pocket_tiles: f64,
    },
    /// The placement would seal the character in and no tile it can reach
    /// both stays open afterwards and is within building reach of the site.
    /// The caller must not build; the message is what the record gets.
    Refuse { from: Position, pocket_tiles: f64 },
}

/// Decides whether `player` may place `name` at `at` from where it stands.
///
/// Every branch logs, for the reason `walk_memory` gives: "it did not fire",
/// "it fired and found nothing" and "it fired and moved the bot" are three
/// different things with one appearance from outside.
pub fn judge_placement(
    world: &FactorioWorld,
    player: PlayerId,
    name: &str,
    at: &Position,
    direction: u8,
) -> PrePlace {
    let Some((here, build_distance)) = world
        .players
        .get(&player)
        .map(|p| (p.position.clone(), f64::from(p.build_distance)))
    else {
        // The placement itself will be refused for the same reason by
        // `place_entity_timed`, with the message that owns it.
        info!(
            player,
            "pre-place check skipped: the world has no position for this player"
        );
        return PrePlace::Proceed;
    };
    let Some(footprint) = footprint_of(world, name, at, direction) else {
        info!(
            player,
            name, "pre-place check skipped: no prototype for this entity, so no footprint to model"
        );
        return PrePlace::Proceed;
    };

    // A belt, a splitter or a loader is not a wall and never was: a character
    // walks over one, and the game lets a player build one under their own
    // feet. Neither question below has an answer worth asking about such a
    // placement -- it cannot seal anybody in, and standing on its tile is not
    // a refusal -- so the check declines by name rather than emitting a walk
    // nobody needs. See `graph::enclosure::blocks_character`.
    if !blocks_character(&world.entity_prototypes, name) {
        info!(
            player,
            name,
            "pre-place check skipped: a character does not collide with this entity, so \
             placing it can neither seal anyone in nor be refused for standing on it"
        );
        return PrePlace::Proceed;
    }

    let (half_x, half_y) = character_half_box(&world.entity_graph);
    // Where a step aside may land, for either reason: outside the footprint
    // by the character's own half-box plus the walk's landing tolerance, and
    // still within building reach of the site.
    let clear_of = Rect::new(
        &Position::new(
            footprint.left_top.x() - half_x - STEP_ASIDE_RADIUS,
            footprint.left_top.y() - half_y - STEP_ASIDE_RADIUS,
        ),
        &Position::new(
            footprint.right_bottom.x() + half_x + STEP_ASIDE_RADIUS,
            footprint.right_bottom.y() + half_y + STEP_ASIDE_RADIUS,
        ),
    );
    let reach = build_distance - REACH_MARGIN;
    let admit =
        |tile: &Position| !contains(&clear_of, tile) && calculate_distance(tile, at) <= reach;

    if character_overlaps(&footprint, &here, half_x, half_y) {
        return match step_aside_target(
            &world.entity_graph,
            &here,
            std::slice::from_ref(&footprint),
            admit,
        ) {
            Some(to) => {
                warn!(
                    player,
                    from = %here,
                    to = %to,
                    placing = name,
                    site = %at,
                    direction,
                    "STEPPING ASIDE: the character stands inside the box this placement \
                     would occupy, so the game would refuse it for the actor's sake; \
                     walking it to the nearest tile outside the box first"
                );
                PrePlace::StepAside {
                    from: here,
                    to,
                    reason: StepAsideReason::Footprint,
                    pocket_tiles: 0.0,
                }
            }
            None => {
                info!(
                    player,
                    from = %here,
                    placing = name,
                    site = %at,
                    "pre-place check: the character stands inside the placement's box and no \
                     tile within reach was found to step to; the placement goes ahead and the \
                     mod's own actor-in-footprint answer will walk the bot instead"
                );
                PrePlace::Proceed
            }
        };
    }

    let before = escape_from(&world.entity_graph, &here);
    let after = escape_with(&world.entity_graph, &here, std::slice::from_ref(&footprint));
    match (before, after) {
        (Escape::Open, Escape::Enclosed { pocket_tiles }) => {
            match step_aside_target(
                &world.entity_graph,
                &here,
                std::slice::from_ref(&footprint),
                admit,
            ) {
                Some(to) => {
                    warn!(
                        player,
                        from = %here,
                        to = %to,
                        placing = name,
                        site = %at,
                        pocket_tiles,
                        "STEPPING ASIDE: this placement would wall the placing character \
                         in; walking it to the nearest tile that stays open first"
                    );
                    PrePlace::StepAside {
                        from: here,
                        to,
                        reason: StepAsideReason::Enclosure,
                        pocket_tiles,
                    }
                }
                None => {
                    warn!(
                        player,
                        from = %here,
                        placing = name,
                        site = %at,
                        pocket_tiles,
                        "REFUSING TO PLACE: it would wall the placing character in, and no \
                         reachable tile within building reach stays open afterwards"
                    );
                    PrePlace::Refuse {
                        from: here,
                        pocket_tiles,
                    }
                }
            }
        }
        (Escape::Enclosed { pocket_tiles }, _) => {
            info!(
                player,
                from = %here,
                pocket_tiles,
                "pre-place check: the character is already walled in here; this placement \
                 does not change that and is allowed"
            );
            PrePlace::Proceed
        }
        (Escape::Unknown(why), _) | (_, Escape::Unknown(why)) => {
            info!(
                player,
                from = %here,
                why = ?why,
                "pre-place check: the escape search declined to answer, so the placement \
                 goes ahead -- which is not the same as it being safe"
            );
            PrePlace::Proceed
        }
        (Escape::Open, Escape::Open) => PrePlace::Proceed,
    }
}

/// The collision box `name` will occupy at `at`, facing `direction`, or
/// `None` when the world has no prototype for it (a zero-width box is what
/// `from_prototype` gives an unknown name, and a zero-width obstacle would
/// model nothing).
fn footprint_of(world: &FactorioWorld, name: &str, at: &Position, direction: u8) -> Option<Rect> {
    let entity = FactorioEntity::from_prototype(
        name,
        at.clone(),
        Direction::from_u8(direction),
        None,
        None,
        world.entity_prototypes.clone(),
    )
    .ok()?;
    (entity.bounding_box.width() > 0. && entity.bounding_box.height() > 0.)
        .then_some(entity.bounding_box)
}

/// Whether a character centred at `here` collides with `footprint`.
///
/// Box against box, as the game tests it, not point against box: the
/// character in `run-1788569499-05724` had its *centre* 0.02 tiles outside
/// the engine's box on the y axis and its own 0.2-tile half-box well inside
/// it. Open intervals -- boxes that merely touch do not collide, which is
/// the same tolerance `PlanState::placement_clearance` argues from.
fn character_overlaps(footprint: &Rect, here: &Position, half_x: f64, half_y: f64) -> bool {
    here.x() - half_x < footprint.right_bottom.x()
        && here.x() + half_x > footprint.left_top.x()
        && here.y() - half_y < footprint.right_bottom.y()
        && here.y() + half_y > footprint.left_top.y()
}

fn contains(rect: &Rect, point: &Position) -> bool {
    point.x() >= rect.left_top.x()
        && point.x() <= rect.right_bottom.x()
        && point.y() >= rect.left_top.y()
        && point.y() <= rect.right_bottom.y()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use factorio_bot_core::test_utils::fixture_entity_prototypes;
    use factorio_bot_core::types::FactorioPlayer;
    use std::sync::Arc;

    /// The older cell bot 1 walked into the middle of in
    /// `run-1788552801-73005`, from the run's own last keyframe: the chest
    /// column at `x = 33.5`, its inserters at `34.5`, the pole at
    /// `[35.5, -4.5]` and the two machines at `x = 36.5`. Nothing here was
    /// placed after the bot arrived.
    const THE_OLD_CELL: [(&str, f64, f64, u8); 13] = [
        ("lab", 36.5, -9.5, 0),
        ("small-electric-pole", 38.5, -7.5, 0),
        ("iron-chest", 33.5, -6.5, 0),
        ("inserter", 34.5, -6.5, 12),
        ("assembling-machine-1", 36.5, -6.5, 0),
        ("steam-engine", 40.5, -5.5, 4),
        ("small-electric-pole", 35.5, -4.5, 0),
        ("inserter", 36.5, -4.5, 0),
        ("iron-chest", 33.5, -3.5, 0),
        ("inserter", 34.5, -3.5, 4),
        ("iron-chest", 33.5, -2.5, 0),
        ("inserter", 34.5, -2.5, 12),
        ("assembling-machine-1", 36.5, -2.5, 0),
    ];

    /// Where bot 1 stood, to the 1/256th, from tick 210 900 to the end.
    fn bot_1_stood_at() -> Position {
        Position::new(34.41796875, -4.62890625)
    }

    /// The site of the placement that sealed it in.
    fn the_site() -> Position {
        Position::new(31.5, -4.5)
    }

    fn world_with_the_old_cell() -> FactorioWorld {
        let prototypes = Arc::new(fixture_entity_prototypes());
        let world = FactorioWorld::new();
        world
            .update_entity_prototypes(prototypes.iter().map(|p| p.clone()).collect())
            .expect("prototypes load");
        let built = THE_OLD_CELL
            .iter()
            .map(|(name, x, y, direction)| {
                FactorioEntity::from_prototype(
                    name,
                    Position::new(*x, *y),
                    Direction::from_u8(*direction),
                    None,
                    None,
                    prototypes.clone(),
                )
                .expect("from_prototype does not fail")
            })
            .collect();
        world.update_chunk_entities(built).expect("the cell loads");
        world.players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                position: bot_1_stood_at(),
                build_distance: 10,
                ..FactorioPlayer::default()
            },
        );
        world
    }

    /// Run 73005, replayed: the assembler placed from where bot 1 stood
    /// would seal it into four tiles, and there is somewhere to step aside
    /// to that keeps the site within reach.
    #[test]
    fn the_placement_that_walled_bot_1_in_is_caught_and_a_step_aside_found() {
        let world = world_with_the_old_cell();
        match judge_placement(&world, 1, "assembling-machine-1", &the_site(), 0) {
            PrePlace::StepAside {
                from,
                to,
                reason,
                pocket_tiles,
            } => {
                assert_eq!(from, bot_1_stood_at());
                assert_eq!(reason, StepAsideReason::Enclosure);
                // Exact: the game's grid sees the two chest tiles, the two
                // pole-side tiles and the assembler's two as walls, and
                // nothing else.
                assert_eq!(pocket_tiles, 4.);
                // A tile centre, within building reach, and not on the site.
                assert_eq!(to.x().fract().abs(), 0.5);
                assert_eq!(to.y().fract().abs(), 0.5);
                assert!(calculate_distance(&to, &the_site()) <= 10. - REACH_MARGIN);
                assert!(calculate_distance(&to, &the_site()) > 1.2 + 0.2 + STEP_ASIDE_RADIUS);
                // And standing there, the placement no longer closes the fill.
                let footprint =
                    footprint_of(&world, "assembling-machine-1", &the_site(), 0).unwrap();
                assert_eq!(
                    escape_with(&world.entity_graph, &to, &[footprint]),
                    Escape::Open,
                    "from {to}"
                );
            }
            other => panic!("run 73005's placement must be caught, got {other:?}"),
        }
    }

    /// The same placement from open ground west of the site is nobody's
    /// enclosure. A check that stepped aside on every build would be a
    /// worse defect than the one it exists to catch.
    #[test]
    fn a_placement_from_open_ground_proceeds() {
        let world = world_with_the_old_cell();
        world.players.get_mut(&1).unwrap().position = Position::new(28.5, -4.5);
        assert_eq!(
            judge_placement(&world, 1, "assembling-machine-1", &the_site(), 0),
            PrePlace::Proceed
        );
    }

    /// A belt is not a wall, and a character may stand on the tile it goes
    /// on: neither question this check asks has an answer for one.
    ///
    /// Put bot 1 exactly on the tile the belt is about to occupy -- the
    /// `character_overlaps` branch, which for any *colliding* entity walks
    /// the bot out of the box first -- and the answer must still be
    /// `Proceed`. The control below is the same placement made of `pipe`,
    /// a one-tile box that does collide.
    #[test]
    fn placing_a_belt_under_the_character_is_not_a_step_aside() {
        let world = world_with_the_old_cell();
        let site = Position::new(28.5, -4.5);
        world.players.get_mut(&1).unwrap().position = site.clone();
        assert_eq!(
            judge_placement(&world, 1, "transport-belt", &site, 4),
            PrePlace::Proceed
        );
        match judge_placement(&world, 1, "pipe", &site, 0) {
            PrePlace::StepAside { reason, .. } => {
                assert_eq!(reason, StepAsideReason::Footprint);
            }
            other => panic!("a pipe does collide with a character, got {other:?}"),
        }
    }

    /// A bot already walled in is not this placement's doing, and the
    /// placement goes ahead: refusing it would stall the run on a condition
    /// the enclosure ledger, not this check, names.
    #[test]
    fn a_bot_already_walled_in_is_not_this_placements_doing() {
        let world = world_with_the_old_cell();
        let prototypes = world.entity_prototypes.clone();
        // Build the assembler that sealed bot 1 in, then ask about another
        // placement from the same spot.
        let assembler = FactorioEntity::from_prototype(
            "assembling-machine-1",
            the_site(),
            None,
            None,
            None,
            prototypes,
        )
        .unwrap();
        world.update_chunk_entities(vec![assembler]).unwrap();
        assert_eq!(
            escape_from(&world.entity_graph, &bot_1_stood_at()),
            Escape::Enclosed { pocket_tiles: 4. }
        );
        assert_eq!(
            judge_placement(&world, 1, "inserter", &Position::new(29.5, -4.5), 12),
            PrePlace::Proceed
        );
    }

    /// An entity the world has no prototype for has no footprint to model,
    /// and the check says so rather than guessing a box.
    #[test]
    fn an_unknown_entity_is_not_modelled() {
        let world = world_with_the_old_cell();
        assert_eq!(
            judge_placement(&world, 1, "no-such-entity", &the_site(), 0),
            PrePlace::Proceed
        );
    }

    /// A player the world has no position for is left to the placement
    /// itself to refuse.
    #[test]
    fn an_unknown_player_is_left_to_the_placement() {
        let world = world_with_the_old_cell();
        assert_eq!(
            judge_placement(&world, 7, "assembling-machine-1", &the_site(), 0),
            PrePlace::Proceed
        );
    }
}
