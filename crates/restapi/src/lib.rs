pub mod error;
pub mod restapi;
pub mod settings;
pub mod webserver;

extern crate miette;
#[macro_use]
extern crate rocket;
#[macro_use]
extern crate rocket_okapi;
#[macro_use]
extern crate serde;
