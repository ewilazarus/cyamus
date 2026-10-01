//! User-facing progress reporting, implemented by the CLI.

use crate::hooks::Event;

/// Receives human-readable progress from core operations.
pub trait Reporter {
    /// An informational message (e.g. a guessed project name).
    fn notice(&mut self, message: &str);

    /// Something went wrong that doesn't stop the operation.
    fn warning(&mut self, message: &str) {
        self.notice(&format!("warning: {message}"));
    }

    /// A hook command is about to run.
    fn hook_started(&mut self, event: Event, command: &str);
}

/// Discards everything. Useful in tests.
#[derive(Debug, Default)]
pub struct NullReporter;

impl Reporter for NullReporter {
    fn notice(&mut self, _message: &str) {}
    fn hook_started(&mut self, _event: Event, _command: &str) {}
}
