mod config;
pub use self::config::*;

mod instances;
pub use instances::*;

mod test;
pub use test::*;

const ERR_TO_STRING: fn(anyhow::Error) -> String = |e| String::from("error: ") + &e.to_string();
