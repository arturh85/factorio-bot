//! Putting the first space platform in orbit.
//!
//! [`Goal::Orbiting`] is what a Factorio 2.0 `create-space-platform` research
//! trigger becomes: the `space-platform` technology carries no science-pack
//! bill at all and is unlocked by the *act* of creating a platform, which in
//! turn is what unlocks the `asteroid-collector`, `crusher` and `cargo-bay`
//! recipes. So this is not a late step on the way to space science -- it is
//! the step that opens the rest of it
//! (`docs/superpowers/notes/2026-09-09-the-first-platform-is-a-science-factory.md`).
//!
//! # The sequence, and why it is two actions and not three
//!
//! Established live on 2.1.17 + Space Age, 2026-09-07
//! (`docs/superpowers/notes/2026-09-07-a-space-platform-needs-one-new-verb.md`):
//!
//! 1. [`ActionKind::CreatePlatform`] -- the platform appears pending, in
//!    `waiting_for_starter_pack`, with `surface = nil`;
//! 2. one ordinary [`ActionKind::Insert`] of the starter pack into the silo's
//!    [`InventorySlot::RocketSiloRocket`], **while the silo is building a
//!    rocket**, so it loads as cargo rather than sitting in a finished
//!    rocket's attached cargo unit and going nowhere;
//! 3. nothing. The silo launches itself. `LuaEntity.launch_rocket` returned
//!    `false` throughout the run that nevertheless produced a platform, and
//!    2.1.17 has no writable `auto_launch` to set, so there is no launch verb
//!    to emit and nothing to configure.
//!
//! # What this method refuses, and what it does not model
//!
//! **A rocket silo has to already stand.** Siting a 9x9 silo, feeding it
//! rocket parts and knowing whether it is mid-build are three separate pieces
//! of work this planner does none of, so the method refuses by name --
//! [`PlannerError::PlatformNeedsSilo`] -- rather than emitting a placement it
//! cannot cost. That is [`crate::method::extract::Extract`]'s shape: refuse
//! with the next missing prerequisite named, so a caller learns which step is
//! not modelled instead of receiving a makespan that is quietly too small.
//!
//! **Nothing here models "the silo is currently building a rocket."** The
//! plan's `Insert` states the tile and the slot honestly, and the game will
//! refuse it if the rocket is not there to load. That refusal is visible in
//! the run record; a planner-side precondition for it would need a reading
//! the mod does not send. Named here so the next reader does not go looking
//! for one.
//!
//! # The platform is invisible the moment it exists
//!
//! `mods/BotBridge/control.lua` drops every chunk that is not Nauvis',
//! counting them in `surface_chunk_drops`, so no platform tile, entity or
//! character ever reaches a [`crate::state::PlanState`]. Neither
//! [`Goal::Orbiting`] nor [`ActionKind::CreatePlatform`] carries a surface,
//! and that is not an oversight: giving them one would imply the planner
//! could then reason about what is up there, which it cannot. The goal is
//! satisfied by having done the act -- exactly what the game rewards -- and
//! closing the bridge gap is separate and much larger work.

use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::Position;

use crate::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::Ticks;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;

/// The prototype whose rocket carries the pack up. One name, in one place, so
/// the refusal and the `Insert` cannot disagree about which building is meant.
pub const SILO: &str = "rocket-silo";

/// Where the first platform goes.
///
/// **It does not move, and that is why the planet is a constant here.** A
/// `space-platform-thruster` costs 500 x `space-science-pack`, so thrusters
/// come *after* space science rather than before it and the first platform
/// sits in the orbit it was created in. Nauvis is where the silo is.
pub const FIRST_PLANET: &str = "nauvis";

/// The item a rocket carries up to make a pending platform real.
///
/// Named here so `method::have::Researched`'s trigger path and this module
/// cannot disagree about it. It is `rocket-silo`'s own unlock -- 20
/// steel-plate, 20 processing-unit and 60 space-platform-foundation -- and the
/// planner has never yet been able to make one; `have:space-platform-starter-pack:1`
/// refuses far upstream, on Nauvis, and that is expected.
pub const STARTER_PACK: &str = "space-platform-starter-pack";

/// What the created platform is called when the goal does not say.
///
/// A platform's name is a Factorio identifier the game hands back, not
/// something the plan reasons about, so it is a constant rather than a field
/// on the goal: a goal carrying a name would make two otherwise identical
/// goals unequal, and `AlreadySatisfied` already cannot answer this goal.
pub const PLATFORM_NAME: &str = "platform-1";

