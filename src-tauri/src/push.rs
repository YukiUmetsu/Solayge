//! Desktop → push-gateway dispatch.
//!
//! When an agent event needs the user's attention, the desktop posts a *minimal*
//! alert to the configured gateway, which relays it to the phone over FCM. The
//! payload carries only opaque identifiers — never prompts, code, tool output,
//! or credentials. The phone reconnects over Iroh/Tailscale to read the real
//! content.
//!
//! Configuration is environment-only, so no gateway secret is compiled into the
//! distributed app:
//!
//! * `SOLAYGE_PUSH_GATEWAY_URL` — e.g. `https://push.example.com/v1/push`.
//! * `SOLAYGE_PUSH_GATEWAY_TOKEN` — the bearer secret the gateway issued.
//!
//! With either unset, push dispatch is a no-op and local notifications still
//! work.

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::devices::DeviceManager;
use crate::models::NotifyKind;

/// One alert to deliver. Identifiers only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushEvent {
    pub event_id: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct GatewayRequest<'a> {
    token: &'a str,
    event: &'a PushEvent,
}

/// Sends one request to the gateway. Implemented for real HTTP and for tests.
pub trait PushTransport: Send + Sync {
    fn send(&self, url: &str, bearer: &str, payload: &Value) -> Result<(), String>;
}

/// The production transport: a blocking HTTPS POST with a bounded timeout.
pub struct HttpPushTransport;

impl PushTransport for HttpPushTransport {
    fn send(&self, url: &str, bearer: &str, payload: &Value) -> Result<(), String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
            .post(url)
            .bearer_auth(bearer)
            .json(payload)
            .send()
            .map_err(|e| e.to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("gateway returned {}", response.status()))
        }
    }
}

/// Map a desktop notification kind to a push event kind. `System` events are
/// local-only and never pushed.
pub fn push_kind(kind: NotifyKind) -> Option<&'static str> {
    match kind {
        NotifyKind::TaskComplete => Some("task_completed"),
        NotifyKind::TaskFailed => Some("task_failed"),
        NotifyKind::TaskReview => Some("question"),
        NotifyKind::NeedsAttention => Some("approval"),
        NotifyKind::System => None,
    }
}

/// The push event kind for an agent ask. `NotifyKind::NeedsAttention` covers both
/// questions and permissions, so the ask kind disambiguates the two.
pub fn ask_push_kind(kind: crate::models::AskKind) -> &'static str {
    match kind {
        crate::models::AskKind::Question => "question",
        crate::models::AskKind::Permission => "approval",
    }
}

/// The canonical event id. It is stable for a given logical event and contains
/// only opaque identifiers, so the phone can deduplicate re-deliveries and the
/// local and pushed copies of the same event.
pub fn canonical_event_id(
    desktop_id: &str,
    kind: &str,
    session_id: Option<&str>,
    approval_id: Option<&str>,
) -> String {
    if let Some(approval_id) = approval_id.filter(|id| !id.is_empty()) {
        format!("{desktop_id}:ap:{approval_id}")
    } else if let Some(session_id) = session_id.filter(|id| !id.is_empty()) {
        format!("{desktop_id}:se:{session_id}:{kind}")
    } else {
        format!("{desktop_id}:{kind}")
    }
}

