// The imports below are intentionally broad for a skeleton module.
#![allow(unused_imports, dead_code)]
//! Compile selected module instances into action networks and schedules.
//!
//! The module compilation path takes selected designs and instances, creates
//! the necessary placement/configuration/fuel actions, feeds them through
//! the existing scheduler, and validates the operating ledger.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::control::{BudgetLimits, PlanControl, WorkKind};
use crate::error::PlannerError;
use crate::goal::Goal;
use crate::ids::BotId;
use crate::method::{MethodRegistry, Step};
use crate::modules::cache::{CacheMode, LibraryCache};
use crate::modules::instance::{InstanceMemory, ModuleInstance};
use crate::modules::ledger::OperatingLedger;
use crate::modules::select::ModuleSelection;
use crate::network::ActionNetwork;
use crate::schedule::{schedule, Schedule};
use crate::state::PlanState;

// ---------------------------------------------------------------------------
// PlannerMode
// ---------------------------------------------------------------------------

/// Which planning engine to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannerMode {
    /// Use the existing goal-based planner (default).
    Legacy,
    /// Use the module-backed planner for supported production families.
    Modules,
}

// ---------------------------------------------------------------------------
// PlannerOptions
// ---------------------------------------------------------------------------

/// Configuration for a planning session.
#[derive(Debug, Clone)]
pub struct PlannerOptions {
    pub mode: PlannerMode,
    pub cache_mode: CacheMode,
    pub candidate_limit: usize,
    pub support_ticks: u32,
}

impl Default for PlannerOptions {
    fn default() -> Self {
        Self {
            mode: PlannerMode::Legacy,
            cache_mode: CacheMode::On,
            candidate_limit: 8,
            support_ticks: 18000, // 5 minutes at 60 UPS
        }
    }
}

// ---------------------------------------------------------------------------
// PlannerSession
// ---------------------------------------------------------------------------

/// Mutable state for a planning session, persisting across replans.
#[derive(Debug, Clone)]
pub struct PlannerSession {
    pub library: LibraryCache,
    pub memory: InstanceMemory,
    pub observed_revision: u64,
}

impl PlannerSession {
    pub fn new() -> Self {
        Self {
            library: LibraryCache::new(),
            memory: InstanceMemory::default(),
            observed_revision: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// CompiledModules
// ---------------------------------------------------------------------------

/// The result of compiling a module selection into executable steps.
#[derive(Debug, Clone)]
pub struct CompiledModules {
    /// Actions emitted for construction, configuration, and fueling.
    pub steps: Vec<Step>,
    /// Module instances that were compiled (with updated state).
    pub instances: Vec<ModuleInstance>,
    /// Operating ledger for the compiled modules.
    pub ledger: OperatingLedger,
}

// ---------------------------------------------------------------------------
// compile_selection
// ---------------------------------------------------------------------------

/// Compile a module selection into construction steps and a ledger.
///
/// This is the bridge between the module representation and the existing
/// action/scheduling machinery. Each instance is translated into placement
/// and configuration actions (mimicking the existing method expansion) and
/// the steps are returned for the scheduler to run.
pub fn compile_selection(
    _selection: &ModuleSelection,
    _ctx: &mut ExpansionCtx,
    _control: &PlanControl,
) -> Result<CompiledModules, PlannerError> {
    // Check budget.
    _control.checkpoint()?;

    // For now, this is a placeholder that returns an empty compilation.
    // Full compilation will:
    // 1. For each instance, create placement/configuration steps from the
    //    design's parts using the same underlying logic as method::produce
    //    and method::assemble
    // 2. Compute fuel and startup costs from the operating contract
    // 3. Build the operating ledger from the design's predictions
    // 4. Return the compiled result for scheduling

    Ok(CompiledModules {
        steps: Vec::new(),
        instances: _selection.instances.clone(),
        ledger: OperatingLedger::default(),
    })
}

// Use the existing ExpansionCtx type.
use crate::method::ExpansionCtx;

// ---------------------------------------------------------------------------
// plan_with_session
// ---------------------------------------------------------------------------

/// Module-mode planning entry point.
///
/// Selects, sites, and compiles modules for supported production goals.
/// Falls back to the legacy planner for unsupported goals.
pub fn plan_with_session(
    goals: &[Goal],
    state: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
    roster: &[BotId],
    control: &PlanControl,
    _options: &PlannerOptions,
    _session: &mut PlannerSession,
) -> crate::request::PlanResult {
    // For now, fall back to the legacy planner.
    // Future implementation will:
    // 1. Scan goals for producible items
    // 2. Select module designs using session.library
    // 3. Site instances and compile steps
    // 4. Schedule and return result with module metadata

    crate::request::plan_controlled(goals, state, registry, chain_actor, roster, control)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::BudgetLimits;
    use crate::goal::{Goal, Holder};
    use crate::state::PlanState;
    use crate::registry_for;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn make_control() -> PlanControl {
        PlanControl::new(BudgetLimits::default())
    }

    #[test]
    fn options_default_to_legacy() {
        let opts = PlannerOptions::default();
        assert_eq!(opts.mode, PlannerMode::Legacy);
        assert_eq!(opts.candidate_limit, 8);
        assert_eq!(opts.support_ticks, 18000);
    }

    #[test]
    fn plan_with_session_legacy_fallback_works() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let control = make_control();
        let options = PlannerOptions::default();
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

        // Should produce a plan (falls back to legacy).
        assert!(result.incumbent.is_some());
    }

    #[test]
    fn compile_selection_budget_check() {
        let selection = ModuleSelection {
            designs: vec![],
            instances: vec![],
            requests: vec![],
        };

        let control = PlanControl::new(BudgetLimits {
            maxima: BTreeMap::from([(WorkKind::Goal, 0)]),
        });

        // Create a minimal ExpansionCtx.
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let mut ctx = ExpansionCtx::new(state, BotId(1));

        let result = compile_selection(&selection, &mut ctx, &control);
        // With zero Goal budget, compile_selection should still succeed
        // since it doesn't charge Goal in the placeholder implementation.
        assert!(result.is_ok());
    }
}
