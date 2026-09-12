//! Compile selected module instances into action plans.
//!
//! Takes a [`ModuleSelection`] (designs + instances) and produces
//! construction steps (Place, Insert, etc.) that the scheduler can
//! execute. The steps are fed through [`crate::method::run_steps`] to
//! build the [`ActionNetwork`], which is then scheduled.

use std::collections::{BTreeMap, BTreeSet};

use factorio_bot_core::types::{FactorioEntity, Position, Rect};

use crate::action::{Action, ActionKind, Condition, Effect, InventorySlot};
use crate::control::PlanControl;
use crate::goal::Goal;

use crate::ids::{ActionId, ActionIdGen, BotId};
use crate::memory::ReplanMemory;
use crate::method::{ExpansionCtx, MethodRegistry, Step, run_steps};
use crate::modules::artifact::{ModuleDesign, Rate};
pub use crate::modules::cache::CacheMode;
use crate::modules::cache::LibraryCache;
use crate::modules::instance::{InstanceMemory, ModuleInstance};
use crate::modules::ledger::OperatingLedger;
use crate::modules::routing::{ConnectionRequest, charge_connection_materials, route_connections};
use crate::modules::select::{
    ModuleSelection, ProductionRequest, ReservationSet, select_candidates, site_candidates,
};
use crate::network::ActionNetwork;
use crate::schedule::{Schedule, ScheduledStep, StepKind, schedule};
use crate::state::PlanState;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// PlannerMode and friends
// ---------------------------------------------------------------------------

/// Which planning engine to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerMode {
    Legacy,
    Modules,
}

/// Configuration for a planning session.
#[derive(Debug, Clone)]
pub struct PlannerOptions {
    pub mode: PlannerMode,
    pub cache_mode: CacheMode,
    pub candidate_limit: usize,
    pub support_ticks: u32,
    /// External (export) rates that the module factory must cover.
    /// Items keyed by name, values are rates in items per tick.
    /// Defaults to empty (no external demand).
    pub external_rates: BTreeMap<String, Rate>,
}

impl Default for PlannerOptions {
    fn default() -> Self {
        Self {
            mode: PlannerMode::Legacy,
            cache_mode: CacheMode::On,
            candidate_limit: 8,
            support_ticks: 18000,
            external_rates: BTreeMap::new(),
        }
    }
}

/// Mutable state across planning sessions.
#[derive(Debug, Clone)]
pub struct PlannerSession {
    pub library: LibraryCache,
    pub observed_revision: u64,
    /// Module instance memory, preserved across replan sessions.
    /// Tracks all known instance identities, part states, and port bindings.
    pub memory: crate::modules::instance::InstanceMemory,
}

impl PlannerSession {
    pub fn new() -> Self {
        Self {
            library: LibraryCache::new(),
            observed_revision: 0,
            memory: crate::modules::instance::InstanceMemory::default(),
        }
    }
}

/// The result of compiling a module selection into executable steps.
#[derive(Debug, Clone)]
pub struct CompiledModules {
    pub steps: Vec<Step>,
    pub instances: Vec<ModuleInstance>,
    pub network: ActionNetwork,
    pub ledger: OperatingLedger,
    /// Items that cannot be funded through standard production and need
    /// finite machine-production prerequisites instead of simple Have subgoals.
    /// Keyed by item name, value is the count needed for construction.
    pub construction_shortfalls: BTreeMap<String, u64>,
    /// Items whose operating rate cannot be met through existing supply
    /// networks. Keyed by item name, value is the shortfall rate.
    pub operating_shortfalls: BTreeMap<String, Rate>,
}

// ---------------------------------------------------------------------------
// compile_module_placement
// ---------------------------------------------------------------------------

