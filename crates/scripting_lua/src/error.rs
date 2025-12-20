use factorio_bot_core::regex::Regex;
use factorio_bot_core::mlua;
use factorio_bot_core::thiserror::Error;
use factorio_bot_scripting::line_offset;
use miette::{miette, Diagnostic, NamedSource, SourceSpan};
use std::collections::HashMap;

pub fn to_lua_error(err: mlua::Error, code_by_path: &HashMap<String, String>) -> miette::Report {
    let message = err.to_string() + "\n";
    let short = message.lines().next().unwrap();
    let r = Regex::new("\\[string \"(.*?)\"]:(\\d+):").expect("failed to compile regex");
    if let Some(matches) = r.captures(&message) {
        let line: usize = matches
            .get(2)
            .expect("missing capture")
            .as_str()
            .parse()
            .expect("failed to parse");
        let filename = matches.get(1).unwrap().as_str();
        let code = code_by_path.get(filename).unwrap();
        let src = NamedSource::new(filename, code.to_owned());
        let offset = line_offset(code, line).expect("line not found");
        let offset_next = line_offset(code, line + 1).unwrap_or(code.len() - 1);
        let default_span = (offset, offset_next - offset).into(); // we only have a line number so mark the hole line
        let err = LuaError {
            short: short.to_owned(),
            message,
            src,
            bad_bit: default_span,
        };

        err.into()
    } else {
        miette!(err)
    }
    // fn position_to_span(pos: Position, code: &str) -> Option<SourceSpan> {
    //     let line_offset = line_offset(code, pos.line()?)?;
    //     let span = (line_offset + pos.position()? - 1, 0).into();
    //     Some(span)
    // }
}

#[derive(Error, Debug, Diagnostic)]
#[error("{message}")]
pub struct LuaError {
    pub message: String,
    pub short: String,
    #[source_code]
    pub src: NamedSource,
    #[label("{short}")]
    pub bad_bit: SourceSpan,
}
