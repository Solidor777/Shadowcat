//! Scripted `SessionMonitor` for tests — a fixed sequence of poll results, replayed once
//! each and then repeating the last entry. Never compiled into the release binary
//! (`#[cfg(test)]` at the `mod fake;` declaration in `mod.rs`).

use super::{MonitorError, SessionLevel, SessionMonitor};

/// A `SessionMonitor` driven by a scripted sequence of `poll()` results, advancing one entry
/// per call and repeating the final entry once the script is exhausted (so a test's 10 Hz
/// loop assertion never runs off the end of a short script).
pub struct FakeMonitor {
    /// The scripted results, one per `poll()` call (repeats the last once exhausted).
    script: Vec<Result<Vec<SessionLevel>, MonitorError>>,
    /// Index into `script` of the next result to return.
    next: usize,
}

impl FakeMonitor {
    /// Builds a `FakeMonitor` that replays `script` in order, one entry per `poll()` call,
    /// repeating the final entry once exhausted.
    ///
    /// # Panics
    /// Panics if `script` is empty (a test always scripts at least one poll outcome).
    pub fn new(script: Vec<Result<Vec<SessionLevel>, MonitorError>>) -> Self {
        assert!(
            !script.is_empty(),
            "FakeMonitor needs at least one scripted result"
        );
        Self { script, next: 0 }
    }
}

impl SessionMonitor for FakeMonitor {
    fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
        let i = self.next.min(self.script.len() - 1);
        if self.next < self.script.len() - 1 {
            self.next += 1;
        }
        self.script[i].clone()
    }
}
