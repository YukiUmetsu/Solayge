//! Typed, wire-stable protocol errors.
//!
//! A [`ProtocolError`] couples a machine-readable [`ErrorCode`] (safe to persist
//! and switch on) with a human-readable message. Messages must never contain
//! secrets, tokens, keys, or file contents: the code is the contract, the
//! message is only a hint.

use std::fmt;

/// Stable wire identifier for a protocol error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// The frame was not valid JSON, had the wrong shape, or a field had the
    /// wrong type.
    InvalidFrame,
    /// The frame was structurally valid but used a different protocol version.
    UnsupportedVersion,
    /// The action string is not part of this protocol version.
    UnknownAction,
    /// A required field for the action was missing or empty.
    MissingField,
    /// A field was present but not acceptable (bad value, wrong JSON kind, …).
    InvalidPayload,
    /// The encoded frame exceeded the maximum frame size.
    FrameTooLarge,
    /// The payload exceeded the maximum payload size.
    PayloadTooLarge,
    /// The same command id was reused for a different action.
    IdempotencyConflict,
    /// A command status transition was not allowed.
    InvalidTransition,
    /// The action is recognized but this peer does not implement it.
    UnsupportedAction,
    /// The referenced entity does not exist.
    NotFound,
    /// Authentication or authorization failed.
    NotAuthorized,
    /// A bounded resource (idempotency table, queue) was exhausted.
    ResourceExhausted,
    /// The underlying transport failed.
    Transport,
    /// An unexpected internal failure.
    Internal,
}

impl ErrorCode {
    /// Every code, so tests can assert they all round-trip.
    pub const ALL: [ErrorCode; 15] = [
        ErrorCode::InvalidFrame,
        ErrorCode::UnsupportedVersion,
        ErrorCode::UnknownAction,
        ErrorCode::MissingField,
        ErrorCode::InvalidPayload,
        ErrorCode::FrameTooLarge,
        ErrorCode::PayloadTooLarge,
        ErrorCode::IdempotencyConflict,
        ErrorCode::InvalidTransition,
        ErrorCode::UnsupportedAction,
        ErrorCode::NotFound,
        ErrorCode::NotAuthorized,
        ErrorCode::ResourceExhausted,
        ErrorCode::Transport,
        ErrorCode::Internal,
    ];

    /// The snake_case string carried on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidFrame => "invalid_frame",
            ErrorCode::UnsupportedVersion => "unsupported_version",
            ErrorCode::UnknownAction => "unknown_action",
            ErrorCode::MissingField => "missing_field",
            ErrorCode::InvalidPayload => "invalid_payload",
            ErrorCode::FrameTooLarge => "frame_too_large",
            ErrorCode::PayloadTooLarge => "payload_too_large",
            ErrorCode::IdempotencyConflict => "idempotency_conflict",
            ErrorCode::InvalidTransition => "invalid_transition",
            ErrorCode::UnsupportedAction => "unsupported_action",
            ErrorCode::NotFound => "not_found",
            ErrorCode::NotAuthorized => "not_authorized",
            ErrorCode::ResourceExhausted => "resource_exhausted",
            ErrorCode::Transport => "transport",
            ErrorCode::Internal => "internal",
        }
    }

    /// Parse a wire code, returning `None` for an unknown string.
    pub fn parse(s: &str) -> Option<ErrorCode> {
        ErrorCode::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// A typed error that can cross the protocol boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    code: ErrorCode,
    message: String,
}

impl ProtocolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        ProtocolError {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn invalid_frame(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidFrame, message)
    }

    pub fn unsupported_version(version: u32) -> Self {
        Self::new(
            ErrorCode::UnsupportedVersion,
            format!("unsupported protocol_version {version}"),
        )
    }

    pub fn unknown_action(action: &str) -> Self {
        Self::new(
            ErrorCode::UnknownAction,
            format!("unknown action: {action}"),
        )
    }

    pub fn missing_field(field: &str) -> Self {
        Self::new(
            ErrorCode::MissingField,
            format!("missing required field: {field}"),
        )
    }

    pub fn invalid_payload(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidPayload, message)
    }

    pub fn frame_too_large(size: usize, max: usize) -> Self {
        Self::new(
            ErrorCode::FrameTooLarge,
            format!("frame of {size} bytes exceeds the {max}-byte limit"),
        )
    }

    pub fn payload_too_large(size: usize, max: usize) -> Self {
        Self::new(
            ErrorCode::PayloadTooLarge,
            format!("payload of {size} bytes exceeds the {max}-byte limit"),
        )
    }

    pub fn idempotency_conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::IdempotencyConflict, message)
    }

    pub fn invalid_transition(from: &str, to: &str) -> Self {
        Self::new(
            ErrorCode::InvalidTransition,
            format!("cannot transition command status from {from} to {to}"),
        )
    }

    pub fn unsupported_action(action: &str) -> Self {
        Self::new(
            ErrorCode::UnsupportedAction,
            format!("action not implemented: {action}"),
        )
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(ErrorCode::NotFound, format!("not found: {what}"))
    }

    pub fn not_authorized(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotAuthorized, message)
    }

    pub fn resource_exhausted(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ResourceExhausted, message)
    }

    pub fn transport(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Transport, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ProtocolError {}

/// Convenience: map a `serde_json` failure into an invalid-frame error.
impl From<serde_json::Error> for ProtocolError {
    fn from(e: serde_json::Error) -> Self {
        ProtocolError::invalid_frame(e.to_string())
    }
}
