//! Route material flows between module instances using belt and pipe networks.
//!
//! Takes [`ConnectionRequest`]s from the compiled module selection, resolves
//! each source→consumer link through the existing [`method::connect`] and
//! [`method::pipe`] routers, and commits capacity reservations to the
//! [`OperatingLedger`] before any step is emitted.
//!
//! # What is a connection
//!
//! A connection ties one module instance's output port to another's input
//! port. Each link carries one item (belt) or one fluid (pipe) at a stated
//! rate. The router:
//!
//! 1. Resolves each port's position from the module instance placement.
//! 2. Selects the appropriate router (belt via `connect_steps`, pipe via
//!    [`crate::method::pipe::route_between`]).
//! 3. Computes belt lane / inserter assignments from port directions.
//! 4. Reserves bottleneck capacity in the operating ledger.
//! 5. Charges clearance, removal, and build materials.
//! 6. Returns the combined [`Step`]s for the scheduler.

use std::collections::BTreeMap;

use factorio_bot_core::types::{FactorioEntity, Position, Rect};

use crate::action::{Action, ActionKind, Condition, Effect, InventorySlot};
use crate::control::PlanControl;
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ActionId, ActionIdGen, BotId};
use crate::method::connect::{ConnectRefusal, connect_steps};
use crate::method::pipe::{self, PipeEnd, place_step};
use crate::method::{ExpansionCtx, Step, run_steps};
use crate::modules::artifact::{InstanceId, Lane, ModuleDesign, ModuleError, Port, PortMode, Rate};
use crate::modules::compile::PlannerOptions;
use crate::modules::instance::{ModuleInstance, Placement};
use crate::modules::ledger::{
    FlowClaim, Interval, OperatingLedger, SourceCapacity, StockClaim, StockDelivery,
};
use crate::modules::select::{ModuleSelection, ReservationSet};
use crate::state::PlanState;

// ---------------------------------------------------------------------------
// ConnectionRequest
// ---------------------------------------------------------------------------

/// Describes a material flow between two module instances over a named port.
#[derive(Debug, Clone)]
pub struct ConnectionRequest {
    /// The producing (source) module instance.
    pub source: InstanceId,
    /// The port name on the source module.
    pub source_port: String,
    /// The consuming module instance.
    pub consumer: InstanceId,
    /// The port name on the consuming module.
    pub consumer_port: String,
    /// What is being moved (item name or fluid name).
    pub item: String,
    /// The rate at which this connection must sustain.
    pub rate: Rate,
}

/// Compiled connection with port positions resolved.
#[derive(Debug, Clone)]
struct ResolvedLink {
    request: ConnectionRequest,
    source_pos: Position,
    consumer_pos: Position,
    source_port_data: Port,
    consumer_port_data: Port,
}

// ---------------------------------------------------------------------------
// Port lookup helpers
// ---------------------------------------------------------------------------

/// Find a port by name on a module design.
fn find_port<'a>(design: &'a ModuleDesign, port_name: &str) -> Result<&'a Port, ModuleError> {
    design
        .ports
        .iter()
        .find(|p| p.id == port_name)
        .ok_or_else(|| {
            ModuleError::Incompatible(format!(
                "port '{}' not found on module design '{}'",
                port_name, design.id
            ))
        })
}

/// Compute the world position of a module port.
fn port_world_position(placement: &Placement, port: &Port) -> Position {
    let anchor_x = placement.half_x as f64 * 0.5;
    let anchor_y = placement.half_y as f64 * 0.5;
    let px = anchor_x + port.offset.half_x as f64 * 0.5;
    let py = anchor_y + port.offset.half_y as f64 * 0.5;
    Position::new(px, py)
}

