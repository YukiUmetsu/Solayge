//! The versioned, transport-neutral message envelope.
//!
//! One envelope is carried unchanged over Iroh QUIC streams or Tailscale
//! WebSockets. It is validated before it is encoded and after it is decoded, so
//! a malformed frame is rejected as close to the transport as possible.
//!
//! ```json
//! {
//!   "protocol_version": 1,
//!   "message_id": "msg_...",
//!   "kind": "command",
//!   "action": "sessions.send_prompt",
//!   "desktop_id": "desktop-public-id",
//!   "session_id": "session-123",
//!   "command_id": "cmd_...",
//!   "payload": { "text": "Run the tests and summarize failures." }
//! }
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::actions::Action;
use crate::error::ProtocolError;

/// The protocol version this crate implements. Bump only with a migration plan.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum size of an encoded frame (the whole JSON envelope).
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

/// Maximum serialized size of a single `payload`.
pub const MAX_PAYLOAD_BYTES: usize = 192 * 1024;

/// Maximum length of an identifier (`message_id`, `session_id`, `command_id`,
/// `desktop_id`).
pub const MAX_ID_LEN: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// A request that expects a [`MessageKind::Response`].
    Command,
    /// The reply to a command.
    Response,
    /// A pushed event (no reply).
    Event,
    /// A reply carrying a typed error. The payload is a [`crate::WireError`].
    Error,
}

impl MessageKind {
    pub const ALL: [MessageKind; 4] = [
        MessageKind::Command,
        MessageKind::Response,
        MessageKind::Event,
        MessageKind::Error,
    ];
}

static MESSAGE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A process-local unique message id. Deterministic tests set ids explicitly.
pub fn next_message_id() -> String {
    let n = MESSAGE_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("msg_{nanos:x}_{n:x}")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub protocol_version: u32,
    pub message_id: String,
    pub kind: MessageKind,
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl Envelope {
    /// A command frame with a fresh message id.
    pub fn command(action: Action, payload: serde_json::Value) -> Self {
        Envelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: next_message_id(),
            kind: MessageKind::Command,
            action,
            desktop_id: None,
            session_id: None,
            command_id: None,
            payload,
        }
    }

    /// An envelope with an explicit id, for deterministic tests and fixtures.
    pub fn with_message_id(mut self, message_id: impl Into<String>) -> Self {
        self.message_id = message_id.into();
        self
    }

    pub fn with_desktop_id(mut self, desktop_id: impl Into<String>) -> Self {
        self.desktop_id = Some(desktop_id.into());
        self
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_command_id(mut self, command_id: impl Into<String>) -> Self {
        self.command_id = Some(command_id.into());
        self
    }

    /// Store a typed payload, surfacing a serialization failure as invalid
    /// payload rather than panicking.
    pub fn with_typed_payload<T: Serialize>(mut self, payload: &T) -> Result<Self, ProtocolError> {
        self.payload = serde_json::to_value(payload)
            .map_err(|e| ProtocolError::invalid_payload(e.to_string()))?;
        Ok(self)
    }

    /// The reply to a command. The request's `message_id` is echoed so the
    /// requester can correlate the response; session/command ids are preserved.
    pub fn response_to(request: &Envelope, payload: serde_json::Value) -> Self {
        Envelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: request.message_id.clone(),
            kind: MessageKind::Response,
            action: request.action,
            desktop_id: request.desktop_id.clone(),
            session_id: request.session_id.clone(),
            command_id: request.command_id.clone(),
            payload,
        }
    }

    /// An error reply to a command, carrying a typed [`crate::WireError`].
    pub fn error_to(request: &Envelope, error: &ProtocolError) -> Self {
        let wire = crate::dto::WireError::from(error);
        let payload = serde_json::to_value(wire).unwrap_or(serde_json::Value::Null);
        Envelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: request.message_id.clone(),
            kind: MessageKind::Error,
            action: request.action,
            desktop_id: request.desktop_id.clone(),
            session_id: request.session_id.clone(),
            command_id: request.command_id.clone(),
            payload,
        }
    }

    /// Deserialize the payload into a typed DTO.
    pub fn payload_as<T: DeserializeOwned>(&self) -> Result<T, ProtocolError> {
        serde_json::from_value(self.payload.clone()).map_err(|e| {
            ProtocolError::invalid_payload(format!(
                "payload does not match {}: {e}",
                std::any::type_name::<T>()
            ))
        })
    }

    /// Validate structure, version, required fields, and size limits.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::unsupported_version(self.protocol_version));
        }
        validate_id("message_id", &self.message_id)?;
        if let Some(id) = &self.desktop_id {
            validate_id("desktop_id", id)?;
        }
        if let Some(id) = &self.session_id {
            validate_id("session_id", id)?;
        }
        if let Some(id) = &self.command_id {
            validate_id("command_id", id)?;
        }

        if self.action.requires_session_id() && self.session_id.as_deref().unwrap_or("").is_empty()
        {
            return Err(ProtocolError::missing_field("session_id"));
        }
        if self.action.is_mutating() && self.command_id.as_deref().unwrap_or("").is_empty() {
            return Err(ProtocolError::missing_field("command_id"));
        }

        match &self.payload {
            serde_json::Value::Object(_) | serde_json::Value::Null => {}
            _ => {
                return Err(ProtocolError::invalid_payload(
                    "payload must be a JSON object or null",
                ))
            }
        }
        let size = serde_json::to_vec(&self.payload)
            .map_err(|e| ProtocolError::invalid_payload(e.to_string()))?
            .len();
        if size > MAX_PAYLOAD_BYTES {
            return Err(ProtocolError::payload_too_large(size, MAX_PAYLOAD_BYTES));
        }
        Ok(())
    }

    /// Validate, then serialize to bytes, enforcing the frame limit.
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(ProtocolError::frame_too_large(bytes.len(), MAX_FRAME_BYTES));
        }
        Ok(bytes)
    }

    /// Decode and validate a frame, enforcing the frame limit before parsing so
    /// an oversized payload is rejected without allocating for it.
    pub fn decode(bytes: &[u8]) -> Result<Envelope, ProtocolError> {
        Self::decode_with_limit(bytes, MAX_FRAME_BYTES)
    }

    pub fn decode_with_limit(bytes: &[u8], max_frame: usize) -> Result<Envelope, ProtocolError> {
        if bytes.len() > max_frame {
            return Err(ProtocolError::frame_too_large(bytes.len(), max_frame));
        }
        let envelope: Envelope = match serde_json::from_slice(bytes) {
            Ok(envelope) => envelope,
            Err(error) => {
                // Distinguish a structurally valid frame that names an action
                // this protocol version does not implement from a malformed
                // frame. The former gets the typed `unknown_action` code so a
                // peer can tell "upgrade/negotiate" apart from "bad frame".
                if let Some(action) = unknown_action_in(bytes) {
                    return Err(ProtocolError::unknown_action(&action));
                }
                return Err(error.into());
            }
        };
        envelope.validate()?;
        Ok(envelope)
    }

    /// Convert this envelope into an error frame with the same identity.
    pub fn into_error(self, error: &ProtocolError) -> Envelope {
        Envelope::error_to(&self, error)
    }
}

