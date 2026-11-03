// SPDX-License-Identifier: GPL-3.0-only
//! The control signal that the handle sends to every worker.

use std::time::Duration;

/// The run state that every worker follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Control {
    /// Start and accept sessions.
    Run,
    /// Start no new session and reset the sessions still open after the ramp
    /// down time, which each worker counts from the moment it sees the signal.
    Drain(Duration),
    /// Reset every open session at once.
    Abort,
}

impl Control {
    /// Returns the state after `next`, which never leaves a stop and never
    /// lengthens a ramp down.
    pub(crate) fn then(self, next: Control) -> Control {
        match (self, next) {
            (Control::Abort, _) | (_, Control::Abort) => Control::Abort,
            (Control::Drain(a), Control::Drain(b)) => Control::Drain(a.min(b)),
            (Control::Drain(a), Control::Run) => Control::Drain(a),
            (Control::Run, n) => n,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stop_is_final_and_an_abort_wins() {
        let d = |s| Control::Drain(Duration::from_secs(s));
        assert_eq!(Control::Run.then(d(5)), d(5));
        assert_eq!(d(5).then(Control::Run), d(5));
        assert_eq!(d(5).then(d(2)), d(2));
        assert_eq!(d(2).then(d(5)), d(2));
        assert_eq!(d(5).then(Control::Abort), Control::Abort);
        assert_eq!(Control::Abort.then(d(1)), Control::Abort);
        assert_eq!(Control::Abort.then(Control::Run), Control::Abort);
    }
}
