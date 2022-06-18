use factorio_bot_core::regex::Regex;
use factorio_bot_core::rlua;
use factorio_bot_core::thiserror::Error;
use factorio_bot_scripting::line_offset;
use miette::{miette, Diagnostic, NamedSource, SourceSpan};

pub fn to_lua_error(err: rlua::Error, filename: &str, code: &str) -> miette::Report {
    let message = err.to_string();
    let r = Regex::new(":(\\d+):").expect("failed to compile regex");
    if let Some(matches) = r.captures(&message) {
        let line: usize = matches
            .get(1)
            .expect("missing capture")
            .as_str()
            .parse()
            .expect("failed to parse");
        let src = NamedSource::new(filename, code.to_owned());
        let offset = line_offset(code, line).expect("line not found");
        let offset_next = line_offset(code, line + 1).unwrap_or(code.len() - 1);
        let default_span = (offset, offset_next - offset).into(); // we only have a line number so mark the hole line
        let err = LuaError {
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
#[error("{message:?}")]
pub struct LuaError {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("{message:?}")]
    pub bad_bit: SourceSpan,
}
