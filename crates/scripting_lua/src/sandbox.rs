use factorio_bot_core::mlua::prelude::*;

/// The base-library functions that take a *path* and load code from it.
///
/// mlua opens the base library unconditionally (`luaopen_base` in
/// `state/raw.rs`), so unlike `io` and `os` these cannot be excluded by not
/// asking for them and have to be removed by hand.
const BASE_FILE_LOADERS: [&str; 2] = ["dofile", "loadfile"];

/// Creates the interpreter that user scripts run in.
///
/// [`Lua::new`] loads `StdLib::ALL_SAFE`, where "safe" means *memory* safe: it
/// still contains `io`, `os` and `package`. That is `io.open` writing anywhere
/// the process can, `os.execute` running arbitrary commands, and
/// `package.cpath` loading arbitrary native libraries -- which made bounding
/// `file_write`, `file_read`, `include` and `world.draw` pointless, because the
/// four doors stood beside an open wall. Scripts reach this from an
/// unauthenticated HTTP API, so the standard library is an allow-list.
///
/// `table`, `string`, `math` and `coroutine` are pure computation and stay.
/// `load` stays too, but only after [`install_text_only_load`] takes its teeth
/// out -- see there for why the obvious reason to keep it was wrong.
pub(crate) fn new_sandboxed_lua() -> LuaResult<Lua> {
    // The one permitted call. `clippy.toml` bans `Lua::new`/`Lua::new_with`
    // workspace-wide so a second construction site cannot be added without
    // tripping the build -- an unsandboxed interpreter looks like ordinary
    // code, and this is the only place that is supposed to build one.
    #[allow(clippy::disallowed_methods)]
    let lua = Lua::new_with(
        LuaStdLib::TABLE | LuaStdLib::STRING | LuaStdLib::MATH | LuaStdLib::COROUTINE,
        LuaOptions::default(),
    )?;
    let globals = lua.globals();
    for loader in BASE_FILE_LOADERS {
        globals.raw_remove(loader)?;
    }
    drop(globals);
    install_text_only_load(&lua)?;
    Ok(lua)
}

/// Replaces the base-library `load` with one that compiles source text only.
///
/// The rationale for keeping `load` used to be that "it takes a string rather
/// than a path, so it compiles nothing a script could not have written
/// inline". That is true of *text* chunks and false in general: Lua's `load`
/// defaults to mode `"bt"`, so it also accepts **binary** chunks. This build is
/// PUC-Rio Lua 5.4 (the `lua54` feature), whose bytecode loader does not
/// validate untrusted input — crafted bytecode is a documented route to type
/// confusion and arbitrary memory access, which walks straight past the
/// standard-library lockdown above. `load(string.dump(f, true))` round-tripped
/// to a working function before this.
///
/// Removing `string.dump` would fix nothing: a script can spell bytecode out
/// as a string literal with escape sequences. The *mode* is the vulnerability,
/// so the mode is what this pins. [`LuaChunkMode::Text`] reaches Lua's own
/// loader as mode `"t"` (`mlua`'s `state/raw.rs`), which is what rejects the
/// binary signature — the check is Lua's, not ours.
///
/// The signature follows `load(chunk [, chunkname [, mode [, env]]])`,
/// including the reader-function form and the `env` argument, because a
/// partial reimplementation that silently ignored either would trade a
/// security bug for a correctness one. Compile failures return `nil` plus a
/// message rather than raising, as the real `load` does — scripts test the
/// second return value.
fn install_text_only_load(lua: &Lua) -> LuaResult<()> {
    let load = lua.create_function(
        |lua,
         (chunk, chunkname, mode, env): (
            LuaValue,
            Option<LuaString>,
            Option<LuaString>,
            Option<LuaTable>,
        )| {
            // Lua defaults `chunkname` to the source itself for a string
            // chunk, and to "=(load)" for a reader function.
            let (source, default_name) = match &chunk {
                LuaValue::String(source) => {
                    let bytes = source.as_bytes().to_vec();
                    let name = String::from_utf8_lossy(&bytes).into_owned();
                    (bytes, name)
                }
                LuaValue::Function(reader) => (read_chunk(reader)?, "=(load)".to_owned()),
                other => {
                    return Err(LuaError::RuntimeError(format!(
                        "bad argument #1 to 'load' (string expected, got {})",
                        other.type_name()
                    )));
                }
            };

            // An explicit mode that excludes text is a request for exactly the
            // thing this function exists to refuse. Say so, rather than
            // quietly compiling it as text and confusing the caller.
            if let Some(mode) = &mode
                && !mode.as_bytes().contains(&b't')
            {
                return Ok((
                    None,
                    Some("attempt to load a binary chunk (binary chunks are disabled)".into()),
                ));
            }

            let name = chunkname
                .map(|name| String::from_utf8_lossy(&name.as_bytes()).into_owned())
                .unwrap_or(default_name);
            let mut compiled = lua.load(source).set_mode(LuaChunkMode::Text).set_name(name);
            if let Some(env) = env {
                compiled = compiled.set_environment(env);
            }
            match compiled.into_function() {
                Ok(function) => Ok((Some(function), None)),
                Err(err) => Ok((None, Some(err.to_string()))),
            }
        },
    )?;
    lua.globals().set("load", load)
}

/// Drains `load`'s reader-function form: call it until it returns nothing or
/// an empty string, concatenating the pieces, exactly as Lua does.
fn read_chunk(reader: &LuaFunction) -> LuaResult<Vec<u8>> {
    let mut source = Vec::new();
    while let Some(piece) = reader.call::<Option<LuaString>>(())? {
        let bytes = piece.as_bytes();
        if bytes.is_empty() {
            break;
        }
        source.extend_from_slice(&bytes);
    }
    Ok(source)
}
