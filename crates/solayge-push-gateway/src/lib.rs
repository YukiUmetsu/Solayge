//! The Solayge push gateway.
//!
//! A desktop host posts a *minimal* alert here when an agent needs attention.
//! The gateway authenticates the desktop, maps the alert to a generic FCM
//! message, and sends it. It is deliberately small and holds the only copy of
//! the Firebase service-account credential; distributed apps never contain it.
//!
//! Privacy: the request carries only opaque identifiers (`event_id`,
//! `desktop_id`, `session_id`, `approval_id`) and an event kind. It never
//! carries prompts, code, credentials, tool output, or message content. The
//! phone, on receiving a push, reconnects to the desktop over Iroh/Tailscale to
//! read the real content.
#![deny(unsafe_code)]

pub mod fcm;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The kind of agent event a push announces. The gateway derives the priority
/// from this rather than trusting a client-supplied flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushEventKind {
    TaskCompleted,
    TaskFailed,
    Question,
    Approval,
}

impl PushEventKind {
    /// Only events that are actively waiting on the user interrupt at high
    /// priority; a finished task can wait for the next unlock.
    pub fn is_time_sensitive(self) -> bool {
        matches!(self, PushEventKind::Question | PushEventKind::Approval)
    }

    /// A generic, non-sensitive notification title.
    pub fn title(self) -> &'static str {
        match self {
            PushEventKind::TaskCompleted => "Task completed",
            PushEventKind::TaskFailed => "Task failed",
            PushEventKind::Question => "Agent has a question",
            PushEventKind::Approval => "Permission needed",
        }
    }

    /// A generic, non-sensitive notification body.
    pub fn body(self) -> &'static str {
        match self {
            PushEventKind::TaskCompleted | PushEventKind::TaskFailed => "Open Solayge for details.",
            PushEventKind::Question => "Open Solayge to answer.",
            PushEventKind::Approval => "Open Solayge to review.",
        }
    }
}

/// One alert submitted by a desktop host. Identifiers only, no content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushEvent {
    /// Stable id for this event, used to deduplicate deliveries on the phone.
    pub event_id: String,
    pub kind: PushEventKind,
    pub desktop_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub approval_id: Option<String>,
}

/// The request body a desktop posts to `POST /v1/push`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRequest {
    /// The destination device's FCM registration token.
    pub token: String,
    pub event: PushEvent,
}

/// A ready-to-send FCM message. `data` is always string-valued, as FCM requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FcmMessage {
    pub token: String,
    pub title: String,
    pub body: String,
    pub high_priority: bool,
    pub data: BTreeMap<String, String>,
}

impl FcmMessage {
    /// The Android delivery channel. It matches the channel the app creates.
    pub const CHANNEL_ID: &'static str = "solayge";

    /// The OS-level collapse tag: re-delivering the same event replaces the
    /// previous notification instead of stacking a duplicate.
    pub fn tag(&self) -> &str {
        self.data
            .get("event_id")
            .map(String::as_str)
            .unwrap_or("solayge")
    }
}

/// Sends a built message. Implemented by the real FCM client and by tests.
pub trait FcmSender: Send + Sync {
    fn send(&self, message: &FcmMessage) -> Result<(), String>;
}

/// Map a validated request onto the FCM message. Pure and content-free.
pub fn build_message(request: &PushRequest) -> FcmMessage {
    let event = &request.event;
    let mut data = BTreeMap::new();
    data.insert("event_id".to_string(), event.event_id.clone());
    data.insert("kind".to_string(), kind_wire(event.kind).to_string());
    data.insert("desktop_id".to_string(), event.desktop_id.clone());
    if let Some(session_id) = event.session_id.as_ref().filter(|id| !id.is_empty()) {
        data.insert("session_id".to_string(), session_id.clone());
    }
    if let Some(approval_id) = event.approval_id.as_ref().filter(|id| !id.is_empty()) {
        data.insert("approval_id".to_string(), approval_id.clone());
    }
    FcmMessage {
        token: request.token.clone(),
        title: event.kind.title().to_string(),
        body: event.kind.body().to_string(),
        high_priority: event.kind.is_time_sensitive(),
        data,
    }
}

fn kind_wire(kind: PushEventKind) -> &'static str {
    match kind {
        PushEventKind::TaskCompleted => "task_completed",
        PushEventKind::TaskFailed => "task_failed",
        PushEventKind::Question => "question",
        PushEventKind::Approval => "approval",
    }
}

/// Validate a request before it reaches FCM. Identifiers must be present and
/// bounded; the gateway never forwards an oversized or empty payload.
pub fn validate(request: &PushRequest) -> Result<(), String> {
    if request.token.trim().is_empty() {
        return Err("missing token".into());
    }
    if request.event.event_id.trim().is_empty() {
        return Err("missing event_id".into());
    }
    if request.event.desktop_id.trim().is_empty() {
        return Err("missing desktop_id".into());
    }
    // A push is an addressing hint, not a content channel: refuse anything that
    // looks like it is smuggling free text.
    for value in [
        request.event.event_id.as_str(),
        request.event.desktop_id.as_str(),
        request.event.session_id.as_deref().unwrap_or(""),
        request.event.approval_id.as_deref().unwrap_or(""),
    ] {
        if value.len() > 256 {
            return Err("identifier too long".into());
        }
        if value.chars().any(|c| c == '\n' || c == '\r') {
            return Err("identifier contains a newline".into());
        }
    }
    Ok(())
}

/// Constant-time string comparison, so a wrong bearer token cannot be recovered
/// byte-by-byte from response timing.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Extract the token from an `Authorization: Bearer …` header.
pub fn bearer_token(header: Option<&str>) -> Option<&str> {
    let header = header?;
    header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
}

