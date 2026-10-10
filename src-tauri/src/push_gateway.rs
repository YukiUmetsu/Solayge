//! The desktop's embedded push gateway.
//!
//! A phone that paired with this desktop registers an FCM token; the desktop
//! needs a gateway to turn a minimal agent alert into an FCM message. Rather
//! than require the user to run `solayge-push-gateway` as a separate process,
//! the desktop runs the same server on a loopback port whenever the user has
//! configured a Firebase service account (`push-gateway.json`, owner-only).
//!
//! The service-account key is read from disk on this machine and is used only
//! by this process; it is never sent to the phone or included in a notification.
//! A remote/shared gateway can still be selected with
//! `SOLAYGE_PUSH_GATEWAY_URL`/`_TOKEN` (see [`crate::push`]).
//!
//! Lifecycle: the manager owns at most one running [`GatewayHandle`]. It is
//! started at launch when enabled, restarted when settings change, and stopped
//! on exit (a `Drop` also joins the serve thread).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use solayge_push_gateway::fcm::ServiceAccount;
use solayge_push_gateway::server::{self, GatewayHandle, DEFAULT_MAX_BODY};

/// Where the gateway configuration is persisted (owner-only).
const CONFIG_FILE: &str = "push-gateway.json";
/// The default loopback port for the embedded gateway.
pub const DEFAULT_GATEWAY_PORT: u16 = 8787;

fn default_port() -> u16 {
    DEFAULT_GATEWAY_PORT
}

/// Persisted embedded-gateway settings. `token` is a bearer secret generated on
/// this machine and never logged; it authenticates the desktop to its own
/// loopback gateway (or a remote one presented with the same token).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushGatewayConfig {
    /// Whether the embedded gateway should run when the app does.
    #[serde(default)]
    pub enabled: bool,
    /// Loopback port; falls back to an ephemeral port if it is taken.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Path to the Firebase service-account JSON the gateway signs with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,
    /// The bearer secret the desktop uses to authenticate to the gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl Default for PushGatewayConfig {
    fn default() -> Self {
        PushGatewayConfig {
            enabled: false,
            port: DEFAULT_GATEWAY_PORT,
            service_account: None,
            token: None,
        }
    }
}

/// The live state the Settings panel shows.
#[derive(Debug, Clone, Serialize)]
pub struct PushGatewayStatus {
    pub enabled: bool,
    pub running: bool,
    /// The desktop-side `POST /v1/push` URL the embedded gateway listens on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub port: u16,
    /// The configured service-account path, if any (not a secret).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,
    /// Authorized devices that have registered an FCM token.
    pub registered_devices: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Owns the persisted configuration and the single running gateway.
pub struct PushGatewayManager {
    data_dir: PathBuf,
    config: Mutex<PushGatewayConfig>,
    running: Mutex<Option<GatewayHandle>>,
    last_error: Mutex<Option<String>>,
}

impl PushGatewayManager {
    /// Open the manager over the app data directory, reading any saved config.
    pub fn new(data_dir: &Path) -> Self {
        let config = read_config(data_dir).unwrap_or_default();
        PushGatewayManager {
            data_dir: data_dir.to_path_buf(),
            config: Mutex::new(config),
            running: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    fn lock_config(&self) -> std::sync::MutexGuard<'_, PushGatewayConfig> {
        self.config.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The local gateway's URL and bearer token, when it is running. Used by
    /// [`crate::push`] as the final configuration source.
    pub fn local_config(&self) -> Option<(String, String)> {
        let token = self.lock_config().token.clone()?;
        if token.is_empty() {
            return None;
        }
        let running = self.running.lock().unwrap_or_else(|p| p.into_inner());
        let handle = running.as_ref()?;
        Some((handle.local_url.clone(), token))
    }

    /// Whether the embedded gateway is currently listening.
    fn is_running(&self) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }

    /// Start the gateway if enabled and configured; a no-op otherwise. Returns
    /// an error only when the user asked for it but it could not start.
    pub fn start(&self) -> Result<(), String> {
        if self.is_running() {
            return Ok(());
        }
        let (enabled, port, service_account) = {
            let config = self.lock_config();
            (
                config.enabled,
                config.port,
                config.service_account.clone(),
            )
        };
        if !enabled {
            return Ok(());
        }
        let Some(path) = service_account.filter(|p| !p.trim().is_empty()) else {
            let message = "Choose a Firebase service-account JSON to enable background push.";
            self.set_error(Some(message.to_string()));
            return Err(message.to_string());
        };
        let account = ServiceAccount::from_file(Path::new(&path)).map_err(|error| {
            let message = format!("Could not read the service account: {error}");
            self.set_error(Some(message.clone()));
            message
        })?;

        let token = self.ensure_token();
        // Persist a token generated here (for example after an upgrade) so it
        // stays stable across restarts.
        let _ = self.persist();
        // Prefer the configured port; fall back to an ephemeral loopback port if
        // it is already in use, so a second instance still works.
        let primary = format!("127.0.0.1:{port}");
        let handle = match server::spawn(&primary, token.clone(), account.clone(), DEFAULT_MAX_BODY) {
            Ok(handle) => handle,
            Err(primary_error) => server::spawn("127.0.0.1:0", token, account, DEFAULT_MAX_BODY)
                .map_err(|fallback_error| {
                    format!("{primary_error}; fallback failed: {fallback_error}")
                })?,
        };
        *self.running.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
        self.set_error(None);
        Ok(())
    }

    /// Stop the gateway if it is running. Idempotent.
    pub fn stop(&self) {
        if let Some(mut handle) = self
            .running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            handle.stop();
        }
    }

