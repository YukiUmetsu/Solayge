//! Transport error classification.
//!
//! The connection manager's security rule depends on correctly telling an
//! *authentication/authorization denial* apart from a *connectivity failure*:
//! the former is terminal and must never trigger a transport fallback or a
//! retry, the latter may be retried and (when configured) fall back.
//!
//! [`NetworkError`] preserves that distinction from the Iroh layer all the way
//! to the state machine. It also maps cleanly onto the wire-stable
//! [`solayge_protocol::ProtocolError`] so the Iroh transport can still implement
//! [`solayge_protocol::RemoteClient`].

use std::fmt;

use solayge_protocol::{ErrorCode, ProtocolError};

/// How a failure should be treated by the connection manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Authentication or authorization was denied (including a revoked device).
    /// Terminal: never retried, never fell back from.
    AuthDenied,
    /// The transport could not reach the peer. Retryable, and eligible for a
    /// configured fallback transport.
    NetworkFailure,
    /// A protocol, framing, or internal failure. Retryable, but never treated
    /// as a network failure (so it does not trigger fallback).
    Other,
}

/// A failure in the Iroh transport or the connection manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkError {
    /// The host rejected our application identity (invalid proof, revoked
    /// device, missing scope). This is an authorization outcome, not a network
    /// problem.
    AuthDenied { reason: String },
    /// The peer could not be reached, a stream failed, or the endpoint closed.
    Connectivity(String),
    /// A frame exceeded the bounded frame size and was not read.
    FrameTooLarge { size: usize, max: usize },
    /// A frame was structurally invalid or named an unsupported action.
    Protocol(ProtocolError),
    /// A bug or misconfiguration (for example a malformed relay URL).
    Internal(String),
}

impl NetworkError {
    pub fn auth_denied(reason: impl Into<String>) -> Self {
        NetworkError::AuthDenied {
            reason: reason.into(),
        }
    }

    pub fn connectivity(message: impl Into<String>) -> Self {
        NetworkError::Connectivity(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        NetworkError::Internal(message.into())
    }

    /// Classify for the state machine. `NotAuthorized` protocol errors are
    /// authorization outcomes, never connectivity.
    pub fn classify(&self) -> ErrorClass {
        match self {
            NetworkError::AuthDenied { .. } => ErrorClass::AuthDenied,
            NetworkError::Connectivity(_) => ErrorClass::NetworkFailure,
            NetworkError::Protocol(error) if error.code() == ErrorCode::NotAuthorized => {
                ErrorClass::AuthDenied
            }
            NetworkError::FrameTooLarge { .. }
            | NetworkError::Protocol(_)
            | NetworkError::Internal(_) => ErrorClass::Other,
        }
    }

    pub fn is_auth_denied(&self) -> bool {
        self.classify() == ErrorClass::AuthDenied
    }

    /// True only for a genuine reachability failure. Fallback requires this.
    pub fn is_network_failure(&self) -> bool {
        self.classify() == ErrorClass::NetworkFailure
    }

    /// Project onto the protocol error type so the sync `RemoteClient` bridge
    /// can report a typed error.
    pub fn to_protocol_error(&self) -> ProtocolError {
        match self {
            NetworkError::AuthDenied { reason } => ProtocolError::not_authorized(reason.clone()),
            NetworkError::Connectivity(message) => ProtocolError::transport(message.clone()),
            NetworkError::FrameTooLarge { size, max } => {
                ProtocolError::frame_too_large(*size, *max)
            }
            NetworkError::Protocol(error) => error.clone(),
            NetworkError::Internal(message) => ProtocolError::internal(message.clone()),
        }
    }
}

impl From<ProtocolError> for NetworkError {
    fn from(error: ProtocolError) -> Self {
        if error.code() == ErrorCode::NotAuthorized {
            NetworkError::AuthDenied {
                reason: error.message().to_string(),
            }
        } else {
            NetworkError::Protocol(error)
        }
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkError::AuthDenied { reason } => write!(f, "access denied: {reason}"),
            NetworkError::Connectivity(message) => write!(f, "connectivity failure: {message}"),
            NetworkError::FrameTooLarge { size, max } => {
                write!(f, "frame of {size} bytes exceeds the {max}-byte limit")
            }
            NetworkError::Protocol(error) => write!(f, "protocol error: {error}"),
            NetworkError::Internal(message) => write!(f, "internal error: {message}"),
        }
    }
}

impl std::error::Error for NetworkError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_authorized_is_an_authorization_outcome_not_a_network_failure() {
        let denied = NetworkError::from(ProtocolError::not_authorized("device has been revoked"));
        assert!(denied.is_auth_denied());
        assert!(!denied.is_network_failure());
        assert_eq!(denied.classify(), ErrorClass::AuthDenied);
    }

    #[test]
    fn connectivity_is_retryable_and_fallback_eligible() {
        let down = NetworkError::connectivity("relay unreachable");
        assert!(down.is_network_failure());
        assert!(!down.is_auth_denied());
    }

    #[test]
    fn other_protocol_errors_are_neither_auth_nor_network() {
        let bad = NetworkError::from(ProtocolError::invalid_frame("not json"));
        assert_eq!(bad.classify(), ErrorClass::Other);
        assert!(!bad.is_network_failure());
    }

    #[test]
    fn errors_map_onto_the_matching_protocol_codes() {
        assert_eq!(
            NetworkError::auth_denied("revoked")
                .to_protocol_error()
                .code(),
            ErrorCode::NotAuthorized
        );
        assert_eq!(
            NetworkError::connectivity("timeout")
                .to_protocol_error()
                .code(),
            ErrorCode::Transport
        );
        assert_eq!(
            NetworkError::FrameTooLarge { size: 10, max: 4 }
                .to_protocol_error()
                .code(),
            ErrorCode::FrameTooLarge
        );
        assert_eq!(
            NetworkError::internal("bad url").to_protocol_error().code(),
            ErrorCode::Internal
        );
    }

    #[test]
    fn a_not_authorized_protocol_error_is_reclassified_as_auth_denied() {
        let error = NetworkError::from(ProtocolError::not_authorized("revoked"));
        assert_eq!(error.classify(), ErrorClass::AuthDenied);
    }
}
