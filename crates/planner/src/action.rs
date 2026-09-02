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
        /// The annulus's inner bound: how close is *too* close.
        ///
        /// Zero for every non-placement use of this condition — mining,
        /// inserting, crafting all want a plain disc, satisfied by standing
        /// anywhere from directly on `pos` out to `radius`. A `Place` sets
        /// this to [`crate::state::PlanState::placement_clearance`] instead:
        /// standing on the tile a placement targets satisfies a disc's
        /// `radius` trivially (distance zero), which is exactly how the
        /// planner used to schedule a furnace on the bot's own feet and have
        /// the game refuse it with `player_blocks_placement`. A nonzero
        /// minimum makes that same standing point fail the condition, so
        /// [`crate::schedule::schedule`] emits a real `Walk` instead of
        /// skipping it.
        min_radius: f64,
    },
    EntityAt {
        pos: Position,
        name: ItemId,
    },
    /// One tile is clear.
    ///
    /// **No method emits this**, and none should: a placement wants
    /// [`Condition::AreaFree`], which knows how big the thing being placed is.
    /// It stays because "is this tile clear" is still a meaningful question of
    /// a `PlanState`, and `tests/scheduling.rs` asks it of hand-built networks.
    PositionFree {
        pos: Position,
    },
    /// Room for an entity of `entity` centred on `pos` — its whole collision
    /// box, not just the tile under its centre.
    ///
    /// The condition every `Place` wants. `PositionFree` asks about one tile,
    /// which is true of both tiles of a pair of stone furnaces sited one tile
    /// apart even though the game refuses the second: the furnace is 1.398
    /// tiles across. The size is looked up from the prototype at check time
    /// (`PlanState::is_area_free`), so nothing here has to know it.
    AreaFree {
        pos: Position,
        entity: ItemId,
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
            Condition::AtPosition {
                who,
                pos,
                radius,
                min_radius,
            } => match state.bot(who.resolve(binding)) {
                Some(bot) => {
                    let distance = calculate_distance(&bot.position, pos);
                    distance.total_cmp(min_radius).is_ge() && distance.total_cmp(radius).is_le()
                }
                None => false,
            },
            Condition::EntityAt { pos, name } => {
                matches!(state.entity_at(pos), Some(e) if &e.name == name)
            }
            Condition::PositionFree { pos } => state.is_position_free(pos),
            Condition::AreaFree { pos, entity } => state.is_area_free(entity, pos),
            Condition::Researched(tech) => state.is_researched(tech),
            Condition::ResourceAvailable { pos, item, count } => {
                state.resource_available(pos, item) >= *count
            }
        }
    }

    /// The position this condition requires the acting bot to stand near, if
    /// any, as `(pos, min_radius, radius)` — the annulus the scheduler must
    /// land the bot inside. `min_radius` is `0.` for every disc (every
    /// non-placement use today), which makes the annulus a disc again.
    pub fn required_position(&self) -> Option<(Position, f64, f64)> {
        match self {
            Condition::AtPosition {
                who: Actor::Role,
                pos,
                radius,
                min_radius,
            } => Some((pos.clone(), *min_radius, *radius)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Condition::HasItem { item, count, .. } => write!(f, "has {} {}", count, item),
            Condition::AtPosition {
                pos,
                radius,
                min_radius,
                ..
            } => {
                if min_radius.total_cmp(&0.0).is_gt() {
                    write!(f, "between {} and {} of {}", min_radius, radius, pos)
                } else {
                    write!(f, "within {} of {}", radius, pos)
                }
            }
            Condition::EntityAt { pos, name } => write!(f, "{} at {}", name, pos),
            Condition::PositionFree { pos } => write!(f, "{} is free", pos),
            Condition::AreaFree { pos, entity } => write!(f, "{} fits at {}", entity, pos),
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
            (Effect::RemoveEntity { pos }, Condition::PositionFree { pos: want })
            | (Effect::RemoveEntity { pos }, Condition::AreaFree { pos: want, .. }) => {
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
/// numbers move between game versions, so the executor resolves these names
/// against the running game and never hardcodes an integer.
///
/// The authority for the names is the game's own `defines.inventory`, as
/// published in `runtime-api.json` (`workspace/factorio-api-docs/`). It is not
/// `mods/BotBridge/control.lua`'s `inventory_type_name`: that function is dead
/// 1.1-era code with no callers, it names slots that 2.1 no longer has, and it
/// would raise "table index is nil" if it were ever called. Do not use it as a
/// reference.
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
    ///
    /// Factorio 2.0 unified furnaces and assemblers into one "crafter" family,
    /// so `FurnaceSource` and `AssemblerInput` both resolve to `crafter_input`,
    /// and both outputs to `crafter_output`. The variants stay distinct because
    /// this enum is the planner's vocabulary, not the game's: a method that
    /// loads a furnace and one that loads an assembler are different methods
    /// even where the game now agrees about the slot.
    ///
    /// The pre-2.0 names `furnace_source`, `furnace_result`,
    /// `assembling_machine_input` and `assembling_machine_output` no longer
    /// exist; asking for them yields no inventory at all.
    pub fn defines_key(self) -> &'static str {
        match self {
            InventorySlot::Chest => "chest",
            InventorySlot::FurnaceSource => "crafter_input",
            InventorySlot::FurnaceResult => "crafter_output",
            InventorySlot::Fuel => "fuel",
            InventorySlot::AssemblerInput => "crafter_input",
            InventorySlot::AssemblerOutput => "crafter_output",
            InventorySlot::LabInput => "lab_input",
        }
    }

    /// Every variant, for exhaustive checks against the game's defines table.
    pub const ALL: [InventorySlot; 7] = [
        InventorySlot::Chest,
        InventorySlot::FurnaceSource,
        InventorySlot::FurnaceResult,
        InventorySlot::Fuel,
        InventorySlot::AssemblerInput,
        InventorySlot::AssemblerOutput,
        InventorySlot::LabInput,
    ];
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
    /// precondition, as `(pos, min_radius, radius)`. The scheduler emits a
    /// walk to satisfy it.
    pub fn required_position(&self) -> Option<(Position, f64, f64)> {
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
            min_radius: 0.0,
        };
        let far = Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(3., 4.),
            radius: 4.9,
            min_radius: 0.0,
        };
        assert!(near.holds(&s, BotId(1)));
        assert!(!far.holds(&s, BotId(1)));
    }

    #[test]
    fn at_position_respects_the_annulus() {
        // The fix in miniature: the exact scenario milestone 4 hit. Standing
        // on the target satisfies a disc (distance zero is `<= radius`) but
        // must fail an annulus whose inner bound is positive, and standing
        // at a legitimate distance must still hold.
        let mut s = state();
        s.set_position(BotId(1), Position::new(3., 4.));
        let annulus = Condition::AtPosition {
            who: Actor::Role,
            pos: Position::new(3., 4.),
            radius: 5.0,
            min_radius: 1.0,
        };
        assert!(
            !annulus.holds(&s, BotId(1)),
            "standing exactly on the target must fail an annulus with a positive inner bound"
        );

        s.set_position(BotId(1), Position::new(4., 4.));
        assert!(
            annulus.holds(&s, BotId(1)),
            "one tile out clears the inner bound and stays within the outer one"
        );

        // The boundaries themselves: touching either edge holds, matching the
        // disc's own inclusive `<=` at its outer edge.
        s.set_position(BotId(1), Position::new(4., 4.0)); // distance 1.0 == min_radius
        assert!(annulus.holds(&s, BotId(1)), "the inner edge itself holds");
        s.set_position(BotId(1), Position::new(8., 4.0)); // distance 5.0 == radius
        assert!(annulus.holds(&s, BotId(1)), "the outer edge itself holds");
        s.set_position(BotId(1), Position::new(8.1, 4.0)); // distance 5.1 > radius
        assert!(
            !annulus.holds(&s, BotId(1)),
            "past the outer edge must still fail, same as a disc"
        );
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
                    min_radius: 1.5,
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

    /// Pins the exact strings sent to the game. These are Factorio 2.1 names,
    /// read from `defines.inventory` in `runtime-api.json`; the executor has
    /// the test that checks them against the game's own table.
    #[test]
    fn every_slot_maps_to_its_2_1_defines_name() {
        assert_eq!(InventorySlot::Chest.defines_key(), "chest");
        assert_eq!(InventorySlot::Fuel.defines_key(), "fuel");
        assert_eq!(InventorySlot::LabInput.defines_key(), "lab_input");
        // 2.0 unified furnaces and assemblers under `crafter_*`.
        assert_eq!(InventorySlot::FurnaceSource.defines_key(), "crafter_input");
        assert_eq!(InventorySlot::AssemblerInput.defines_key(), "crafter_input");
        assert_eq!(InventorySlot::FurnaceResult.defines_key(), "crafter_output");
        assert_eq!(
            InventorySlot::AssemblerOutput.defines_key(),
            "crafter_output"
        );
    }

    #[test]
    fn no_slot_still_uses_a_removed_pre_2_0_name() {
        // These four exist in `mods/BotBridge/control.lua`'s dead
        // `inventory_type_name` and nowhere in the 2.1 game.
        let gone = [
            "furnace_source",
            "furnace_result",
            "assembling_machine_input",
            "assembling_machine_output",
        ];
        for slot in InventorySlot::ALL {
            let key = slot.defines_key();
            assert!(!gone.contains(&key), "{slot:?} uses removed name {key}");
        }
    }

    #[test]
    fn all_lists_every_variant() {
        // Guards the exhaustiveness the executor's defines check relies on.
        let mut keys: Vec<&str> = InventorySlot::ALL.iter().map(|s| s.defines_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(InventorySlot::ALL.len(), 7);
        assert_eq!(
            keys,
            vec![
                "chest",
                "crafter_input",
                "crafter_output",
                "fuel",
                "lab_input"
            ]
        );
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
