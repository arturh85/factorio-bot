//! A terminal-aware wrapper around [`paris::Logger`]'s loading animation.
//!
//! `paris`' `loading()` spawns a thread that rewrites the same line six times a
//! second. On a terminal that reads as a spinner; piped to a file or captured
//! for streaming it is just the same sentence repeated until the work finishes.
//! One real run produced a 111 KB log that was almost entirely
//! `Creating Level at "..."` frames.
//!
//! [`Spinner`] keeps the animation for interactive use and degrades to printing
//! the message exactly once when stdout is not a terminal.

use paris::Logger;
use std::fmt::Display;
use std::io::IsTerminal;

/// True when stdout is attached to a terminal, i.e. when an animation has
/// somebody to animate for.
pub fn stdout_is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

/// Progress reporter that only animates on a terminal.
pub struct Spinner {
    logger: Logger<'static>,
    animate: bool,
    animating: bool,
}

impl Spinner {
    /// Animates when stdout is a terminal, prints once otherwise.
    pub fn new() -> Self {
        Self::with_animation(stdout_is_terminal())
    }

    /// Explicit-animation constructor, for tests and for callers that already
    /// know they are writing to a captured stream.
    pub fn with_animation(animate: bool) -> Self {
        Self {
            logger: Logger::new(),
            animate,
            animating: false,
        }
    }

    /// Whether a spinner thread is currently running. Always false when this
    /// `Spinner` was built without animation.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Announce ongoing work: an animated spinner on a terminal, a single
    /// `info` line otherwise.
    pub fn loading<T: Display>(&mut self, message: T) -> &mut Self {
        if self.animate {
            self.logger.loading(message);
            self.animating = true;
        } else {
            self.logger.info(message);
        }
        self
    }

    /// Stop any animation and report the work as finished.
    pub fn success<T: Display>(&mut self, message: T) -> &mut Self {
        self.animating = false;
        self.logger.success(message);
        self
    }

    /// Stop any animation without printing anything.
    pub fn done(&mut self) -> &mut Self {
        self.animating = false;
        self.logger.done();
        self
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_terminal_output_never_starts_the_animation_thread() {
        let mut spinner = Spinner::with_animation(false);
        spinner.loading("Creating Level at somewhere...");
        assert!(
            !spinner.is_animating(),
            "spinner must not animate when stdout is not a terminal"
        );
        spinner.done();
        assert!(!spinner.is_animating());
    }

    #[test]
    fn terminal_output_still_animates() {
        let mut spinner = Spinner::with_animation(true);
        spinner.loading("Creating Level at somewhere...");
        assert!(
            spinner.is_animating(),
            "interactive progress indication must be preserved"
        );
        spinner.success("Created Level");
        assert!(!spinner.is_animating());
    }
}
