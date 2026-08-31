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

    /// Receives a finished run's replay document, already serialised as JSON.
    ///
    /// A replay is only meaningful as part of the run that produced it, so it
    /// travels out beside that run's output rather than through a route of its
    /// own — the server has no execution state to serve it from, and inventing
    /// some would be inventing a second, weaker copy of the run.
    ///
    /// # Why a string and not the document
    ///
    /// The payload is a `factorio_bot_executor::replay::Replay`, and this
    /// method deliberately does not say so in its type:
    ///
    /// - This crate depends on neither the executor nor the planner. A typed
    ///   parameter would invert that dependency and drag execution into a crate
    ///   whose job is transport. A transport that knows what it carries is
    ///   coupled to it.
    /// - **Serialise at the call site, never in here.** The rule on
    ///   [`OutputSink::line`] applies with more force: an implementation must be
    ///   cheap and must not block. A replay for a real run holds two rows per
    ///   scheduled step and is not small, so an implementation that serialised
    ///   would do that work wherever the caller happens to be — for `line` that
    ///   is inside the Lua interpreter with its lock held. The caller hands over
    ///   a prepared string precisely so an implementation never has to.
    ///
    /// The default discards it, so a sink that only wants lines stays a
    /// one-method implementation.
    fn replay(&self, json: &str) {
        let _ = json;
    }
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
