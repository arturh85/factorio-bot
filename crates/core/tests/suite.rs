//! Every integration test of this crate that is not a BotBridge mod test --
//! one binary, not 24.
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
//! otherwise it is compiled by nothing and runs on no one. `tests/common.rs`
//! is in neither: it is empty, and has been since it was added.
//!
//! The other root is `tests/botbridge.rs`.
#[path = "a_chunk_knows_which_surface_it_is_on.rs"]
mod a_chunk_knows_which_surface_it_is_on;
#[path = "a_routed_surface_is_a_real_surface.rs"]
mod a_routed_surface_is_a_real_surface;
#[path = "blueprint_decode.rs"]
mod blueprint_decode;
#[path = "craft_is_awaited.rs"]
mod craft_is_awaited;
#[path = "enclosure_bounds.rs"]
mod enclosure_bounds;
#[path = "enclosure_run13.rs"]
mod enclosure_run13;
#[path = "enclosure_run73005.rs"]
mod enclosure_run73005;
#[path = "enclosure_two_row_smelter.rs"]
mod enclosure_two_row_smelter;
#[path = "ground_names_its_own_surface.rs"]
mod ground_names_its_own_surface;
#[path = "helper_boxes_match_the_game.rs"]
mod helper_boxes_match_the_game;
#[path = "live_2_1_payloads.rs"]
mod live_2_1_payloads;
#[path = "mining_drill_radius.rs"]
mod mining_drill_radius;
#[path = "output_parser_recovers_from_malformed_events.rs"]
mod output_parser_recovers_from_malformed_events;
#[path = "rcon_oversized_reply.rs"]
mod rcon_oversized_reply;
#[path = "rcontest_blueprints_decode.rs"]
mod rcontest_blueprints_decode;
#[path = "recovery_crosstalk_probe.rs"]
mod recovery_crosstalk_probe;
#[path = "research_is_awaited.rs"]
mod research_is_awaited;
#[path = "route_grid.rs"]
mod route_grid;
#[path = "surface_census_lands.rs"]
mod surface_census_lands;
#[path = "surface_id.rs"]
mod surface_id;
#[path = "the_rook_book_decodes.rs"]
mod the_rook_book_decodes;
#[path = "the_rook_decodes.rs"]
mod the_rook_decodes;
#[path = "the_stamp_clears_only_its_own_ground.rs"]
mod the_stamp_clears_only_its_own_ground;
#[path = "world_holds_surfaces.rs"]
mod world_holds_surfaces;
