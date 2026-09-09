//! The world a plan leaves standing, so the *next* plan can be made against it.
//!
//! # Why this exists
//!
//! Every baseline this project takes is `plan --world <t=0 dump>`, and a t=0
//! dump has no factory in it. A live run does not stay at t=0: the supervisor
//! plans, executes, and **replans** whenever a batch truncates, and the second
//! expansion meets a world where the first plan's work already stands as map
//! facts. On 2026-09-09 `9f549b1c` moved all eight baselines correctly, was
//! reviewed and merged, and refused in the live run with the byte-identical
//! blocker it had been written to remove -- *"a cell ALREADY MAKES
//! copper-plate"*, a sentence only a replan can say. Offline there was no
//! cell, so the fixed path was never reached and the fix looked perfect.
//!
//! This module is the offline half of that path. Given a plan and the state it
//! was made against, [`world_after`] returns a **new surface** with the plan's
//! placements, recipes and choppings applied as ordinary map facts -- the same
//! `on_some_entity_created` the mod's writeout reaches -- so that a fresh
//! [`PlanState::from_world`] over it is what the supervisor's next round would
//! build (`plan_rounds` in `crates/scripting_lua` rebuilds its state from the
//! live world every round). No Factorio, no RCON, seconds.
//!
//! # A fresh state, not a fork -- and that is the whole point
//!
//! The sustain tests have a `built_world` helper that forks the *plan state*
//! and creates the placed entities in its overlay. That is right for asking
//! "what did this expansion build" and wrong for a replan: a fork carries
//! every reservation, claim and queue the first expansion made, so a promise
//! one method kept privately (the reserved exit that `a69ae64c` turned into
//! state) would survive into the second plan and mask exactly the defect the
//! replan check exists to find. The live replan starts from the world and
//! nothing else, so this does too.
//!
//! # What it applies, and what it does not
//!
//! Applied, in the network's topological order:
//! - [`ActionKind::Place`] -- the entity is created on the surface, with its
//!   collision box filled from the prototype when the action carried none,
//!   exactly as `PlanState::create_entity` fills it for the overlay;
//! - [`ActionKind::SetRecipe`] -- the recipe is set on the standing machine;
//! - [`ActionKind::Chop`] -- the tree or rock is deleted;
//! - [`ActionKind::Mine`] -- the resource tile is debited, so a replan sees
//!   what a mined patch has left.
//!
//! **Not applied, on purpose, and each is a stated limit rather than an
//! oversight**: bot inventories and positions (`GainItem`/`LoseItem`, walks),
//! chest contents (`BufferGain`/`BufferLose`), and research. A replan after a
//! *lost* batch is exactly a replan whose inventory the plan cannot vouch for,
//! and the planner already treats an unread buffer as unknown. The subject of
//! this module is **what stands on the ground**, because that is what every
//! siting, routing and recovery method reads and what no t=0 dump can carry.
//! A caller that needs the rest has `--resume-from` and a live game.
//!
//! **A whole plan applied is "the batch ran to completion".** The live
//! replans of 2026-09-09 followed *partial* batches -- 80 abandoned steps
//! behind one failed take -- so [`world_after`] takes a predicate saying which
//! actions count as done, and the CLI's `--done-by <tick>` cuts a schedule at
//! a tick the way a truncated batch would.

