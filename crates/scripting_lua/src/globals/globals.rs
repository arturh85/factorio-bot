use factorio_bot_core::paris::{error, info, warn};
use factorio_bot_core::parking_lot::Mutex;
use factorio_bot_core::rlua;
use factorio_bot_core::rlua::{Context, Variadic};
use factorio_bot_core::types::PlayerId;
use itertools::Itertools;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

pub fn create_lua_globals(
    ctx: Context,
    all_bots: Vec<PlayerId>,
    cwd: PathBuf,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
) -> rlua::Result<()> {
    let map_table = ctx.globals();

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
        "__doc_fn_include",
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
        ctx.create_function(move |ctx, source_path: String| {
            let content = fs::read_to_string(_cwd.join(source_path).to_str().unwrap())
                .expect("file not found");
            let chunk = ctx.load(&content);
            chunk.exec().expect("failed to execute");
            Ok(())
        })?,
    )?;
    map_table.set(
        "__doc_fn_read_file",
        String::from(
            r#"
--- reads file to string
-- @string source_path
-- @return string contents of file
function globals.read_file(source_path)
end
"#,
        ),
    )?;
    let _cwd = cwd.clone();
    map_table.set(
        "read_file",
        ctx.create_function(move |_ctx, source_path: String| {
            let content = fs::read_to_string(_cwd.join(source_path).to_str().unwrap())
                .expect("file not found");
            Ok(content)
        })?,
    )?;
    map_table.set(
        "__doc_fn_write_file",
        String::from(
            r#"
--- writes string to file
-- @string source_path path to file
-- @string contents contenst of file
function globals.write_file(source_path, contents)
end
"#,
        ),
    )?;
    let _cwd = cwd;
    map_table.set(
        "write_file",
        ctx.create_function(move |_ctx, (source_path, contents): (String, String)| {
            let content = fs::write(_cwd.join(source_path).to_str().unwrap(), contents)
                .expect("failed to write");
            Ok(content)
        })?,
    )?;

    let _stdout = stdout.clone();
    map_table.set(
        "__doc_fn_print",
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
        ctx.create_function(move |_, strings: Variadic<String>| {
            info!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
            let mut stdout = _stdout.lock();
            *stdout += &strings.iter().join(" ");
            *stdout += "\n";
            Ok(())
        })?,
    )?;
    let _stderr = stderr;
    map_table.set(
        "__doc_fn_print_err",
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
        ctx.create_function(move |_, strings: Variadic<String>| {
            error!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
            let mut stderr = _stderr.lock();
            *stderr += "ERROR: ";
            *stderr += &strings.iter().join(" ");
            *stderr += "\n";
            Ok(())
        })?,
    )?;
    let _stdout = stdout;
    map_table.set(
        "__doc_fn_print_warn",
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
        ctx.create_function(move |_, strings: Variadic<String>| {
            warn!("<cyan>lua</>   ⮞ {}", strings.iter().join(" "));
            let mut stdout = _stdout.lock();
            *stdout += "WARN: ";
            *stdout += &strings.iter().join(" ");
            *stdout += "\n";
            Ok(())
        })?,
    )?;

    map_table.set(
        "__doc_fn_all_bots",
        String::from(
            r#"
--- List of Player ids for available bots
globals.all_bots = {}
"#,
        ),
    )?;
    map_table.set("all_bots", all_bots)?;

    Ok(())
}
