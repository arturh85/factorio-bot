use factorio_bot_core::num_traits::{FromPrimitive, ToPrimitive};
use factorio_bot_core::paris::{error, info, warn};
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::mlua::Variadic as LuaVariadic;
use factorio_bot_core::types::{Direction, PlayerId};
use itertools::Itertools;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

pub fn create_lua_globals(
    lua: &Lua,
    all_bots: Vec<PlayerId>,
    cwd: PathBuf,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
    code_by_path: Arc<Mutex<HashMap<String, String>>>,
) -> LuaResult<()> {
    let map_table = lua.globals();

    map_table.set(
        "__doc__header",
        String::from(
            r#"
--- Global functions
--
-- @module globals

local globals = {}
    "#,
        ),
    )?;
    map_table.set("__doc__footer", String::from(r#"return globals"#))?;

    map_table.set(
        "__doc_entry_include",
        String::from(
            r#"
--- include lua code files
-- @string source_path
function globals.include(source_path)
end
"#,
        ),
    )?;
    let _cwd = cwd.clone();
    map_table.set(
        "include",
        lua.create_function(move |lua, source_path: String| {
            let content = fs::read_to_string(_cwd.join(&source_path).to_str().unwrap())
                .expect("file not found");
            let mut code_by_path_lock = code_by_path.lock();
            code_by_path_lock.insert(source_path.to_owned(), content.clone());
            drop(code_by_path_lock);
            let chunk = lua.load(&content).set_name(&source_path);
            chunk.exec().expect("failed to execute");
            Ok(())
        })?,
    )?;
    map_table.set(
        "__doc_entry_file_read",
        String::from(
            r#"
--- reads file to string
-- @string source_path
-- @return string contents of file
function globals.file_read(source_path)
end
"#,
        ),
    )?;
    let _cwd = cwd.clone();
    map_table.set(
        "file_read",
        lua.create_function(move |_lua, source_path: String| {
            let content = fs::read_to_string(_cwd.join(source_path).to_str().unwrap())
                .expect("file not found");
            Ok(content)
        })?,
    )?;
    map_table.set(
        "__doc_entry_file_write",
        String::from(
            r#"
--- writes string to file
-- @string target_path path to file
-- @string contents contenst of file
function globals.file_write(target_path, contents)
end
"#,
        ),
    )?;
    let _cwd = cwd;
    map_table.set(
        "file_write",
        lua.create_function(move |_lua, (target_path, contents): (String, String)| {
            fs::write(_cwd.join(target_path).to_str().unwrap(), contents).expect("failed to write");
            Ok(())
        })?,
    )?;

    let _stdout = stdout.clone();
    map_table.set(
        "__doc_entry_print",
        String::from(
            r#"
--- Print given strings on stdout
-- @string message
function globals.print(...)
end
"#,
        ),
    )?;
    map_table.set(
        "print",
        lua.create_function(move |_, strings: LuaVariadic<String>| {
            info!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
            let mut stdout_lock = _stdout.lock();
            *stdout_lock += &strings.iter().join(" ");
            *stdout_lock += "\n";
            Ok(())
        })?,
    )?;
    let _stderr = stderr;
    map_table.set(
        "__doc_entry_print_err",
        String::from(
            r#"
--- Print given strings on stderr with ERROR: prefix
-- @string message
function globals.print_err(...)
end
"#,
        ),
    )?;
    map_table.set(
        "print_err",
        lua.create_function(move |_, strings: LuaVariadic<String>| {
            error!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
            let mut stderr_lock = _stderr.lock();
            *stderr_lock += "ERROR: ";
            *stderr_lock += &strings.iter().join(" ");
            *stderr_lock += "\n";
            Ok(())
        })?,
    )?;
    let _stdout = stdout;
    map_table.set(
        "__doc_entry_print_warn",
        String::from(
            r#"
--- Print given strings on stdout with WARN: prefix
-- @string message
function globals.print_warn(...)
end
"#,
        ),
    )?;
    map_table.set(
        "print_warn",
        lua.create_function(move |_, strings: LuaVariadic<String>| {
            warn!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
            let mut stdout_lock = _stdout.lock();
            *stdout_lock += "WARN: ";
            *stdout_lock += &strings.iter().join(" ");
            *stdout_lock += "\n";
            Ok(())
        })?,
    )?;

    map_table.set(
        "__doc_entry_all_bots",
        String::from(
            r#"
--- List of Player ids for available bots
globals.all_bots = nil -- {number}
"#,
        ),
    )?;
    map_table.set("all_bots", all_bots)?;

    map_table.set(
        "__doc_entry_Direction",
        String::from(
            r#"
--- Available Directions
globals.Direction = {
    North = 0, -- North = 0 
    NorthEast = 1, -- NorthEast = 1
    East = 2,  -- East = 2
    SouthEast = 3, -- SouthEast = 3
    South = 4, -- South = 4
    SouthWest = 5, -- SouthWest = 5
    West = 6, -- West = 6
    NorthWest = 7, -- NorthWest = 7
}
"#,
        ),
    )?;
    let direction = lua.create_table()?;
    direction.set("North", 0)?;
    direction.set("NorthEast", 1)?;
    direction.set("East", 2)?;
    direction.set("SouthEast", 3)?;
    direction.set("South", 4)?;
    direction.set("SouthWest", 5)?;
    direction.set("West", 6)?;
    direction.set("NorthWest", 7)?;
    map_table.set("Direction", direction)?;

    map_table.set(
        "__doc_entry_directions_all",
        String::from(
            r#"
--- Return all 8 available directions as list table
-- @return {number,...}
function globals.directions_all()
end
"#,
        ),
    )?;
    map_table.set(
        "directions_all",
        lua.create_function(move |_, ()| {
            Ok(Direction::all()
                .iter()
                .map(|d| d.to_u8().unwrap())
                .collect::<Vec<u8>>())
        })?,
    )?;

    map_table.set(
        "__doc_entry_directions_orthogonal",
        String::from(
            r#"
--- Return 4 orthogonal directions as list table
-- @return {number,...}
function globals.directions_orthogonal()
end
"#,
        ),
    )?;
    map_table.set(
        "directions_orthogonal",
        lua.create_function(move |_, ()| {
            Ok(Direction::orthogonal()
                .iter()
                .map(|d| d.to_u8().unwrap())
                .collect::<Vec<u8>>())
        })?,
    )?;

    map_table.set(
        "__doc_entry_direction_clockwise",
        String::from(
            r#"
--- Turn direction clockwise
-- @number direction start `Direction`
-- @return number
function globals.direction_clockwise(direction)
end
"#,
        ),
    )?;
    map_table.set(
        "direction_clockwise",
        lua.create_function(move |_, direction: u8| {
            let direction = Direction::from_u8(direction).unwrap();
            Ok(direction.clockwise().to_u8().unwrap())
        })?,
    )?;

    map_table.set(
        "__doc_entry_direction_opposite",
        String::from(
            r#"
--- Turn direction 180° to opposite side
-- @number direction start `Direction`
-- @return number
function globals.direction_opposite(direction)
end
"#,
        ),
    )?;
    map_table.set(
        "direction_opposite",
        lua.create_function(move |_, direction: u8| {
            let direction = Direction::from_u8(direction).unwrap();
            Ok(direction.opposite().to_u8().unwrap())
        })?,
    )?;

    Ok(())
}
