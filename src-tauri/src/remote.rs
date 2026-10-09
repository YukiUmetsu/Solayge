//! Desktop remote-access lifecycle management.
//!
//! [`RemoteConnectionManager`] is the single owner of the desktop's remote
//! listeners. When the user enables mobile connections it:
//!
//! 1. starts the authenticated loopback WebSocket listener on `127.0.0.1`;
//! 2. detects the installed Tailscale CLI and reports its status;
//! 3. when the backup is enabled and Tailscale is ready, configures a
//!    **Solayge-owned** `tailscale serve` HTTPS proxy to that loopback endpoint
//!    and verifies it;
//! 4. keeps the listener running while remote access is enabled — including with
//!    zero connected clients — and removes only its own Serve handler when
//!    disabled or on normal exit.
//!
//! Authentication is not the transport's job: the WebSocket listener reuses the
//! same [`crate::host::HostHandler`] as Iroh, so pairing, the challenge-response
//! identity proof, scopes, and revocation apply identically. Tailscale
//! membership alone never grants access.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, State};

use solayge_network::tailscale::{TailscaleCli, TailscaleStatus};
use solayge_network::websocket::{WsHost, WsHostConfig};
use solayge_network::FrameHandler;

use crate::host::HostHandler;

/// Where the remote-access preferences are persisted.
const PREFS_FILE: &str = "remote-access.json";

/// Default HTTPS port for the Tailscale Serve endpoint.
pub const DEFAULT_HTTPS_PORT: u16 = 443;
/// Default loopback port for the WebSocket listener. A stable port keeps the
/// Tailscale Serve target stable across restarts, so a crashed process leaves a
/// Serve entry that the next launch can verify and reuse rather than conflict
/// with.
pub const DEFAULT_LOCAL_PORT: u16 = 51234;

fn default_https_port() -> u16 {
    DEFAULT_HTTPS_PORT
}
fn default_local_port() -> u16 {
    DEFAULT_LOCAL_PORT
}

/// Persisted remote-access preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAccessPreferences {
    /// Whether mobile connections are enabled at all.
    #[serde(default)]
    pub enabled: bool,
    /// Whether the Tailscale Serve backup should be configured.
    #[serde(default)]
    pub tailscale_backup: bool,
    /// HTTPS port for the Tailscale Serve endpoint.
    #[serde(default = "default_https_port")]
    pub https_port: u16,
    /// Loopback port for the WebSocket listener.
    #[serde(default = "default_local_port")]
    pub local_port: u16,
}

impl Default for RemoteAccessPreferences {
    fn default() -> Self {
        RemoteAccessPreferences {
            enabled: false,
            tailscale_backup: false,
            https_port: DEFAULT_HTTPS_PORT,
            local_port: DEFAULT_LOCAL_PORT,
        }
    }
}

/// The live remote-access status shown in Settings.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteStatus {
    pub enabled: bool,
    /// `disabled` | `listening` | `active` | `degraded` | `error`.
    pub state: String,
    pub listener_running: bool,
    pub listener_port: Option<u16>,
    pub connected_clients: usize,
    pub tailscale_backup: bool,
    pub tailscale: TailscaleStatus,
    pub serve_enabled: bool,
    pub serve_owned: bool,
    /// The HTTPS URL a phone should dial, when Serve is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serve_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serve_port: Option<u16>,
    pub https_port: u16,
    pub local_port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ServeRuntime {
    enabled: bool,
    owned: bool,
    url: Option<String>,
    port: Option<u16>,
    local_port: Option<u16>,
}

/// Owns the WebSocket listener and the Tailscale Serve lifecycle.
pub struct RemoteConnectionManager {
    data_dir: PathBuf,
    prefs: Mutex<RemoteAccessPreferences>,
    /// The live listener. Async mutex because shutdown is async.
    host: tokio::sync::Mutex<Option<WsHost>>,
    /// The listener's bound address, readable without the async lock.
    listener_addr: Mutex<Option<SocketAddr>>,
    /// The shared live-client counter, readable without the async lock.
    clients: Mutex<Option<Arc<AtomicUsize>>>,
    serve: Mutex<ServeRuntime>,
    tailscale: Mutex<TailscaleStatus>,
    last_error: Mutex<Option<String>>,
    /// Serializes lifecycle transitions so start/stop cannot race.
    op: tokio::sync::Mutex<()>,
}

