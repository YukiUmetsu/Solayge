//! Desktop → push-gateway dispatch.
//!
//! When an agent event needs the user's attention, the desktop posts a *minimal*
//! alert to the configured gateway, which relays it to the phone over FCM. The
//! payload carries only opaque identifiers — never prompts, code, tool output,
//! or credentials. The phone reconnects over Iroh/Tailscale to read the real
//! content.
//!
//! The gateway is normally embedded: the desktop runs its own loopback gateway
//! when the user configures a Firebase service account (see
//! [`crate::push_gateway`]), so background alerts work without any external
//! process. Two explicit overrides exist for advanced setups:
//!
//! * `SOLAYGE_PUSH_GATEWAY_URL` / `SOLAYGE_PUSH_GATEWAY_TOKEN` — a remote or
//!   shared gateway (environment, so no secret is compiled in).
//! * `push-dev.json` — a manual loopback override written by
//!   `pnpm push:gateway` for development.
//!
//! Precedence is environment, then `push-dev.json`, then the embedded gateway.
//! With none configured, push dispatch is a no-op and local notifications still
//! work.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::devices::{DeviceManager, PushToken};
use crate::models::NotifyKind;

/// A manual, loopback gateway override written by `pnpm push:gateway`. It is
/// not created by the app; production installs use the embedded gateway.
const DEV_CONFIG_FILE: &str = "push-dev.json";

/// One alert to deliver. Identifiers only.
///
/// `desktop_id` is part of the gateway's contract (and of the phone's FCM
/// `data`), so it is always sent — the gateway rejects a payload without it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushEvent {
    pub event_id: String,
    pub kind: &'static str,
    pub desktop_id: String,
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

/// A push delivery failure. `unregistered` is the only recoverable case: the
/// gateway reported the device's FCM token is dead, so the desktop forgets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushError {
    pub unregistered: bool,
    pub message: String,
}

impl PushError {
    fn other(message: impl Into<String>) -> Self {
        PushError {
            unregistered: false,
            message: message.into(),
        }
    }

    fn unregistered(message: impl Into<String>) -> Self {
        PushError {
            unregistered: true,
            message: message.into(),
        }
    }
}

/// Sends one request to the gateway. Implemented for real HTTP and for tests.
pub trait PushTransport: Send + Sync {
    fn send(&self, url: &str, bearer: &str, payload: &Value) -> Result<(), PushError>;
}

/// The production transport: a blocking HTTP(S) POST with a bounded timeout.
pub struct HttpPushTransport;

impl PushTransport for HttpPushTransport {
    fn send(&self, url: &str, bearer: &str, payload: &Value) -> Result<(), PushError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| PushError::other(e.to_string()))?;
        let response = client
            .post(url)
            .bearer_auth(bearer)
            .json(payload)
            .send()
            .map_err(|e| PushError::other(e.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        // `410 Gone` means the gateway reported the token is no longer
        // registered. Treat it as terminal for this device, not a retry.
        if status.as_u16() == 410 {
            return Err(PushError::unregistered(format!(
                "gateway returned {status}"
            )));
        }
        // Surface the gateway's small JSON error (for example a shape hint)
        // so Settings can show something actionable. The gateway returns
        // only fixed error strings, and this is length-capped regardless.
        let detail = response
            .text()
            .unwrap_or_default()
            .trim()
            .chars()
            .take(300)
            .collect::<String>();
        if detail.is_empty() {
            Err(PushError::other(format!("gateway returned {status}")))
        } else {
            Err(PushError::other(format!(
                "gateway returned {status}: {detail}"
            )))
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

fn env_gateway_config() -> Option<(String, String)> {
    let url = std::env::var("SOLAYGE_PUSH_GATEWAY_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let bearer = std::env::var("SOLAYGE_PUSH_GATEWAY_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    Some((url, bearer))
}

/// A manual `push-dev.json` override (only created by `pnpm push:gateway`).
/// `token` is a bearer secret; never log it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DevPushConfig {
    url: String,
    token: String,
}

fn app_data_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok()
}

fn dev_config_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DEV_CONFIG_FILE)
}

fn read_dev_config(data_dir: &Path) -> Option<DevPushConfig> {
    let text = std::fs::read_to_string(dev_config_path(data_dir)).ok()?;
    let config: DevPushConfig = serde_json::from_str(&text).ok()?;
    if config.url.trim().is_empty() || config.token.is_empty() {
        return None;
    }
    Some(config)
}