fn gateway_config() -> Option<(String, String)> {
    let url = std::env::var("SOLAYGE_PUSH_GATEWAY_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let bearer = std::env::var("SOLAYGE_PUSH_GATEWAY_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    Some((url, bearer))
}

/// Deliver one event to every authorized device's token. Kept separate from the
/// app handle so it is unit-testable with a fake transport.
pub fn deliver(
    transport: &dyn PushTransport,
    url: &str,
    bearer: &str,
    targets: &[String],
    event: &PushEvent,
) {
    for token in targets {
        let payload = serde_json::to_value(GatewayRequest { token, event })
            .unwrap_or(Value::Null);
        // A push failure is never fatal: the phone still syncs on reconnect.
        let _ = transport.send(url, bearer, &payload);
    }
}

/// Dispatch a push with an explicit kind. Used by agent asks, where
/// `NotifyKind::NeedsAttention` alone cannot tell a question from a permission.
pub(crate) fn dispatch_kind(app: &AppHandle, push_kind: &'static str, session_id: Option<&str>) {
    let Some((url, bearer)) = gateway_config() else {
        return;
    };
    let manager = app.state::<DeviceManager>();
    let targets: Vec<String> = manager
        .push_targets()
        .into_iter()
        .map(|(_, token)| token.token)
        .collect();
    if targets.is_empty() {
        return;
    }
    let desktop_id = manager.desktop_id_string().unwrap_or_default();
    let event = PushEvent {
        event_id: canonical_event_id(&desktop_id, push_kind, session_id, None),
        kind: push_kind,
        session_id: session_id.map(str::to_string),
        approval_id: None,
    };
    tauri::async_runtime::spawn_blocking(move || {
        deliver(&HttpPushTransport, &url, &bearer, &targets, &event);
    });
}

// ============================================================================
// TEMPORARY DEV-ONLY TEST HOOK — remove this block before release.
//
// It fires a synthetic `question` background push to every paired phone so the
// FCM path (gateway → FCM → Android notification → tap deep-link) can be
// exercised on demand, without waiting for a real agent to ask something. It
// deliberately bypasses the desktop's local notification settings.
// ============================================================================

/// TEMPORARY: send a synthetic `question` push to every paired phone.
///
/// Each call uses a unique event id so repeated tests are not swallowed by the
/// phone's deduplication. Returns a human-readable summary; the error describes
/// the first failure so Settings can show something actionable.
pub(crate) fn dispatch_test_question(
    app: &AppHandle,
    session_id: Option<String>,
) -> Result<String, String> {
    let (url, bearer) = gateway_config().ok_or_else(|| {
        "Set SOLAYGE_PUSH_GATEWAY_URL and SOLAYGE_PUSH_GATEWAY_TOKEN, then restart Solayge"
            .to_string()
    })?;
    let manager = app.state::<DeviceManager>();
    let targets: Vec<String> = manager
        .push_targets()
        .into_iter()
        .map(|(_, token)| token.token)
        .collect();
    if targets.is_empty() {
        return Err(
            "No paired phone has registered a push token yet. Enable notifications on the \
             phone and reconnect to this desktop."
                .to_string(),
        );
    }
    let desktop_id = manager.desktop_id_string().unwrap_or_default();
    let session = session_id.filter(|id| !id.trim().is_empty());
    // A unique suffix keeps every test distinct for the phone's seen-id set,
    // unlike real events whose stable id is what deduplicates redeliveries.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let event = PushEvent {
        event_id: format!("{desktop_id}:test:question:{stamp}"),
        kind: "question",
        session_id: session,
        approval_id: None,
    };

    let transport = HttpPushTransport;
    let mut delivered = 0usize;
    let mut first_error: Option<String> = None;
    for token in &targets {
        let payload = serde_json::to_value(GatewayRequest {
            token: token.as_str(),
            event: &event,
        })
        .unwrap_or(Value::Null);
        match transport.send(&url, &bearer, &payload) {
            Ok(()) => delivered += 1,
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    match first_error {
        None => Ok(format!(
            "Sent a test question push to {delivered} device(s). Background the phone and \
             watch for the notification."
        )),
        Some(error) if delivered == 0 => Err(format!("Test push failed: {error}")),
        Some(error) => Ok(format!(
            "Sent to {delivered} of {} device(s); first error: {error}",
            targets.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingTransport {
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl PushTransport for RecordingTransport {
        fn send(&self, _url: &str, _bearer: &str, payload: &Value) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((String::new(), payload.clone()));
            Ok(())
        }
    }

    #[test]
    fn system_events_are_never_pushed() {
        assert_eq!(push_kind(NotifyKind::System), None);
        assert_eq!(push_kind(NotifyKind::TaskComplete), Some("task_completed"));
        assert_eq!(push_kind(NotifyKind::TaskFailed), Some("task_failed"));
        assert_eq!(push_kind(NotifyKind::NeedsAttention), Some("approval"));
    }

    #[test]
    fn agent_asks_map_to_question_versus_approval() {
        assert_eq!(
            ask_push_kind(crate::models::AskKind::Question),
            "question"
        );
        assert_eq!(
            ask_push_kind(crate::models::AskKind::Permission),
            "approval"
        );
    }

    #[test]
    fn event_ids_are_stable_and_opaque() {
        assert_eq!(
            canonical_event_id("desk", "approval", Some("ses"), Some("apr")),
            "desk:ap:apr"
        );
        assert_eq!(
            canonical_event_id("desk", "task_completed", Some("ses"), None),
            "desk:se:ses:task_completed"
        );
        assert_eq!(canonical_event_id("desk", "task_failed", None, None), "desk:task_failed");
    }

    #[test]
    fn deliver_posts_a_minimal_payload_to_every_target() {
        let transport = RecordingTransport::default();
        let event = PushEvent {
            event_id: "desk:se:ses:task_completed".into(),
            kind: "task_completed",
            session_id: Some("ses".into()),
            approval_id: None,
        };
        deliver(
            &transport,
            "https://gateway/v1/push",
            "secret",
            &["token_a".to_string(), "token_b".to_string()],
            &event,
        );
        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let payload = &calls[0].1;
        assert_eq!(payload["event"]["event_id"], "desk:se:ses:task_completed");
        assert_eq!(payload["event"]["session_id"], "ses");
        // No content keys are ever present.
        let serialized = payload.to_string();
        assert!(!serialized.contains("prompt"));
        assert!(!serialized.contains("text"));
        assert!(!serialized.contains("command"));
    }
}