impl RemoteConnectionManager {
    /// Load persisted preferences from `data_dir`. Never starts anything.
    pub fn new(data_dir: &Path) -> Self {
        let prefs = read_prefs(data_dir);
        RemoteConnectionManager {
            data_dir: data_dir.to_path_buf(),
            prefs: Mutex::new(prefs),
            host: tokio::sync::Mutex::new(None),
            listener_addr: Mutex::new(None),
            clients: Mutex::new(None),
            serve: Mutex::new(ServeRuntime::default()),
            tailscale: Mutex::new(TailscaleStatus::absent()),
            last_error: Mutex::new(None),
            op: tokio::sync::Mutex::new(()),
        }
    }

    pub fn prefs(&self) -> RemoteAccessPreferences {
        self.prefs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// A point-in-time snapshot for the Settings UI.
    pub fn status(&self) -> RemoteStatus {
        let prefs = self.prefs();
        let listener_addr = *self.listener_addr.lock().unwrap_or_else(|e| e.into_inner());
        let connected_clients = self
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(0);
        let serve = self
            .serve
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let tailscale = self
            .tailscale
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let last_error = self
            .last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let state = if !prefs.enabled {
            "disabled"
        } else if listener_addr.is_none() {
            "error"
        } else if !prefs.tailscale_backup {
            "listening"
        } else if serve.owned {
            "active"
        } else {
            "degraded"
        }
        .to_string();

        RemoteStatus {
            enabled: prefs.enabled,
            state,
            listener_running: listener_addr.is_some(),
            listener_port: listener_addr.map(|a| a.port()),
            connected_clients,
            tailscale_backup: prefs.tailscale_backup,
            tailscale,
            serve_enabled: serve.enabled,
            serve_owned: serve.owned,
            serve_url: serve.url,
            serve_port: serve.port,
            https_port: prefs.https_port,
            local_port: prefs.local_port,
            last_error,
        }
    }

    /// Apply new preferences and bring the services to the matching state.
    ///
    /// Persists first so a crash cannot lose the user's choice. Start errors are
    /// reported in [`RemoteStatus::last_error`] rather than discarded, so the UI
    /// can show an actionable message while the listener state stays truthful.
    pub async fn apply<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        prefs: RemoteAccessPreferences,
    ) -> Result<RemoteStatus, String> {
        validate_prefs(&prefs)?;
        let _guard = self.op.lock().await;
        write_prefs(&self.data_dir, &prefs)?;
        *self.prefs.lock().unwrap_or_else(|e| e.into_inner()) = prefs.clone();

        if !prefs.enabled {
            self.stop_inner(&prefs).await;
            self.set_last_error(None);
            self.refresh_tailscale_status().await;
            return Ok(self.status());
        }

        self.set_last_error(None);
        if let Err(error) = self.ensure_listener(app, &prefs).await {
            self.set_last_error(Some(error));
            self.refresh_tailscale_status().await;
            return Ok(self.status());
        }

        self.apply_tailscale(&prefs).await;
        Ok(self.status())
    }

