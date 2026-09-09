/// Intent memory: what the last plan intended for the entities it placed.
///
/// Carried across replan boundaries as a recoverable claim rather than as
/// ground truth. The memory is **advisory**: every claim is verified against
/// the world before being acted on (Phase 2+).
///
/// See `docs/superpowers/specs/2026-09-09-replan-memory-design.md`.

use crate::action::{ActionKind, Effect};
use crate::ids::{ActionId, ChainId};
use crate::network::ActionNetwork;
use crate::schedule::Schedule;
use crate::state::PlanState;
use factorio_bot_core::types::{Direction, Pos, Position};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What a single entity was placed FOR.

/// Serde helpers for serializing `BTreeMap<Pos, T>` with string keys.
///
/// JSON requires object keys to be strings, but `Pos` serializes as a tuple
/// `[x, y]` by default. These helpers flatten `Pos` to the string `"x,y"`.
mod pos_key {
    use factorio_bot_core::types::Pos;
    use serde::de::{MapAccess, Visitor};
    use serde::ser::SerializeMap;
    use serde::{Deserializer, Serializer};
    use std::collections::BTreeMap;
    use std::fmt;
    use std::marker::PhantomData;

    pub fn serialize<T, S>(map: &BTreeMap<Pos, T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: serde::Serialize,
        S: Serializer,
    {
        let mut map_ser = serializer.serialize_map(Some(map.len()))?;
        for (key, value) in map {
            let key_str = format!("{},{}", key.0, key.1);
            map_ser.serialize_entry(&key_str, value)?;
        }
        map_ser.end()
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<BTreeMap<Pos, T>, D::Error>
    where
        T: serde::Deserialize<'de>,
        D: Deserializer<'de>,
    {
        struct PosMapVisitor<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for PosMapVisitor<T>
        where
            T: serde::Deserialize<'de>,
        {
            type Value = BTreeMap<Pos, T>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map with string keys of the form 'x,y'")
            }

            fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut map = BTreeMap::new();
                while let Some((key_str, value)) = access.next_entry::<String, T>()? {
                    let parts: Vec<&str> = key_str.split(',').collect();
                    if parts.len() != 2 {
                        return Err(serde::de::Error::custom(format!(
                            "expected Pos key as 'x,y', got '{}'",
                            key_str
                        )));
                    }
                    let x: i32 = parts[0]
                        .parse()
                        .map_err(|e| serde::de::Error::custom(format!("invalid x: {}", e)))?;
                    let y: i32 = parts[1]
                        .parse()
                        .map_err(|e| serde::de::Error::custom(format!("invalid y: {}", e)))?;
                    map.insert(Pos(x, y), value);
                }
                Ok(map)
            }
        }

        deserializer.deserialize_map(PosMapVisitor(PhantomData))
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityIntent {
    /// The goal kind that caused this placement.
    pub goal: IntentGoalKind,
    /// The goal's parameters, enough to map it to the caller's own goal list.
    pub goal_params: String,
    /// Which plan round placed it.
    pub plan_round: u32,
    /// The action id that placed it in that plan, for cross-referencing the
    /// run record.
    pub action_id: Option<ActionId>,
    /// The anchor position of the cell this entity belongs to, if any.
    pub cell_anchor: Option<Position>,
    /// The goal index in this plan's top-level goal list, for re-mapping on
    /// replan.
    pub goal_index: Option<usize>,
}

/// A compact enum of the goal families that place entities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntentGoalKind {
    /// `Goal::Built` — a blueprint block.
    Built(String),
    /// `Goal::Producing` / `Goal::Sustain` — a cell.
    Cell {
        item: String,
        per_minute: u32,
    },
    /// `Goal::Produced` — a machine and its infrastructure.
    Produce {
        item: String,
        count: u32,
    },
    /// `Goal::Have` — infrastructure for hand-smelting or gathering.
    Have {
        item: String,
    },
    /// `Goal::Extracted` / `Goal::Gathered` — pumpjack and tank.
    Extract,
    /// Power plant.
    Power,
    /// `Goal::Charted` — a radar or scouting walk.
    Chart,
}

/// A mouth of a cell: one belted input, defined by the chest it draws from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouthIntent {
    /// The item this mouth feeds.
    pub item: String,
    /// The chest position this mouth draws from.
    pub chest: Position,
    /// The inserter that feeds the chest, if placed.
    pub inserter_placed: bool,
}

