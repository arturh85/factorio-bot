pub extern crate async_trait;
#[macro_use]
extern crate enum_primitive_derive;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate strum;

#[allow(unused_imports)]
#[macro_use]
extern crate include_dir;
#[macro_use]
pub extern crate schemars;
// DO NOT add `#[macro_use]` for `tracing` alongside this.
//
// This attribute makes paris' `info!`/`warn!`/`error!` resolve anywhere in the
// crate with no import, so 148 call sites name no logging system at all. While
// both systems are present that is not a convenience, it is a way to be wrong
// invisibly: swapping one import in `entity_graph.rs` produced a file with
// `error!` on tracing and `warn!` still on paris, compiling cleanly, with
// nothing at either call site to show it.
//
// `tracing` is imported explicitly at every site precisely so a converted line
// says which system it uses. Granting it the same global would erase the only
// signal that survives the transition.
#[macro_use]
pub extern crate paris;

pub use dashmap;
pub use factorio_blueprint;
pub use miette;
pub use mlua;
pub use num_traits;
pub use parking_lot;
pub use petgraph;
pub use rand;
pub use regex;
// pub use rlua;
// pub use rlua_serde;
pub use serde;
pub use serde_json;
pub use thiserror;
pub use tokio;
pub use tracing;

pub mod aabb_quadtree;
pub mod app_settings;
pub mod blueprint;
pub mod constants;
pub mod draw;
pub mod errors;
pub mod factorio;
pub mod graph;
pub mod paths;
pub mod plan;
pub mod process;
pub mod record;
pub mod scripts;
pub mod settings;

pub mod test_utils; // #[cfg(test)] not possible because lua crate needs this
pub mod types;
// pub mod windows;
