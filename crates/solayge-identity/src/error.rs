//! Typed errors for the identity and pairing layer.
//!
//! Messages are hints and must never contain private key material or a full
//! proof. The variant itself is the contract callers switch on.

use std::fmt;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, IdentityError>;

/// A failure in identity handling, pairing, storage, or authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// A key or signature was not valid for its stated type or length.
    InvalidKey(String),
    /// A signature did not verify against the message and public key.
    InvalidSignature,
    /// A value could not be encoded or decoded (base64, JSON, hex).
    InvalidEncoding(String),
    /// A required field was missing or empty.
    MissingField(String),
    /// A field was present but not acceptable.
    InvalidField(String),
    /// No pairing session matched the id or manual code.
    PairingNotFound(String),
    /// The pairing session has expired.
    PairingExpired(String),
    /// The pairing session was already consumed (approved) or rejected.
    PairingNotActive(String),
    /// The session was rejected by the desktop user.
    PairingRejected(String),
    /// The pairing session is in the wrong state for this operation.
    InvalidPairingState(String),
    /// A rate limit (pairing begins or proof attempts) was exceeded.
    RateLimited(String),
    /// A proof did not verify, or did not match the bound transcript.
    ProofInvalid(String),
    /// The response did not match the transcript the client expected.
    TranscriptMismatch,
    /// The device is registered but not allowed to perform the action.
    NotAuthorized(String),
    /// The device has been explicitly revoked and must be re-paired.
    DeviceRevoked(String),
    /// The device is not registered at all.
    UnknownDevice(String),
    /// A secure store operation failed.
    Storage(String),
    /// An unexpected internal failure.
    Internal(String),
}

impl IdentityError {
    /// A short, stable, machine-readable code for logs and tests.
    pub fn code(&self) -> &'static str {
        match self {
            IdentityError::InvalidKey(_) => "invalid_key",
            IdentityError::InvalidSignature => "invalid_signature",
            IdentityError::InvalidEncoding(_) => "invalid_encoding",
            IdentityError::MissingField(_) => "missing_field",
            IdentityError::InvalidField(_) => "invalid_field",
            IdentityError::PairingNotFound(_) => "pairing_not_found",
            IdentityError::PairingExpired(_) => "pairing_expired",
            IdentityError::PairingNotActive(_) => "pairing_not_active",
            IdentityError::PairingRejected(_) => "pairing_rejected",
            IdentityError::InvalidPairingState(_) => "invalid_pairing_state",
            IdentityError::RateLimited(_) => "rate_limited",
            IdentityError::ProofInvalid(_) => "proof_invalid",
            IdentityError::TranscriptMismatch => "transcript_mismatch",
            IdentityError::NotAuthorized(_) => "not_authorized",
            IdentityError::DeviceRevoked(_) => "device_revoked",
            IdentityError::UnknownDevice(_) => "unknown_device",
            IdentityError::Storage(_) => "storage",
            IdentityError::Internal(_) => "internal",
        }
    }
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentityError::InvalidKey(m) => write!(f, "invalid key: {m}"),
            IdentityError::InvalidSignature => write!(f, "signature verification failed"),
            IdentityError::InvalidEncoding(m) => write!(f, "invalid encoding: {m}"),
            IdentityError::MissingField(m) => write!(f, "missing required field: {m}"),
            IdentityError::InvalidField(m) => write!(f, "invalid field: {m}"),
            IdentityError::PairingNotFound(m) => write!(f, "pairing not found: {m}"),
            IdentityError::PairingExpired(m) => write!(f, "pairing expired: {m}"),
            IdentityError::PairingNotActive(m) => write!(f, "pairing is not active: {m}"),
            IdentityError::PairingRejected(m) => write!(f, "pairing rejected: {m}"),
            IdentityError::InvalidPairingState(m) => write!(f, "invalid pairing state: {m}"),
            IdentityError::RateLimited(m) => write!(f, "rate limited: {m}"),
            IdentityError::ProofInvalid(m) => write!(f, "proof invalid: {m}"),
            IdentityError::TranscriptMismatch => {
                write!(f, "the response does not match the expected transcript")
            }
            IdentityError::NotAuthorized(m) => write!(f, "not authorized: {m}"),
            IdentityError::DeviceRevoked(m) => write!(f, "device revoked: {m}"),
            IdentityError::UnknownDevice(m) => write!(f, "unknown device: {m}"),
            IdentityError::Storage(m) => write!(f, "secure storage error: {m}"),
            IdentityError::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for IdentityError {}

impl From<solayge_protocol::ProtocolError> for IdentityError {
    fn from(e: solayge_protocol::ProtocolError) -> Self {
        IdentityError::Internal(format!("protocol error: {e}"))
    }
}
