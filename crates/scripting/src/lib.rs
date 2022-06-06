use gag::BufferRedirect;
use miette::{IntoDiagnostic, Result};
use std::io::Read;

pub fn redirect_buffers(redirect: bool) -> Option<(BufferRedirect, BufferRedirect)> {
    if !redirect {
        return None;
    }
    if let Ok(stdout) = BufferRedirect::stdout() {
        if let Ok(stderr) = BufferRedirect::stderr() {
            return Some((stdout, stderr));
        }
    }
    None
}

pub fn buffers_to_string(
    buffers: Option<(BufferRedirect, BufferRedirect)>,
) -> Result<(String, String)> {
    let mut stdout_str = String::new();
    let mut stderr_str = String::new();
    if let Some((mut stdout, mut stderr)) = buffers {
        stdout.read_to_string(&mut stdout_str).into_diagnostic()?;
        stderr.read_to_string(&mut stderr_str).into_diagnostic()?;
    }
    Ok((stdout_str, stderr_str))
}