/// The `create_space_platform` call itself is one RCON round trip against the
/// force, with no walking, crafting or machine time behind it.
///
/// Priced the same way [`ActionKind::SetRecipe`] is: a dispatch whose cost is
/// the round trip. It is deliberately **not** zero -- a zero-duration action
/// is invisible in a makespan and reads as free, which is the defect
/// [`PlannerError::UnsupportedResearchTrigger`] exists to prevent.
pub const CREATE_TICKS: Ticks = 60;

/// How long the pack takes to go into the silo's rocket, in the same terms
/// every other [`ActionKind::Insert`] in this crate is priced.
pub const INSERT_TICKS: Ticks = 30;

/// The method for [`Goal::Orbiting`].
pub struct Orbit;

/// Where the plan will put the pack: the standing silo nearest `from`.
///
/// **Nearest, not first**, so a world with two silos does not have the answer
/// decided by whatever order the entity graph happened to return -- the same
/// reason every siting search in this crate orders by distance.
pub(crate) fn silo_for(state: &PlanState, from: &Position) -> Option<Position> {
    let mut silos: Vec<Position> = state
        .entities_named(SILO)
        .into_iter()
        .map(|e| e.position)
        .collect();
    silos.sort_by(|a, b| {
        calculate_distance(from, a)
            .total_cmp(&calculate_distance(from, b))
            .then_with(|| a.x().total_cmp(&b.x()))
            .then_with(|| a.y().total_cmp(&b.y()))
    });
    silos.into_iter().next()
}

/// Where the acting bot stands when it asks the force for a platform.
///
/// The call needs no position at all, but the action still has to be dealt to
/// a bot and settle somewhere, so it is pinned to the silo it is about to
/// load -- the bot is going there next regardless, and pinning it anywhere
/// else would buy a walk the plan does not need.
fn origin_of(ctx: &ExpansionCtx) -> Position {
    ctx.state
        .bot(ctx.chain_actor)
        .map(|b| b.position.clone())
        .unwrap_or_default()
}

