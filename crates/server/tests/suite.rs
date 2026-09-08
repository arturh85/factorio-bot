//! Every integration test of this crate except the OpenAPI snapshot -- one
//! binary, not 14.
//!
//! Cargo compiles and links a separate binary for every file placed *directly*
//! in `tests/`, and each links the whole crate. `autotests = false` in
//! `Cargo.toml` switches that discovery off and this file is a `[[test]]`
//! target pulling them all in with `#[path]`.
//!
//! **The test files did not move**, deliberately, and for a reason this crate
//! does not itself show: `include_str!` resolves against its containing file,
//! and the sibling consolidations in `crates/core` and `crates/planner` carry
//! fixture paths that a subdirectory move would have rewritten. Keeping one
//! shape across all three crates is worth more than the tidier directory.
//!
//! **`tests/openapi.rs` is deliberately NOT here.** It stays its own target so
//! that `cargo test -p factorio-bot-server --features lua --test openapi`
//! keeps working -- the command that regenerates the snapshot, named in
//! `CLAUDE.md` and printed by the test's own failure message. Folding it in
//! would have broken both, silently, since a `--test` naming no target is an
//! error but a filter matching nothing is a green run of zero tests.
//!
//! **Adding a test file means adding a line here**, and that is the one cost
//! otherwise it is compiled by nothing and runs on no one. It does not fail:
//! it simply never runs, and a suite missing a file looks exactly like a
//! suite that passes. The same hazard, and the same remedy, is in
//! `crates/core/tests/{suite,botbridge}.rs`, `crates/planner/tests/suite.rs`
//! and `crates/server/tests/suite.rs`.
#[path = "bind.rs"]
mod bind;
#[path = "game_control.rs"]
mod game_control;
#[path = "game_query.rs"]
mod game_query;
#[path = "health.rs"]
mod health;
#[path = "manage_execute.rs"]
mod manage_execute;
#[path = "manage_fs.rs"]
mod manage_fs;
#[path = "manage_instance.rs"]
mod manage_instance;
#[path = "manage_rcon.rs"]
mod manage_rcon;
#[path = "manage_scripts.rs"]
mod manage_scripts;
#[path = "manage_settings.rs"]
mod manage_settings;
#[path = "manage_video.rs"]
mod manage_video;
#[path = "runs.rs"]
mod runs;
#[path = "shutdown.rs"]
mod shutdown;
#[path = "spa.rs"]
mod spa;
