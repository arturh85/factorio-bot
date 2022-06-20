use crate::error::{to_rhai_error, Result};
use factorio_bot_core::graph::task_graph::PositionRadius;
use factorio_bot_core::plan::plan_builder::PlanBuilder;
use factorio_bot_core::types::{FactorioEntity, PlayerId, Position};
use rhai::Engine;
use std::sync::Arc;

#[derive(Clone)]
pub struct RhaiPlanBuilder {
    plan_builder: Arc<PlanBuilder>,
}

impl RhaiPlanBuilder {
    pub fn new(plan_builder: Arc<PlanBuilder>) -> Self {
        RhaiPlanBuilder { plan_builder }
    }

    pub fn register(engine: &mut Engine) {
        engine
            .register_type::<RhaiPlanBuilder>()
            .register_result_fn("add_place", Self::add_place)
            .register_result_fn("add_walk", Self::add_walk)
            .register_result_fn("mine", Self::mine)
            .register_fn("group_start", Self::group_start)
            .register_fn("group_end", Self::group_end);
    }

    pub fn mine(
        &mut self,
        player_id: PlayerId,
        position: Position,
        name: &str,
        count: i64,
    ) -> Result<()> {
        self.plan_builder
            .mine(player_id, position, name, count as u32)
            .map_err(to_rhai_error)
    }
    pub fn add_walk(&mut self, player_id: PlayerId, goal: PositionRadius) -> Result<()> {
        self.plan_builder
            .add_walk(player_id, goal)
            .map_err(to_rhai_error)
    }

    pub fn add_place(
        &mut self,
        player_id: PlayerId,
        entity: FactorioEntity,
    ) -> Result<FactorioEntity> {
        self.plan_builder
            .add_place(player_id, entity)
            .map_err(to_rhai_error)
    }

    pub fn group_start(&mut self, label: &str) {
        self.plan_builder.group_start(label);
    }

    pub fn group_end(&mut self) {
        self.plan_builder.group_end();
    }
}
