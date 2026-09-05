//! Scouting: sending a bot to look at ground nobody has looked at.
//!
//! The method behind [`Goal::Charted`], and the third piece of
//! `docs/superpowers/specs/2026-09-04-exploration-design.md` -- the primitive
//! that turns [`crate::PlannerError::NotCharted`] from a dead end into
//! something a plan can *do something about*. Pieces 0 (the threat index),
//! 1 (the free-vision measurement) and the refusal itself already landed;
//! this is the smallest thing that closes the loop.
//!
//! # The pattern: square rings, walked outwards
//!
//! Cells are the squares of a lattice of pitch [`REVEAL_PITCH`], and they are
//! visited in **Chebyshev rings** around the goal's centre: ring 0 is the
//! centre cell, ring `k` is the 8k cells at Chebyshev distance `k`. Squares
//! rather than circles because squares **tile the plane exactly** -- no
//! overlap, no gaps -- which is the property a covering search wants and the
//! one a disc of any radius cannot have.
//!
//! Three things fall out of ring order rather than being arranged:
//!
//! * **Nearest-first.** The first thing found is the nearest thing, and cost
//!   grows with distance instead of arbitrarily. On seed 31337 copper's
//!   nearest charted tile is 54.9 tiles against iron's 18.4, so a goal that
//!   wants copper is reached early in the spiral rather than at the end of it.
//! * **A truncated plan is still useful.** A run that stops halfway has
//!   looked at the near ground, which is the ground it was most likely to
//!   want.
//! * **The roster splits it for free.** The scheduler deals the actions
//!   across bots; because the cells of one ring are all roughly the same
//!   distance out, that deal is naturally balanced without this method
//!   knowing a roster exists.
//!
//! # What it skips, and why the skips are recorded
//!
//! A cell is skipped when it is **already charted** (the point is covering
//! the unknown) or when it is **within [`THREAT_STANDOFF`] of a charted enemy
//! structure**. Both skips are counted and the threat skips are named, so a
//! spiral that is boxed in by nests reports that rather than looking like a
//! search that simply stopped -- the failure mode this repo has paid for four
//! times over under "silence is not success".
//!
//! # Why it terminates
//!
//! `expand` emits actions and no subgoals, over a finite lattice, so there is
//! no recursion to bottom out. A fully charted disc expands to the empty
//! plan, which is the correct answer to "go and look at ground you can
//! already see" and is what makes the goal idempotent across the replans this
//! planner does constantly.
//!
//! # What it does *not* do
//!
//! * **Stop on find.** A plan is expanded before anything runs, so nothing at
//!   expansion time can know what a survey will reveal; a stop-on-find
//!   predicate baked in here would be a promise about the future. It belongs
//!   one level up and is *already expressible*: because `Goal::Charted` is
//!   idempotent and `PlanState` reads the live world, a supervisor that plans
//!   ring by ring -- widening the radius and re-planning -- stops the moment
//!   the goal it actually wanted stops raising `NotCharted`. That loop is
//!   also what makes a **newly charted nest** exclude the cells behind it:
//!   the exclusions below are the ones known when the plan was made, and the
//!   next plan knows more.
//! * **Combat.** Nothing here shoots, arms a bot or builds a turret. If
//!   avoidance alone cannot reach something the plan needs, the honest
//!   outcome is the refusal and the skip census, not a fight.

use crate::action::{Action, ActionKind, Actor, Condition};
use crate::error::PlannerError;
use crate::goal::Goal;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;
use factorio_bot_core::types::Position;

/// How close to a lattice point a bot has to stand for the cell to count as
/// looked at, in tiles.
///
/// Eight -- a quarter of a chunk, and deliberately **not** half the pitch.
/// The reveal is centred on the character, so a bot that stops short does not
/// look at the cell it was sent to; it looks at somewhere else. The slack
/// exists only to absorb the pathfinder's own inability to land a character
/// on an exact float, and 8 against a 128-tile reveal radius is noise.
pub const SURVEY_RADIUS: f64 = 8.;