impl Method for Orbit {
    fn name(&self) -> &'static str {
        "orbit"
    }

    /// Claims every [`Goal::Orbiting`], including the ones it will refuse.
    ///
    /// Deliberately unconditional, for [`crate::method::have::Researched`]'s
    /// reason: declining here would leave the registry with no method for the
    /// goal and hand the caller `NoApplicableMethod` -- "no method can satisfy
    /// goal: put a platform in orbit of nauvis", which reads as "space is out
    /// of reach in this world" rather than "there is no rocket silo".
    fn applicable(&self, goal: &Goal, _state: &PlanState) -> bool {
        matches!(goal, Goal::Orbiting { .. })
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Orbiting {
            planet,
            starter_pack,
            unlocks,
        } = goal
        else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };

        let origin = origin_of(ctx);
        // Asked before the pack is billed, for `Extract`'s reason: a starter
        // pack is 20 steel plate, 20 processing units and 60 foundations, and
        // planning all of that before discovering there is nowhere to launch
        // it from would be a true refusal in the wrong place.
        let silo = silo_for(&ctx.state, &origin).ok_or_else(|| PlannerError::PlatformNeedsSilo {
            planet: planet.clone(),
            silo: SILO.to_string(),
        })?;

        let mut steps: Vec<Step> = Vec::new();
        // `Have`, not `Produced`: the trigger fires on creating the platform,
        // not on making the pack, so a bot already carrying one is honestly
        // done with this part. `Holder::Share` because one action reads one
        // bot's inventory.
        steps.push(Step::Subgoal(Goal::Have {
            item: starter_pack.clone(),
            count: 1,
            whose: Holder::Share(ctx.chain_actor),
            via: None,
        }));

        let reach = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.build_distance)
            .unwrap_or(10.0);

        // The create call comes first, and the order is the finding rather
        // than a preference: a pack inserted into a silo with no pending
        // platform to receive it is consumed by a flight that delivers
        // nothing. `Step::Act` twice with the insert's `pre` naming the pack
        // is what orders them -- the create neither gains nor loses an item,
        // so nothing else would.
        let create_id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: create_id,
            kind: ActionKind::CreatePlatform {
                name: PLATFORM_NAME.to_string(),
                planet: planet.clone(),
                starter_pack: starter_pack.clone(),
            },
            pre: vec![
                // The pack is not spent here and this action does not touch
                // it, but requiring it orders the whole sequence behind the
                // subgoal above rather than leaving the create to float to
                // the front of the schedule and create a platform the plan
                // then fails to supply.
                Condition::HasItem {
                    who: Actor::Role,
                    item: starter_pack.clone(),
                    count: 1,
                },
            ],
            eff: Vec::new(),
            duration: CREATE_TICKS,
            pinned: None,
            label: format!("create platform in orbit of {planet}"),
        })));

        // The unlock rides on the *insert*, not on the create, and the choice
        // is conservative rather than certain. The act the trigger names is
        // creating a platform, and the live run did not separate the two --
        // by the time anything was read back, both had happened. What is
        // certain is that a platform created and never supplied stays
        // `waiting_for_starter_pack` forever, so hanging the technology on
        // the create would let a plan believe `space-platform` is researched
        // after an action that, alone, achieves nothing. Claiming it late is
        // recoverable; claiming it early is the failure this crate refuses
        // trigger technologies to avoid.
        let mut eff = vec![Effect::LoseItem {
            who: Actor::Role,
            item: starter_pack.clone(),
            count: 1,
        }];
        if let Some(tech) = unlocks {
            eff.push(Effect::Researched(tech.clone()));
        }
        steps.push(Step::Act(Box::new(Action {
            id: ctx.ids.next(),
            kind: ActionKind::Insert {
                pos: silo.clone(),
                entity: SILO.to_string(),
                slot: InventorySlot::RocketSiloRocket,
                item: starter_pack.clone(),
                count: 1,
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: silo.clone(),
                    radius: reach,
                    min_radius: 0.0,
                },
                Condition::HasItem {
                    who: Actor::Role,
                    item: starter_pack.clone(),
                    count: 1,
                },
            ],
            eff,
            duration: INSERT_TICKS,
            pinned: None,
            label: format!("load {starter_pack} into the rocket at {silo}"),
        })));

        Ok(steps)
    }

    fn refusal(&self, goal: &Goal, ctx: &ExpansionCtx) -> Option<PlannerError> {
        let Goal::Orbiting { planet, .. } = goal else {
            return None;
        };
        if silo_for(&ctx.state, &origin_of(ctx)).is_none() {
            return Some(PlannerError::PlatformNeedsSilo {
                planet: planet.clone(),
                silo: SILO.to_string(),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use factorio_bot_core::factorio::util::add_to_rect;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{Direction, FactorioEntity, Rect};

    use super::*;
    use crate::ids::BotId;

    /// A `rocket-silo` at its real 9x9 collision box, written out because there
    /// is no production constructor for one. An odd footprint covers nine tiles
    /// per axis, so a legal centre is a tile centre -- a half-integer. See
    /// `method::util::tile_alignment`.
    fn silo(position: &Position) -> FactorioEntity {
        FactorioEntity {
            name: SILO.into(),
            entity_type: "rocket-silo".into(),
            position: position.clone(),
            bounding_box: add_to_rect(&Rect::from_wh(8.8, 8.8), position),
            direction: Direction::North as u8,
            ..Default::default()
        }
    }

    fn state_with(entities: Vec<FactorioEntity>) -> PlanState {
        let world = fixture_world();
        world
            .update_chunk_entities(entities)
            .expect("a fixture world accepts these entities");
        PlanState::from_world(Arc::new(world), &[BotId(1)])
    }

    fn goal() -> Goal {
        Goal::Orbiting {
            planet: "nauvis".into(),
            starter_pack: "space-platform-starter-pack".into(),
            unlocks: Some("space-platform".into()),
        }
    }

    fn acts(steps: &[Step]) -> Vec<&Action> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::Act(a) => Some(a.as_ref()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_world_with_no_silo_refuses_by_name_rather_than_planning_a_launch() {
        let ctx = ExpansionCtx::new(state_with(Vec::new()), BotId(1));
        let err = Orbit
            .refusal(&goal(), &ctx)
            .expect("no silo stands, so this refuses");
        let text = err.to_string();
        assert!(text.contains("rocket-silo"), "{text}");
        assert!(text.contains("nauvis"), "{text}");
    }

    /// And `expand` refuses too, not only `refusal` -- a method's `expand` is
    /// reachable from tests and from a caller that never consulted
    /// applicability, and this one always claims the goal.
    #[test]
    fn expand_refuses_with_no_silo_as_well() {
        let mut ctx = ExpansionCtx::new(state_with(Vec::new()), BotId(1));
        assert!(matches!(
            Orbit.expand(&goal(), &mut ctx),
            Err(PlannerError::PlatformNeedsSilo { .. })
        ));
    }

    #[test]
    fn the_create_comes_before_the_insert_and_the_unlock_rides_on_the_insert() {
        let state = state_with(vec![silo(&Position::new(60.5, 0.5))]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let steps = Orbit.expand(&goal(), &mut ctx).expect("plans");
        let acts = acts(&steps);
        assert_eq!(acts.len(), 2, "one create and one insert: {acts:?}");
        assert!(
            matches!(acts[0].kind, ActionKind::CreatePlatform { .. }),
            "{:?}",
            acts[0].kind
        );
        // The order is the finding of 2026-09-07, not a preference: a pack
        // inserted before a platform is pending is consumed for nothing.
        assert!(
            matches!(
                &acts[1].kind,
                ActionKind::Insert {
                    slot: InventorySlot::RocketSiloRocket,
                    ..
                }
            ),
            "{:?}",
            acts[1].kind
        );
        assert!(
            !acts[0]
                .eff
                .iter()
                .any(|e| matches!(e, Effect::Researched(_))),
            "the create alone achieves nothing and must not claim the technology"
        );
        assert!(
            acts[1]
                .eff
                .iter()
                .any(|e| matches!(e, Effect::Researched(t) if t == "space-platform")),
            "{:?}",
            acts[1].eff
        );
    }

    /// A goal with no `unlocks` -- `goal.orbiting("nauvis")` from a script --
    /// plans the same two actions and marks no technology.
    #[test]
    fn a_platform_goal_with_no_unlock_marks_no_technology() {
        let state = state_with(vec![silo(&Position::new(60.5, 0.5))]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let steps = Orbit
            .expand(
                &Goal::Orbiting {
                    planet: "nauvis".into(),
                    starter_pack: "space-platform-starter-pack".into(),
                    unlocks: None,
                },
                &mut ctx,
            )
            .expect("plans");
        assert_eq!(acts(&steps).len(), 2);
        assert!(
            !steps.iter().any(|step| matches!(step, Step::Act(a)
                if a.eff.iter().any(|e| matches!(e, Effect::Researched(_))))),
            "nothing asked for a technology, so nothing may mark one"
        );
    }

    #[test]
    fn the_pack_is_a_subgoal_and_the_create_waits_on_it() {
        let state = state_with(vec![silo(&Position::new(60.5, 0.5))]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let steps = Orbit.expand(&goal(), &mut ctx).expect("plans");
        assert!(
            steps.iter().any(|s| matches!(
                s,
                Step::Subgoal(Goal::Have { item, count: 1, .. })
                    if item == "space-platform-starter-pack"
            )),
            "{steps:?}"
        );
        let create = acts(&steps)[0];
        assert!(
            create.pre.iter().any(|c| matches!(
                c,
                Condition::HasItem { item, .. } if item == "space-platform-starter-pack"
            )),
            "without this the create floats ahead of the pack it needs: {:?}",
            create.pre
        );
    }

    /// The insert names the *nearest* silo, so a world with two does not have
    /// the answer decided by entity-graph order.
    #[test]
    fn the_nearest_silo_takes_the_pack() {
        let state = state_with(vec![
            silo(&Position::new(150.5, 0.5)),
            silo(&Position::new(60.5, 0.5)),
        ]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let steps = Orbit.expand(&goal(), &mut ctx).expect("plans");
        let ActionKind::Insert { pos, .. } = &acts(&steps)[1].kind else {
            panic!("the second action is the insert");
        };
        // The bot stands at the fixture's origin, so 60.5 is the nearer of
        // the two. Derived from the roster rather than hard-coded twice: the
        // assertion is "the nearer one", not "this coordinate".
        assert_eq!(pos, &Position::new(60.5, 0.5));
    }

    /// The registry claims it. Without this the goal reaches nothing and the
    /// driver answers `NoApplicableMethod`, which reads as a fact about the
    /// world rather than a gap in the registry.
    #[test]
    fn the_production_registry_reaches_it() {
        let state = state_with(vec![silo(&Position::new(60.5, 0.5))]);
        let registry = crate::method::have::registry_for(&[BotId(1)]);
        assert_eq!(
            registry
                .find(&goal(), &state, crate::method::GoalSite::root())
                .map(crate::method::Method::name),
            Some("orbit")
        );
    }
}
