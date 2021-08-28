#[macro_use]
extern crate anyhow;
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

pub mod draw;
pub mod factorio;
pub mod settings;
pub mod types;