    async fn ensure_listener<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        prefs: &RemoteAccessPreferences,
    ) -> Result<(), String> {
        if self
            .listener_addr
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            return Ok(());
        }
        let handler: Arc<dyn FrameHandler> = Arc::new(HostHandler::new(app.clone()));
        let bind = SocketAddr::from(([127, 0, 0, 1], prefs.local_port));
        let host = WsHost::bind(WsHostConfig { bind_addr: bind }, handler)
            .await
            .map_err(|e| {
                format!(
                    "Could not start the mobile listener on 127.0.0.1:{}: {e}. \
                     Another app may be using that port.",
                    prefs.local_port
                )
            })?;
        let addr = host.local_addr();
        let counter = host.client_counter();
        *self.host.lock().await = Some(host);
        *self.listener_addr.lock().unwrap_or_else(|e| e.into_inner()) = Some(addr);
        *self.clients.lock().unwrap_or_else(|e| e.into_inner()) = Some(counter);
        Ok(())
    }

    async fn stop_inner(&self, prefs: &RemoteAccessPreferences) {
        // Remove only Solayge's own Serve handler; unrelated entries are left
        // intact (see `TailscaleCli::serve_stop`).
        let had_serve = {
            let serve = self.serve.lock().unwrap_or_else(|e| e.into_inner());
            serve.enabled || serve.owned
        };
        if had_serve || prefs.tailscale_backup {
            let (local_port, https_port) = (prefs.local_port, prefs.https_port);
            let _ = tokio::task::spawn_blocking(move || {
                if let Some(cli) = TailscaleCli::detect() {
                    let _ = cli.serve_stop(local_port, https_port);
                }
            })
            .await;
        }
        crate::host_runtime::clear_tailscale_authority();
        *self.serve.lock().unwrap_or_else(|e| e.into_inner()) = ServeRuntime::default();
        if let Some(host) = self.host.lock().await.take() {
            host.shutdown().await;
        }
        *self.clients.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.listener_addr.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    async fn apply_tailscale(&self, prefs: &RemoteAccessPreferences) {
        let local_port = prefs.local_port;
        let https_port = prefs.https_port;
        let backup = prefs.tailscale_backup;

        // The endpoint this manager currently owns, if any, so a port change (or
        // disabling the backup) removes the old Serve handler instead of leaving
        // it exposed.
        let previous_owned = {
            let serve = self.serve.lock().unwrap_or_else(|e| e.into_inner());
            if serve.owned {
                serve.port.zip(serve.local_port)
            } else {
                None
            }
        };
        let stale = stale_serve_to_stop(previous_owned, local_port, https_port, backup);

        let outcome = tokio::task::spawn_blocking(move || {
            let cli = TailscaleCli::detect();
            let status = match &cli {
                Some(cli) => cli.status(),
                None => TailscaleStatus::absent(),
            };

            if !backup {
                // Remove Solayge's own previous handler; `serve_stop` never
                // touches unrelated configuration.
                let stop_error = match stale {
                    Some((stale_local, stale_https)) => cli
                        .as_ref()
                        .and_then(|cli| cli.serve_stop(stale_local, stale_https).err())
                        .map(|e| e.to_string()),
                    None => None,
                };
                return TailscaleOutcome {
                    status,
                    authority: None,
                    url: None,
                    error: stop_error,
                };
            }

            let Some(cli) = cli else {
                return TailscaleOutcome {
                    status,
                    authority: None,
                    url: None,
                    error: Some(
                        "Tailscale is not installed; the backup is unavailable".to_string(),
                    ),
                };
            };
            if !status.ready() {
                let message = status.message.clone();
                return TailscaleOutcome {
                    status,
                    authority: None,
                    url: None,
                    error: message,
                };
            }

            // When our target moved but the HTTPS port is unchanged (a
            // local-port change), the old handler occupies the very port we
            // need, so it must be removed first. When the HTTPS port itself
            // changed the new one is independent and is created first, so a
            // failure can never drop the working backup.
            let remove_before_start = stale_occupies_port(stale, https_port);
            if remove_before_start {
                if let Some((stale_local, stale_https)) = stale {
                    let _ = cli.serve_stop(stale_local, stale_https);
                }
            }
            match cli.serve_start(local_port, https_port) {
                Ok(_) => {
                    if !remove_before_start {
                        if let Some((stale_local, stale_https)) = stale {
                            let _ = cli.serve_stop(stale_local, stale_https);
                        }
                    }
                    let authority = match status.dns_name.as_deref() {
                        Some(name) if https_port != DEFAULT_HTTPS_PORT => {
                            format!("{name}:{https_port}")
                        }
                        Some(name) => name.to_string(),
                        None => String::new(),
                    };
                    let url = format!("https://{authority}/");
                    TailscaleOutcome {
                        status,
                        authority: Some(authority),
                        url: Some(url),
                        error: None,
                    }
                }
                Err(error) => TailscaleOutcome {
                    status,
                    authority: None,
                    url: None,
                    error: Some(error.to_string()),
                },
            }
        })
        .await
        .unwrap_or_else(|_| TailscaleOutcome {
            status: TailscaleStatus::absent(),
            authority: None,
            url: None,
            error: Some("Tailscale status task failed".to_string()),
        });

        *self.tailscale.lock().unwrap_or_else(|e| e.into_inner()) = outcome.status;
        self.set_last_error(outcome.error);

        let mut serve = self.serve.lock().unwrap_or_else(|e| e.into_inner());
        match (outcome.authority, outcome.url) {
            (Some(authority), Some(url)) => {
                crate::host_runtime::set_tailscale_authority(authority);
                *serve = ServeRuntime {
                    enabled: true,
                    owned: true,
                    url: Some(url),
                    port: Some(https_port),
                    local_port: Some(local_port),
                };
            }
            _ => {
                crate::host_runtime::clear_tailscale_authority();
                *serve = ServeRuntime::default();
            }
        }
    }

    async fn refresh_tailscale_status(&self) {
        let status = tokio::task::spawn_blocking(|| match TailscaleCli::detect() {
            Some(cli) => cli.status(),
            None => TailscaleStatus::absent(),
        })
        .await
        .unwrap_or_else(|_| TailscaleStatus::absent());
        *self.tailscale.lock().unwrap_or_else(|e| e.into_inner()) = status;
    }

    fn set_last_error(&self, error: Option<String>) {
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = error;
    }
}