/// The lattice pitch: how far apart the points a bot must stand at are, in
/// tiles.
///
/// **This is the one number the whole pattern rests on, and it is measured**
/// -- on a live 2.1.17 headless server on seed 31337, not derived. A lone
/// character placed on empty ground causes the engine to generate a **9x9
/// block of chunks centred on it**: chunks `x 42..50, y 42..50` for a
/// character at (1500, 1500), and `x -51..-43, y 42..50` for one at
/// (-1500, 1500), 81 chunks each time. So the reveal is **+/-4 chunks =
/// +/-128 tiles**, and a lattice of pitch **8 chunks = 256 tiles** tiles the
/// plane with exactly one chunk of overlap and no gap: a point at chunk 0
/// covers -4..4, its neighbour at chunk 8 covers 4..12.
///
/// # It is *generation*, not charting -- and that distinction is the finding
///
/// The same measurement establishes that `force.is_chunk_charted` is **false
/// everywhere, including the chunk the character is standing in**, after 700
/// ticks. A server-side character charts nothing at all. That does not stop
/// exploration working, because **the world model is fed by
/// `on_chunk_generated`, never by charting** -- see
/// `docs/superpowers/specs/2026-09-04-exploration-design.md` Q1 -- so moving
/// a bot somewhere new is exactly what makes the model learn it. But it does
/// mean the honest word for what this buys is *generated ground*, and that
/// any future check written against `is_chunk_charted` would read zero on a
/// working run.
///
/// # Getting this wrong is silent in one direction
///
/// A pitch **finer** than the reveal costs walks and cannot leave a hole. A
/// pitch **coarser** leaves unlooked-at ground *inside* a spiral that reports
/// itself complete -- invisible, and exactly the class of failure this repo
/// keeps paying for. 256 is the measured tiling value; anything above it must
/// be re-measured, not reasoned about.
pub const REVEAL_PITCH: f64 = 256.;

/// How far from a standing character the engine generates ground, in tiles.
///
/// **+/-4 chunks**, from the measurement in [`REVEAL_PITCH`]: 128 tiles.
/// [`REVEAL_PITCH`] is twice this, which is what makes the lattice tile.
pub const REVEAL_RADIUS: f64 = REVEAL_PITCH / 2.;

/// How far a lattice point must be from a charted enemy structure before a
/// bot is sent to it, in tiles.
///
/// **50**, which is a biter spawner's `call_for_help_radius`: inside it, a
/// disturbance pulls the nest's units onto the intruder, so it is the radius
/// at which "walking past" becomes "being attacked". Stated as the scale to
/// start from rather than a proven-safe distance -- nothing here has yet met
/// a nest, and the number to replace it with is one measured off a run in
/// which a bot was chased.
///
/// It is deliberately generous relative to what exploration costs: skipping a
/// cell costs one lattice point of coverage, and being wrong costs a bot,
/// which nothing in this project yet notices (piece 2 of the exploration
/// design -- a bot death is *not* an event, and the roster is computed once).
/// Until that exists, over-avoiding is the only safe direction to err in.
///
/// # The soundness gap, stated rather than hidden
///
/// "Enemy" is inferred from the entity **type** string
/// (`ENEMY_STRUCTURE_TYPES = ["unit-spawner", "turret"]`), because the mod
/// never serialises an entity's `force`. `unit-spawner` is enemy-only in
/// vanilla so it is safe by luck; **`turret` is not** -- a player's own gun
/// turret is a `turret` too, so once this project builds defences its own
/// turrets will read as threats and push the spiral away from its own base.
/// That is a real gap in the world model, not in this constant, and the fix
/// is to serialise `force` in `mods/BotBridge/types.lua` and filter on it.
pub const THREAT_STANDOFF: f64 = 50.;

