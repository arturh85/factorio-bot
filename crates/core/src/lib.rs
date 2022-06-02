extern crate miette;
#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate enum_primitive_derive;
extern crate num_traits;
#[macro_use]
extern crate paris;
#[macro_use]
extern crate serde_derive;
// #[macro_use]
extern crate serde_json;
extern crate strum;
#[macro_use]
extern crate strum_macros;

#[allow(unused_imports)]
#[macro_use]
extern crate include_dir;
#[macro_use]
extern crate schemars;

pub mod constants;
pub mod draw;
pub mod errors;
pub mod factorio;
pub mod graph;
pub mod plan;
pub mod process;
pub mod settings;
// #[cfg(test)] not possible because lua crate needs this
pub mod test_utils;
pub mod types;