    /// Apply new settings: validate, persist, and restart the gateway.
    pub fn apply(
        &self,
        enabled: bool,
        service_account: Option<String>,
        port: Option<u16>,
    ) -> Result<(), String> {
        let service_account = service_account
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty());
        if let Some(port) = port {
            if port == 0 {
                return Err("Choose a port between 1 and 65535".to_string());
            }
        }
        if enabled {
            let Some(path) = service_account.as_deref() else {
                return Err("Choose a Firebase service-account JSON to enable background push.".into());
            };
            // Validate before persisting so a bad path cannot leave the app
            // believing push is configured.
            ServiceAccount::from_file(Path::new(path))
                .map_err(|error| format!("Could not read the service account: {error}"))?;
        }
        {
            let mut config = self.lock_config();
            config.enabled = enabled;
            config.service_account = service_account;
            if let Some(port) = port {
                config.port = port;
            }
            if config.token.as_deref().unwrap_or("").is_empty() {
                config.token = Some(generate_token());
            }
        }
        self.persist()?;
        self.stop();
        self.start()
    }

    /// Build the live status.
    pub fn status(&self, device_count: usize) -> PushGatewayStatus {
        let (enabled, port, service_account) = {
            let config = self.lock_config();
            (
                config.enabled,
                config.port,
                config.service_account.clone(),
            )
        };
        let url = self
            .running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|handle| handle.local_url.clone());
        PushGatewayStatus {
            enabled,
            running: url.is_some(),
            url,
            port,
            service_account,
            registered_devices: device_count,
            last_error: self
                .last_error
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
        }
    }

    fn ensure_token(&self) -> String {
        let mut config = self.lock_config();
        match config.token.clone() {
            Some(token) if !token.is_empty() => token,
            _ => {
                let token = generate_token();
                config.token = Some(token.clone());
                token
            }
        }
    }

    fn set_error(&self, error: Option<String>) {
        *self.last_error.lock().unwrap_or_else(|p| p.into_inner()) = error;
    }

    fn persist(&self) -> Result<(), String> {
        let config = self.lock_config().clone();
        write_config(&self.data_dir, &config)
    }
}

