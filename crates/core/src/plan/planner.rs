#[cfg_attr(test, mockall_double::double)]
use crate::factorio::rcon::FactorioRcon;
use crate::factorio::world::FactorioWorld;
use crate::types::{EntityName, PlayerChangedMainInventoryEvent};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct Planner {
    #[allow(dead_code)]
    pub rcon: Option<Arc<FactorioRcon>>,
    pub real_world: Arc<FactorioWorld>,
    pub plan_world: Arc<FactorioWorld>,
}

impl Planner {
    pub fn new(world: Arc<FactorioWorld>, rcon: Option<Arc<FactorioRcon>>) -> Planner {
        let plan_world = (*world).clone();

        Planner {
            rcon,
            real_world: world,
            plan_world: Arc::new(plan_world),
        }
    }

    pub fn reset(&mut self) {
        let plan_world = (*self.real_world).clone();
        self.plan_world = Arc::new(plan_world);
    }
    pub fn update_plan_world(&mut self) {
        self.plan_world = Arc::new((*self.real_world).clone());
    }

    pub fn world(&self) -> Arc<FactorioWorld> {
        self.plan_world.clone()
    }

    pub fn initiate_missing_players_with_default_inventory(&mut self, bot_count: u8) -> Vec<u8> {
        let mut player_ids: Vec<u8> = vec![];
        for player_id in 1u8..=bot_count {
            player_ids.push(player_id);
            // initialize missing players with default inventory
            if self.real_world.players.get(&player_id).is_none() {
                let mut main_inventory: BTreeMap<String, u32> = BTreeMap::new();
                main_inventory.insert(EntityName::Wood.to_string(), 1);
                main_inventory.insert(EntityName::StoneFurnace.to_string(), 1);
                main_inventory.insert(EntityName::BurnerMiningDrill.to_string(), 1);
                self.real_world
                    .player_changed_main_inventory(PlayerChangedMainInventoryEvent::from_btreemap(
                        player_id,
                        main_inventory,
                    ))
                    .expect("failed to set player inventory");
            }
        }
        player_ids
    }
}