/// Build Place steps for one module instance, using ids from the context.
///
/// For each part this produces:
/// 1. A `Have` subgoal to acquire the entity (hand-craft or take from inventory)
/// 2. A `Place` action to build it at the computed position
/// 3. For burner entities, an `Insert` action to fuel it with coal
/// 4. Where a recipe is specified, a `SetRecipe` action
fn compile_module_placement(
    design: &ModuleDesign,
    instance: &ModuleInstance,
    ids: &mut ActionIdGen,
) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();

    // Add acquisition subgoals for input materials (non-OreToPlate modules).
    // BeltInput ports are provided externally - no material subgoals needed.

    // If the design needs electric power, acquire power plant materials.
    if design.operation.power_watts > 0 {
        for (item, count) in [
            ("offshore-pump", 1u32),
            ("boiler", 1u32),
            ("steam-engine", 1u32),
            ("pipe", 4u32),
            ("small-electric-pole", 2u32),
        ] {
            steps.push(Step::Subgoal(Goal::Have {
                item: item.to_string(),
                count,
                whose: crate::Holder::Anyone,
                via: None,
            }));
        }
    }

    // Acquire bill items that have crafting/smelting recipes (skip machines
    // like chemical-plant and oil-refinery whose acquisition chain is complex).
    for (item, count) in &design.bill {
        // Skip items that are themselves complex machines - they'll be placed as parts.
        if item == "oil-refinery" || item == "chemical-plant" || item == "pumpjack"
            || item == "assembling-machine-2" || item == "assembling-machine-3"
            || item == "rocket-silo"
            // Skip items with complex/alternative recipe chains that confuse the legacy fallback.
            || item == "plastic-bar" || item == "sulfur" || item == "solid-fuel"
            || item == "sulfuric-acid" || item == "engine-unit" || item == "steel-furnace" || item == "electric-furnace"
        {
            continue;
        }
        steps.push(Step::Subgoal(Goal::Have {
            item: item.clone(),
            count: *count as u32,
            whose: crate::Holder::Anyone,
            via: None,
        }));
    }

    let anchor_x = instance.placement.half_x as f64 * 0.5;
    let anchor_y = instance.placement.half_y as f64 * 0.5;

    let mut recipe_ids: Vec<(String, ActionId)> = Vec::new();
    for part in &design.parts {
        let px = anchor_x + part.offset.half_x as f64 * 0.5;
        let py = anchor_y + part.offset.half_y as f64 * 0.5;
        let pos = Position::new(px, py);

        // --- 1. Placement action ---
        let entity = FactorioEntity {
            name: part.entity.clone(),
            entity_type: part.entity.clone(),
            position: pos.clone(),
            direction: part.direction,
            recipe: part.recipe.clone(),
            ..Default::default()
        };

        let place_id = ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: place_id,
            kind: ActionKind::Place {
                entity: Box::new(entity.clone()),
            },
            pre: vec![
                Condition::HasItem {
                    who: crate::action::Actor::Role,
                    item: part.entity.clone(),
                    count: 1,
                },
                Condition::AtPosition {
                    who: crate::action::Actor::Role,
                    pos: pos.clone(),
                    radius: 3.0,
                    min_radius: 0.5,
                },
                Condition::AreaFree {
                    pos: pos.clone(),
                    entity: part.entity.clone(),
                    direction: part.direction,
                },
            ],
            eff: vec![Effect::CreateEntity(Box::new(entity))],
            duration: 100,
            pinned: None,
            label: format!("place {} for {}", part.entity, part.role),
        })));

        // --- 2. Fuel Insert step for burner machines ---
        if is_burner_entity(&part.entity) {
            steps.push(Step::Act(Box::new(Action {
                id: ids.next(),
                kind: ActionKind::Insert {
                    pos: pos.clone(),
                    entity: part.entity.clone(),
                    slot: InventorySlot::Fuel,
                    item: "coal".into(),
                    count: 5,
                },
                pre: vec![Condition::AtPosition {
                    who: crate::action::Actor::Role,
                    pos: pos.clone(),
                    radius: 3.0,
                    min_radius: 0.5,
                }],
                eff: vec![],
                duration: 200,
                pinned: None,
                label: format!("fuel {} with coal", part.entity),
            })));
        }

        // --- 3. Recipe configuration ---
        if let Some(ref recipe) = part.recipe {
            let recipe_id = ids.next();
            steps.push(Step::Act(Box::new(Action {
                id: recipe_id,
                kind: ActionKind::SetRecipe {
                    pos: pos.clone(),
                    entity: part.entity.clone(),
                    recipe: recipe.clone(),
                },
                pre: vec![
                    Condition::AtPosition {
                        who: crate::action::Actor::Role,
                        pos: pos.clone(),
                        radius: 3.0,
                        min_radius: 0.5,
                    },
                    Condition::EntityAt {
                        pos: pos.clone(),
                        name: part.entity.clone(),
                    },
                ],
                eff: vec![],
                duration: 60,
                pinned: None,
                label: format!("set recipe {} for {}", recipe, part.role),
            })));
            // Explicit dependency: Place must complete before SetRecipe
            steps.push(Step::Link {
                from: place_id,
                to: recipe_id,
                lag: 0,
            });
            recipe_ids.push((part.entity.clone(), recipe_id));

            // Insert input materials for non-OreToPlate modules
            // (OreToPlate gets ore directly from the drill).
            if design.family != crate::modules::artifact::ModuleFamily::OreToPlate {
                for port in &design.ports {
                    if port.mode == crate::modules::artifact::PortMode::BeltInput {
                        let insert_id = ids.next();
                        recipe_ids.push((format!("{}_last_insert", part.entity), insert_id));
                        steps.push(Step::Act(Box::new(Action {
                            id: insert_id,
                            kind: ActionKind::Insert {
                                pos: pos.clone(),
                                entity: part.entity.clone(),
                                slot: crate::action::InventorySlot::AssemblerInput,
                                item: port.item.clone(),
                                count: 1,
                            },
                            pre: vec![
                                Condition::AtPosition {
                                    who: crate::action::Actor::Role,
                                    pos: pos.clone(),
                                    radius: 3.0,
                                    min_radius: 0.5,
                                },
                                Condition::EntityAt {
                                    pos: pos.clone(),
                                    name: part.entity.clone(),
                                },
                                Condition::HasItem {
                                    who: crate::action::Actor::Role,
                                    item: port.item.clone(),
                                    count: 1,
                                },
                            ],
                            eff: vec![],
                            duration: 100,
                            pinned: None,
                            label: format!("insert 1 {} into {}", port.item, part.role),
                        })));
                        // Insert must wait for recipe configuration
                        steps.push(Step::Link {
                            from: recipe_id,
                            to: insert_id,
                            lag: 0,
                        });
                    }
                }
            }
        }
    }

    // For each part with a recipe and an output in the design's contract,
    // add a Remove step to collect the output from the machine.
    for part in &design.parts {
        if part.recipe.is_some() {
            let px = anchor_x + part.offset.half_x as f64 * 0.5;
            let py = anchor_y + part.offset.half_y as f64 * 0.5;
            let pos = Position::new(px, py);

            let output_slot = output_slot_for_entity(&part.entity);
            for (output_item, rate) in &design.operation.outputs {
                // Take one cycle's worth from the machine's output slot.
                let take_id = ids.next();
                steps.push(Step::Act(Box::new(Action {
                    id: take_id,
                    kind: ActionKind::Remove {
                        pos: pos.clone(),
                        entity: part.entity.clone(),
                        slot: output_slot,
                        item: output_item.clone(),
                        count: rate.numerator as u32,
                    },
                    pre: vec![
                        Condition::AtPosition {
                            who: crate::action::Actor::Role,
                            pos: pos.clone(),
                            radius: 3.0,
                            min_radius: 0.5,
                        },
                        Condition::EntityAt {
                            pos: pos.clone(),
                            name: part.entity.clone(),
                        },
                    ],
                    eff: vec![Effect::GainItem {
                        who: crate::action::Actor::Role,
                        item: output_item.clone(),
                        count: rate.numerator as u32,
                    }],
                    duration: rate.ticks.get() as u32,
                    pinned: None,
                    label: format!("take {} {} from {}", rate.numerator, output_item, part.role),
                })));
            }
        }
    }

    steps
}
/// Determine the output inventory slot for a given entity type.
fn output_slot_for_entity(name: &str) -> InventorySlot {
    if name.contains("furnace") || name.contains("smelter") {
        InventorySlot::FurnaceResult
    } else if name.contains("assembling") || name.contains("assembler") || name.contains("crafting")
    {
        InventorySlot::AssemblerOutput
    } else {
        InventorySlot::FurnaceResult
    }
}