/// The gateway to dispatch to. Precedence: process environment (a remote
/// gateway), then a manual `push-dev.json`, then the embedded gateway the
/// desktop runs itself. A normal install therefore uses the embedded gateway
/// with no environment configuration at all.
fn app_gateway_config(app: &AppHandle) -> Option<(String, String)> {
    if let Some(env) = env_gateway_config() {
        return Some(env);
    }
    if let Some(config) = app_data_dir(app).as_deref().and_then(read_dev_config) {
        return Some((config.url, config.token));
    }
    app.try_state::<crate::push_gateway::PushGatewayManager>()
        .and_then(|manager| manager.local_config())
}

/// Derive the gateway's `/healthz` URL from the configured push endpoint.
pub(crate) fn health_url(push_url: &str) -> Option<String> {
    let url = push_url.trim();
    let scheme_end = url.find("://")?;
    let after = &url[scheme_end + 3..];
    let authority_end = after.find('/').unwrap_or(after.len());
    let authority = &after[..authority_end];
    if authority.is_empty() {
        return None;
    }
    Some(format!("{}://{}/healthz", &url[..scheme_end], authority))
}

/// Best-effort reachability check against the gateway's `/healthz`.
pub(crate) fn check_gateway(app: &AppHandle) -> Result<String, String> {
    let Some((url, _token)) = app_gateway_config(app) else {
        return Err("No push gateway is configured yet".to_string());
    };
    let health = health_url(&url).ok_or_else(|| "The gateway URL is invalid".to_string())?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(&health)
        .send()
        .map_err(|e| format!("Could not reach {health}: {e}"))?;
    if response.status().is_success() {
        Ok(format!("Gateway reachable at {health} ({})", response.status()))
    } else {
        Err(format!("Gateway answered {} at {health}", response.status()))
    }
}

/// Deliver one event to every authorized device that has opted into this kind.
///
/// Returns the device ids whose FCM token the gateway reported as dead
/// (`410 Gone`), so the caller can forget them and stop retrying. Kept separate
/// from the app handle so it is unit-testable with a fake transport.
pub fn deliver(
    transport: &dyn PushTransport,
    url: &str,
    bearer: &str,
    targets: &[(String, PushToken)],
    event: &PushEvent,
) -> Vec<String> {
    let mut unregistered = Vec::new();
    for (device_id, token) in targets {
        // The phone's per-kind settings are authoritative for background pushes.
        if !token.allows(event.kind) {
            continue;
        }
        let payload = serde_json::to_value(GatewayRequest {
            token: &token.token,
            event,
        })
        .unwrap_or(Value::Null);
        // A push failure is never fatal: the phone still syncs on reconnect.
        if let Err(error) = transport.send(url, bearer, &payload) {
            if error.unregistered {
                unregistered.push(device_id.clone());
            }
        }
    }
    unregistered
}

/// Forget the tokens the gateway reported as unregistered.
fn forget_unregistered(app: &AppHandle, device_ids: &[String]) {
    if device_ids.is_empty() {
        return;
    }
    let Some(manager) = app.try_state::<DeviceManager>() else {
        return;
    };
    for device_id in device_ids {
        let _ = manager.clear_push_token(device_id);
    }
}

/// Dispatch a push with an explicit kind. Used by agent asks, where
/// `NotifyKind::NeedsAttention` alone cannot tell a question from a permission.
pub(crate) fn dispatch_kind(app: &AppHandle, push_kind: &'static str, session_id: Option<&str>) {
    let Some((url, bearer)) = app_gateway_config(app) else {
        return;
    };
    let manager = app.state::<DeviceManager>();
    let targets = manager.push_targets();
    if targets.is_empty() {
        return;
    }
    let desktop_id = manager.desktop_id_string().unwrap_or_default();
    let event = PushEvent {
        event_id: canonical_event_id(&desktop_id, push_kind, session_id, None),
        kind: push_kind,
        desktop_id: desktop_id.clone(),
        session_id: session_id.map(str::to_string),
        approval_id: None,
    };
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dead = deliver(&HttpPushTransport, &url, &bearer, &targets, &event);
        forget_unregistered(&app, &dead);
    });
}

