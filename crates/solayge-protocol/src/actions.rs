//! The versioned set of protocol actions (section 6.1 of the mobile spec).
//!
//! The wire form is a dotted string (`sessions.send_prompt`). Actions are
//! matched exhaustively so adding a variant is a compile error until it is
//! classified by the helpers below.

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // --- pairing ---
    PairBegin,
    PairProveIdentity,
    PairApprove,
    PairReject,
    // --- connection ---
    ConnectionAuthenticate,
    ConnectionHealth,
    // --- hosts ---
    HostsGetMetadata,
    // --- projects ---
    ProjectsList,
    // --- sessions ---
    SessionsList,
    SessionsGet,
    SessionsEventsSubscribe,
    SessionsSendPrompt,
    SessionsGetCommandStatus,
    SessionsCancel,
    SessionsStart,
    SessionsAnswer,
    // --- approvals ---
    ApprovalsList,
    ApprovalsGet,
    ApprovalsRespond,
    // --- files ---
    FilesChangedSummary,
    // --- insights ---
    InsightsSummary,
    InsightsErrors,
    InsightsAgentQuestions,
    // --- devices ---
    DevicesList,
    DevicesRevoke,
    // --- push ---
    PushRegister,
    PushUnregister,
}

impl Action {
    /// Every action in this protocol version. Keep in sync with the `match` in
    /// [`Action::as_str`]; a test asserts the two agree.
    pub const ALL: [Action; 27] = [
        Action::PairBegin,
        Action::PairProveIdentity,
        Action::PairApprove,
        Action::PairReject,
        Action::ConnectionAuthenticate,
        Action::ConnectionHealth,
        Action::HostsGetMetadata,
        Action::ProjectsList,
        Action::SessionsList,
        Action::SessionsGet,
        Action::SessionsEventsSubscribe,
        Action::SessionsSendPrompt,
        Action::SessionsGetCommandStatus,
        Action::SessionsCancel,
        Action::SessionsStart,
        Action::ApprovalsList,
        Action::ApprovalsGet,
        Action::ApprovalsRespond,
        Action::SessionsAnswer,
        Action::FilesChangedSummary,
        Action::InsightsSummary,
        Action::InsightsErrors,
        Action::InsightsAgentQuestions,
        Action::DevicesList,
        Action::DevicesRevoke,
        Action::PushRegister,
        Action::PushUnregister,
    ];

    /// The dotted wire string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Action::PairBegin => "pair.begin",
            Action::PairProveIdentity => "pair.prove_identity",
            Action::PairApprove => "pair.approve",
            Action::PairReject => "pair.reject",
            Action::ConnectionAuthenticate => "connection.authenticate",
            Action::ConnectionHealth => "connection.health",
            Action::HostsGetMetadata => "hosts.get_metadata",
            Action::ProjectsList => "projects.list",
            Action::SessionsList => "sessions.list",
            Action::SessionsGet => "sessions.get",
            Action::SessionsEventsSubscribe => "sessions.events.subscribe",
            Action::SessionsSendPrompt => "sessions.send_prompt",
            Action::SessionsGetCommandStatus => "sessions.get_command_status",
            Action::SessionsCancel => "sessions.cancel",
            Action::SessionsStart => "sessions.start",
            Action::SessionsAnswer => "sessions.answer",
            Action::ApprovalsList => "approvals.list",
            Action::ApprovalsGet => "approvals.get",
            Action::ApprovalsRespond => "approvals.respond",
            Action::FilesChangedSummary => "files.changed_summary",
            Action::InsightsSummary => "insights.summary",
            Action::InsightsErrors => "insights.errors",
            Action::InsightsAgentQuestions => "insights.agent_questions",
            Action::DevicesList => "devices.list",
            Action::DevicesRevoke => "devices.revoke",
            Action::PushRegister => "push.register",
            Action::PushUnregister => "push.unregister",
        }
    }

    /// Parse a dotted wire string, rejecting unknown actions.
    pub fn parse(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// Whether the action changes host state and therefore requires an
    /// idempotency key (`command_id`).
    pub const fn is_mutating(self) -> bool {
        matches!(
            self,
            Action::PairApprove
                | Action::PairReject
                | Action::SessionsSendPrompt
                | Action::SessionsCancel
                | Action::SessionsStart
                | Action::SessionsAnswer
                | Action::ApprovalsRespond
                | Action::DevicesRevoke
                | Action::PushRegister
                | Action::PushUnregister
        )
    }

    /// Whether the action addresses one session and therefore requires
    /// `session_id`.
    pub const fn requires_session_id(self) -> bool {
        matches!(
            self,
            Action::SessionsGet
                | Action::SessionsSendPrompt
                | Action::SessionsCancel
                | Action::SessionsStart
                | Action::SessionsAnswer
                | Action::FilesChangedSummary
        )
    }

    /// Pairing and authentication handshakes are the only actions allowed
    /// before a device is authenticated.
    pub const fn is_pre_auth(self) -> bool {
        matches!(
            self,
            Action::PairBegin
                | Action::PairProveIdentity
                | Action::PairApprove
                | Action::PairReject
                | Action::ConnectionAuthenticate
                | Action::ConnectionHealth
        )
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Action {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ActionVisitor;

        impl Visitor<'_> for ActionVisitor {
            type Value = Action;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a Solayge protocol action, e.g. \"sessions.send_prompt\"")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Action, E> {
                Action::parse(value)
                    .ok_or_else(|| de::Error::custom(format!("unknown action: {value}")))
            }
        }

        deserializer.deserialize_str(ActionVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::Action;

    #[test]
    fn every_action_round_trips_through_its_wire_string() {
        assert_eq!(Action::ALL.len(), 27);
        for action in Action::ALL {
            assert_eq!(Action::parse(action.as_str()), Some(action));
        }
    }

    #[test]
    fn unknown_actions_are_rejected() {
        assert_eq!(Action::parse("sessions.delete_everything"), None);
        assert_eq!(Action::parse(""), None);
        // Singular `session.*` from the spec's illustrative JSON is not an
        // action; the versioned list uses the plural namespace.
        assert_eq!(Action::parse("session.send_prompt"), None);
    }

    #[test]
    fn mutating_actions_require_a_command_id() {
        assert!(Action::SessionsSendPrompt.is_mutating());
        assert!(Action::DevicesRevoke.is_mutating());
        assert!(!Action::SessionsList.is_mutating());
        assert!(!Action::ConnectionHealth.is_mutating());
    }

    #[test]
    fn only_pre_auth_actions_are_allowed_before_authentication() {
        assert!(Action::PairBegin.is_pre_auth());
        assert!(Action::ConnectionAuthenticate.is_pre_auth());
        assert!(!Action::SessionsList.is_pre_auth());
        assert!(!Action::DevicesRevoke.is_pre_auth());
    }
}