use crate::action::ActionKind;
use crate::ids::{ActionId, BotId};
use crate::network::ActionNetwork;
use crate::state::PlanState;
use factorio_bot_core::constants::BOT_FORCE;
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::miette::Result;
use factorio_bot_core::num_traits::FromPrimitive;
use factorio_bot_core::record::standing::StandingSnapshot;
use factorio_bot_core::types::{Direction, FactorioEntity};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The world a run's own record says it had, on top of `base`.
///
/// The other half of [`world_after`]: that one applies a plan the planner
/// made, this one applies what a finished run's `map.jsonl` keyframe and
/// `samples.jsonl` recorded -- see
/// [`factorio_bot_core::record::standing`] for what a snapshot carries and
/// what it cannot. Both return a new surface with the entities as map facts,
/// so `PlanState::from_world` over either is the state the supervisor's next
/// round would build.
///
/// A technology the snapshot names is marked researched on the acting force
/// (`BOT_FORCE`), so a recipe it unlocks reads open to `recipe_gate` -- the
/// caller's stated hypothesis, never a fact off the record. Bots named in
/// the snapshot are moved and given the snapshot's inventory;
/// a bot the dump does not know is skipped and counted in
/// [`Standing::unapplied`], because inventing a player is the fabricated
/// roster this repo already warns about. Each entity's collision box is
/// filled from the prototype, as [`world_after`] fills it: `EntityGraph::add`
/// **drops** an entity whose box is zero, silently, so a snapshot applied
/// without this step would be a world with nothing in it that reads as
/// clean ground.
pub fn world_with(
    base: &Arc<FactorioSurface>,
    snapshot: &StandingSnapshot,
) -> (Arc<FactorioSurface>, Standing) {
    let mut surface: FactorioSurface = (**base).clone();
    let mut standing = Standing::default();
    {
        let globals = Arc::get_mut(&mut surface.globals)
            .expect("a freshly cloned surface holds its globals alone");
        for bot in &snapshot.bots {
            match globals.players.get_mut(&bot.id) {
                Some(mut player) => {
                    player.position = bot.position.clone();
                    player.main_inventory = bot.inventory.clone();
                }
                None => standing.unapplied += 1,
            }
        }
    }
    let world = Arc::new(surface);
    // A probe over the clone, only for the prototype's collision box.
    let bots: Vec<BotId> = snapshot.bots.iter().map(|b| BotId(b.id)).collect();
    let probe = PlanState::from_world(world.clone(), &bots);
    for entity in &snapshot.entities {
        let entity_type = world
            .globals
            .entity_prototypes
            .get(&entity.name)
            .map(|p| p.entity_type.clone())
            .unwrap_or_default();
        let bounding_box = Direction::from_u8(entity.direction)
            .and_then(|facing| probe.collision_area_facing(&entity.name, &entity.position, facing))
            .unwrap_or_default();
        let standing_entity = FactorioEntity {
            name: entity.name.clone(),
            entity_type,
            position: entity.position.clone(),
            direction: entity.direction,
            bounding_box,
            ..Default::default()
        };
        if standing_entity.bounding_box.width() == 0. {
            // `EntityGraph::add` would drop it without a word.
            standing.unapplied += 1;
            continue;
        }
        if world.on_some_entity_created(standing_entity).is_ok() {
            standing.placed += 1;
        }
    }
    for recipe in &snapshot.recipes {
        if world
            .entity_graph
            .set_recipe(&recipe.position, &recipe.recipe)
        {
            standing.recipes_set += 1;
        }
    }
    // Research is on the force, which lives in the globals; `probe` shares
    // the world now, so `Arc::get_mut` is closed and this goes in through
    // the force map's own interior mutability -- the door `update_force`
    // uses.
    for tech in &snapshot.researched {
        let mut done = false;
        if let Some(mut force) = world.globals.forces.get_mut(BOT_FORCE)
            && let Some(technology) = force.technologies.get_mut(tech)
        {
            technology.researched = true;
            done = true;
        }
        if done {
            standing.researched += 1;
        } else {
            standing.unapplied += 1;
        }
    }
    (world, standing)
}

/// What a batch leaves done when `failed` fails: everything except it and
/// every action downstream of it.
///
/// This is the executor's own rule -- a failed step abandons its dependents
/// and nothing else (`abandoned: predecessor 643 failed`), while every other
/// bot's work and the failed bot's independent steps go on. The live replans
/// of 2026-09-09 met exactly this world: `take 10 copper-plate from the cell`
/// failed and 48 steps behind it were abandoned, the science tail among
/// them, while the copper cell and every coal run stood finished.
///
/// A cut by tick cannot produce this shape. The abandoned subtree is a
/// dependency cone, not a suffix of the schedule: the supply link's first
/// belt at `[27.5,-43.5]` stood while its arm did not, because the belt was
/// upstream of the take and the arm downstream. That partial link is what
/// the live replan then tripped over.
pub fn survivors_of_failure(net: &ActionNetwork, failed: ActionId) -> BTreeSet<ActionId> {
    let mut successors: BTreeMap<ActionId, Vec<ActionId>> = BTreeMap::new();
    for action in net.actions() {
        for (pred, _) in net.preds(action.id) {
            successors.entry(pred).or_default().push(action.id);
        }
    }
    let mut abandoned: BTreeSet<ActionId> = BTreeSet::new();
    let mut frontier = vec![failed];
    while let Some(id) = frontier.pop() {
        if !abandoned.insert(id) {
            continue;
        }
        if let Some(next) = successors.get(&id) {
            frontier.extend(next.iter().copied());
        }
    }
    net.actions()
        .map(|action| action.id)
        .filter(|id| !abandoned.contains(id))
        .collect()
}