/// Send a synthetic `question` push to every paired phone.
///
/// This exercises the exact production path (gateway bearer auth → FCM HTTP v1
/// → Android notification with a tap deep-link) on demand, without waiting for
/// an agent to ask. Each call uses a unique event id so it is not swallowed by
/// the phone's deduplication; it deliberately bypasses the per-kind settings.
/// Returns a human-readable summary, or the first error so Settings can show
/// something actionable.
pub(crate) fn dispatch_test_question(
    app: &AppHandle,
    session_id: Option<String>,
) -> Result<String, String> {
    let (url, bearer) = app_gateway_config(app).ok_or_else(|| {
        "No push gateway is running. Add a Firebase service account under \
         Settings → Devices → Background push notifications, or start one with \
         `pnpm push:gateway`."
            .to_string()
    })?;
    let manager = app.state::<DeviceManager>();
    let targets = manager.push_targets();
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
        desktop_id: desktop_id.clone(),
        session_id: session,
        approval_id: None,
    };

    let transport = HttpPushTransport;
    let mut delivered = 0usize;
    let mut dead: Vec<String> = Vec::new();
    let mut first_error: Option<String> = None;
    for (device_id, token) in &targets {
        let payload = serde_json::to_value(GatewayRequest {
            token: &token.token,
            event: &event,
        })
        .unwrap_or(Value::Null);
        match transport.send(&url, &bearer, &payload) {
            Ok(()) => delivered += 1,
            Err(error) => {
                if error.unregistered {
                    dead.push(device_id.clone());
                }
                if first_error.is_none() {
                    first_error = Some(error.message);
                }
            }
        }
    }
    forget_unregistered(app, &dead);

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
        /// When set, every send fails as an unregistered token.
        unregistered: bool,
    }

    impl PushTransport for RecordingTransport {
        fn send(&self, _url: &str, _bearer: &str, payload: &Value) -> Result<(), PushError> {
            if self.unregistered {
                return Err(PushError::unregistered("token is dead"));
            }
            self.calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((String::new(), payload.clone()));
            Ok(())
        }
    }

    fn token(allowed: bool) -> PushToken {
        PushToken {
            token: "token".into(),
            platform: "android".into(),
            updated_at: 0,
            on_questions: allowed,
            on_completed: allowed,
            on_failed: allowed,
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
    fn the_health_url_is_the_gateway_origin_plus_healthz() {
        assert_eq!(
            health_url("https://push.example.com/v1/push"),
            Some("https://push.example.com/healthz".to_string())
        );
        assert_eq!(
            health_url("http://127.0.0.1:8787/v1/push"),
            Some("http://127.0.0.1:8787/healthz".to_string())
        );
        assert_eq!(health_url("not a url"), None);
    }

    #[test]
    fn deliver_posts_a_minimal_payload_to_every_target() {
        let transport = RecordingTransport::default();
        let event = PushEvent {
            event_id: "desk:se:ses:task_completed".into(),
            kind: "task_completed",
            desktop_id: "desk".into(),
            session_id: Some("ses".into()),
            approval_id: None,
        };
        let dead = deliver(
            &transport,
            "https://gateway/v1/push",
            "secret",
            &[
                ("dev_a".to_string(), token(true)),
                ("dev_b".to_string(), token(true)),
            ],
            &event,
        );
        assert!(dead.is_empty());
        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let payload = &calls[0].1;
        assert_eq!(payload["event"]["event_id"], "desk:se:ses:task_completed");
        assert_eq!(payload["event"]["session_id"], "ses");
        // The gateway requires `desktop_id`; a payload without it is rejected
        // with 400, so it must always be present (and non-empty).
        assert_eq!(payload["event"]["desktop_id"], "desk");
        // No content keys are ever present.
        let serialized = payload.to_string();
        assert!(!serialized.contains("prompt"));
        assert!(!serialized.contains("text"));
        assert!(!serialized.contains("command"));
    }

    #[test]
    fn deliver_skips_devices_that_opted_out_of_the_event_kind() {
        let transport = RecordingTransport::default();
        let event = PushEvent {
            event_id: "desk:se:ses:task_completed".into(),
            kind: "task_completed",
            desktop_id: "desk".into(),
            session_id: Some("ses".into()),
            approval_id: None,
        };
        deliver(
            &transport,
            "https://gateway/v1/push",
            "secret",
            &[
                ("on".to_string(), {
                    let mut t = token(false);
                    t.on_completed = true;
                    t
                }),
                ("off".to_string(), {
                    let mut t = token(true);
                    t.on_completed = false;
                    t
                }),
            ],
            &event,
        );
        // Only the device that enabled completions is contacted.
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn deliver_reports_devices_with_dead_tokens() {
        let transport = RecordingTransport {
            calls: Mutex::new(Vec::new()),
            unregistered: true,
        };
        let event = PushEvent {
            event_id: "desk:se:ses:task_failed".into(),
            kind: "task_failed",
            desktop_id: "desk".into(),
            session_id: Some("ses".into()),
            approval_id: None,
        };
        let dead = deliver(
            &transport,
            "https://gateway/v1/push",
            "secret",
            &[
                ("dev_a".to_string(), token(true)),
                ("dev_b".to_string(), token(true)),
            ],
            &event,
        );
        assert_eq!(dead, vec!["dev_a".to_string(), "dev_b".to_string()]);
    }
}
