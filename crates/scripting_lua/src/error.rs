// False positive warnings from thiserror/miette derive macros using struct fields in format strings
#![allow(unused_assignments)]

use factorio_bot_core::mlua;
use factorio_bot_core::regex::Regex;
use factorio_bot_core::thiserror::Error;
use factorio_bot_scripting::line_offset;
use miette::{Diagnostic, NamedSource, SourceSpan, miette};
use std::collections::HashMap;

/// Turns an mlua error into a report, attaching the offending source line when
/// it can be located.
///
/// Everything parsed out of `err`'s message is attacker-controlled: `error()`
/// takes an arbitrary string and `load()` lets a script name its own chunk, so
/// a script can put any `[string "name"]:line:` prefix it likes into the text.
/// This crate is built with `panic = "abort"`, so a panic here is a remote kill
/// of the whole process rather than a failed script -- which is what
/// `code_by_path.get(filename).unwrap()` used to be, reachable from the
/// one-liner `load("error('x')", "evil")()`. Anything that does not parse or
/// does not resolve now degrades to the plain message.
pub fn to_lua_error(err: mlua::Error, code_by_path: &HashMap<String, String>) -> miette::Report {
    let message = err.to_string() + "\n";
    located_lua_error(&message, code_by_path).unwrap_or_else(|| miette!("{}", err))
}

/// `None` whenever the message does not name a chunk this run actually loaded,
/// or names a line that chunk does not have.
fn located_lua_error(
    message: &str,
    code_by_path: &HashMap<String, String>,
) -> Option<miette::Report> {
    let short = message.lines().next()?;
    let pattern = Regex::new("\\[string \"(.*?)\"]:(\\d+):").ok()?;
    let matches = pattern.captures(message)?;
    let filename = matches.get(1)?.as_str();
    // A crafted message can carry a line number that overflows `usize`, so this
    // parse is fallible in practice and not merely in principle.
    let line: usize = matches.get(2)?.as_str().parse().ok()?;
    let code = code_by_path.get(filename)?;

    let offset = line_offset(code, line)?;
    // Past the last line there is no following offset; the label then runs to
    // the end of the source. `saturating_sub` because `line_offset` is not
    // guaranteed to be monotonic for a caller-supplied line number.
    let offset_next = line_offset(code, line + 1).unwrap_or(code.len());
    // We only have a line number, so the label marks the whole line.
    let bad_bit: SourceSpan = (offset, offset_next.saturating_sub(offset)).into();

    Some(
        LuaError {
            short: short.to_owned(),
            message: message.to_owned(),
            src: NamedSource::new(filename, code.to_owned()),
            bad_bit,
        }
        .into(),
    )
}

#[derive(Error, Debug, Diagnostic)]
#[error("{message}")]
pub struct LuaError {
    pub message: String,
    pub short: String,
    #[source_code]
    pub src: NamedSource<String>,
    #[label("{short}")]
    pub bad_bit: SourceSpan,
}