/// The outcome of handling one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayResponse {
    pub status: u16,
    pub body: String,
}

impl GatewayResponse {
    fn json(status: u16, body: &str) -> Self {
        GatewayResponse {
            status,
            body: body.to_string(),
        }
    }
}

/// Handle one `POST /v1/push` body. `expected_token` is the gateway's own
/// bearer secret; the desktop must present it.
pub fn handle_push(
    body: &[u8],
    authorization: Option<&str>,
    expected_token: &str,
    sender: &dyn FcmSender,
) -> GatewayResponse {
    if expected_token.trim().is_empty() {
        return GatewayResponse::json(500, "{\"error\":\"gateway is not configured\"}");
    }
    match bearer_token(authorization) {
        Some(token) if constant_time_eq(token, expected_token) => {}
        _ => return GatewayResponse::json(401, "{\"error\":\"unauthorized\"}"),
    }
    let request: PushRequest = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(_) => return GatewayResponse::json(400, "{\"error\":\"invalid payload\"}"),
    };
    if let Err(message) = validate(&request) {
        return GatewayResponse::json(400, &format!("{{\"error\":\"{message}\"}}"));
    }
    let message = build_message(&request);
    match sender.send(&message) {
        Ok(()) => GatewayResponse::json(202, "{\"delivered\":true}"),
        Err(_) => GatewayResponse::json(502, "{\"error\":\"delivery failed\"}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSender {
        messages: Mutex<Vec<FcmMessage>>,
        fail: bool,
    }

    impl FcmSender for RecordingSender {
        fn send(&self, message: &FcmMessage) -> Result<(), String> {
            if self.fail {
                return Err("nope".into());
            }
            self.messages
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(message.clone());
            Ok(())
        }
    }

    fn request(kind: PushEventKind) -> PushRequest {
        PushRequest {
            token: "fcm-token".into(),
            event: PushEvent {
                event_id: "evt_1".into(),
                kind,
                desktop_id: "desk_1".into(),
                session_id: Some("ses_1".into()),
                approval_id: None,
            },
        }
    }

    #[test]
    fn message_carries_only_opaque_identifiers() {
        let message = build_message(&request(PushEventKind::Question));
        assert_eq!(message.title, "Agent has a question");
        assert_eq!(message.data.get("session_id").unwrap(), "ses_1");
        assert_eq!(message.data.get("desktop_id").unwrap(), "desk_1");
        assert_eq!(message.data.get("kind").unwrap(), "question");
        // No content fields exist at all.
        assert!(!message.data.contains_key("text"));
        assert!(!message.data.contains_key("command"));
        assert!(!message.data.contains_key("title"));
    }

    #[test]
    fn only_waiting_events_are_high_priority() {
        assert!(build_message(&request(PushEventKind::Question)).high_priority);
        assert!(build_message(&request(PushEventKind::Approval)).high_priority);
        assert!(!build_message(&request(PushEventKind::TaskCompleted)).high_priority);
        assert!(!build_message(&request(PushEventKind::TaskFailed)).high_priority);
    }

    #[test]
    fn validate_rejects_missing_and_smuggled_content() {
        let mut missing = request(PushEventKind::Question);
        missing.token = "  ".into();
        assert!(validate(&missing).is_err());

        let mut newline = request(PushEventKind::Question);
        newline.event.session_id = Some("ses_1\nsecret".into());
        assert!(validate(&newline).is_err());

        let mut huge = request(PushEventKind::Question);
        huge.event.desktop_id = "x".repeat(300);
        assert!(validate(&huge).is_err());

        assert!(validate(&request(PushEventKind::Question)).is_ok());
    }

    #[test]
    fn bearer_tokens_are_compared_constant_time() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert_eq!(bearer_token(Some("Bearer secret")), Some("secret"));
        assert_eq!(bearer_token(Some("Basic nope")), None);
        assert_eq!(bearer_token(None), None);
    }

    #[test]
    fn authorized_requests_are_forwarded_and_unauthorized_are_not() {
        let sender = RecordingSender::default();
        let body = serde_json::to_vec(&request(PushEventKind::Approval)).unwrap();

        let unauthorized = handle_push(&body, Some("Bearer wrong"), "secret", &sender);
        assert_eq!(unauthorized.status, 401);
        assert!(sender.messages.lock().unwrap().is_empty());

        let ok = handle_push(&body, Some("Bearer secret"), "secret", &sender);
        assert_eq!(ok.status, 202);
        let sent = sender.messages.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].data.get("event_id").unwrap(), "evt_1");
    }

    #[test]
    fn malformed_and_oversized_bodies_are_rejected() {
        let sender = RecordingSender::default();
        assert_eq!(
            handle_push(b"not json", Some("Bearer secret"), "secret", &sender).status,
            400
        );
        let mut missing = request(PushEventKind::TaskCompleted);
        missing.event.desktop_id = String::new();
        let body = serde_json::to_vec(&missing).unwrap();
        assert_eq!(
            handle_push(&body, Some("Bearer secret"), "secret", &sender).status,
            400
        );
    }

    #[test]
    fn a_delivery_failure_is_reported_not_swallowed() {
        let sender = RecordingSender {
            messages: Mutex::new(Vec::new()),
            fail: true,
        };
        let body = serde_json::to_vec(&request(PushEventKind::TaskFailed)).unwrap();
        assert_eq!(
            handle_push(&body, Some("Bearer secret"), "secret", &sender).status,
            502
        );
    }

    #[test]
    fn an_unconfigured_gateway_refuses_everything() {
        let sender = RecordingSender::default();
        let body = serde_json::to_vec(&request(PushEventKind::TaskCompleted)).unwrap();
        assert_eq!(
            handle_push(&body, Some("Bearer x"), "", &sender).status,
            500
        );
    }
}
