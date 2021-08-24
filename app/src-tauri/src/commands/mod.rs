mod settings;
pub use self::settings::*;

mod instances;
pub use instances::*;

mod window;
pub use window::*;

mod io;
pub use io::*;

mod script;
pub use script::*;

mod rcon;
pub use rcon::*;

mod rest_api;
pub use rest_api::*;

const ERR_TO_STRING: fn(anyhow::Error) -> String = |e| String::from("error: ") + &e.to_string();