fn validate_id(field: &str, value: &str) -> Result<(), ProtocolError> {
    if value.is_empty() {
        return Err(ProtocolError::missing_field(field));
    }
    if value.len() > MAX_ID_LEN {
        return Err(ProtocolError::invalid_payload(format!(
            "{field} exceeds the {MAX_ID_LEN}-character limit"
        )));
    }
    Ok(())
}

/// Inspect a frame that failed to deserialize for an `action` string that is not
/// part of this protocol version. Returns the offending action when found.
///
/// This only runs on the error path, and only over bytes already bounded by the
/// frame limit, so a malformed frame never costs more than one extra parse.
fn unknown_action_in(bytes: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let action = value.get("action")?.as_str()?;
    Action::parse(action).is_none().then(|| action.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_command() -> Envelope {
        Envelope::command(Action::SessionsSendPrompt, json!({ "text": "run tests" }))
            .with_message_id("msg_fixed_1")
            .with_session_id("ses_1")
            .with_command_id("cmd_fixed_1")
    }

    #[test]
    fn a_valid_command_round_trips_byte_for_byte() {
        let env = valid_command();
        let bytes = env.encode().expect("valid frame");
        let decoded = Envelope::decode(&bytes).expect("decodes");
        assert_eq!(decoded, env);
    }

    #[test]
    fn the_wire_shape_matches_the_spec_example() {
        let env = valid_command();
        let value = serde_json::to_value(&env).unwrap();
        assert_eq!(value["protocol_version"], json!(1));
        assert_eq!(value["message_id"], json!("msg_fixed_1"));
        assert_eq!(value["kind"], json!("command"));
        assert_eq!(value["action"], json!("sessions.send_prompt"));
        assert_eq!(value["session_id"], json!("ses_1"));
        assert_eq!(value["command_id"], json!("cmd_fixed_1"));
        assert_eq!(value["payload"]["text"], json!("run tests"));
    }

    #[test]
    fn malformed_json_is_rejected() {
        assert!(Envelope::decode(b"{not json").is_err());
        assert!(Envelope::decode(b"").is_err());
        assert!(Envelope::decode(b"[]").is_err());
    }

    #[test]
    fn missing_required_fields_are_rejected() {
        // No protocol_version.
        let err = Envelope::decode(
            br#"{"message_id":"m","kind":"command","action":"connection.health","payload":{}}"#,
        )
        .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::InvalidFrame);

        // Empty message_id.
        let err = Envelope::decode(
            br#"{"protocol_version":1,"message_id":"","kind":"command","action":"connection.health","payload":{}}"#,
        )
        .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::MissingField);

        // Missing command_id on a mutating action.
        let err = Envelope::decode(
            br#"{"protocol_version":1,"message_id":"m","kind":"command","action":"sessions.send_prompt","session_id":"s","payload":{"text":"x"}}"#,
        )
        .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::MissingField);

        // Missing session_id where required.
        let err = Envelope::decode(
            br#"{"protocol_version":1,"message_id":"m","kind":"command","action":"sessions.get","command_id":"c","payload":{}}"#,
        )
        .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::MissingField);
    }

    #[test]
    fn unknown_actions_are_rejected_at_decode() {
        let err = Envelope::decode(
            br#"{"protocol_version":1,"message_id":"m","kind":"command","action":"sessions.delete_all","payload":{}}"#,
        )
        .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::UnknownAction);

        // A known action on an otherwise malformed frame is still a bad frame,
        // not an unsupported action.
        let err = Envelope::decode(br#"{"protocol_version":1,"action":"connection.health"}"#)
            .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::InvalidFrame);
    }

    #[test]
    fn unsupported_versions_are_rejected() {
        let err = Envelope::decode(
            br#"{"protocol_version":999,"message_id":"m","kind":"command","action":"connection.health","payload":{}}"#,
        )
        .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::UnsupportedVersion);
    }

    #[test]
    fn non_object_payloads_are_rejected() {
        let err = Envelope::decode(
            br#"{"protocol_version":1,"message_id":"m","kind":"command","action":"connection.health","payload":"nope"}"#,
        )
        .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::InvalidPayload);
    }

    #[test]
    fn oversized_frames_are_rejected_before_parsing() {
        let bytes = vec![b' '; MAX_FRAME_BYTES + 1];
        let err = Envelope::decode(&bytes).unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::FrameTooLarge);
    }

    #[test]
    fn oversized_payloads_are_rejected() {
        let big = "x".repeat(MAX_PAYLOAD_BYTES + 1);
        let env = Envelope::command(Action::SessionsSendPrompt, json!({ "text": big }))
            .with_session_id("ses")
            .with_command_id("cmd");
        let err = env.encode().unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::PayloadTooLarge);
    }

    #[test]
    fn overlong_ids_are_rejected() {
        let env =
            Envelope::command(Action::ConnectionHealth, json!({})).with_message_id("m".repeat(129));
        assert_eq!(
            env.validate().unwrap_err().code(),
            crate::ErrorCode::InvalidPayload
        );
    }

    #[test]
    fn responses_echo_the_request_message_id() {
        let request = valid_command();
        let response = Envelope::response_to(&request, json!({ "status": "accepted" }));
        assert_eq!(response.kind, MessageKind::Response);
        assert_eq!(response.message_id, request.message_id);
        assert_eq!(response.command_id, request.command_id);
        assert_eq!(response.session_id, request.session_id);
    }

    #[test]
    fn payload_as_surfaces_a_typed_error() {
        let env = valid_command();
        let payload: crate::dto::SendPromptPayload = env.payload_as().unwrap();
        assert_eq!(payload.text, "run tests");

        let err = env
            .payload_as::<crate::dto::ConnectionHealth>()
            .unwrap_err();
        assert_eq!(err.code(), crate::ErrorCode::InvalidPayload);
    }

    #[test]
    fn generated_message_ids_are_unique() {
        let a = next_message_id();
        let b = next_message_id();
        assert_ne!(a, b);
    }
}