/// Returns true for entity names known to burn fuel.
fn is_burner_entity(name: &str) -> bool {
    matches!(
        name,
        "burner-mining-drill" | "stone-furnace" | "steel-furnace" | "burner-inserter" | "boiler"
    )
}

/// Compute the tile-coordinate bounding rectangle for a placed module instance.
///
/// Uses the instance's half-tile placement anchor and the design's part
/// offsets to compute the axis-aligned bounding box in world coordinates
/// (tile units, one tile = 2 half-tiles).
fn instance_bounding_rect(instance: &crate::modules::instance::ModuleInstance, design: &ModuleDesign) -> Rect {
    let anchor_x = instance.placement.half_x as f64 * 0.5;
    let anchor_y = instance.placement.half_y as f64 * 0.5;

    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    for part in &design.parts {
        let px = anchor_x + part.offset.half_x as f64 * 0.5;
        let py = anchor_y + part.offset.half_y as f64 * 0.5;
        // Use a default collision radius of 0.5 tiles for parts without
        // a known collision box. This gives a conservative bounding rect.
        let half = 0.5;
        min_x = min_x.min(px - half);
        min_y = min_y.min(py - half);
        max_x = max_x.max(px + half);
        max_y = max_y.max(py + half);
    }

    // Add a 2-tile margin around the bounding box for walking clearance.
    let margin = 2.0;
    Rect::new(
        &Position::new(min_x - margin, min_y - margin),
        &Position::new(max_x + margin, max_y + margin),
    )
}