/// Why a lattice cell was not surveyed.
#[derive(Clone, Debug, PartialEq)]
pub enum Skip {
    /// The model already has ground here. Not a problem -- the goal.
    AlreadyCharted,
    /// A charted enemy structure is inside [`THREAT_STANDOFF`]: its name,
    /// where it stands, and how far it is from the cell.
    Threat {
        name: String,
        at: Position,
        distance: f64,
    },
}

/// What one expansion of [`Goal::Charted`] decided, cell by cell.
///
/// Returned beside the steps by [`survey_plan`] so that a caller -- a test, a
/// supervisor script, a run record -- can say *how much ground was refused
/// and why*, rather than inferring it from an action count. A spiral boxed in
/// by nests and a spiral over fully charted ground both emit zero actions and
/// are completely different situations.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurveyPlan {
    /// The lattice points a bot is being sent to, nearest ring first.
    pub visit: Vec<Position>,
    /// The cells that were not visited, in the same ring order, each with
    /// its reason.
    pub skipped: Vec<(Position, Skip)>,
}

impl SurveyPlan {
    /// How many cells were skipped because a nest was too close.
    #[must_use]
    pub fn threat_skips(&self) -> usize {
        self.skipped
            .iter()
            .filter(|(_, skip)| matches!(skip, Skip::Threat { .. }))
            .count()
    }

    /// Every cell was refused for a threat and none for being charted: the
    /// spiral is **boxed in**, not finished.
    ///
    /// The distinction a run record has to be able to make. `false` for an
    /// empty plan, because "nothing to do" is not "blocked".
    #[must_use]
    pub fn is_boxed_in(&self) -> bool {
        self.visit.is_empty() && !self.skipped.is_empty() && self.threat_skips() == self.skipped.len()
    }
}

/// The lattice points of the Chebyshev ring `k` around `centre`, in a fixed
/// order (by y then x, so two runs on one world agree).
///
/// Ring 0 is the centre alone; ring `k > 0` is the 8k points whose greatest
/// axis offset is exactly `k`.
fn ring(centre: &Position, k: i32) -> Vec<Position> {
    if k == 0 {
        return vec![centre.clone()];
    }
    let mut out = Vec::with_capacity((8 * k) as usize);
    for dy in -k..=k {
        for dx in -k..=k {
            // The ring, not the filled square: the interior belongs to the
            // rings already walked, and re-emitting it would make the spiral
            // quadratic in the radius rather than linear in the new ground.
            if dx.abs() != k && dy.abs() != k {
                continue;
            }
            out.push(Position::new(
                centre.x() + f64::from(dx) * REVEAL_PITCH,
                centre.y() + f64::from(dy) * REVEAL_PITCH,
            ));
        }
    }
    out
}

/// Whether standing at `point` would teach the model nothing.
///
/// **Not `PlanState::is_charted(point)`**, and the difference is a real bug
/// this had: that asks whether the model has the one tile under the lattice
/// point, and what a visit actually buys is the whole `+/-REVEAL_RADIUS`
/// block around it. On the seed-31337 t=0 dump the two disagree exactly where
/// it matters -- the map is generated out to `+/-320` at creation, so the
/// ring-1 cell at (256, -256) has a charted centre tile while the ground a
/// bot standing there would generate (out to y = -384) is unknown, and the
/// nearest crude oil at (131.5, -348.5) sits in it. The centre-tile test
/// skipped that cell and the spiral could never have found the oil.
///
/// So this probes the centre and the four cardinal edges of the reveal. Five
/// points rather than the whole block for the reason
/// [`PlanState::charting`] gives for probing at all: reading every tile of a
/// charted region costs more than a map score, for an answer no better.
fn is_covered(state: &PlanState, point: &Position) -> bool {
    const EDGES: [(f64, f64); 5] = [(0., 0.), (1., 0.), (-1., 0.), (0., 1.), (0., -1.)];
    EDGES.iter().all(|(dx, dy)| {
        state.is_charted(&Position::new(
            point.x() + dx * REVEAL_RADIUS,
            point.y() + dy * REVEAL_RADIUS,
        ))
    })
}

