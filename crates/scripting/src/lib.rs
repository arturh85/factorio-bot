use factorio_bot_core::miette::{IntoDiagnostic, Result};
use gag::BufferRedirect;
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
    stdout: &str,
    stderr: &str,
    buffers: Option<(BufferRedirect, BufferRedirect)>,
) -> Result<(String, String)> {
    let mut stdout_str = String::new();
    let mut stderr_str = String::new();
    if let Some((mut stdout, mut stderr)) = buffers {
        stdout.read_to_string(&mut stdout_str).into_diagnostic()?;
        stderr.read_to_string(&mut stderr_str).into_diagnostic()?;
    } else {
        stdout_str = stdout.to_owned();
        stderr_str = stderr.to_owned();
    }
    Ok((stdout_str, stderr_str))
}

///  Returns byte offset for given line if found
pub fn line_offset(input: &str, line: usize) -> Option<usize> {
    let mut cursor = input;
    let mut offset = 0;
    for _ in 1..line {
        if let Some(pos) = cursor.find('\n') {
            cursor = &cursor[pos + 1..cursor.len()];
            offset += pos + 1;
        } else {
            return None;
        }
    }
    Some(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test() {
        assert_eq!(line_offset("", 1), Some(0));
        assert_eq!(line_offset("\n", 1), Some(0));
        assert_eq!(line_offset("\n", 2), Some(1));
        assert_eq!(line_offset("\n", 3), None);
        assert_eq!(line_offset("abc\ndef", 1), Some(0));
        assert_eq!(line_offset("abc\ndef", 2), Some(4));
        assert_eq!(line_offset("abc\ndef", 3), None);
    }
}