/// A cell the last plan assembled, by anchor and facing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellIntent {
    /// The anchor position of the intermediate machine.
    pub anchor: Position,
    /// Facing.
    pub facing: Direction,
    /// The spec's item.
    pub item: String,
    /// Which of the cell's parts the last plan placed. Empty means it
    /// was fully placed; a partial list means the batch was cut.
    pub placed: Vec<String>,
    /// The supply chain's sink — the chest or lab the belts run to.
    pub supply_source: Option<Position>,
    /// The belted input positions, for chain recovery.
    pub mouths: Vec<MouthIntent>,
    /// The plan round that assembled this cell.
    pub plan_round: u32,
}

/// A `Site::Built` block the last plan placed, by anchor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockIntent {
    /// The resolved anchor.
    pub anchor: Position,
    /// The site variant — preserved so the next round knows whether to
    /// search again or honour the anchor.
    pub site: crate::goal::Site,
    /// The blueprint string, for re-verification.
    pub blueprint: String,
    /// How many entities the plan actually placed of the blueprint's total.
    pub placed: u32,
    /// How many entities the blueprint has.
    pub total: u32,
}

/// A belt run the last plan built, from source to sink.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainIntent {
    /// The entity this chain starts from.
    pub source: Position,
    /// What it carries.
    pub item: String,
    /// The entity it delivers to.
    pub sink: Position,
    /// Whether both arms were placed.
    pub arms_placed: bool,
    /// How many belts were placed.
    pub belts_placed: u32,
    /// How many belts the run needs.
    pub belts_needed: u32,
    /// Positions of every belt tile, for recognition.
    pub belt_positions: Vec<Pos>,
}

/// What the LAST plan intended for the entities it placed, carried across
/// replan boundaries as a recoverable claim rather than as ground truth.
///
/// Not serialized as part of a `PlanState`; read from the run record or
/// passed explicitly to `plan_best` / `plan_round`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplanMemory {
    /// The plan round this memory was captured at.
    pub plan_round: u32,
    /// Every entity placed by the last plan, keyed by position, annotated
    /// with the goal or sub-goal that placed it.
    #[serde(with = "pos_key")]
    pub entities: BTreeMap<Pos, EntityIntent>,
    /// Every cell assembled by the last plan, by anchor position.
    pub cells: Vec<CellIntent>,
    /// Every belt chain built by the last plan, by its terminal positions.
    pub chains: Vec<ChainIntent>,
    /// Every block placed by the last plan, by Site.
    pub blocks: Vec<BlockIntent>,
    /// The overrides `complete_cell` made: which entities were accepted as
    /// standing even though the layout would not have claimed them.
    pub recovery_overrides: BTreeSet<Pos>,
}

