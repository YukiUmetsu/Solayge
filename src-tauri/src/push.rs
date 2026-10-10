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

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::devices::DeviceManager;
use crate::models::NotifyKind;

/// Where a *temporary, dev-only* gateway override is stored. Production and CI
/// configure the gateway through `SOLAYGE_PUSH_GATEWAY_URL`/`_TOKEN`; this file
/// only exists when the developer creates it (through the Settings test panel
/// or `pnpm push:gateway`), so it never affects a normal install.
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
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
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
                Err(format!("gateway returned {status}"))
            } else {
                Err(format!("gateway returned {status}: {detail}"))
            }
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

// ============================================================================
// TEMPORARY DEV-ONLY gateway configuration — remove with the test hook below.
//
// Production config is environment-only. This lets the desktop's Settings test
// panel point at a local gateway at runtime (no restarting the app with new
// environment variables). The file is owner-only (0600) and lives in the app's
// private data directory; it is never logged or sent anywhere.
// ============================================================================

/// The persisted dev override. `token` is a bearer secret; never log it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DevPushConfig {
    url: String,
    token: String,
}

/// What the Settings panel shows about the current gateway configuration.
#[derive(Debug, Clone, Serialize)]
pub struct DevPushConfigView {
    pub url: String,
    pub has_token: bool,
    /// `dev` (the file above), `env` (process environment), or `none`.
    pub source: String,
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

fn write_dev_config(data_dir: &Path, config: &DevPushConfig) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(dev_config_path(data_dir))
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(dev_config_path(data_dir), &bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn remove_dev_config_file(data_dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(dev_config_path(data_dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// The gateway to dispatch to: the dev override wins when present, otherwise
/// the process environment. A normal install has no override file, so this is
/// identical to the previous environment-only behavior.
fn app_gateway_config(app: &AppHandle) -> Option<(String, String)> {
    app_data_dir(app)
        .as_deref()
        .and_then(read_dev_config)
        .map(|config| (config.url, config.token))
        .or_else(env_gateway_config)
}

/// Validate and normalize a gateway URL. Only `http`/`https` is accepted.
pub(crate) fn normalize_gateway_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Err("Enter the gateway URL".to_string());
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| "The gateway URL must start with http:// or https://".to_string())?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err("The gateway URL is missing a host".to_string());
    }
    Ok(url.to_string())
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

/// The current gateway configuration, as the Settings panel should show it.
pub(crate) fn dev_config_view(app: &AppHandle) -> DevPushConfigView {
    if let Some(config) = app_data_dir(app).as_deref().and_then(read_dev_config) {
        return DevPushConfigView {
            url: config.url,
            has_token: !config.token.is_empty(),
            source: "dev".to_string(),
        };
    }
    if let Some((url, token)) = env_gateway_config() {
        return DevPushConfigView {
            url,
            has_token: !token.is_empty(),
            source: "env".to_string(),
        };
    }
    DevPushConfigView {
        url: String::new(),
        has_token: false,
        source: "none".to_string(),
    }
}

/// Persist a dev gateway override (owner-only). Returns the updated view.
pub(crate) fn set_dev_config(
    app: &AppHandle,
    url: &str,
    token: &str,
) -> Result<DevPushConfigView, String> {
    let url = normalize_gateway_url(url)?;
    let dir = app_data_dir(app)
        .ok_or_else(|| "Could not resolve the app data directory".to_string())?;
    // A blank token keeps the previously stored one, so the URL can be changed
    // without re-pasting the secret.
    let token = match token.trim() {
        "" => read_dev_config(&dir)
            .map(|config| config.token)
            .ok_or_else(|| "Enter the gateway bearer token".to_string())?,
        token => token.to_string(),
    };
    write_dev_config(&dir, &DevPushConfig { url, token })?;
    Ok(dev_config_view(app))
}

/// Remove the dev gateway override. Returns the updated view.
pub(crate) fn clear_dev_config(app: &AppHandle) -> Result<DevPushConfigView, String> {
    if let Some(dir) = app_data_dir(app) {
        remove_dev_config_file(&dir)?;
    }
    Ok(dev_config_view(app))
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
    let Some((url, bearer)) = app_gateway_config(app) else {
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
        desktop_id: desktop_id.clone(),
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
    let (url, bearer) = app_gateway_config(app).ok_or_else(|| {
        "No push gateway is configured. Run `pnpm push:gateway` or fill in the gateway \
         URL and token above."
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
        desktop_id: desktop_id.clone(),
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
    fn gateway_urls_must_be_http_and_have_a_host() {
        assert_eq!(
            normalize_gateway_url(" https://push.example.com/v1/push ").unwrap(),
            "https://push.example.com/v1/push"
        );
        assert!(normalize_gateway_url("http://127.0.0.1:8787/v1/push").is_ok());
        assert!(normalize_gateway_url("").is_err());
        assert!(normalize_gateway_url("ftp://push.example.com").is_err());
        assert!(normalize_gateway_url("https:///nohost").is_err());
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
        // The gateway requires `desktop_id`; a payload without it is rejected
        // with 400, so it must always be present (and non-empty).
        assert_eq!(payload["event"]["desktop_id"], "desk");
        // No content keys are ever present.
        let serialized = payload.to_string();
        assert!(!serialized.contains("prompt"));
        assert!(!serialized.contains("text"));
        assert!(!serialized.contains("command"));
    }
}