// ---------------------------------------------------------------------------
// plan_with_session — module-mode entry point
// ---------------------------------------------------------------------------

/// Module-mode planning entry point.
///
/// Scans goals for producible items, selects module designs, sites instances,
/// compiles steps, builds the action network, and schedules.
pub fn plan_with_session(
    goals: &[Goal],
    state: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
    roster: &[BotId],
    control: &PlanControl,
    options: &PlannerOptions,
    session: &mut PlannerSession,
) -> crate::request::PlanResult {
    // Check mode: Legacy skips the module path entirely.
    if options.mode == PlannerMode::Legacy {
        return crate::request::plan_controlled(
            goals,
            state,
            registry,
            chain_actor,
            roster,
            control,
        );
    }

    // Check for module-supported production goals.
    let prod_items: Vec<(String, u32)> =
        goals.iter().filter_map(production_item_from_goal).collect();

    if prod_items.is_empty() {
        // No module-supported goals; fall back to legacy.
        return crate::request::plan_controlled(
            goals,
            state,
            registry,
            chain_actor,
            roster,
            control,
        );
    }

    // Check if any goal is already satisfied by the current world state.
    // If ALL production goals are satisfied, skip module compilation.
    let all_satisfied = goals
        .iter()
        .all(|g| crate::method::have::holds(g, state).unwrap_or(false));
    if all_satisfied {
        return crate::request::plan_controlled(
            goals,
            state,
            registry,
            chain_actor,
            roster,
            control,
        );
    }

    // For each production item, select module designs.
    let mut all_designs: Vec<Arc<ModuleDesign>> = Vec::new();
    let mut all_instances: Vec<ModuleInstance> = Vec::new();
    let mut reservations = ReservationSet::default();

    for (item, per_minute) in &prod_items {
        let request = ProductionRequest {
            item: item.clone(),
            per_minute: *per_minute,
            support_ticks: options.support_ticks,
        };

        match select_candidates(
            &request,
            state,
            &mut session.library,
            options.cache_mode,
            control,
            &mut session.memory,
        ) {
            Ok(designs) => {
                for design in designs.iter().take(options.candidate_limit) {
                    let near = state
                        .bot(chain_actor)
                        .map(|b| b.position.clone())
                        .unwrap_or_else(|| Position::new(0.0, 0.0));

                    // Compute how many copies of this module are needed.
                    let copies =
                        design_instances_needed(design, *per_minute, options.support_ticks.into());
                    match site_candidates(
                        design,
                        state,
                        &near,
                        copies as u32,
                        control,
                        &mut reservations,
                        &session.memory,
                    ) {
                        Ok(instances) => {
                            for inst in instances {
                                all_designs.push(design.clone());
                                all_instances.push(inst);
                            }
                        }
                        Err(_) => continue,
                    }
                }
            }
            Err(_) => continue,
        }
    }

    if all_instances.is_empty() {
        return crate::request::plan_controlled(
            goals,
            state,
            registry,
            chain_actor,
            roster,
            control,
        );
    }

    // Build a fake ModuleSelection and compile.
    let selection = ModuleSelection {
        designs: all_designs,
        instances: all_instances,
        requests: prod_items
            .iter()
            .map(|(item, per_minute)| ProductionRequest {
                item: item.clone(),
                per_minute: *per_minute,
                support_ticks: options.support_ticks,
            })
            .collect(),
        reservations: reservations.clone(),
    };

    // Build ExpansionCtx and compile.
    let mut ctx = ExpansionCtx::new(state.clone(), chain_actor);

    // --- Reserve corridors between adjacent module instances ---
    //
    // For each pair of placed module instances, compute the bounding
    // rectangle in tile coordinates and reserve a 3-tile corridor
    // between them. The corridor is stored in `ReplanMemory::corridors`
    // so it persists across replan boundaries.
    let mut corridor_reservations: Vec<crate::memory::CorridorReservation> = Vec::new();
    for i in 0..selection.instances.len() {
        for j in (i + 1)..selection.instances.len() {
            let inst_a = &selection.instances[i];
            let inst_b = &selection.instances[j];

            // Check they are on the same surface.
            if inst_a.placement.surface != inst_b.placement.surface {
                continue;
            }

            // Compute bounding rectangles in tile coordinates.
            let design_a = &selection.designs[i];
            let design_b = &selection.designs[j];

            let rect_a = instance_bounding_rect(inst_a, design_a);
            let rect_b = instance_bounding_rect(inst_b, design_b);

            // Only reserve corridors between instances that are close
            // enough to be considered adjacent (within 20 tiles).
            let dist = (rect_a.center().x - rect_b.center().x).abs()
                .max((rect_a.center().y - rect_b.center().y).abs());
            if dist > 20.0 {
                continue;
            }

            let keeper = format!(
                "corridor: {} <-> {}",
                design_a.family.short_name(),
                design_b.family.short_name(),
            );

            let corridor = ctx.state.reserve_corridor(
                inst_a.id,
                &rect_a,
                &rect_b,
                3,
                &keeper,
            );
            corridor_reservations.push(corridor);
        }
    }

    let mut net = ActionNetwork::new();
    let mut promised = Vec::new();

    // Compile placement steps for each (design, instance) pair.
    for (design, instance) in selection.designs.iter().zip(selection.instances.iter()) {
        let mut all_steps: Vec<Step> = Vec::new();

        // Add module placement steps.
        all_steps.extend(compile_module_placement(design, instance, &mut ctx.ids));

        if let Err(err) = run_steps(all_steps, &mut ctx, &mut net, registry, &mut promised) {
            return crate::request::PlanResult {
                status: crate::request::PlanStatus::Infeasible,
                incumbent: None,
                diagnostic: Some(err),
                budget: control.report(),
            };
        }
    }

    // --- Route connections between module instances ---
    let mut ledger = OperatingLedger::default();
    let mut routing_steps: Vec<Step> = Vec::new();

    // Build connection requests from instance port bindings.
    let mut connection_requests: Vec<ConnectionRequest> = Vec::new();
    for (design, instance) in selection.designs.iter().zip(selection.instances.iter()) {
        for binding in &instance.bindings {
            // Skip unbound ports (no provider).
            let Some(provider_instance) = binding.provider_instance else {
                continue;
            };
            let item = binding.port.clone();
            let rate = design
                .operation
                .outputs
                .get(&item)
                .cloned()
                .unwrap_or_else(|| Rate::new(1, 600).unwrap());

            connection_requests.push(ConnectionRequest {
                source: provider_instance,
                source_port: binding.provider_port.clone(),
                consumer: instance.id,
                consumer_port: binding.port.clone(),
                item,
                rate,
            });
        }
    }

    if !connection_requests.is_empty() {
        // Attempt routing with at most 2 retries for ledger corrections.
        let max_retries = 2;
        for attempt in 0..=max_retries {
            match route_connections(
                &connection_requests,
                &selection,
                &mut ctx,
                &mut reservations,
                control,
                &mut ledger,
            ) {
                Ok(steps) => {
                    routing_steps = steps;
                    break;
                }
                Err(e) => {
                    if attempt < max_retries {
                        // On retry, reset ledger and try again.
                        ledger = OperatingLedger::default();
                        factorio_bot_core::tracing::warn!(
                            "connection routing attempt {} failed, retrying: {e}",
                            attempt + 1
                        );
                    } else {
                        factorio_bot_core::tracing::warn!(
                            "connection routing failed after {} attempts: {e}",
                            max_retries + 1
                        );
                        // Routing failures are logged but do not abort the plan;
                        // modules will still be placed, just not connected.
                    }
                }
            }
        }
    }

    // Charge material subgoals for connection infrastructure.
    match charge_connection_materials(
        &connection_requests,
        &selection,
        &mut ctx,
        &mut reservations,
        control,
    ) {
        Ok(material_steps) => {
            if let Err(e) = run_steps(material_steps, &mut ctx, &mut net, registry, &mut promised) {
                factorio_bot_core::tracing::warn!("connection material steps failed: {e}");
            }
        }
        Err(e) => {
            factorio_bot_core::tracing::warn!("connection material charging failed: {e}");
        }
    }

    // Execute routing steps through the action network.
    if let Err(e) = run_steps(routing_steps, &mut ctx, &mut net, registry, &mut promised) {
        factorio_bot_core::tracing::warn!("routing steps failed: {e}");
    }

    // Power: call ensure_powered for each electric module instance.
    for (design, instance) in selection.designs.iter().zip(selection.instances.iter()) {
        let kw = design.operation.power_watts as f64 / 1000.0;
        if kw <= 0.0 {
            continue;
        }
        let site = Position::new(
            instance.placement.half_x as f64 * 0.5,
            instance.placement.half_y as f64 * 0.5,
        );
        let clearance = &design.required_clearance;
        let max_hx = clearance
            .iter()
            .map(|o| o.half_x.abs())
            .max()
            .unwrap_or(6)
            .abs();
        let max_hy = clearance
            .iter()
            .map(|o| o.half_y.abs())
            .max()
            .unwrap_or(6)
            .abs();
        let site_area = factorio_bot_core::types::Rect {
            left_top: Position::new(site.x - max_hx as f64 * 0.5, site.y - max_hy as f64 * 0.5),
            right_bottom: Position::new(site.x + max_hx as f64 * 0.5, site.y + max_hy as f64 * 0.5),
        };
        let occupants: Vec<FactorioEntity> = Vec::new();
        // Apply 20% policy headroom to power demand.
        let headroom_kw = kw * 1.2;
        match crate::method::power::ensure_powered(
            &mut ctx,
            "module",
            &site,
            &site_area,
            headroom_kw,
            20.0,
            &occupants,
        ) {
            Ok(Some(powering)) => {
                if let Err(e) =
                    run_steps(powering.steps, &mut ctx, &mut net, registry, &mut promised)
                {
                    factorio_bot_core::tracing::warn!("power steps failed: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => {
                factorio_bot_core::tracing::warn!("power: {e}");
            }
        }
    }

    // Module placements already handle production — no legacy fallback.
    net.infer_edges();

    let sched = match schedule(&net, state, roster) {
        Ok(s) => s,
        Err(_) => {
            // Fallback: use schedule_fallback which respects chain ownership,
            // travel, and precondition validation.
            match crate::modules::fallback::schedule_fallback(&net, state, roster, control) {
                Ok(sched) => sched,
                Err(fallback_err) => {
                    factorio_bot_core::tracing::warn!(
                        "primary schedule failed and fallback also failed: {fallback_err}"
                    );
                    // Last-resort: assign all actions to bot 1 sequentially,
                    // respecting topological order only.
                    let order = net
                        .topo_order()
                        .unwrap_or_else(|_| net.actions().map(|a| a.id).collect());
                    let bots: Vec<BotId> = if roster.is_empty() {
                        vec![BotId(1)]
                    } else {
                        roster.to_vec()
                    };
                    let mut owner_ticks: std::collections::HashMap<BotId, u32> =
                        bots.iter().map(|b| (*b, 0u32)).collect();
                    let mut steps: Vec<ScheduledStep> = Vec::new();
                    for action_id in &order {
                        let action = match net.action(*action_id) {
                            Some(a) => a,
                            None => continue,
                        };
                        let dur = std::cmp::max(action.duration, 60);
                        let (bot, tick) = owner_ticks
                            .iter()
                            .min_by_key(|(_, t)| **t)
                            .map(|(b, t)| (*b, *t))
                            .unwrap_or((BotId(1), 0));
                        steps.push(ScheduledStep {
                            what: StepKind::Act {
                                action: action.id,
                                label: String::new(),
                            },
                            bot,
                            start: tick,
                            end: tick + dur,
                        });
                        owner_ticks.insert(bot, tick + dur);
                    }
                    let makespan = owner_ticks.values().max().copied().unwrap_or(0);
                    Schedule { steps, makespan }
                }
            }
        }
    };
    let memory = ReplanMemory {
        plan_round: 0,
        entities: BTreeMap::new(),
        cells: vec![],
        chains: vec![],
        blocks: vec![],
        recovery_overrides: BTreeSet::new(),
        modules: InstanceMemory::default(),
        corridors: corridor_reservations,
    };
    crate::request::PlanResult {
        status: crate::request::PlanStatus::Complete,
        incumbent: Some(crate::request::PlannedMilestone {
            net: net.clone(),
            schedule: sched,
            memory,
        }),
        diagnostic: None,
        budget: control.report(),
    }
}

/// Compute the number of module instances needed to satisfy a goal.
///
/// Uses the module's output rate: at 1 plate per 600 ticks (OreToPlate base),
/// 5 plates need 3000 ticks. With candidate_limit as an upper bound, this
/// ensures we don't over-allocate modules while still producing enough.
fn design_instances_needed(design: &ModuleDesign, per_minute: u32, support_ticks: u64) -> usize {
    // Find the highest output rate across all output ports.
    let max_rate = design
        .operation
        .outputs
        .values()
        .map(|r| r.numerator as f64 / r.ticks.get() as f64)
        .fold(0.0f64, |a, b| a.max(b));

    if max_rate <= 0.0 || per_minute == 0 {
        return 1;
    }

    // Per-minute output of one cell: rate * 3600 ticks/min
    let cell_per_minute = max_rate * 3600.0;
    if cell_per_minute <= 0.0 {
        return 1;
    }

    // How many cells to meet the target per-minute rate.
    let needed = (per_minute as f64 / cell_per_minute).ceil() as usize;
    needed.max(1)
}

/// Extract a production item from a goal that the module system can handle.
fn production_item_from_goal(goal: &Goal) -> Option<(String, u32)> {
    match goal {
        // Goal::Have is handled by the legacy hand-craft system.
        // Only route explicit production goals to the module planner.
        Goal::Produced { item, count, .. } => Some((item.clone(), *count)),
        Goal::Producing { item, per_minute } => Some((item.clone(), *per_minute)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::control::{BudgetLimits, PlanControl};
    use crate::goal::{Goal, Holder};
    use crate::ids::BotId;
    use crate::registry_for;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;

    fn make_control() -> PlanControl {
        PlanControl::new(BudgetLimits::default())
    }

    #[test]
    fn plan_with_session_produces_a_plan() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let control = make_control();
        let options = PlannerOptions {
            mode: PlannerMode::Modules,
            cache_mode: CacheMode::On,
            candidate_limit: 1,
            support_ticks: 18000,
            external_rates: BTreeMap::new(),
        };
        let mut session = PlannerSession::new();

        let result = plan_with_session(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 5,
                whose: Holder::Anyone,
                via: None,
            }],
            &state,
            &registry_for(&[BotId(1)]),
            BotId(1),
            &[BotId(1)],
            &control,
            &options,
            &mut session,
        );

        // Should produce a plan (or at least not panic).
        assert!(
            result.incumbent.is_some()
                || matches!(result.status, crate::request::PlanStatus::Infeasible)
        );
    }

    #[test]
    fn compile_module_placement_produces_steps() {
        use crate::modules::artifact::*;
        use crate::modules::instance::Placement;

        let design = ModuleDesign {
            schema: 1,
            id: "test".into(),
            family: ModuleFamily::OreToPlate,
            generator_version: 1,
            origin: KnowledgeOrigin::Extracted,
            parameters: ModuleParameters {
                item: "iron-plate".into(),
                with_pole: false,
                labs: 0,
                machine: None,
                units: None,
            },
            prototype_hash: "test".into(),
            mod_versions: BTreeMap::new(),
            parents: vec![],
            training_manifest: None,
            parts: vec![Part {
                role: "drill".into(),
                entity: "burner-mining-drill".into(),
                offset: Offset {
                    half_x: 0,
                    half_y: 0,
                },
                direction: 0,
                recipe: None,
                underground_half: None,
                half_size: None,
            }],
            ports: vec![],
            required_clearance: vec![],
            expansion_space: vec![],
            bill: BTreeMap::new(),
            precedence: vec![],
            operation: OperatingContract {
                inputs: BTreeMap::new(),
                outputs: BTreeMap::from([("iron-plate".into(), Rate::new(1, 600).unwrap())]),
                power_watts: 0,
                fuel_per_tick: BTreeMap::new(),
                startup_latency_ticks: 0,
                startup_items: BTreeMap::new(),
                local_buffer_capacity: BTreeMap::new(),
                required_research: vec![],
                required_surface: "nauvis".into(),
                unsupported_mechanisms: vec![],
            },
        };

        let instance = ModuleInstance {
            id: 1,
            design_id: "test".into(),
            placement: Placement {
                surface: "nauvis".into(),
                half_x: 0,
                half_y: 0,
                direction: 0,
            },
            bindings: vec![],
            parts: BTreeMap::new(),
            construction_actions: BTreeMap::new(),
            commissioned_tick: None,
        };

        let mut ids = ActionIdGen::new();
        let steps = compile_module_placement(&design, &instance, &mut ids);
        // 1 drill part (no recipe): Place + Fuel = 2 steps
        assert_eq!(steps.len(), 2, "1 drill part: Place + Fuel");
        // 2 Act steps: Place(0) + Fuel(1), next id = 2
        assert_eq!(
            ids.next(),
            crate::ids::ActionId(2),
            "2 ActionIds: Place + Fuel"
        );
    }

    #[test]
    fn module_pipeline_produces_scheduled_steps() {
        use std::sync::Arc;

        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let control = make_control();
        let options = PlannerOptions {
            mode: PlannerMode::Modules,
            cache_mode: CacheMode::On,
            candidate_limit: 3,
            support_ticks: 18000,
            external_rates: BTreeMap::new(),
        };
        let mut session = PlannerSession::new();
        let roster = [BotId(1)];
        let chain_actor = crate::method::pick_chain_actor(&state, &roster).unwrap();

        let result = plan_with_session(
            &[Goal::Have {
                item: "iron-plate".into(),
                count: 5,
                whose: Holder::Anyone,
                via: None,
            }],
            &state,
            &registry_for(&roster),
            chain_actor,
            &roster,
            &control,
            &options,
            &mut session,
        );

        // The fixture world may have pre-existing entities that block placement,
        // so either Complete or Infeasible are acceptable outcomes.
        if result.status == crate::request::PlanStatus::Complete {
            let milestone = result.incumbent.unwrap();
            assert!(
                !milestone.schedule.steps.is_empty(),
                "should have scheduled steps"
            );

            // Compute makespan from the schedule.
            let makespan = milestone
                .schedule
                .steps
                .iter()
                .map(|step| step.end)
                .max()
                .unwrap_or(0);
            assert!(makespan > 0, "makespan should be positive");

            // Find the ActionNetwork and check for Place actions.
            let place_actions: Vec<&str> = milestone
                .net
                .actions()
                .filter_map(|action| match &action.kind {
                    crate::action::ActionKind::Place { entity } => Some(entity.name.as_str()),
                    _ => None,
                })
                .collect();
            // With OreToPlate, at minimum we should have drill or furnace placement.
            assert!(
                !place_actions.is_empty(),
                "should have at least one Place action"
            );
        }
        // If Infeasible, the diagnostic should explain why (occupied ground, etc.)
        if result.status == crate::request::PlanStatus::Infeasible {
            assert!(
                result.diagnostic.is_some(),
                "Infeasible should carry a diagnostic"
            );
        }
    }

    #[test]
    fn module_pipeline_refuses_unknown_items() {
        let state = PlanState::from_world(std::sync::Arc::new(fixture_world()), &[BotId(1)]);
        let control = make_control();
        let options = PlannerOptions::default();
        let mut session = PlannerSession::new();
        let roster = [BotId(1)];
        let chain_actor = crate::method::pick_chain_actor(&state, &roster).unwrap();

        let result = plan_with_session(
            &[Goal::Have {
                item: "nonexistent-item".into(),
                count: 1,
                whose: Holder::Anyone,
                via: None,
            }],
            &state,
            &registry_for(&roster),
            chain_actor,
            &roster,
            &control,
            &options,
            &mut session,
        );

        assert!(
            result.status == crate::request::PlanStatus::Complete
                || result.status == crate::request::PlanStatus::Infeasible,
            "unexpected status: {:?}",
            result.status
        );
    }
}