/// What [`world_after`] put on the surface, so a caller can say whether the
/// standing world it is about to replan against actually has anything in it.
///
/// A replan check whose first plan placed nothing measures nothing; the
/// counts are how the check refuses to be green by accident.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Standing {
    /// Entities created on the surface.
    pub placed: usize,
    /// Recipes set on standing machines.
    pub recipes_set: usize,
    /// Trees and rocks deleted.
    pub chopped: usize,
    /// Resource tiles debited.
    pub mined: usize,
    /// Technologies marked researched on the acting force.
    pub researched: usize,
    /// What was asked for and not applied: actions this module does not
    /// model (crafts, inserts, walks, research...), a snapshot entity with
    /// no prototype to size it, a snapshot bot the dump has no player for.
    /// Reported, not hidden: a reader deciding whether the standing world is
    /// faithful enough needs the number.
    pub unapplied: usize,
}

/// The surface `net` leaves behind, as a new, independent world.
///
/// `done` says which actions count as executed; pass `|_| true` for the whole
/// plan. The returned surface is a deep clone of `state.base()` with the done
/// actions' physical effects applied through the same entry points the mod's
/// writeout uses, so `PlanState::from_world(world_after(..), bots)` is the
/// state a supervisor round would build after that batch settled.
///
/// Errors only when the network has no topological order, which `plan_best`
/// already refuses before handing a network back.
pub fn world_after(
    state: &PlanState,
    net: &ActionNetwork,
    done: impl Fn(ActionId) -> bool,
) -> Result<(Arc<FactorioSurface>, Standing), crate::error::PlannerError> {
    let surface: FactorioSurface = (**state.base()).clone();
    let mut standing = Standing::default();
    for id in net.topo_order()? {
        if !done(id) {
            continue;
        }
        let Some(action) = net.action(id) else {
            continue;
        };
        match &action.kind {
            ActionKind::Place { entity } => {
                let mut entity = (**entity).clone();
                if (entity.bounding_box.width() == 0. || entity.bounding_box.height() == 0.)
                    && let Some(area) = Direction::from_u8(entity.direction).and_then(|facing| {
                        state.collision_area_facing(&entity.name, &entity.position, facing)
                    })
                {
                    entity.bounding_box = area;
                }
                if surface.on_some_entity_created(entity).is_ok() {
                    standing.placed += 1;
                }
            }
            ActionKind::SetRecipe { pos, recipe, .. } => {
                if surface.entity_graph.set_recipe(pos, recipe) {
                    standing.recipes_set += 1;
                }
            }
            ActionKind::Chop { pos, .. } => {
                // The standing entity, read off the surface being built rather
                // than off `state`: an earlier action in this same order may
                // already have removed or replaced it.
                let standing_entity = surface
                    .entity_graph
                    .entity_at(pos)
                    .and_then(|id| surface.entity_graph.entity_by_id(id));
                if let Some(entity) = standing_entity
                    && surface.on_some_entity_deleted(entity).is_ok()
                {
                    standing.chopped += 1;
                }
            }
            ActionKind::Mine { pos, item, count } => {
                surface.entity_graph.resource_mined(item, pos, *count);
                standing.mined += 1;
            }
            _ => standing.unapplied += 1,
        }
    }
    Ok((Arc::new(surface), standing))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::ids::BotId;

    fn action(net: &mut ActionNetwork, kind: ActionKind) -> ActionId {
        let id = ActionId(u32::try_from(net.len()).unwrap() + 1);
        net.add(Action {
            id,
            kind,
            pre: Vec::new(),
            eff: Vec::new(),
            duration: 0,
            pinned: None,
            label: String::new(),
        })
    }
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{FactorioEntity, Position};

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    fn place(net: &mut ActionNetwork, entity: FactorioEntity) -> ActionId {
        action(
            net,
            ActionKind::Place {
                entity: Box::new(entity),
            },
        )
    }

    /// The one property the module exists for: a placement is a MAP FACT on
    /// the returned surface, visible to a state built fresh from it.
    #[test]
    fn a_placement_stands_on_the_new_surface_as_a_map_fact() {
        let state = state();
        let at = Position::new(10.5, 10.5);
        assert!(
            state.entity_at(&at).is_none(),
            "the fixture has nothing at {at} to start with"
        );
        let mut net = ActionNetwork::new();
        place(
            &mut net,
            FactorioEntity::new_stone_furnace(&at, Direction::North),
        );

        let (world, standing) = world_after(&state, &net, |_| true).expect("a one-action network");
        assert_eq!(standing.placed, 1);

        let fresh = PlanState::from_world(world, &[BotId(1)]);
        let found = fresh
            .entity_at(&at)
            .expect("the furnace stands on the new surface");
        assert_eq!(found.name, "stone-furnace");
        assert!(
            found.bounding_box.width() > 0.,
            "the box is filled from the prototype, not left at zero: {:?}",
            found.bounding_box
        );
    }

    /// And the state the plan was made against is untouched -- the returned
    /// world is a clone, not a mutation of the base behind `state`.
    #[test]
    fn the_original_world_is_not_mutated() {
        let state = state();
        let at = Position::new(10.5, 10.5);
        let mut net = ActionNetwork::new();
        place(
            &mut net,
            FactorioEntity::new_stone_furnace(&at, Direction::North),
        );
        let _ = world_after(&state, &net, |_| true).unwrap();
        assert!(
            state.entity_at(&at).is_none(),
            "the base surface behind the first plan must stay as it was"
        );
    }

    /// `done` is honoured: an action it refuses is not applied and is not
    /// counted anywhere, because a step that never ran left nothing behind.
    #[test]
    fn an_action_not_yet_done_leaves_nothing_standing() {
        let state = state();
        let first = Position::new(10.5, 10.5);
        let second = Position::new(12.5, 12.5);
        assert!(
            state.entity_at(&first).is_none() && state.entity_at(&second).is_none(),
            "the fixture must have nothing on either tile, or this test measures the fixture"
        );
        let mut net = ActionNetwork::new();
        let a = place(
            &mut net,
            FactorioEntity::new_stone_furnace(&first, Direction::North),
        );
        let _b = place(
            &mut net,
            FactorioEntity::new_stone_furnace(&second, Direction::North),
        );
        let (world, standing) = world_after(&state, &net, |id| id == a).unwrap();
        assert_eq!(standing.placed, 1);
        assert_eq!(standing.unapplied, 0);
        let fresh = PlanState::from_world(world, &[BotId(1)]);
        assert!(fresh.entity_at(&first).is_some());
        assert!(fresh.entity_at(&second).is_none());
    }

    /// A recipe set by the plan is on the machine the next plan reads.
    #[test]
    fn a_recipe_set_by_the_plan_is_on_the_standing_machine() {
        let state = state();
        let at = Position::new(10.5, 10.5);
        let mut net = ActionNetwork::new();
        let mut machine = FactorioEntity::new_stone_furnace(&at, Direction::North);
        machine.name = "assembling-machine-1".to_string();
        let placed = place(&mut net, machine);
        let set = action(
            &mut net,
            ActionKind::SetRecipe {
                pos: at.clone(),
                entity: "assembling-machine-1".to_string(),
                recipe: "iron-gear-wheel".to_string(),
            },
        );
        net.link(placed, set, 0);
        let (world, standing) = world_after(&state, &net, |_| true).unwrap();
        assert_eq!(standing.recipes_set, 1, "{standing:?}");
        let fresh = PlanState::from_world(world, &[BotId(1)]);
        assert_eq!(
            fresh.entity_at(&at).unwrap().recipe.as_deref(),
            Some("iron-gear-wheel")
        );
    }

    /// A failure abandons its cone and nothing else.
    #[test]
    fn a_failure_abandons_its_dependents_and_spares_the_rest() {
        let state = state();
        let mut net = ActionNetwork::new();
        let a = place(
            &mut net,
            FactorioEntity::new_stone_furnace(&Position::new(10.5, 10.5), Direction::North),
        );
        let b = place(
            &mut net,
            FactorioEntity::new_stone_furnace(&Position::new(14.5, 10.5), Direction::North),
        );
        let c = place(
            &mut net,
            FactorioEntity::new_stone_furnace(&Position::new(18.5, 10.5), Direction::North),
        );
        let other = place(
            &mut net,
            FactorioEntity::new_stone_furnace(&Position::new(30.5, 10.5), Direction::North),
        );
        net.link(a, b, 0);
        net.link(b, c, 0);
        let done = survivors_of_failure(&net, b);
        assert!(done.contains(&a), "upstream of the failure is done");
        assert!(!done.contains(&b), "the failed action itself is not");
        assert!(!done.contains(&c), "downstream is abandoned");
        assert!(done.contains(&other), "an unrelated action goes on");
        let (world, standing) = world_after(&state, &net, |id| done.contains(&id)).unwrap();
        assert_eq!(standing.placed, 2);
        let fresh = PlanState::from_world(world, &[BotId(1)]);
        assert!(fresh.entity_at(&Position::new(14.5, 10.5)).is_none());
    }

    /// A snapshot's entity is a map fact on the new surface, and its bot is
    /// where the snapshot says.
    #[test]
    fn a_snapshot_stands_on_the_new_surface_with_its_bots_moved() {
        use factorio_bot_core::record::standing::{BotAt, RecipeAt, StandingEntity};
        use factorio_bot_core::types::FactorioPlayer;
        let mut base = fixture_world();
        Arc::get_mut(&mut base.globals).unwrap().players.insert(
            1,
            FactorioPlayer {
                player_id: 1,
                ..FactorioPlayer::default()
            },
        );
        let base = Arc::new(base);
        let at = Position::new(10.5, 10.5);
        let snapshot = StandingSnapshot {
            entities: vec![
                StandingEntity {
                    name: "stone-furnace".to_string(),
                    position: at.clone(),
                    direction: 0,
                },
                StandingEntity {
                    name: "no-such-prototype".to_string(),
                    position: Position::new(12.5, 12.5),
                    direction: 0,
                },
            ],
            bots: vec![
                BotAt {
                    id: 1,
                    position: Position::new(40.5, 40.5),
                    inventory: [("coal".to_string(), 9)].into_iter().collect(),
                },
                BotAt {
                    id: 200,
                    position: Position::new(0.5, 0.5),
                    inventory: BTreeMap::new(),
                },
            ],
            recipes: vec![RecipeAt {
                position: at.clone(),
                recipe: "iron-plate".to_string(),
            }],
            researched: vec!["no-such-technology".to_string()],
            ..StandingSnapshot::default()
        };
        let (world, standing) = world_with(&base, &snapshot);
        assert_eq!(standing.placed, 1, "{standing:?}");
        assert_eq!(standing.recipes_set, 1, "{standing:?}");
        assert_eq!(
            standing.unapplied, 3,
            "the unknown prototype, the unknown bot and the unknown technology are counted: \
             {standing:?}"
        );
        let fresh = PlanState::from_world(world, &[BotId(1)]);
        let found = fresh.entity_at(&at).expect("the furnace stands");
        assert!(found.bounding_box.width() > 0.);
        assert_eq!(found.recipe.as_deref(), Some("iron-plate"));
        let bot = fresh.bot(BotId(1)).expect("bot 1 is in the dump");
        assert_eq!(bot.position, Position::new(40.5, 40.5));
        assert_eq!(fresh.inventory_count(BotId(1), "coal"), 9);
        assert!(
            PlanState::from_world(base, &[BotId(1)])
                .entity_at(&at)
                .is_none(),
            "the base world is untouched"
        );
    }

    /// Actions this module does not model are counted, not silently dropped.
    #[test]
    fn what_is_not_applied_is_counted() {
        let state = state();
        let mut net = ActionNetwork::new();
        action(
            &mut net,
            ActionKind::Craft {
                item: "iron-gear-wheel".to_string(),
                count: 1,
            },
        );
        let (_, standing) = world_after(&state, &net, |_| true).unwrap();
        assert_eq!(standing.unapplied, 1);
        assert_eq!(standing.placed, 0);
    }
}