/// The whole decision for one [`Goal::Charted`]: which lattice cells to
/// survey, and why each of the others was skipped.
///
/// Split out of [`Scout::expand`] so it can be tested, and read, without an
/// `ExpansionCtx`.
#[must_use]
pub fn survey_plan(state: &PlanState, centre: &Position, radius: f64) -> SurveyPlan {
    let mut plan = SurveyPlan::default();
    if !radius.is_finite() || radius <= 0. {
        return plan;
    }
    // How many rings fit: the furthest ring whose points are still inside the
    // radius on the axes. Rounded down, so the goal never sends a bot outside
    // the disc it named.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "radius/pitch is bounded by the caller's radius, checked finite above"
    )]
    let rings = (radius / REVEAL_PITCH).floor() as i32;
    for k in 0..=rings {
        for point in ring(centre, k) {
            if is_covered(state, &point) {
                plan.skipped.push((point, Skip::AlreadyCharted));
                continue;
            }
            // The first non-test caller of the threat index. `None` here is
            // *unknown*, not safe -- see `PlanState::nearest_threat` -- and at
            // the frontier it is the expected answer, which is exactly why a
            // supervisor re-plans per ring instead of trusting one expansion.
            match state.nearest_threat(&point) {
                Some((name, at, distance)) if distance < THREAT_STANDOFF => {
                    plan.skipped
                        .push((point, Skip::Threat { name, at, distance }));
                }
                _ => plan.visit.push(point),
            }
        }
    }
    plan
}

/// The method for [`Goal::Charted`].
pub struct Scout;