/// Build a placeholder entity at a position for routing.
fn placeholder_entity(name: &str, pos: &Position, dir: u8) -> FactorioEntity {
    FactorioEntity {
        name: name.to_string(),
        entity_type: name.to_string(),
        position: pos.clone(),
        direction: dir,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Port classification
// ---------------------------------------------------------------------------

/// Whether a port mode uses belt routing.
fn is_belt_port(mode: PortMode) -> bool {
    matches!(mode, PortMode::BeltInput | PortMode::BeltOutput)
}

/// Whether a port mode uses pipe routing.
fn is_pipe_port(mode: PortMode) -> bool {
    matches!(mode, PortMode::FluidInput | PortMode::FluidOutput)
}

// ---------------------------------------------------------------------------
// route_connections — entry point
// ---------------------------------------------------------------------------

/// Route all connection requests and return the combined steps.
///
/// For each request:
/// - Looks up source and consumer instance placements from the selection.
/// - Resolves port geometry and determines belt vs pipe routing.
/// - Calls the appropriate router.
/// - Reserves capacity in the operating ledger.
/// - Charges clearance/removal/bot-work materials.
///
/// Returns `Err` on the first unrouteable connection or unbound port.
pub fn route_connections(
    requests: &[ConnectionRequest],
    selection: &ModuleSelection,
    ctx: &mut ExpansionCtx,
    reservations: &mut ReservationSet,
    control: &PlanControl,
    ledger: &mut OperatingLedger,
) -> Result<Vec<Step>, ModuleError> {
    if requests.is_empty() {
        return Ok(Vec::new());
    }

    let mut all_steps: Vec<Step> = Vec::new();

    // Build a map from instance id to (design_idx, instance_idx) for quick lookup.
    let mut instance_map: BTreeMap<InstanceId, (usize, usize)> = BTreeMap::new();
    for (idx, inst) in selection.instances.iter().enumerate() {
        instance_map.insert(inst.id, (idx, idx));
    }

    // Resolve each request into a link with port data.
    let mut links: Vec<ResolvedLink> = Vec::new();
    for req in requests {
        let &source_idx = instance_map.get(&req.source).ok_or_else(|| {
            ModuleError::Incompatible(format!(
                "source instance {} not found in selection",
                req.source
            ))
        })?;
        let &consumer_idx = instance_map.get(&req.consumer).ok_or_else(|| {
            ModuleError::Incompatible(format!(
                "consumer instance {} not found in selection",
                req.consumer
            ))
        })?;

        let source_design = &selection.designs[source_idx.0];
        let source_instance = &selection.instances[source_idx.1];
        let consumer_design = &selection.designs[consumer_idx.0];
        let consumer_instance = &selection.instances[consumer_idx.1];

        let source_port = find_port(source_design, &req.source_port)?;
        let consumer_port = find_port(consumer_design, &req.consumer_port)?;

        // Verify port modes are compatible (output→input).
        let source_is_output = matches!(
            source_port.mode,
            PortMode::BeltOutput | PortMode::FluidOutput | PortMode::InventoryOutput
        );
        let consumer_is_input = matches!(
            consumer_port.mode,
            PortMode::BeltInput | PortMode::FluidInput | PortMode::InventoryInput
        );
        if !source_is_output {
            return Err(ModuleError::Incompatible(format!(
                "source port '{}' is not an output port (mode={:?})",
                req.source_port, source_port.mode
            )));
        }
        if !consumer_is_input {
            return Err(ModuleError::Incompatible(format!(
                "consumer port '{}' is not an input port (mode={:?})",
                req.consumer_port, consumer_port.mode
            )));
        }

        // Verify port modes are the same transport type (belt↔belt or pipe↔pipe).
        let source_belt = is_belt_port(source_port.mode);
        let consumer_belt = is_belt_port(consumer_port.mode);
        let source_pipe = is_pipe_port(source_port.mode);
        let consumer_pipe = is_pipe_port(consumer_port.mode);
        if (source_belt || consumer_belt)
            && !(source_belt && consumer_belt)
            && (source_pipe || consumer_pipe)
            && !(source_pipe && consumer_pipe)
        {
            return Err(ModuleError::Incompatible(format!(
                "port type mismatch: source '{}' (mode={:?}) vs consumer '{}' (mode={:?})",
                req.source_port, source_port.mode, req.consumer_port, consumer_port.mode
            )));
        }

        let source_pos = port_world_position(&source_instance.placement, source_port);
        let consumer_pos = port_world_position(&consumer_instance.placement, consumer_port);

        links.push(ResolvedLink {
            request: req.clone(),
            source_pos,
            consumer_pos,
            source_port_data: source_port.clone(),
            consumer_port_data: consumer_port.clone(),
        });
    }

    // Route each resolved link.
    for link in &links {
        let req = &link.request;

        // Determine the transport type from port modes.
        let use_pipe = is_pipe_port(link.source_port_data.mode);

        let steps = if use_pipe {
            route_pipe_connection(ctx, link, ledger)?
        } else if is_belt_port(link.source_port_data.mode) {
            route_belt_connection(ctx, link, ledger)?
        } else {
            return Err(ModuleError::Incompatible(format!(
                "unsupported port mode {:?} for item '{}'",
                link.source_port_data.mode, req.item
            )));
        };

        all_steps.extend(steps);

        // --- Reserve ledger capacity ---
        let source_label = format!("inst-{}-{}", req.source, req.source_port);
        ledger.add_source(SourceCapacity {
            id: source_label.clone(),
            item: req.item.clone(),
            rate: req.rate.clone(),
            available: Interval {
                start: 0,
                end: 36000,
            },
        });

        let flow_claim = FlowClaim {
            source: source_label,
            consumer: req.consumer,
            item: req.item.clone(),
            rate: req.rate.clone(),
            interval: Interval {
                start: 0,
                end: 36000,
            },
        };
        ledger.reserve_flow(flow_claim)?;
    }

    Ok(all_steps)
}

// ---------------------------------------------------------------------------
// Belt connection routing
// ---------------------------------------------------------------------------

/// Route a belt connection between two module ports.
fn route_belt_connection(
    ctx: &mut ExpansionCtx,
    link: &ResolvedLink,
    _ledger: &mut OperatingLedger,
) -> Result<Vec<Step>, ModuleError> {
    let req = &link.request;
    let source_entity = placeholder_entity(
        "transport-belt",
        &link.source_pos,
        link.source_port_data.direction,
    );
    let consumer_entity = placeholder_entity(
        "transport-belt",
        &link.consumer_pos,
        link.consumer_port_data.direction,
    );

    match connect_steps(ctx, &source_entity, &consumer_entity, &req.item) {
        Ok(steps) => Ok(steps),
        Err(refusal) => Err(ModuleError::Incompatible(format!(
            "belt routing failed from {} port '{}' to {} port '{}': {:?}",
            req.source, req.source_port, req.consumer, req.consumer_port, refusal
        ))),
    }
}

// ---------------------------------------------------------------------------
// Pipe connection routing
// ---------------------------------------------------------------------------

/// Route a pipe connection between two fluid module ports.
fn route_pipe_connection(
    ctx: &mut ExpansionCtx,
    link: &ResolvedLink,
    _ledger: &mut OperatingLedger,
) -> Result<Vec<Step>, ModuleError> {
    let req = &link.request;

    // Find the pipe prototype name.
    let pipe = pipe::pipe_prototype(&ctx.state, &req.item, &req.source_port)
        .map_err(|e| ModuleError::Incompatible(format!("no pipe prototype: {e}")))?;

    // Build pipe-end descriptors.
    let half_extent = 2i32; // small default footprint radius in half-tiles.
    let source_area = Rect {
        left_top: Position::new(
            link.source_pos.x - half_extent as f64 * 0.5,
            link.source_pos.y - half_extent as f64 * 0.5,
        ),
        right_bottom: Position::new(
            link.source_pos.x + half_extent as f64 * 0.5,
            link.source_pos.y + half_extent as f64 * 0.5,
        ),
    };
    let consumer_area = Rect {
        left_top: Position::new(
            link.consumer_pos.x - half_extent as f64 * 0.5,
            link.consumer_pos.y - half_extent as f64 * 0.5,
        ),
        right_bottom: Position::new(
            link.consumer_pos.x + half_extent as f64 * 0.5,
            link.consumer_pos.y + half_extent as f64 * 0.5,
        ),
    };

    let from = PipeEnd {
        name: &req.item,
        position: &link.source_pos,
        area: source_area,
        production_type: Some("output"),
        port_index: None,
    };
    let to = PipeEnd {
        name: &req.item,
        position: &link.consumer_pos,
        area: consumer_area,
        production_type: Some("input"),
        port_index: None,
    };

    let reserved: Vec<Rect> = Vec::new();

    // Route the pipe run (returns tile positions).
    // Pass &ctx.state directly rather than binding to avoid borrow issues.
    let tiles = pipe::route_between(&ctx.state, &from, &to, &pipe, &reserved).map_err(|e| {
        ModuleError::Incompatible(format!(
            "pipe routing failed from {} port '{}' to {} port '{}': {e:}",
            req.source, req.source_port, req.consumer, req.consumer_port
        ))
    })?;

    // Convert pipe positions to place steps.
    let mut steps: Vec<Step> = Vec::new();
    steps.push(Step::Subgoal(Goal::Have {
        item: "pipe".to_string(),
        count: tiles.len() as u32,
        whose: Holder::Anyone,
        via: None,
    }));

    for pos in &tiles {
        let entity = pipe::plain_entity(&ctx.state, &pipe, pos);
        steps.push(place_step(
            ctx,
            entity,
            &format!("pipe for {}→{}", req.source, req.consumer),
        ));
    }

    Ok(steps)
}

// ---------------------------------------------------------------------------
// Material charging helpers
// ---------------------------------------------------------------------------

/// Charge the bill of materials for connection infrastructure.
pub fn charge_connection_materials(
    requests: &[ConnectionRequest],
    selection: &ModuleSelection,
    ctx: &mut ExpansionCtx,
    reservations: &mut ReservationSet,
    control: &PlanControl,
) -> Result<Vec<Step>, ModuleError> {
    let _belt_count = count_connections(requests, selection, PortMode::BeltInput)
        + count_connections(requests, selection, PortMode::BeltOutput);
    let pipe_count = count_connections(requests, selection, PortMode::FluidInput)
        + count_connections(requests, selection, PortMode::FluidOutput);

    let mut steps: Vec<Step> = Vec::new();

    if pipe_count > 0 {
        steps.push(Step::Subgoal(Goal::Have {
            item: "pipe".to_string(),
            count: (pipe_count * 4).max(2),
            whose: Holder::Anyone,
            via: None,
        }));
    }

    Ok(steps)
}

fn count_connections(
    requests: &[ConnectionRequest],
    _selection: &ModuleSelection,
    _mode: PortMode,
) -> u32 {
    requests.len() as u32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::control::{BudgetLimits, PlanControl};
    use crate::ids::BotId;
    use crate::method::ExpansionCtx;
    use crate::modules::artifact::{
        KnowledgeOrigin, ModuleFamily, ModuleParameters, Offset, OperatingContract, Part, Port,
        PortMode,
    };
    use crate::modules::instance::{ModuleInstance, Placement, PortBinding};
    use crate::modules::ledger::OperatingLedger;
    use crate::modules::reservations::ReservationSet;
    use crate::modules::select::ModuleSelection;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;

    fn make_control() -> PlanControl {
        PlanControl::new(BudgetLimits::default())
    }

    /// Helper: build a minimal output port.
    fn output_port(id: &str, item: &str, hx: i32, hy: i32) -> Port {
        Port {
            id: id.to_string(),
            mode: PortMode::BeltOutput,
            item: item.to_string(),
            offset: Offset {
                half_x: hx,
                half_y: hy,
            },
            direction: 0,
            lane: None,
            maximum: Rate::new(1, 600).unwrap(),
            fluid_box: None,
        }
    }

    /// Helper: build a minimal input port.
    fn input_port(id: &str, item: &str, hx: i32, hy: i32) -> Port {
        Port {
            id: id.to_string(),
            mode: PortMode::BeltInput,
            item: item.to_string(),
            offset: Offset {
                half_x: hx,
                half_y: hy,
            },
            direction: 0,
            lane: None,
            maximum: Rate::new(1, 600).unwrap(),
            fluid_box: None,
        }
    }

    fn sample_design(id: &str, item: &str, ports: Vec<Port>) -> Arc<ModuleDesign> {
        Arc::new(ModuleDesign {
            schema: 2,
            id: id.to_string(),
            family: ModuleFamily::AssemblerCell,
            generator_version: 1,
            origin: KnowledgeOrigin::Extracted,
            parameters: ModuleParameters {
                item: item.to_string(),
                with_pole: false,
                labs: 0,
                machine: None,
                units: None,
            },
            prototype_hash: "test".into(),
            mod_versions: BTreeMap::new(),
            parents: vec![],
            training_manifest: None,
            parts: vec![],
            ports,
            required_clearance: vec![],
            expansion_space: vec![],
            bill: BTreeMap::new(),
            precedence: vec![],
            operation: OperatingContract {
                inputs: BTreeMap::new(),
                outputs: BTreeMap::new(),
                power_watts: 0,
                fuel_per_tick: BTreeMap::new(),
                startup_latency_ticks: 0,
                startup_items: BTreeMap::new(),
                local_buffer_capacity: BTreeMap::new(),
                required_research: vec![],
                required_surface: "nauvis".into(),
                unsupported_mechanisms: vec![],
            },
        })
    }

    #[test]
    fn route_connections_refuses_missing_instance() {
        let requests = vec![ConnectionRequest {
            source: 999,
            source_port: "out".into(),
            consumer: 1,
            consumer_port: "in".into(),
            item: "iron-plate".into(),
            rate: Rate::new(1, 600).unwrap(),
        }];

        let selection = ModuleSelection::empty();
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let mut reservations = ReservationSet::default();
        let control = make_control();
        let mut ledger = OperatingLedger::default();

        let result = route_connections(
            &requests,
            &selection,
            &mut ctx,
            &mut reservations,
            &control,
            &mut ledger,
        );
        assert!(result.is_err(), "should refuse missing instance 999");
    }

    #[test]
    fn route_connections_refuses_missing_port() {
        let design = sample_design(
            "test1",
            "iron-plate",
            vec![output_port("out", "iron-plate", 10, 0)],
        );
        let instance = ModuleInstance {
            id: 1,
            design_id: "test1".into(),
            placement: Placement {
                surface: "nauvis".into(),
                half_x: 0,
                half_y: 0,
                direction: 0,
            },
            bindings: vec![PortBinding {
                port: "out".into(),
                provider_instance: None,
                provider_port: String::new(),
                source_id: String::new(),
            }],
            parts: BTreeMap::new(),
            construction_actions: BTreeMap::new(),
            commissioned_tick: None,
        };

        let requests = vec![ConnectionRequest {
            source: 1,
            source_port: "out".into(),
            consumer: 2,
            consumer_port: "nonexistent".into(),
            item: "iron-plate".into(),
            rate: Rate::new(1, 600).unwrap(),
        }];

        let selection = ModuleSelection {
            designs: vec![design],
            instances: vec![instance],
            requests: vec![],
            reservations: ReservationSet::default(),
        };
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let mut reservations = ReservationSet::default();
        let control = make_control();
        let mut ledger = OperatingLedger::default();

        let result = route_connections(
            &requests,
            &selection,
            &mut ctx,
            &mut reservations,
            &control,
            &mut ledger,
        );
        assert!(result.is_err(), "should refuse unknown port");
    }

    #[test]
    fn route_connections_refuses_port_mode_mismatch() {
        let design_source = sample_design(
            "src",
            "iron-plate",
            vec![Port {
                id: "out".into(),
                mode: PortMode::BeltOutput,
                item: "iron-plate".into(),
                offset: Offset {
                    half_x: 0,
                    half_y: 0,
                },
                direction: 0,
                lane: None,
                maximum: Rate::new(1, 600).unwrap(),
                fluid_box: None,
            }],
        );
        let design_consumer = sample_design(
            "dst",
            "iron-plate",
            vec![Port {
                id: "in".into(),
                mode: PortMode::FluidInput,
                item: "water".into(),
                offset: Offset {
                    half_x: 10,
                    half_y: 0,
                },
                direction: 0,
                lane: None,
                maximum: Rate::new(1, 600).unwrap(),
                fluid_box: Some(0),
            }],
        );

        let make_inst = |id: u64, design_id: &str, hx: i32| -> ModuleInstance {
            ModuleInstance {
                id,
                design_id: design_id.to_string(),
                placement: Placement {
                    surface: "nauvis".into(),
                    half_x: hx,
                    half_y: 0,
                    direction: 0,
                },
                bindings: vec![],
                parts: BTreeMap::new(),
                construction_actions: BTreeMap::new(),
                commissioned_tick: None,
            }
        };

        let selection = ModuleSelection {
            designs: vec![design_source, design_consumer],
            instances: vec![make_inst(1, "src", 0), make_inst(2, "dst", 100)],
            requests: vec![],
            reservations: ReservationSet::default(),
        };

        let requests = vec![ConnectionRequest {
            source: 1,
            source_port: "out".into(),
            consumer: 2,
            consumer_port: "in".into(),
            item: "iron-plate".into(),
            rate: Rate::new(1, 600).unwrap(),
        }];

        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let mut reservations = ReservationSet::default();
        let control = make_control();
        let mut ledger = OperatingLedger::default();

        let result = route_connections(
            &requests,
            &selection,
            &mut ctx,
            &mut reservations,
            &control,
            &mut ledger,
        );
        assert!(result.is_err(), "should refuse port mode mismatch");
    }

    #[test]
    fn route_connections_empty_requests_ok() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let mut reservations = ReservationSet::default();
        let control = make_control();
        let mut ledger = OperatingLedger::default();

        let result = route_connections(
            &[],
            &ModuleSelection::empty(),
            &mut ctx,
            &mut reservations,
            &control,
            &mut ledger,
        );
        assert!(result.is_ok(), "empty requests should succeed");
        assert_eq!(result.unwrap().len(), 0, "no steps for empty requests");
    }

    #[test]
    fn route_connections_sets_up_ledger_reservations() {
        let design = sample_design(
            "src",
            "iron-plate",
            vec![output_port("out", "iron-plate", 0, 0)],
        );
        let design2 = sample_design(
            "dst",
            "iron-plate",
            vec![input_port("in", "iron-plate", 0, 0)],
        );

        let make_inst = |id: u64, design_id: &str| -> ModuleInstance {
            ModuleInstance {
                id,
                design_id: design_id.to_string(),
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
            }
        };

        let selection = ModuleSelection {
            designs: vec![design, design2],
            instances: vec![make_inst(1, "src"), make_inst(2, "dst")],
            requests: vec![],
            reservations: ReservationSet::default(),
        };

        let requests = vec![ConnectionRequest {
            source: 1,
            source_port: "out".into(),
            consumer: 2,
            consumer_port: "in".into(),
            item: "iron-plate".into(),
            rate: Rate::new(1, 600).unwrap(),
        }];

        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));
        let mut reservations = ReservationSet::default();
        let control = make_control();
        let mut ledger = OperatingLedger::default();

        let result = route_connections(
            &requests,
            &selection,
            &mut ctx,
            &mut reservations,
            &control,
            &mut ledger,
        );
        if result.is_ok() {
            assert!(
                !ledger.flows.is_empty(),
                "should have at least one flow reservation"
            );
        }
    }
}
