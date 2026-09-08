//! One test binary for every integration test in this crate.
//!
//! Cargo compiles and links a separate binary for each file placed *directly*
//! in `tests/`, and each one links the whole crate: 34 files cost ~1.4 s of
//! link apiece, ~48 s of every full verification run. `autotests = false` in
//! `Cargo.toml` switches that discovery off and this file is the single
//! `[[test]]` target, so all 34 link once.
//!
//! **The test files did not move**, deliberately. They stay directly in
//! `tests/` and are pulled in by `#[path]` below, which keeps every
//! `include_str!` in them resolving against the same directory it always did
//! (several reach `../../core/tests/*.json`). Moving them into a subdirectory
//! would have rewritten those paths in a change whose whole safety argument is
//! that it edits no test.
//!
//! **Adding a test file means adding a line here**, otherwise it is compiled
//! by nothing and runs on no one -- the one cost of this arrangement.
#[path = "common/mod.rs"]
mod common;

#[path = "benched_bot.rs"]
mod benched_bot;
#[path = "buffers.rs"]
mod buffers;
#[path = "cell_ground.rs"]
mod cell_ground;
#[path = "drill_over_mined_tile.rs"]
mod drill_over_mined_tile;
#[path = "enclosure_prevention.rs"]
mod enclosure_prevention;
#[path = "fluid_have.rs"]
mod fluid_have;
#[path = "furnace_bank.rs"]
mod furnace_bank;
#[path = "furnace_ground.rs"]
mod furnace_ground;
#[path = "furnace_reuse.rs"]
mod furnace_reuse;
#[path = "goal_names_its_recipe.rs"]
mod goal_names_its_recipe;
#[path = "machine_named_by_category.rs"]
mod machine_named_by_category;
#[path = "oil_category_gate.rs"]
mod oil_category_gate;
#[path = "ore_underfoot.rs"]
mod ore_underfoot;
#[path = "placement_occupancy.rs"]
mod placement_occupancy;
#[path = "planning_work_ceilings.rs"]
mod planning_work_ceilings;
#[path = "product_index_live_capture.rs"]
mod product_index_live_capture;
#[path = "recipe_probability.rs"]
mod recipe_probability;
#[path = "red_science_cell.rs"]
mod red_science_cell;
#[path = "red_science.rs"]
mod red_science;
#[path = "refusal_expiry.rs"]
mod refusal_expiry;
#[path = "refusal_memory.rs"]
mod refusal_memory;
#[path = "roster_shares.rs"]
mod roster_shares;
#[path = "scheduling.rs"]
mod scheduling;
#[path = "seeded_roster.rs"]
mod seeded_roster;
#[path = "smelt_roots.rs"]
mod smelt_roots;
#[path = "split_capacity.rs"]
mod split_capacity;
#[path = "standing_goals.rs"]
mod standing_goals;
#[path = "standing_site_reuse.rs"]
mod standing_site_reuse;
#[path = "substance_live_capture.rs"]
mod substance_live_capture;
#[path = "tile_capacity.rs"]
mod tile_capacity;
#[path = "tile_occupancy.rs"]
mod tile_occupancy;
#[path = "tile_reservation.rs"]
mod tile_reservation;
#[path = "unreachable_memory.rs"]
mod unreachable_memory;
#[path = "world_round_trip.rs"]
mod world_round_trip;