impl Method for Scout {
    fn name(&self) -> &'static str {
        "scout"
    }

    /// Any `Charted` goal. There is no world state that makes looking
    /// impossible -- an already-charted disc is *satisfied*, not
    /// inapplicable, and expands to nothing.
    fn applicable(&self, goal: &Goal, _state: &PlanState) -> bool {
        matches!(goal, Goal::Charted { .. })
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Charted { around, radius } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let plan = survey_plan(&ctx.state, around, *radius);
        let boxed_in = plan.is_boxed_in();
        let threat_skips = plan.threat_skips();
        let steps = plan
            .visit
            .into_iter()
            .map(|point| {
                let id = ctx.ids.next();
                Step::Act(Box::new(Action {
                    id,
                    kind: ActionKind::Survey { to: point.clone() },
                    pre: vec![Condition::AtPosition {
                        who: Actor::Role,
                        pos: point.clone(),
                        radius: SURVEY_RADIUS,
                        min_radius: 0.0,
                    }],
                    // No effect. The plan learns nothing from a survey until
                    // the survey has actually run and the mod has written the
                    // chunk out -- that is the entire point of the action.
                    // Asserting a charting effect here would let a *plan*
                    // satisfy a later `NotCharted` check against ground no bot
                    // has been to yet, which is precisely the free-vision
                    // dishonesty `EventKind::VisionMeasured` exists to expose.
                    eff: Vec::new(),
                    duration: 0,
                    pinned: None,
                    // The label is what reaches the run record verbatim (see
                    // `method::util::evacuation_step`), so it carries the
                    // reason: a reader grepping "survey" gets exploration's
                    // cost separated from every other walk.
                    label: format!("survey {point} -- uncharted ground"),
                }))
            })
            .collect::<Vec<_>>();
        if boxed_in {
            // Every candidate cell was refused for a nest and none for being
            // charted. Emitting an empty plan silently here is the exact
            // shape of failure this repo keeps paying for: it is
            // indistinguishable from "already explored". Refuse instead, and
            // say how many.
            return Err(PlannerError::NoApplicableMethod {
                goal: format!(
                    "{goal}: every one of the {threat_skips} unlooked-at cells is within \
                     {THREAT_STANDOFF:.0} tiles of a charted nest, so exploring further \
                     needs a way past them rather than a bigger radius"
                ),
            });
        }
        Ok(steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::method::GoalSite;
    use factorio_bot_core::factorio::world::FactorioWorld;
    use factorio_bot_core::types::{Direction, FactorioEntity, Rect};
    use std::sync::Arc;

    /// Two rings' worth, so a test can tell ring 1 from ring 2.
    const RADIUS: f64 = REVEAL_PITCH * 2.;

    fn goal_at(centre: Position, radius: f64) -> Goal {
        Goal::Charted {
            around: centre,
            radius,
        }
    }

    fn goal() -> Goal {
        goal_at(Position::new(0., 0.), RADIUS)
    }

    /// A world whose ground is charted over the given box and nowhere else.
    fn world_charted_over(rect: Option<Rect>) -> FactorioWorld {
        let world = FactorioWorld::new();
        if let Some(rect) = rect {
            let mut tiles = Vec::new();
            factorio_bot_core::test_utils::spawn_water(&mut tiles, rect);
            world.update_chunk_tiles(tiles).expect("tiles are accepted");
        }
        world
    }

    /// A world charted at exactly these points and nowhere else.
    ///
    /// One tile per point rather than a rect over the whole lattice: at a
    /// 256-tile pitch the lattice spans 512 tiles each way, and filling that
    /// as a rect is a quarter-million tiles for a unit test. `is_charted`
    /// asks a one-tile box, so one tile per cell tests exactly the thing the
    /// method reads and nothing else.
    fn world_charted_at(points: &[Position]) -> FactorioWorld {
        let world = FactorioWorld::new();
        let mut tiles = Vec::new();
        // Deduplicated: adjacent cells share reveal edges (cell 0's edge at
        // +128 is cell 256's edge at -128), and the tile tree panics rather
        // than ignoring a second rect at the same place.
        let mut seen: Vec<(i64, i64)> = Vec::new();
        for point in points {
            #[expect(clippy::cast_possible_truncation, reason = "test lattice is small")]
            let key = (point.x() as i64, point.y() as i64);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            factorio_bot_core::test_utils::spawn_water(
                &mut tiles,
                Rect::new(
                    &Position::new(point.x() - 0.5, point.y() - 0.5),
                    &Position::new(point.x() + 0.5, point.y() + 0.5),
                ),
            );
        }
        world.update_chunk_tiles(tiles).expect("tiles are accepted");
        world
    }

    /// Every lattice cell a goal of `radius` about the origin would visit.
    fn lattice(radius: f64) -> Vec<Position> {
        #[expect(clippy::cast_possible_truncation, reason = "test lattice is small")]
        let rings = (radius / REVEAL_PITCH).floor() as i32;
        (0..=rings)
            .flat_map(|k| ring(&Position::new(0., 0.), k))
            .flat_map(|cell| covered(&cell))
            .collect()
    }

    /// The five points `is_covered` probes for a cell: charting all of them
    /// is what makes that cell read as needing no visit.
    fn covered(cell: &Position) -> Vec<Position> {
        [(0., 0.), (1., 0.), (-1., 0.), (0., 1.), (0., -1.)]
            .iter()
            .map(|(dx, dy)| {
                Position::new(cell.x() + dx * REVEAL_RADIUS, cell.y() + dy * REVEAL_RADIUS)
            })
            .collect()
    }

    fn state_of(world: FactorioWorld) -> PlanState {
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    fn charted_nowhere() -> PlanState {
        state_of(world_charted_over(None))
    }

    /// A spawner at `at`, ingested the way a chunk writeout would deliver it.
    fn with_spawner(world: &FactorioWorld, at: Position) {
        let mut entity = FactorioEntity::new_stone_furnace(&at, Direction::North);
        entity.name = "biter-spawner".to_owned();
        entity.entity_type = "unit-spawner".to_owned();
        world
            .update_chunk_entities(vec![entity])
            .expect("entities are accepted");
    }

    // --------------------------------------------------------- the lattice

    /// Squares tile the plane: ring `k` is exactly 8k cells, and no cell is
    /// ever emitted twice across rings. This is the property that makes the
    /// pattern a *covering* search rather than a pretty one.
    #[test]
    fn chebyshev_rings_are_disjoint_and_exactly_eight_k() {
        let centre = Position::new(0., 0.);
        let mut seen: Vec<(i64, i64)> = Vec::new();
        for k in 0..=4 {
            let cells = ring(&centre, k);
            let expected = if k == 0 { 1 } else { (8 * k) as usize };
            assert_eq!(cells.len(), expected, "ring {k} is the wrong size");
            for cell in cells {
                #[expect(clippy::cast_possible_truncation, reason = "test lattice is small")]
                let key = (cell.x() as i64, cell.y() as i64);
                assert!(!seen.contains(&key), "ring {k} repeats {cell}");
                seen.push(key);
            }
        }
    }

    /// Nearest-first is the property the owner's spiral was asked for, and it
    /// is what makes "the first copper found is the nearest copper" true.
    #[test]
    fn cells_come_out_nearest_ring_first() {
        let plan = survey_plan(&charted_nowhere(), &Position::new(0., 0.), RADIUS);
        let rings: Vec<i64> = plan
            .visit
            .iter()
            .map(|p| {
                #[expect(clippy::cast_possible_truncation, reason = "test lattice is small")]
                let ring = (p.x().abs().max(p.y().abs()) / REVEAL_PITCH).round() as i64;
                ring
            })
            .collect();
        assert!(
            rings.windows(2).all(|w| w[0] <= w[1]),
            "rings must not interleave: {rings:?}"
        );
        assert_eq!(rings.first(), Some(&0), "the centre is looked at first");
        assert_eq!(plan.visit.len(), 1 + 8 + 16, "rings 0, 1 and 2");
    }

    // --------------------------------------------------- charted and empty

    /// "Go and look at ground you can already see" is **satisfied**, not
    /// refused and not re-walked. This is what makes the goal idempotent
    /// across replans: a second expansion over a disc the first one charted
    /// must cost nothing.
    #[test]
    fn a_charted_disc_expands_to_no_actions() {
        let state = state_of(world_charted_at(&lattice(RADIUS)));
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let steps = Scout
            .expand(&goal(), &mut ctx)
            .expect("a charted disc expands");
        assert!(
            steps.is_empty(),
            "a disc the plan can already see needs no survey, got {steps:?}"
        );
    }

    /// Cells the model already has ground for are skipped as *charted*, which
    /// is a different fact from being skipped for a nest and has to be
    /// countable separately.
    #[test]
    fn already_charted_cells_are_skipped_and_named_as_such() {
        // Charted around the origin only, so ring 0 is known and rings 1-2
        // are not.
        let state = state_of(world_charted_at(&covered(&Position::new(0., 0.))));
        let plan = survey_plan(&state, &Position::new(0., 0.), RADIUS);
        assert_eq!(
            plan.skipped,
            vec![(Position::new(0., 0.), Skip::AlreadyCharted)],
            "only the centre cell is charted in this fixture"
        );
        assert_eq!(plan.visit.len(), 8 + 16, "rings 1 and 2 are still unknown");
        assert!(!plan.is_boxed_in(), "charted is not blocked");
    }

    /// **The regression that would have made the whole feature useless.**
    ///
    /// A cell is skipped only when standing there would teach the model
    /// nothing -- not when the single tile under it happens to be known. The
    /// seed-31337 t=0 dump is generated out to +/-320 at map creation, so the
    /// ring-1 cell at (256, -256) has a charted centre tile while the ground
    /// a bot standing there would generate reaches y = -384; the nearest
    /// crude oil, measured live at (131.5, -348.5), is inside that block and
    /// outside the +/-320 the model holds. Skipping on the centre tile alone
    /// skipped exactly that cell, so the spiral could never have found the
    /// oil it exists to find.
    #[test]
    fn a_cell_whose_centre_is_charted_but_whose_reveal_is_not_is_still_visited() {
        let cell = Position::new(REVEAL_PITCH, -REVEAL_PITCH);
        // The map-creation box: everything inside +/-320 is known, nothing
        // outside it is. The cell's centre is inside; its southern edge is not.
        let inside: Vec<Position> = lattice(REVEAL_PITCH * 2.)
            .into_iter()
            .filter(|p| p.x().abs() <= 320. && p.y().abs() <= 320.)
            .collect();
        let state = state_of(world_charted_at(&inside));
        assert!(
            state.is_charted(&cell),
            "fixture precondition: the centre tile is known"
        );
        assert!(
            !state.is_charted(&Position::new(cell.x(), cell.y() - REVEAL_RADIUS)),
            "fixture precondition: the ground it would reveal is not"
        );
        let plan = survey_plan(&state, &Position::new(0., 0.), REVEAL_PITCH * 2.);
        assert!(
            plan.visit.contains(&cell),
            "a cell that would reveal unknown ground must be visited; visiting {:?}",
            plan.visit
        );
    }

    // ------------------------------------------------------------- threats

    /// The first non-test caller of the threat index. A cell inside a
    /// spawner's stand-off is not walked to, and the skip names the nest --
    /// so a run can say which nest cost it which ground.
    #[test]
    fn a_cell_inside_the_standoff_of_a_nest_is_skipped_and_the_nest_is_named() {
        let world = world_charted_over(None);
        // Right on the ring-1 cell due east, so that cell is well inside the
        // stand-off and the cell due west is well outside it.
        let nest = Position::new(REVEAL_PITCH, 0.);
        with_spawner(&world, nest.clone());
        let plan = survey_plan(&state_of(world), &Position::new(0., 0.), RADIUS);
        let skipped_east = plan
            .skipped
            .iter()
            .find(|(cell, _)| *cell == nest)
            .map(|(_, skip)| skip.clone());
        assert!(
            matches!(skipped_east, Some(Skip::Threat { ref name, .. }) if name == "biter-spawner"),
            "the cell under the nest must be skipped and the nest named, got {skipped_east:?}"
        );
        assert!(
            !plan.visit.contains(&nest),
            "a cell inside the stand-off must not be visited"
        );
        assert!(
            plan.visit.contains(&Position::new(-REVEAL_PITCH * 2., 0.)),
            "ground away from the nest must still be explored"
        );
        assert!(plan.threat_skips() > 0);
    }

    /// The failure that must never be silent: every remaining cell is behind
    /// a nest. An empty plan here is indistinguishable from "already
    /// explored", so it refuses and says how many cells and why.
    #[test]
    fn a_spiral_boxed_in_by_nests_refuses_instead_of_planning_nothing() {
        let world = world_charted_over(None);
        // Nests on the centre and on every ring-1 cell, so that no cell of a
        // one-ring lattice is outside the stand-off. With a 256-tile pitch a
        // single nest cannot box the spiral in, which is itself the point:
        // being blocked takes a wall of nests, and when it happens the plan
        // must say so rather than emit nothing.
        with_spawner(&world, Position::new(0., 0.));
        for cell in ring(&Position::new(0., 0.), 1) {
            with_spawner(&world, cell);
        }
        let state = state_of(world);
        let plan = survey_plan(&state, &Position::new(0., 0.), REVEAL_PITCH);
        assert!(plan.visit.is_empty(), "fixture precondition");
        assert!(plan.is_boxed_in(), "every skip is a threat skip");

        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let err = Scout
            .expand(&goal_at(Position::new(0., 0.), REVEAL_PITCH), &mut ctx)
            .expect_err("being boxed in must not read as being finished");
        let rendered = err.to_string();
        assert!(rendered.contains("charted nest"), "{rendered}");
        assert!(rendered.contains("50 tiles"), "{rendered}");
    }

    /// Being finished and being blocked are different, and `is_boxed_in`
    /// is what separates them. A fully charted disc skips everything and is
    /// **not** boxed in.
    #[test]
    fn a_fully_charted_disc_is_not_reported_as_boxed_in() {
        let state = state_of(world_charted_at(&lattice(RADIUS)));
        let plan = survey_plan(&state, &Position::new(0., 0.), RADIUS);
        assert!(plan.visit.is_empty());
        assert!(!plan.skipped.is_empty());
        assert!(
            !plan.is_boxed_in(),
            "everything charted is finished, not blocked"
        );
    }

    // ------------------------------------------------------ shape and wiring

    /// A survey asserts no effect. The plan must not be able to satisfy a
    /// later charting check against ground no bot has been to yet -- that is
    /// exactly the free-vision dishonesty `EventKind::VisionMeasured` exists
    /// to expose, and it would be far worse asserted by the planner itself.
    #[test]
    fn a_survey_promises_the_plan_nothing() {
        let mut ctx = ExpansionCtx::new(charted_nowhere(), BotId(1));
        let steps = Scout.expand(&goal(), &mut ctx).expect("uncharted expands");
        assert!(!steps.is_empty());
        for step in &steps {
            let Step::Act(action) = step else {
                panic!("scout emits actions and no subgoals, got {step:?}");
            };
            assert!(
                matches!(action.kind, ActionKind::Survey { .. }),
                "got {:?}",
                action.kind
            );
            assert!(
                action.eff.is_empty(),
                "a survey learns nothing until it has run: {:?}",
                action.eff
            );
            assert!(
                action.label.starts_with("survey "),
                "the record greps this label: {}",
                action.label
            );
        }
    }

    /// **`holds` and `Scout` must be one predicate.** They were two, and the
    /// disagreement was silent: `PlanState::charting`'s seventeen fixed
    /// probes all land inside the +/-320 a seed-31337 map is created with, so
    /// `charted:0:0:256` read as already-satisfied while the lattice still
    /// had all eight ring-1 cells to visit -- and because `AlreadySatisfied`
    /// is registered ahead of `Scout`, the goal planned nothing at all and
    /// said nothing about it.
    #[test]
    fn a_goal_reads_satisfied_exactly_when_the_lattice_has_nothing_left() {
        // Charted over the seventeen-probe disc but not over the lattice:
        // the shape the two predicates disagree on.
        let inside: Vec<Position> = lattice(REVEAL_PITCH * 2.)
            .into_iter()
            .filter(|p| p.x().abs() <= 320. && p.y().abs() <= 320.)
            .collect();
        let state = state_of(world_charted_at(&inside));
        let goal = goal_at(Position::new(0., 0.), REVEAL_PITCH);
        let plan = survey_plan(&state, &Position::new(0., 0.), REVEAL_PITCH);
        assert!(!plan.visit.is_empty(), "fixture precondition: work remains");
        assert_eq!(
            crate::method::have::holds(&goal, &state),
            Some(false),
            "work remains, so the goal must not read as satisfied"
        );
        // And the registry must therefore route it to `scout`, not to
        // `already-satisfied`.
        assert_eq!(
            crate::method::have::default_registry()
                .find(&goal, &state, GoalSite::root())
                .map(Method::name),
            Some("scout")
        );
    }

    /// The registry has to route the goal, not just the method: this is the
    /// half that fails until `Scout` is registered in `default_registry`.
    #[test]
    fn the_registry_routes_a_charted_goal_to_scout() {
        let state = charted_nowhere();
        let method = crate::method::have::default_registry()
            .find(&goal(), &state, GoalSite::root())
            .map(Method::name);
        assert_eq!(method, Some("scout"));
    }
}
