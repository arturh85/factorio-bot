//! Tests for rocket launch evidence and idempotent request/launch logic.
//!
//! These tests exercise the Rust-side evidence struct and the isolation
//! between request state, launch events and platform establishment.
//! They do NOT load the BotBridge Lua mod -- those tests belong in a
//! test fixture against a stub game.

use factorio_bot_core::record::rocket_launch::RocketLaunchEvidence;

// ---------------------------------------------------------------------------
// RocketLaunchEvidence unit tests
// ---------------------------------------------------------------------------

/// A request with no game-side events is NOT achieved.
#[test]
fn launch_ordered_alone_is_not_achieved() {
    let evidence = RocketLaunchEvidence {
        run_key: "test-1".into(),
        silo_unit_number: 42,
        platform_index: 1,
        payload: "space-platform-starter-pack".into(),
        launch_ordered_tick: Some(1000),
        launched_tick: None,
        platform_established_tick: None,
    };
    assert!(!evidence.achieved());
}

/// A launch that happened but no platform was established is NOT achieved.
#[test]
fn launched_without_platform_is_not_achieved() {
    let evidence = RocketLaunchEvidence {
        run_key: "test-2".into(),
        silo_unit_number: 42,
        platform_index: 1,
        payload: "space-platform-starter-pack".into(),
        launch_ordered_tick: Some(1000),
        launched_tick: Some(3000),
        platform_established_tick: None,
    };
    assert!(!evidence.achieved());
}

/// A platform that appeared without a launch is NOT achieved.
#[test]
fn platform_without_launch_is_not_achieved() {
    let evidence = RocketLaunchEvidence {
        run_key: "test-3".into(),
        silo_unit_number: 42,
        platform_index: 1,
        payload: "space-platform-starter-pack".into(),
        launch_ordered_tick: Some(1000),
        launched_tick: None,
        platform_established_tick: Some(5000),
    };
    assert!(!evidence.achieved());
}

/// Wrong payload (not starter pack) is NOT achieved even with both ticks.
#[test]
fn wrong_payload_is_not_achieved() {
    let evidence = RocketLaunchEvidence {
        run_key: "test-4".into(),
        silo_unit_number: 42,
        platform_index: 1,
        payload: "satellite".into(),
        launch_ordered_tick: Some(1000),
        launched_tick: Some(3000),
        platform_established_tick: Some(5000),
    };
    assert!(!evidence.achieved());
}

/// All fields present with correct payload IS achieved.
#[test]
fn full_success_is_achieved() {
    let evidence = RocketLaunchEvidence {
        run_key: "test-5".into(),
        silo_unit_number: 42,
        platform_index: 1,
        payload: "space-platform-starter-pack".into(),
        launch_ordered_tick: Some(1000),
        launched_tick: Some(3000),
        platform_established_tick: Some(5000),
    };
    assert!(evidence.achieved());
}

/// launch_ordered_tick is optional for achievement.
#[test]
fn null_launch_ordered_is_still_achieved() {
    let evidence = RocketLaunchEvidence {
        run_key: "test-6".into(),
        silo_unit_number: 42,
        platform_index: 1,
        payload: "space-platform-starter-pack".into(),
        launch_ordered_tick: None,
        launched_tick: Some(3000),
        platform_established_tick: Some(5000),
    };
    assert!(evidence.achieved());
}

/// Serialize and deserialize round-trip.
#[test]
fn rocket_launch_evidence_round_trip() {
    let evidence = RocketLaunchEvidence {
        run_key: "run-abc".into(),
        silo_unit_number: 7,
        platform_index: 3,
        payload: "space-platform-starter-pack".into(),
        launch_ordered_tick: Some(100),
        launched_tick: Some(500),
        platform_established_tick: Some(1200),
    };
    let json = serde_json::to_string(&evidence).expect("serialize");
    let decoded: RocketLaunchEvidence =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(evidence, decoded);
}

/// Partial evidence (only launch_ordered) round-trips correctly.
#[test]
fn partial_evidence_round_trip() {
    let evidence = RocketLaunchEvidence {
        run_key: "run-xyz".into(),
        silo_unit_number: 1,
        platform_index: 0,
        payload: "space-platform-starter-pack".into(),
        launch_ordered_tick: Some(2000),
        launched_tick: None,
        platform_established_tick: None,
    };
    let json = serde_json::to_string(&evidence).expect("serialize partial");
    let decoded: RocketLaunchEvidence =
        serde_json::from_str(&json).expect("deserialize partial");
    assert_eq!(evidence, decoded);
}
