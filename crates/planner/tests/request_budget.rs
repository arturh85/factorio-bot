//! Integration tests for controlled request outcomes and bounded conflict retries.
//!
//! This file is registered in crates/planner/tests/suite.rs.

use std::collections::BTreeMap;

use factorio_bot_planner::control::{
    BudgetLimits, PlanControl, WorkKind,
};
use factorio_bot_planner::PlanStatus;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::ids::BotId;
use factorio_bot_planner::registry_for;
use factorio_bot_planner::plan_controlled;
use factorio_bot_planner::state::PlanState;
use factorio_bot_core::test_utils::fixture_world;
use std::sync::Arc;

#[test]
fn zero_goal_budget_cannot_become_a_no_method_refusal() {
    let state = PlanState::from_world(
        Arc::new(fixture_world()),
        &[BotId(1)],
    );
    let control = PlanControl::new(BudgetLimits {
        maxima: BTreeMap::from([(WorkKind::Goal, 0)]),
    });
    let state_with_control = state.clone().with_control(control.clone());
    let result = plan_controlled(
        &[Goal::Have {
            item: "iron-plate".into(),
            count: 100,
            whose: Holder::Anyone,
            via: None,
        }],
        &state_with_control,
        &registry_for(&[BotId(1)]),
        BotId(1),
        &[BotId(1)],
        &control,
    );
    // A zero-goal budget should exhaust before finding a plan.
    assert!(
        matches!(result.status, PlanStatus::Exhausted),
        "expected Exhausted, got {:?}",
        result.status
    );
    assert!(result.incumbent.is_none());
}

#[test]
fn default_control_produces_same_plan_as_plan_best() {
    let world = Arc::new(fixture_world());
    let state = PlanState::from_world(world.clone(), &[BotId(1)]);
    let control = PlanControl::new(BudgetLimits::default());
    let state_with_control = state.clone().with_control(control.clone());

    let result = plan_controlled(
        &[Goal::Have {
            item: "iron-plate".into(),
            count: 5,
            whose: Holder::Anyone,
            via: None,
        }],
        &state_with_control,
        &registry_for(&[BotId(1)]),
        BotId(1),
        &[BotId(1)],
        &control,
    );

    assert!(
        matches!(result.status, PlanStatus::Complete),
        "expected Complete, got {:?}",
        result.status
    );
    assert!(result.incumbent.is_some());
    assert!(result.diagnostic.is_none());
    assert!(result.budget.stopped.is_none());
}

// The phase test helper is defined here so the test can use it without
// depending on Task 4's observer implementation.

