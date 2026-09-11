//! Every test of the BotBridge Lua mod -- one binary, not 23.
//!
//! Cargo compiles and links a separate binary for every file placed *directly*
//! in `tests/`, and each links the whole crate. 48 files did that here.
//! `autotests = false` in `Cargo.toml` switches that discovery off; this file
//! is one of two `[[test]]` targets that pull them all in with `#[path]`.
//!
//! **The test files did not move**, deliberately: many resolve fixture JSON
//! and blueprint text through `include_str!`, which resolves against the
//! containing file, so a subdirectory move would have rewritten those paths in
//! a change whose whole safety argument is that it edits no test.
//!
//! **Adding a test file means adding a line to one of the two roots**,
//! otherwise it is compiled by nothing and runs on no one. It does not fail:
//! it simply never runs, and a suite missing a file looks exactly like a
//! suite that passes. The same hazard, and the same remedy, is in
//! `crates/core/tests/{suite,botbridge}.rs`, `crates/planner/tests/suite.rs`
//! and `crates/server/tests/suite.rs`.
//!
//! `tests/common.rs` is in neither root: it is empty, and always has been.
//!
//! The other root is `tests/suite.rs`.
#[path = "botbridge_bot_polling.rs"]
mod botbridge_bot_polling;
#[path = "botbridge_bulk_entities.rs"]
mod botbridge_bulk_entities;
#[path = "botbridge_character_spawn.rs"]
mod botbridge_character_spawn;
#[path = "botbridge_craft_action.rs"]
mod botbridge_craft_action;
#[path = "botbridge_crash_site_cutscene.rs"]
mod botbridge_crash_site_cutscene;
#[path = "botbridge_dead_is_not_disconnected.rs"]
mod botbridge_dead_is_not_disconnected;
#[path = "botbridge_generate_chunks.rs"]
mod botbridge_generate_chunks;
#[path = "botbridge_machine_counters.rs"]
mod botbridge_machine_counters;
#[path = "botbridge_pathfinder_cache.rs"]
mod botbridge_pathfinder_cache;
#[path = "botbridge_placement_material.rs"]
mod botbridge_placement_material;
#[path = "botbridge_pre_tick_handlers.rs"]
mod botbridge_pre_tick_handlers;
#[path = "botbridge_ready_beacon.rs"]
mod botbridge_ready_beacon;
#[path = "botbridge_research_action.rs"]
mod botbridge_research_action;
#[path = "botbridge_research_triggers.rs"]
mod botbridge_research_triggers;
#[path = "botbridge_rest_position.rs"]
mod botbridge_rest_position;
#[path = "botbridge_rocket_launch.rs"]
mod botbridge_rocket_launch;
#[path = "botbridge_returns_fire.rs"]
mod botbridge_returns_fire;
#[path = "botbridge_sampling_session.rs"]
mod botbridge_sampling_session;
#[path = "botbridge_serialisers.rs"]
mod botbridge_serialisers;
#[path = "botbridge_set_recipe.rs"]
mod botbridge_set_recipe;
#[path = "botbridge_surface_guard.rs"]
mod botbridge_surface_guard;
#[path = "botbridge_walk_stuck.rs"]
mod botbridge_walk_stuck;
#[path = "botbridge_writeout_forces.rs"]
mod botbridge_writeout_forces;
#[path = "botbridge_writeout_tiles.rs"]
mod botbridge_writeout_tiles;
