use factorio_bot_scripting::line_offset;
use miette::{Diagnostic, NamedSource, SourceSpan};
use rhai::{EvalAltResult, Position};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Box<EvalAltResult>>;

pub fn to_rhai_error(err: miette::Report) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorSystem(err.to_string(), err.into()))
}

#[derive(Error, Debug, Diagnostic)]
#[error("Function not found: {message:?}")]
#[diagnostic(help("fix spelling"))]
struct ErrorFunctionNotFound {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Function not found: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("System Error: {message:?}")]
struct ErrorSystem {
    pub message: String,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Parsing: {message:?}")]
#[diagnostic(help("fix syntax"))]
struct ErrorParsing {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Parsing: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Variable Exists: {message:?}")]
#[diagnostic(help("use other name"))]
struct ErrorVariableExists {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Variable Exists: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Forbidden Variable: {message:?}")]
#[diagnostic(help("use other name"))]
struct ErrorForbiddenVariable {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Forbidden Variable: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Variable not found: {message:?}")]
#[diagnostic(help("use correct name"))]
struct ErrorVariableNotFound {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Variable not found: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Property not found: {message:?}")]
#[diagnostic(help("use correct name"))]
struct ErrorPropertyNotFound {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Property not found: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Module not found: {message:?}")]
#[diagnostic(help("use correct name"))]
struct ErrorModuleNotFound {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Module not found: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Error in function call: {message:?} {other:?}")]
struct ErrorInFunctionCall {
    pub message: String,
    pub other: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Error in function call: {other:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Error in module: {message:?}")]
struct ErrorInModule {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Error in module: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Unbound this")]
struct ErrorUnboundThis {
    #[source_code]
    pub src: NamedSource,
    #[label("Unbound this")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Mismatch data type: {requested:?} != {actual:?}")]
#[diagnostic(help("use correct type"))]
struct ErrorMismatchDataType {
    pub requested: String,
    pub actual: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Mismatch data type: {requested:?} != {actual:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Mismatch output type: {requested:?} != {actual:?}")]
#[diagnostic(help("use correct type"))]
struct ErrorMismatchOutputType {
    pub actual: String,
    pub requested: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Mismatch output type: {requested:?} != {actual:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Indexing type: {message:?}")]
#[diagnostic(help("use correct type"))]
struct ErrorIndexingType {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Indexing type: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Array bounds: {array_size} / {access_index}")]
#[diagnostic(help("use correct type"))]
struct ErrorArrayBounds {
    pub array_size: usize,
    pub access_index: i64,
    #[source_code]
    pub src: NamedSource,
    #[label("Array bounds: {array_size} / {access_index}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("String bounds: {string_size} / {access_index}")]
#[diagnostic(help("use correct type"))]
struct ErrorStringBounds {
    pub string_size: usize,
    pub access_index: i64,
    #[source_code]
    pub src: NamedSource,
    #[label("String bounds: {string_size} / {access_index}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("BitField bounds: {array_size} / {access_index}")]
#[diagnostic(help("use correct type"))]
struct ErrorBitFieldBounds {
    pub array_size: usize,
    pub access_index: i64,
    #[source_code]
    pub src: NamedSource,
    #[label("BitField bounds: {array_size} / {access_index}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Error for")]
#[diagnostic(help("use correct type"))]
struct ErrorFor {
    #[source_code]
    pub src: NamedSource,
    #[label("Error for")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Data race: {message:?}")]
#[diagnostic(help("use correct type"))]
struct ErrorDataRace {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("Data race: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("AssignmentToConstant: {message:?}")]
#[diagnostic(help("use correct type"))]
struct ErrorAssignmentToConstant {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("AssignmentToConstant: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("DotExpr: {message:?}")]
#[diagnostic(help("use correct type"))]
struct ErrorDotExpr {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("DotExpr: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("ErrorArithmetic: {message:?}")]
struct ErrorArithmetic {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("ErrorArithmetic: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Too many operations")]
#[diagnostic(help("use fewer operations"))]
struct ErrorTooManyOperations {
    #[source_code]
    pub src: NamedSource,
    #[label("Too many operations")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Too many modules")]
#[diagnostic(help("use fewer modules"))]
struct ErrorTooManyModules {
    #[source_code]
    pub src: NamedSource,
    #[label("Too many modules")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Stack overflow")]
#[diagnostic(help("use fewer modules"))]
struct ErrorStackOverflow {
    #[source_code]
    pub src: NamedSource,
    #[label("Stack overflow")]
    pub bad_bit: SourceSpan,
}
#[derive(Error, Debug, Diagnostic)]
#[error("ErrorDataTooLarge: {message:?}")]
struct ErrorDataTooLarge {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("ErrorDataTooLarge: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("ErrorTerminated")]
struct ErrorTerminated {
    #[source_code]
    pub src: NamedSource,
    #[label("ErrorTerminated")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("ErrorCustomSyntax: {message:?}")]
struct ErrorCustomSyntax {
    pub message: String,
    #[source_code]
    pub src: NamedSource,
    #[label("ErrorCustomSyntax: {message:?}")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("ErrorRuntime")]
struct ErrorRuntime {
    #[source_code]
    pub src: NamedSource,
    #[label("ErrorRuntime")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("LoopBreak")]
struct LoopBreak {
    #[source_code]
    pub src: NamedSource,
    #[label("LoopBreak")]
    pub bad_bit: SourceSpan,
}

#[derive(Error, Debug, Diagnostic)]
#[error("Return")]
struct Return {
    #[source_code]
    pub src: NamedSource,
    #[label("Return")]
    pub bad_bit: SourceSpan,
}

pub fn handle_rhai_err(
    err: EvalAltResult,
    rhai_code: &str,
    filename: Option<&str>,
) -> miette::Result<()> {
    let src = NamedSource::new(filename.unwrap_or("unknown"), rhai_code.to_string());
    let default_span = (0, 0).into();
    match err {
        EvalAltResult::ErrorFunctionNotFound(message, pos) => Err(ErrorFunctionNotFound {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorSystem(message, _err) => Err(ErrorSystem { message })?,
        EvalAltResult::ErrorParsing(error, pos) => Err(ErrorParsing {
            src,
            message: format!("{:?}", error),
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorVariableExists(message, pos) => Err(ErrorVariableExists {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorForbiddenVariable(message, pos) => Err(ErrorForbiddenVariable {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorVariableNotFound(message, pos) => Err(ErrorVariableNotFound {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorPropertyNotFound(message, pos) => Err(ErrorPropertyNotFound {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorModuleNotFound(message, pos) => Err(ErrorModuleNotFound {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorInFunctionCall(message, other, _, pos) => Err(ErrorInFunctionCall {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
            other,
        })?,
        EvalAltResult::ErrorInModule(message, _, pos) => Err(ErrorInModule {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorUnboundThis(pos) => Err(ErrorUnboundThis {
            src,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorMismatchDataType(requested, actual, pos) => {
            Err(ErrorMismatchDataType {
                src,
                requested,
                actual,
                bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
            })?
        }
        EvalAltResult::ErrorMismatchOutputType(requested, actual, pos) => {
            Err(ErrorMismatchOutputType {
                src,
                requested,
                actual,
                bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
            })?
        }
        EvalAltResult::ErrorIndexingType(message, pos) => Err(ErrorIndexingType {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorArrayBounds(array_size, access_index, pos) => Err(ErrorArrayBounds {
            array_size,
            access_index,
            src,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorStringBounds(string_size, access_index, pos) => {
            Err(ErrorStringBounds {
                string_size,
                access_index,
                src,
                bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
            })?
        }
        EvalAltResult::ErrorBitFieldBounds(array_size, access_index, pos) => {
            Err(ErrorBitFieldBounds {
                array_size,
                access_index,
                src,
                bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
            })?
        }
        EvalAltResult::ErrorFor(pos) => Err(ErrorFor {
            src,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorDataRace(message, pos) => Err(ErrorDataRace {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorAssignmentToConstant(message, pos) => Err(ErrorAssignmentToConstant {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorDotExpr(message, pos) => Err(ErrorDotExpr {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorArithmetic(message, pos) => Err(ErrorArithmetic {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorTooManyOperations(pos) => Err(ErrorTooManyOperations {
            src,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorTooManyModules(pos) => Err(ErrorTooManyModules {
            src,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorStackOverflow(pos) => Err(ErrorStackOverflow {
            src,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorDataTooLarge(message, pos) => Err(ErrorDataTooLarge {
            src,
            message,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorTerminated(_termination_token, pos) => Err(ErrorTerminated {
            src,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::ErrorCustomSyntax(message, _syntax_symbol_stream, pos) => {
            Err(ErrorCustomSyntax {
                src,
                message,
                bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
            })?
        }
        EvalAltResult::ErrorRuntime(_error_token, pos) => Err(ErrorRuntime {
            src,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::LoopBreak(_break_type, pos) => Err(LoopBreak {
            src,
            bad_bit: position_to_span(pos, rhai_code).unwrap_or(default_span),
        })?,
        EvalAltResult::Return(_return_result, _pos) => Ok(()),
        _ => Err(miette::miette!(format!("{:?}", err)))?,
    }
}

fn position_to_span(pos: Position, code: &str) -> Option<SourceSpan> {
    let line_offset = line_offset(code, pos.line()?)?;
    let span = (line_offset + pos.position()? - 1, 0).into();
    Some(span)
}
