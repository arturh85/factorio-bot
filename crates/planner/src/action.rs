//! What a bot can do: actions, and the conditions and effects that describe them as data.

use crate::error::PlannerError;
use crate::ids::{ActionId, BotId, ItemId, Ticks};
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::types::{FactorioEntity, Pos, Position};
use serde::{Deserialize, Serialize};

/// Who an action's condition or effect applies to.
///
/// `Role` means "whichever bot runs this action" and is bound by the
/// scheduler. Nothing outside `schedule()` may resolve it without an
/// explicit binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Actor {
    Bound(BotId),
    Role,
}

impl Actor {
    pub fn resolve(&self, binding: BotId) -> BotId {
        match self {
            Actor::Bound(id) => *id,
            Actor::Role => binding,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    HasItem {
        who: Actor,
        item: ItemId,
        count: u32,
    },
    AtPosition {
        who: Actor,
        pos: Position,
        radius: f64,
    },
    EntityAt {
        pos: Position,
        name: ItemId,
    },
    PositionFree {
        pos: Position,
    },
    Researched(String),
    ResourceAvailable {
        pos: Position,
        item: ItemId,
        count: u32,
    },
}

impl Condition {
    pub fn holds(&self, state: &PlanState, binding: BotId) -> bool {
        match self {
            Condition::HasItem { who, item, count } => {
                state.inventory_count(who.resolve(binding), item) >= *count
            }
            Condition::AtPosition { who, pos, radius } => match state.bot(who.resolve(binding)) {
                Some(bot) => calculate_distance(&bot.position, pos) <= *radius,
                None => false,
            },
            Condition::EntityAt { pos, name } => {
                matches!(state.entity_at(pos), Some(e) if &e.name == name)
            }
            Condition::PositionFree { pos } => state.is_position_free(pos),
            Condition::Researched(tech) => state.is_researched(tech),
            Condition::ResourceAvailable { pos, item, count } => {
                state.resource_available(pos, item) >= *count
            }
        }
    }

    /// The position this condition requires the acting bot to stand near, if any.
    /// The scheduler reads this to decide whether a walk is needed.
    pub fn required_position(&self) -> Option<(Position, f64)> {
        match self {
            Condition::AtPosition {
                who: Actor::Role,
                pos,
                radius,
            } => Some((pos.clone(), *radius)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Condition::HasItem { item, count, .. } => write!(f, "has {} {}", count, item),
            Condition::AtPosition { pos, radius, .. } => {
                write!(f, "within {} of {}", radius, pos)
            }
            Condition::EntityAt { pos, name } => write!(f, "{} at {}", name, pos),
            Condition::PositionFree { pos } => write!(f, "{} is free", pos),
            Condition::Researched(tech) => write!(f, "{} researched", tech),
            Condition::ResourceAvailable { pos, item, count } => {
                write!(f, "{} {} available at {}", count, item, pos)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    GainItem {
        who: Actor,
        item: ItemId,
        count: u32,
    },
    LoseItem {
        who: Actor,
        item: ItemId,
        count: u32,
    },
    CreateEntity(Box<FactorioEntity>),
    RemoveEntity {
        pos: Position,
    },
    ConsumeResource {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    Researched(String),
}

impl Effect {
    pub fn apply(&self, state: &mut PlanState, binding: BotId) -> Result<(), PlannerError> {
        match self {
            Effect::GainItem { who, item, count } => {
                state.gain(who.resolve(binding), item, *count);
                Ok(())
            }
            Effect::LoseItem { who, item, count } => state.lose(who.resolve(binding), item, *count),
            Effect::CreateEntity(entity) => {
                state.create_entity((**entity).clone());
                Ok(())
            }
            Effect::RemoveEntity { pos } => {
                state.remove_entity(pos);
                Ok(())
            }
            Effect::ConsumeResource { pos, item, count } => {
                state.consume_resource(pos, item, *count)
            }
            Effect::Researched(tech) => {
                state.set_researched(tech);
                Ok(())
            }
        }
    }

    /// Does this effect contribute to making `cond` true? Ordering inference only:
    /// item matching deliberately ignores counts — see `ActionNetwork::infer_edges`.
    pub fn satisfies(&self, cond: &Condition) -> bool {
        match (self, cond) {
            (Effect::GainItem { item, .. }, Condition::HasItem { item: want, .. }) => item == want,
            (Effect::CreateEntity(e), Condition::EntityAt { pos, name }) => {
                &e.name == name && Pos::from(&e.position) == Pos::from(pos)
            }
            (Effect::RemoveEntity { pos }, Condition::PositionFree { pos: want }) => {
                Pos::from(pos) == Pos::from(want)
            }
            (Effect::Researched(t), Condition::Researched(want)) => t == want,
            _ => false,
        }
    }
}

/// Which inventory of a target entity an insert or remove addresses.
///
/// Deliberately semantic rather than numeric. Factorio's `defines.inventory`
/// integers are entity-type dependent — the same number means different things
/// for a chest and a furnace (see `mods/BotBridge/control.lua`
/// `inventory_type_name(invtype, enttype)`) — and they move between game
/// versions. The executor resolves these to numbers against the running game.
///
/// Ordering is derived and load-bearing: the planner is deterministic, so every
/// type reachable from an `ActionNetwork` must order totally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InventorySlot {
    Chest,
    FurnaceSource,
    FurnaceResult,
    Fuel,
    AssemblerInput,
    AssemblerOutput,
    LabInput,
}

impl InventorySlot {
    /// The name used to look this slot up in the game's `defines.inventory`.
    pub fn defines_key(self) -> &'static str {
        match self {
            InventorySlot::Chest => "chest",
            InventorySlot::FurnaceSource => "furnace_source",
            InventorySlot::FurnaceResult => "furnace_result",
            InventorySlot::Fuel => "fuel",
            InventorySlot::AssemblerInput => "assembling_machine_input",
            InventorySlot::AssemblerOutput => "assembling_machine_output",
            InventorySlot::LabInput => "lab_input",
        }
    }
}

/// What a bot physically does. Carries the payload the executor needs;
/// the planner reasons from `pre` and `eff`, never from this.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ActionKind {
    Mine {
        pos: Position,
        item: ItemId,
        count: u32,
    },
    Craft {
        item: ItemId,
        count: u32,
    },
    Place {
        entity: Box<FactorioEntity>,
    },
    Insert {
        pos: Position,
        entity: String,
        slot: InventorySlot,
        item: ItemId,
        count: u32,
    },
    Remove {
        pos: Position,
        entity: String,
        slot: InventorySlot,
        item: ItemId,
        count: u32,
    },
    Research {
        tech: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub kind: ActionKind,
    pub pre: Vec<Condition>,
    pub eff: Vec<Effect>,
    /// Nominal estimate. The observed duration lives in the execution log.
    pub duration: Ticks,
    /// An escape hatch for hand-tuned work. Normally `None`.
    pub pinned: Option<BotId>,
    pub label: String,
}

impl Action {
    /// Where the acting bot must stand, taken from its `AtPosition`
    /// precondition. The scheduler emits a walk to satisfy it.
    pub fn required_position(&self) -> Option<(Position, f64)> {
        self.pre.iter().find_map(|c| c.required_position())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::{FactorioEntity, Position};
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)])
    }

    #[test]
    fn role_resolves_to_the_binding_and_bound_ignores_it() {
        assert_eq!(Actor::Role.resolve(BotId(2)), BotId(2));
        assert_eq!(Actor::Bound(BotId(1)).resolve(BotId(2)), BotId(1));
    }

    #[test]
    fn has_item_checks_the_bound_actor_not_the_binding() {
        let mut s = state();
        s.gain(BotId(1), "coal", 5);
        let cond = Condition::HasItem {
            who: Actor::Bound(BotId(1)),
            item: "coal".into(),
            count: 5,
        };
        // Holds for bot 1's inventory even when bot 2 is executing.
        assert!(cond.holds(&s, BotId(2)));
    }

    #[test]
    fn has_item_with_role_follows_the_binding() {
        let mut s = state();
        s.gain(BotId(1), "coal", 5);
        let cond = Condition::HasItem {
            who: Actor::Role,
            item: "coal".into(),
            count: 5,
        };
        assert!(cond.holds(&s, BotId(1)));
        assert!(!cond.holds(&s, BotId(2)));
    }

    #[test]
    fn at_position_respects_the_radius() {
        let mut s = state();
        s.set_position(BotId(1), Position::new(0., 0.));
        let near = Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(3., 4.),
            radius: 5.0,
        };
        let far = Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(3., 4.),
            radius: 4.9,
        };
        assert!(near.holds(&s, BotId(1)));
        assert!(!far.holds(&s, BotId(1)));
    }