struct TailscaleOutcome {
    status: TailscaleStatus,
    authority: Option<String>,
    url: Option<String>,
    error: Option<String>,
}

/// The previously-owned Serve endpoint that must be removed before `prefs` are
/// applied, if any.
///
/// Returns `Some((local_port, https_port))` when the manager owns an endpoint
/// that the new preferences no longer describe (a changed port, or the backup
/// being disabled). When the manager has no live record — for example after a
/// crash and restart — but the backup is being turned off, it still returns the
/// configured endpoint so a stale handler is cleaned up. `serve_stop` itself
/// leaves any unrelated handler untouched.
fn stale_serve_to_stop(
    previous_owned: Option<(u16, u16)>,
    new_local_port: u16,
    new_https_port: u16,
    new_backup: bool,
) -> Option<(u16, u16)> {
    match previous_owned {
        Some((prev_local, prev_https)) => {
            if !new_backup || prev_local != new_local_port || prev_https != new_https_port {
                Some((prev_local, prev_https))
            } else {
                None
            }
        }
        None if !new_backup => Some((new_local_port, new_https_port)),
        None => None,
    }
}

/// Whether the stale endpoint occupies the very HTTPS port we are about to
/// configure (so it must be removed before the new handler is created).
fn stale_occupies_port(stale: Option<(u16, u16)>, https_port: u16) -> bool {
    matches!(stale, Some((_, stale_https)) if stale_https == https_port)
}

fn validate_prefs(prefs: &RemoteAccessPreferences) -> Result<(), String> {
    if prefs.https_port == 0 {
        return Err("The HTTPS port must be between 1 and 65535".to_string());
    }
    if prefs.local_port < 1024 {
        return Err("The local listener port must be between 1024 and 65535".to_string());
    }
    Ok(())
}

fn prefs_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PREFS_FILE)
}