/// Walk the network and state to capture intent records for every action that
/// placed an entity.
///
/// This is the Phase 1 capture function. It reads through every action in the
/// network, identifies `Place` and `StampGhosts` actions, and records the
/// placed entity positions along with chain associations and structural
/// groupings (cells, blocks, belt chains).
///
/// The schedule provides bot assignments which are used to group actions
/// into per-bot chains for belt-run identification.
pub fn capture_intent(
    _state: &PlanState,
    net: &ActionNetwork,
    _schedule: &Schedule,
    plan_round: u32,
) -> ReplanMemory {
    let mut entities: BTreeMap<Pos, EntityIntent> = BTreeMap::new();
    let mut chains: Vec<ChainIntent> = Vec::new();
    let mut belt_positions_by_chain: BTreeMap<ChainId, Vec<Pos>> = BTreeMap::new();
    let mut chain_items: BTreeMap<ChainId, String> = BTreeMap::new();
    let mut chain_sources: BTreeMap<ChainId, Position> = BTreeMap::new();
    let mut chain_sinks: BTreeMap<ChainId, Position> = BTreeMap::new();
    let mut chain_arm_count: BTreeMap<ChainId, u32> = BTreeMap::new();
    let mut chain_belt_count: BTreeMap<ChainId, u32> = BTreeMap::new();
    let chain_arm_needed: BTreeMap<ChainId, u32> = BTreeMap::new();
    let _chain_belt_needed: BTreeMap<ChainId, u32> = BTreeMap::new();

    for action in net.actions() {
        let pos = match &action.kind {
            ActionKind::Place { entity } => Some(entity.position.clone()),
            ActionKind::StampGhosts { anchor, .. } => Some(anchor.clone()),
            _ => None,
        };

        // Record placed entities
        if let Some(position) = pos {
            let intent_goal = classify_goal(&action.label);
            let chain_id = net.chain_of(action.id);
            let cell_anchor = chain_id.and_then(|_cid| {
                // Derive cell anchor from chain: the earliest placement in
                // this chain that looks like a cell's anchor tile.
                // For now we leave it as None — Phase 2 consumption code
                // will populate this from the CellIntent list.
                None
            });

            entities.insert(
                Pos::from(&position),
                EntityIntent {
                    goal: intent_goal,
                    goal_params: action.label.clone(),
                    plan_round,
                    action_id: Some(action.id),
                    cell_anchor,
                    goal_index: None,
                },
            );
        }

        // Track belt chains
        if matches!(&action.kind, ActionKind::Place { entity } if entity.name == "transport-belt") {
            if let Some(chain_id) = net.chain_of(action.id) {
                belt_positions_by_chain
                    .entry(chain_id)
                    .or_default()
                    .push(Pos::from(&action.kind.target_position().unwrap_or_default()));
                *chain_belt_count.entry(chain_id).or_insert(0) += 1;
                if let Some(have_eff) = action.eff.iter().find_map(|e| match e {
                    Effect::GainItem { item, .. } => Some(item.clone()),
                    _ => None,
                }) {
                    chain_items.entry(chain_id).or_insert(have_eff);
                }
            }
        }

        // Track arms for each chain
        if matches!(&action.kind, ActionKind::Place { entity } if entity.name.contains("inserter")) {
            if let Some(chain_id) = net.chain_of(action.id) {
                *chain_arm_count.entry(chain_id).or_insert(0) += 1;
            }
        }

        // Track source and sink positions for chains
        if let Some(chain_id) = net.chain_of(action.id) {
            match &action.kind {
                ActionKind::Place { entity } => {
                    if !chain_sources.contains_key(&chain_id) {
                        chain_sources.insert(chain_id, entity.position.clone());
                    }
                    chain_sinks.insert(chain_id, entity.position.clone());
                }
                _ => {}
            }
        }
    }

    // Build chain intents
    for (chain_id, positions) in &belt_positions_by_chain {
        let belts_placed = chain_belt_count.get(chain_id).copied().unwrap_or(0);
        // belts_needed is an estimate; precise value requires blueprint
        // inspection or cell definition data not yet available in Phase 1.
        let belts_needed = belts_placed;
        let arms_placed = chain_arm_count.get(chain_id).copied().unwrap_or(0);
        // A belt chain typically needs two arms (one at each end).
        let arms_needed = chain_arm_needed.get(chain_id).copied().unwrap_or(2);

        chains.push(ChainIntent {
            source: chain_sources
                .get(chain_id)
                .cloned()
                .unwrap_or(Position::new(0., 0.)),
            item: chain_items
                .get(chain_id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            sink: chain_sinks
                .get(chain_id)
                .cloned()
                .unwrap_or(Position::new(0., 0.)),
            arms_placed: arms_placed >= arms_needed,
            belts_placed,
            belts_needed,
            belt_positions: positions.clone(),
        });
    }

    // For Phase 1, cells and blocks are empty — they need goal-level context
    // that Phase 2+ will supply.
    ReplanMemory {
        plan_round,
        entities,
        cells: Vec::new(),
        chains,
        blocks: Vec::new(),
        recovery_overrides: BTreeSet::new(),
    }
}

/// Classify an action label into an IntentGoalKind.
///
/// This is a best-effort inference from the action label string. Future phases
/// will thread the actual goal through so this classification is exact.
fn classify_goal(label: &str) -> IntentGoalKind {
    if label.contains("block") || label.contains("blueprint") || label.contains("furnace block") || label.contains("mining array") {
        IntentGoalKind::Built(label.to_string())
    } else if label.contains("sustain") {
        // e.g. "sustain:iron-plate:30:36000"
        let parts: Vec<&str> = label.split(':').collect();
        let item = parts.get(1).unwrap_or(&"unknown").to_string();
        let per_minute = parts
            .get(2)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        IntentGoalKind::Cell { item, per_minute }
    } else if label.contains("produce") || label.contains("craft") {
        // e.g. "produce 50 iron-plate" or "craft 1 burner-mining-drill"
        let parts: Vec<&str> = label.split_whitespace().collect();
        if let (Some(count_str), Some(item)) = (parts.get(1), parts.get(2)) {
            let count = count_str.parse::<u32>().unwrap_or(0);
            IntentGoalKind::Produce {
                item: item.to_string(),
                count,
            }
        } else {
            IntentGoalKind::Produce {
                item: label.to_string(),
                count: 0,
            }
        }
    } else if label.contains("have ") || label.contains("mine ") {
        let parts: Vec<&str> = label.split_whitespace().collect();
        if let Some(item) = parts.last() {
            IntentGoalKind::Have {
                item: item.to_string(),
            }
        } else {
            IntentGoalKind::Have {
                item: label.to_string(),
            }
        }
    } else if label.contains("power") || label.contains("boiler") || label.contains("steam-engine") {
        IntentGoalKind::Power
    } else if label.contains("chart") || label.contains("survey") || label.contains("radar") {
        IntentGoalKind::Chart
    } else if label.contains("extract") || label.contains("pumpjack") {
        IntentGoalKind::Extract
    } else {
        // Fallback: try to detect from the label shape
        IntentGoalKind::Have {
            item: label.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ActionIdGen;
    use crate::action::{Action, ActionKind, Effect, Actor};
    use crate::ids::BotId;
    use crate::network::ActionNetwork;
    use crate::schedule::Schedule;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::FactorioEntity;
    use std::sync::Arc;

    fn test_state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    #[test]
    fn capture_empty_plan_produces_empty_memory() {
        let state = test_state();
        let net = ActionNetwork::new();
        let schedule = Schedule::default();
        let memory = capture_intent(&state, &net, &schedule, 0);
        assert!(memory.entities.is_empty());
        assert!(memory.cells.is_empty());
        assert!(memory.chains.is_empty());
        assert!(memory.blocks.is_empty());
        assert!(memory.recovery_overrides.is_empty());
        assert_eq!(memory.plan_round, 0);
    }

    #[test]
    fn capture_placement_records_entity() {
        let state = test_state();
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let pos = Position::new(10.5, 20.5);
        let action = Action {
            id: id_gen.next(),
            kind: ActionKind::Place {
                entity: Box::new(FactorioEntity {
                    name: "stone-furnace".into(),
                    entity_type: "furnace".into(),
                    position: pos.clone(),
                    ..Default::default()
                }),
            },
            pre: vec![],
            eff: vec![],
            duration: 30,
            pinned: None,
            label: "place stone-furnace".into(),
        };
        let aid = action.id;
        net.add(action);
        let schedule = Schedule::default();
        let memory = capture_intent(&state, &net, &schedule, 1);
        assert_eq!(memory.entities.len(), 1);
        let key = Pos::from(&pos);
        let intent = memory.entities.get(&key).expect("entity recorded at its tile");
        assert_eq!(intent.goal_params, "place stone-furnace");
        assert_eq!(intent.plan_round, 1);
        assert_eq!(intent.action_id, Some(aid));
    }

    #[test]
    fn capture_produces_the_right_entity_count() {
        let state = test_state();
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let positions = vec![
            Position::new(10.5, 20.5),
            Position::new(12.5, 20.5),
            Position::new(14.5, 20.5),
        ];
        for pos in &positions {
            net.add(Action {
                id: id_gen.next(),
                kind: ActionKind::Place {
                    entity: Box::new(FactorioEntity {
                        name: "stone-furnace".into(),
                        entity_type: "furnace".into(),
                        position: pos.clone(),
                        ..Default::default()
                    }),
                },
                pre: vec![],
                eff: vec![],
                duration: 30,
                pinned: None,
                label: "place stone-furnace".into(),
            });
        }
        let schedule = Schedule::default();
        let memory = capture_intent(&state, &net, &schedule, 1);
        assert_eq!(memory.entities.len(), 3);
    }

    #[test]
    fn capture_belt_chain_records_belt_positions() {
        let state = test_state();
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        let belt_positions: Vec<Position> = (0..5)
            .map(|i| Position::new(10.5 + i as f64, 30.5))
            .collect();
        for pos in &belt_positions {
            let aid = id_gen.next();
            net.add(Action {
                id: aid,
                kind: ActionKind::Place {
                    entity: Box::new(FactorioEntity {
                        name: "transport-belt".into(),
                        entity_type: "transport-belt".into(),
                        position: pos.clone(),
                        ..Default::default()
                    }),
                },
                pre: vec![],
                eff: vec![Effect::GainItem {
                    who: Actor::Role,
                    item: "iron-plate".into(),
                    count: 1,
                }],
                duration: 10,
                pinned: None,
                label: "place transport-belt".into(),
            });
            net.set_chain(aid, crate::ids::ChainId(1));
        }
        let schedule = Schedule::default();
        let memory = capture_intent(&state, &net, &schedule, 0);
        assert_eq!(memory.chains.len(), 1);
        assert_eq!(memory.chains[0].belts_placed, 5);
        assert_eq!(memory.chains[0].belt_positions.len(), 5);
        assert!(!memory.chains[0].arms_placed);
    }

    #[test]
    fn entity_intent_survives_a_json_round_trip() {
        let intent = EntityIntent {
            goal: IntentGoalKind::Cell {
                item: "iron-plate".into(),
                per_minute: 30,
            },
            goal_params: "sustain:iron-plate:30:36000".into(),
            plan_round: 2,
            action_id: Some(crate::ids::ActionId(42)),
            cell_anchor: Some(Position::new(27.5, -43.5)),
            goal_index: Some(0),
        };
        let json = factorio_bot_core::serde_json::to_string(&intent).expect("serialises");
        let back: EntityIntent =
            factorio_bot_core::serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back.goal, intent.goal);
        assert_eq!(back.goal_params, intent.goal_params);
        assert_eq!(back.plan_round, intent.plan_round);
        assert_eq!(back.action_id, intent.action_id);
    }

    #[test]
    fn replan_memory_holds_what_it_is_given() {
        let mut entities: BTreeMap<Pos, EntityIntent> = BTreeMap::new();
        entities.insert(
            Pos(10, 20),
            EntityIntent {
                goal: IntentGoalKind::Cell {
                    item: "iron-plate".into(),
                    per_minute: 30,
                },
                goal_params: "cell at [10.5, 20.5]".into(),
                plan_round: 1,
                action_id: None,
                cell_anchor: None,
                goal_index: None,
            },
        );
        let memory = ReplanMemory {
            plan_round: 1,
            entities,
            cells: vec![],
            chains: vec![],
            blocks: vec![],
            recovery_overrides: BTreeSet::new(),
        };
        assert_eq!(memory.plan_round, 1);
        assert_eq!(memory.entities.len(), 1);
        assert!(memory.entities.contains_key(&Pos(10, 20)));
        assert!(memory.cells.is_empty());
        assert!(memory.chains.is_empty());
        assert!(memory.blocks.is_empty());
        assert!(memory.recovery_overrides.is_empty());
    }

    /// Two placements at different tiles — they should produce two distinct
    /// entity records.
    #[test]
    fn capture_counts_each_entity_once() {
        let state = test_state();
        let mut id_gen = ActionIdGen::new();
        let mut net = ActionNetwork::new();
        net.add(Action {
            id: id_gen.next(),
            kind: ActionKind::Place {
                entity: Box::new(FactorioEntity {
                    name: "wooden-chest".into(),
                    entity_type: "container".into(),
                    position: Position::new(5.5, 5.5),
                    ..Default::default()
                }),
            },
            pre: vec![],
            eff: vec![],
            duration: 10,
            pinned: None,
            label: "place wooden-chest".into(),
        });
        net.add(Action {
            id: id_gen.next(),
            kind: ActionKind::Place {
                entity: Box::new(FactorioEntity {
                    name: "stone-furnace".into(),
                    entity_type: "furnace".into(),
                    position: Position::new(8.5, 8.5),
                    ..Default::default()
                }),
            },
            pre: vec![],
            eff: vec![],
            duration: 30,
            pinned: None,
            label: "place stone-furnace".into(),
        });
        let schedule = Schedule::default();
        let memory = capture_intent(&state, &net, &schedule, 0);
        assert_eq!(memory.entities.len(), 2);
        assert!(memory.entities.contains_key(&Pos(5, 5)));
        assert!(memory.entities.contains_key(&Pos(8, 8)));
    }
}
