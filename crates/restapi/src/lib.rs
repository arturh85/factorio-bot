pub mod error;
pub mod rest_api;
pub mod webserver;

extern crate miette;
#[macro_use]
extern crate rocket;
#[macro_use]
extern crate rocket_okapi;
#[macro_use]
extern crate serde_derive;
