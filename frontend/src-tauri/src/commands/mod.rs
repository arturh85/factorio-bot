mod settings;
pub use self::settings::*;

mod instances;
pub use instances::*;

mod window;
pub use window::*;

mod test;
pub use test::*;

mod script;
pub use script::*;

mod rcon;
pub use rcon::*;

const ERR_TO_STRING: fn(anyhow::Error) -> String = |e| String::from("error: ") + &e.to_string();
