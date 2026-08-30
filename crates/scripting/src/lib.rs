/// Which of a script's two output streams a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Receives a script's output one line at a time, as it is produced.
///
/// This exists because the previous implementation redirected the *process's*
/// file descriptors with `gag`, which is unusable in a server: it captures the
/// server's own logging along with the script's, it cannot say which job a
/// line belongs to, and it only yields anything once the run is over. An
/// implementation must be cheap and must not block — it is called from inside
/// the Lua interpreter, with the interpreter's lock held.
pub trait OutputSink: Send + Sync {
    fn line(&self, stream: Stream, text: &str);
}

///  Returns byte offset for given line if found
pub fn line_offset(input: &str, line: usize) -> Option<usize> {
    let mut cursor = input;
    let mut offset = 0;
    for _ in 1..line {
        {
            let pos = cursor.find('\n')?;
            cursor = &cursor[pos + 1..cursor.len()];
            offset += pos + 1;
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
