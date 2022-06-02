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

mod restapi;
pub use restapi::*;

const ERR_TO_STRING: fn(miette::Report) -> String =
  |e| String::from("error: ") + &*format!("{:?}", e);
