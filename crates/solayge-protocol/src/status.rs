//! Command lifecycle states.
//!
//! A command moves `created → delivered → accepted → running →
//! succeeded/failed/canceled`. `unknown` means the outcome could not be
//! determined (for example after a transport timeout) and is deliberately
//! distinct from `failed`: resending blindly is unsafe, so the caller must
//! query [`crate::Action::SessionsGetCommandStatus`] instead.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    /// Recorded locally, not yet handed to the transport.
    Created,
    /// The transport accepted the frame for delivery.
    Delivered,
    /// The host accepted the command for dispatch.
    Accepted,
    /// The agent/host is actively executing it.
    Running,
    /// Finished successfully.
    Succeeded,
    /// Finished with a definite failure.
    Failed,
    /// Canceled (by request or before dispatch).
    Canceled,
    /// Outcome could not be determined; query, do not blind-retry.
    Unknown,
}

impl CommandStatus {
    /// Every status, in lifecycle order. Tests use this to prove the transition
    /// table and helpers cover every variant.
    pub const ALL: [CommandStatus; 8] = [
        CommandStatus::Created,
        CommandStatus::Delivered,
        CommandStatus::Accepted,
        CommandStatus::Running,
        CommandStatus::Succeeded,
        CommandStatus::Failed,
        CommandStatus::Canceled,
        CommandStatus::Unknown,
    ];

    /// A definitive end state. `unknown` is intentionally not terminal: it is a
    /// pending question whose answer may still arrive.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            CommandStatus::Succeeded | CommandStatus::Failed | CommandStatus::Canceled
        )
    }

    /// Still in flight, or awaiting resolution.
    pub fn is_pending(self) -> bool {
        !self.is_terminal()
    }

    pub fn is_success(self) -> bool {
        self == CommandStatus::Succeeded
    }

    /// Every status, in lifecycle order. Alias kept for call sites that read
    /// better as "in order".
    pub const fn wire_name(self) -> &'static str {
        match self {
            CommandStatus::Created => "created",
            CommandStatus::Delivered => "delivered",
            CommandStatus::Accepted => "accepted",
            CommandStatus::Running => "running",
            CommandStatus::Succeeded => "succeeded",
            CommandStatus::Failed => "failed",
            CommandStatus::Canceled => "canceled",
            CommandStatus::Unknown => "unknown",
        }
    }

    /// Whether a command may move from `self` to `next`. Restating the current
    /// state is always allowed so a duplicate observation is harmless.
    ///
    /// Terminal states never advance. `unknown` is unusual: it may only be
    /// *resolved* into a definite terminal state, never back into a running one.
    pub fn can_transition_to(self, next: CommandStatus) -> bool {
        if self == next {
            return true;
        }
        match self {
            CommandStatus::Created => matches!(
                next,
                CommandStatus::Delivered
                    | CommandStatus::Accepted
                    | CommandStatus::Running
                    | CommandStatus::Succeeded
                    | CommandStatus::Failed
                    | CommandStatus::Canceled
                    | CommandStatus::Unknown
            ),
            CommandStatus::Delivered => matches!(
                next,
                CommandStatus::Accepted
                    | CommandStatus::Running
                    | CommandStatus::Succeeded
                    | CommandStatus::Failed
                    | CommandStatus::Canceled
                    | CommandStatus::Unknown
            ),
            CommandStatus::Accepted => matches!(
                next,
                CommandStatus::Running
                    | CommandStatus::Succeeded
                    | CommandStatus::Failed
                    | CommandStatus::Canceled
                    | CommandStatus::Unknown
            ),
            CommandStatus::Running => matches!(
                next,
                CommandStatus::Succeeded
                    | CommandStatus::Failed
                    | CommandStatus::Canceled
                    | CommandStatus::Unknown
            ),
            CommandStatus::Succeeded | CommandStatus::Failed | CommandStatus::Canceled => false,
            CommandStatus::Unknown => matches!(
                next,
                CommandStatus::Succeeded | CommandStatus::Failed | CommandStatus::Canceled
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CommandStatus as S;

    #[test]
    fn only_definite_end_states_are_terminal() {
        for s in S::ALL {
            let expected = matches!(s, S::Succeeded | S::Failed | S::Canceled);
            assert_eq!(s.is_terminal(), expected, "{s:?}");
            assert_eq!(s.is_pending(), !expected, "{s:?}");
        }
        assert!(!S::Unknown.is_terminal(), "unknown is not failed");
    }

    #[test]
    fn restating_a_status_is_always_allowed() {
        for s in S::ALL {
            assert!(s.can_transition_to(s), "{s:?} should be idempotent");
        }
    }

    #[test]
    fn terminal_statuses_never_advance() {
        for terminal in [S::Succeeded, S::Failed, S::Canceled] {
            for next in S::ALL {
                if next == terminal {
                    continue;
                }
                assert!(
                    !terminal.can_transition_to(next),
                    "{terminal:?} must not move to {next:?}"
                );
            }
        }
    }

    #[test]
    fn unknown_can_only_be_resolved_to_a_definite_outcome() {
        // Restating `unknown` is allowed (the idempotence rule); every *other*
        // target must be a definite terminal outcome.
        assert!(S::Unknown.can_transition_to(S::Unknown));
        for next in S::ALL {
            if next == S::Unknown {
                continue;
            }
            let expected = matches!(next, S::Succeeded | S::Failed | S::Canceled);
            assert_eq!(S::Unknown.can_transition_to(next), expected, "{next:?}");
        }
    }

    #[test]
    fn the_lifecycle_advances_forward() {
        let path = [
            S::Created,
            S::Delivered,
            S::Accepted,
            S::Running,
            S::Succeeded,
        ];
        for pair in path.windows(2) {
            assert!(pair[0].can_transition_to(pair[1]), "{pair:?}");
        }
    }

    #[test]
    fn every_status_has_a_wire_name() {
        for s in S::ALL {
            assert!(!s.wire_name().is_empty());
        }
    }
}