    #[test]
    fn gain_and_lose_effects_move_items() {
        let mut s = state();
        Effect::GainItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 4,
        }
        .apply(&mut s, BotId(1))
        .unwrap();
        assert_eq!(s.inventory_count(BotId(1), "iron-plate"), 4);

        Effect::LoseItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 3,
        }
        .apply(&mut s, BotId(1))
        .unwrap();
        assert_eq!(s.inventory_count(BotId(1), "iron-plate"), 1);
    }

    #[test]
    fn losing_items_the_bot_lacks_is_an_error() {
        let mut s = state();
        let result = Effect::LoseItem {
            who: Actor::Role,
            item: "iron-plate".into(),
            count: 1,
        }
        .apply(&mut s, BotId(1));
        assert!(result.is_err());
    }

    #[test]
    fn entity_at_matches_by_name_and_tile() {
        let mut s = state();
        let pos = Position::new(3., 3.);
        let cond = Condition::EntityAt {
            pos: pos.clone(),
            name: "stone-furnace".into(),
        };
        assert!(!cond.holds(&s, BotId(1)), "nothing stands there yet");

        s.create_entity(FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: pos.clone(),
            ..Default::default()
        });
        assert!(cond.holds(&s, BotId(1)));

        // The right tile, the wrong entity.
        let wrong_name = Condition::EntityAt {
            pos,
            name: "wooden-chest".into(),
        };
        assert!(!wrong_name.holds(&s, BotId(1)));
        // The right entity, the wrong tile.
        let wrong_tile = Condition::EntityAt {
            pos: Position::new(4., 3.),
            name: "stone-furnace".into(),
        };
        assert!(!wrong_tile.holds(&s, BotId(1)));
    }

    #[test]
    fn removing_an_entity_frees_its_tile() {
        let mut s = state();
        let pos = Position::new(3., 3.);
        Effect::CreateEntity(Box::new(FactorioEntity {
            name: "stone-furnace".into(),
            position: pos.clone(),
            ..Default::default()
        }))
        .apply(&mut s, BotId(1))
        .unwrap();
        assert!(!Condition::PositionFree { pos: pos.clone() }.holds(&s, BotId(1)));

        Effect::RemoveEntity { pos: pos.clone() }
            .apply(&mut s, BotId(1))
            .unwrap();
        assert!(Condition::PositionFree { pos }.holds(&s, BotId(1)));
    }

    #[test]
    fn researching_a_technology_satisfies_the_condition() {
        let mut s = state();
        let cond = Condition::Researched("automation".into());
        assert!(!cond.holds(&s, BotId(1)));

        Effect::Researched("automation".into())
            .apply(&mut s, BotId(1))
            .unwrap();
        assert!(cond.holds(&s, BotId(1)));
        // Research is global, not per bot, so the binding is irrelevant.
        assert!(cond.holds(&s, BotId(2)));
        assert!(!Condition::Researched("logistics".into()).holds(&s, BotId(1)));
    }

    #[test]
    fn an_action_survives_a_json_round_trip() {
        // Every payload an action can carry, including the boxed core entity.
        use factorio_bot_core::serde_json;
        let action = Action {
            id: crate::ids::ActionId(7),
            kind: ActionKind::Place {
                entity: Box::new(FactorioEntity {
                    name: "stone-furnace".into(),
                    entity_type: "furnace".into(),
                    position: Position::new(3., 4.),
                    ..Default::default()
                }),
            },
            pre: vec![
                Condition::HasItem {
                    who: Actor::Role,
                    item: "stone-furnace".into(),
                    count: 1,
                },
                Condition::AtPosition {
                    who: Actor::Bound(BotId(1)),
                    pos: Position::new(3., 4.),
                    radius: 10.0,
                },
                Condition::PositionFree {
                    pos: Position::new(3., 4.),
                },
                Condition::Researched("automation".into()),
                Condition::ResourceAvailable {
                    pos: Position::new(3., 4.),
                    item: "iron-ore".into(),
                    count: 2,
                },
                Condition::EntityAt {
                    pos: Position::new(3., 4.),
                    name: "stone-furnace".into(),
                },
            ],
            eff: vec![
                Effect::LoseItem {
                    who: Actor::Role,
                    item: "stone-furnace".into(),
                    count: 1,
                },
                Effect::GainItem {
                    who: Actor::Role,
                    item: "iron-plate".into(),
                    count: 1,
                },
                Effect::RemoveEntity {
                    pos: Position::new(3., 4.),
                },
                Effect::ConsumeResource {
                    pos: Position::new(3., 4.),
                    item: "iron-ore".into(),
                    count: 1,
                },
                Effect::Researched("automation".into()),
            ],
            duration: 30,
            pinned: Some(BotId(1)),
            label: "place stone-furnace".into(),
        };
        let json = serde_json::to_string(&action).expect("serialises");
        let back: Action = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, action);
    }

    #[test]
    fn conditions_render_for_error_messages() {
        let cond = Condition::HasItem {
            who: Actor::Role,
            item: "coal".into(),
            count: 2,
        };
        assert_eq!(cond.to_string(), "has 2 coal");
    }

    #[test]
    fn an_insert_names_the_entity_and_slot_it_targets() {
        let a = ActionKind::Insert {
            pos: Position::new(3.0, 4.0),
            entity: "stone-furnace".to_string(),
            slot: InventorySlot::FurnaceSource,
            item: "iron-ore".into(),
            count: 8,
        };
        match a {
            ActionKind::Insert { entity, slot, .. } => {
                assert_eq!(entity, "stone-furnace");
                assert_eq!(slot, InventorySlot::FurnaceSource);
            }
            _ => panic!("expected Insert"),
        }
    }

    #[test]
    fn inventory_slots_order_deterministically() {
        let mut v = vec![
            InventorySlot::FurnaceResult,
            InventorySlot::Chest,
            InventorySlot::FurnaceSource,
        ];
        v.sort();
        assert_eq!(
            v,
            vec![
                InventorySlot::Chest,
                InventorySlot::FurnaceSource,
                InventorySlot::FurnaceResult
            ]
        );
    }
}
