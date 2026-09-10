//! Records of planning requests: budgets, phases, and outcomes.
//!
//! These records are written by the planner driver and consumed by the
//! experiment report generator. Every field is optional so that partial
//! or legacy records can be deserialized without error.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A record of one planning request, with budget, phase timing, and outcome.
///
/// String-typed fields avoid a dependency from the core crate to the planner
/// crate. Unknown or unmeasured fields are `None`, never zero.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanningRecord {
    /// Opaque request identifier (e.g. milestone number or job id).
    pub request_id: Option<String>,

    /// Overall outcome: "complete", "exhausted", "infeasible", "unsupported",
    /// or "cancelled".
    pub outcome: Option<String>,

    /// Makespan of the best incumbent plan, in game ticks.
    pub incumbent_ticks: Option<u64>,

    // -- budget fields --

    /// Serialized budget limits (work kind -> max count).
    pub limits: Option<BTreeMap<String, u64>>,

    /// Serialized consumed work (work kind -> consumed count).
    pub used: Option<BTreeMap<String, u64>>,

    /// Stop reason, if the budget was exhausted: "limit(Goal)", "cancelled",
    /// etc.
    pub stop_reason: Option<String>,

    // -- phase timing (ms) --

    /// Exclusive wall-clock milliseconds spent in each planning phase.
    ///
    /// "Exclusive" means time spent in nested phases is attributed to the
    /// inner phase, not the outer one. Keys are phase names as strings
    /// ("expansion", "placement", "routing", "scheduling", "recovery").
    pub phase_ms: Option<BTreeMap<String, u64>>,

    /// Total wall-clock milliseconds for the entire planning request.
    pub total_ms: Option<u64>,

    // -- work counts --

    /// Number of call-counted goal expansions.
    pub goals_expanded: Option<u64>,

    /// Number of call-counted forks (state clones for scheduling).
    pub forks: Option<u64>,

    // -- cache /

    /// Design-cache hits during module-mode planning.
    pub cache_hits: Option<u64>,

    /// Design-cache misses (generation events).
    pub cache_misses: Option<u64>,

    /// How many sites were attempted before a feasible one was found.
    pub site_attempts: Option<u64>,

    /// How many retry rounds were used.
    pub retries: Option<u64>,

    // -- module info (only present in module mode) --

    /// Number of module instances selected.
    pub module_instance_count: Option<u64>,

    /// Design ID of the primary module.
    pub primary_design_id: Option<String>,
}