/// Restore the gateway at app launch when the user enabled it.
pub fn restore_on_startup(app: &AppHandle) {
    let Some(manager) = app.try_state::<PushGatewayManager>() else {
        return;
    };
    if let Err(error) = manager.start() {
        eprintln!("solayge: embedded push gateway not started: {error}");
    }
}

/// Stop the gateway on process exit.
pub fn shutdown(app: &AppHandle) {
    if let Some(manager) = app.try_state::<PushGatewayManager>() {
        manager.stop();
    }
}

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CONFIG_FILE)
}

fn read_config(data_dir: &Path) -> Option<PushGatewayConfig> {
    let text = std::fs::read_to_string(config_path(data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write owner-only, matching the app's other at-rest data.
fn write_config(data_dir: &Path, config: &PushGatewayConfig) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
    let path = config_path(data_dir);
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A fresh 32-byte bearer secret, hex-encoded.
fn generate_token() -> String {
    use rand::Rng as _;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

// ---- Tauri commands ----

fn registered_device_count(app: &AppHandle) -> usize {
    app.try_state::<crate::devices::DeviceManager>()
        .map(|manager| manager.push_targets().len())
        .unwrap_or(0)
}

/// The embedded gateway's current status for Settings.
#[tauri::command]
pub fn push_gateway_status(app: AppHandle) -> PushGatewayStatus {
    let devices = registered_device_count(&app);
    app.state::<PushGatewayManager>().status(devices)
}

/// Enable/disable the embedded gateway, set the service account and port, and
/// restart it. Validation happens before anything is persisted.
#[tauri::command]
pub fn push_gateway_apply(
    app: AppHandle,
    enabled: bool,
    service_account: Option<String>,
    port: Option<u16>,
) -> Result<PushGatewayStatus, String> {
    let devices = registered_device_count(&app);
    let manager = app.state::<PushGatewayManager>();
    manager.apply(enabled, service_account, port)?;
    Ok(manager.status(devices))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "solayge-push-gateway-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    #[test]
    fn config_round_trips_and_tokens_are_distinct() {
        let dir = temp_dir();
        let manager = PushGatewayManager::new(&dir);
        assert!(!manager.status(0).running);
        // A disabled manager never starts, even with no service account.
        manager.start().unwrap();

        let config = PushGatewayConfig {
            enabled: true,
            port: 8_788,
            service_account: Some("/tmp/service-account.json".into()),
            token: Some("token-abc".into()),
        };
        write_config(&dir, &config).unwrap();
        let reopened = PushGatewayManager::new(&dir);
        assert!(reopened.lock_config().enabled);
        assert_eq!(reopened.lock_config().token.as_deref(), Some("token-abc"));

        assert_ne!(generate_token(), generate_token());
        assert_eq!(generate_token().len(), 64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_rejects_enabling_without_a_service_account() {
        let dir = temp_dir();
        let manager = PushGatewayManager::new(&dir);
        assert!(manager.apply(true, None, None).is_err());
        // Nothing was persisted as enabled.
        assert!(!manager.lock_config().enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enabling_with_a_valid_service_account_starts_the_gateway() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // `from_file` parses the JSON without validating the key, so a shaped
        // dummy is enough to exercise the full start path.
        let path = dir.join("service-account.json");
        std::fs::write(
            &path,
            r#"{"project_id":"solayge-test","client_email":"a@b.c","private_key":"-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----\n"}"#,
        )
        .unwrap();

        let manager = PushGatewayManager::new(&dir);
        manager
            .apply(true, Some(path.to_string_lossy().to_string()), Some(58_123))
            .unwrap();
        // Even if 58123 is taken, `start` falls back to an ephemeral port.
        let status = manager.status(2);
        assert!(status.running);
        assert!(status.enabled);
        let (url, token) = manager.local_config().expect("local gateway config");
        assert!(url.starts_with("http://127.0.0.1:"));
        assert!(url.ends_with("/v1/push"));
        assert!(!token.is_empty());

        manager.stop();
        assert!(!manager.status(0).running);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