fn read_prefs(data_dir: &Path) -> RemoteAccessPreferences {
    std::fs::read_to_string(prefs_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_prefs(data_dir: &Path, prefs: &RemoteAccessPreferences) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(prefs).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(prefs_path(data_dir))
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(prefs_path(data_dir), &bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Restore services at startup when the user previously enabled remote access.
/// Spawned so app setup never blocks on networking or the Tailscale CLI.
pub fn restore_on_startup(app: AppHandle) {
    let prefs = app.state::<RemoteConnectionManager>().prefs();
    if !prefs.enabled {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let manager = app.state::<RemoteConnectionManager>();
        if let Err(error) = manager.apply(&app, prefs).await {
            eprintln!("solayge: could not restore remote access: {error}");
        }
    });
}

/// Best-effort synchronous cleanup for process exit. Removes Solayge's Serve
/// handler so a quit does not leave the desktop exposed on the tailnet.
pub fn shutdown(app: &AppHandle) {
    let Some(manager) = app.try_state::<RemoteConnectionManager>() else {
        return;
    };
    let prefs = manager.prefs();
    let owned = manager
        .serve
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .owned;
    if prefs.tailscale_backup || owned {
        if let Some(cli) = TailscaleCli::detect() {
            let _ = cli.serve_stop(prefs.local_port, prefs.https_port);
        }
    }
    crate::host_runtime::clear_tailscale_authority();
}

// ---- Tauri commands ----

/// The current remote-access status.
#[tauri::command]
pub fn remote_status(manager: State<'_, RemoteConnectionManager>) -> RemoteStatus {
    manager.status()
}

/// Enable/disable remote access and/or the Tailscale backup.
#[tauri::command]
pub async fn remote_apply(
    app: AppHandle,
    manager: State<'_, RemoteConnectionManager>,
    prefs: RemoteAccessPreferences,
) -> Result<RemoteStatus, String> {
    manager.apply(&app, prefs).await
}

/// Re-run detection and configuration without changing preferences.
#[tauri::command]
pub async fn remote_retry(
    app: AppHandle,
    manager: State<'_, RemoteConnectionManager>,
) -> Result<RemoteStatus, String> {
    let prefs = manager.prefs();
    manager.apply(&app, prefs).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "solayge-remote-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    /// A loopback port that is free right now. Tests must not bind the
    /// production default (`DEFAULT_LOCAL_PORT`), which a running Solayge
    /// instance already holds — otherwise the suite fails on a developer
    /// machine that has the app open.
    fn free_local_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        listener.local_addr().expect("read local addr").port()
    }

    #[test]
    fn preferences_round_trip_and_default_when_absent() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();

        let manager = RemoteConnectionManager::new(&dir);
        assert_eq!(manager.prefs(), RemoteAccessPreferences::default());
        assert!(!manager.status().enabled);
        assert_eq!(manager.status().state, "disabled");

        let prefs = RemoteAccessPreferences {
            enabled: true,
            tailscale_backup: true,
            https_port: 8443,
            local_port: 52000,
        };
        write_prefs(&dir, &prefs).unwrap();

        let restarted = RemoteConnectionManager::new(&dir);
        assert_eq!(restarted.prefs(), prefs);
        assert!(restarted.status().enabled);
        assert_eq!(restarted.status().https_port, 8443);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn enabling_starts_a_loopback_listener_and_disabling_stops_it() {
        use tauri::test::mock_app;
        let dir = temp_dir();
        let app = mock_app();
        app.manage(RemoteConnectionManager::new(&dir));

        let manager = app.state::<RemoteConnectionManager>();
        let status = manager
            .apply(
                app.handle(),
                RemoteAccessPreferences {
                    enabled: true,
                    tailscale_backup: false,
                    local_port: free_local_port(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(status.listener_running);
        assert_eq!(status.connected_clients, 0);
        assert_eq!(status.state, "listening");
        let port = status.listener_port.expect("listener port");
        assert_ne!(port, 0);

        // Disabling stops the listener and closes the port.
        let stopped = manager
            .apply(
                app.handle(),
                RemoteAccessPreferences {
                    enabled: false,
                    tailscale_backup: false,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!stopped.listener_running);
        assert_eq!(stopped.state, "disabled");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn re_enabling_after_a_stop_restarts_the_same_stable_port() {
        use tauri::test::mock_app;
        let dir = temp_dir();
        let app = mock_app();
        app.manage(RemoteConnectionManager::new(&dir));
        let manager = app.state::<RemoteConnectionManager>();
        let prefs = RemoteAccessPreferences {
            enabled: true,
            tailscale_backup: false,
            local_port: 51_337,
            ..Default::default()
        };

        let first = manager.apply(app.handle(), prefs.clone()).await.unwrap();
        assert_eq!(first.listener_port, Some(51_337));
        manager
            .apply(
                app.handle(),
                RemoteAccessPreferences {
                    enabled: false,
                    ..prefs.clone()
                },
            )
            .await
            .unwrap();
        let second = manager.apply(app.handle(), prefs).await.unwrap();
        assert_eq!(second.listener_port, Some(51_337));
        assert!(second.listener_running);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalid_ports_are_rejected_without_starting_anything() {
        use tauri::test::mock_app;
        let dir = temp_dir();
        let app = mock_app();
        app.manage(RemoteConnectionManager::new(&dir));
        let manager = app.state::<RemoteConnectionManager>();

        let error = manager
            .apply(
                app.handle(),
                RemoteAccessPreferences {
                    enabled: true,
                    local_port: 80,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(error.contains("1024"));
        assert!(!manager.status().listener_running);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_endpoint_is_stopped_only_when_it_changed_or_the_backup_is_off() {
        // Same endpoint, backup still on: keep it and let `serve_start` verify.
        assert_eq!(stale_serve_to_stop(Some((51234, 443)), 51234, 443, true), None);
        // HTTPS port changed: remove the old endpoint.
        assert_eq!(
            stale_serve_to_stop(Some((51234, 443)), 51234, 8443, true),
            Some((51234, 443))
        );
        // Local port changed: remove the old endpoint.
        assert_eq!(
            stale_serve_to_stop(Some((51234, 443)), 52000, 443, true),
            Some((51234, 443))
        );
        // Backup disabled: remove the previously owned endpoint.
        assert_eq!(
            stale_serve_to_stop(Some((51234, 443)), 51234, 443, false),
            Some((51234, 443))
        );
        // No live record but the backup is off: clean the configured port.
        assert_eq!(
            stale_serve_to_stop(None, 51234, 443, false),
            Some((51234, 443))
        );
        // No live record and the backup is on: nothing to stop.
        assert_eq!(stale_serve_to_stop(None, 51234, 443, true), None);
    }

    #[test]
    fn the_old_endpoint_is_removed_first_only_when_it_holds_the_same_https_port() {
        assert!(stale_occupies_port(Some((51234, 443)), 443));
        assert!(!stale_occupies_port(Some((51234, 443)), 8443));
        assert!(!stale_occupies_port(None, 443));
    }
}
