//! Stub remote-access commands for builds without the `mobile-network`
//! feature. The real implementation lives in `remote.rs`; this keeps the
//! command surface and the QR pairing UI compiling (but disabled) when the
//! network transport is not compiled in.

use serde::{Deserialize, Serialize};

fn default_https_port() -> u16 {
    443
}
fn default_local_port() -> u16 {
    51234
}

/// Mirrors `remote::RemoteAccessPreferences`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAccessPreferences {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub tailscale_backup: bool,
    #[serde(default = "default_https_port")]
    pub https_port: u16,
    #[serde(default = "default_local_port")]
    pub local_port: u16,
}

/// Mirrors the subset of `remote::RemoteStatus` the UI reads.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteStatus {
    pub enabled: bool,
    pub state: String,
    pub listener_running: bool,
    pub connected_clients: usize,
    pub tailscale_backup: bool,
    pub last_error: Option<String>,
}

fn unavailable() -> RemoteStatus {
    RemoteStatus {
        enabled: false,
        state: "unavailable".to_string(),
        listener_running: false,
        connected_clients: 0,
        tailscale_backup: false,
        last_error: Some(
            "Remote access is unavailable in this build (mobile-network is disabled)".to_string(),
        ),
    }
}

#[tauri::command]
pub fn remote_status() -> RemoteStatus {
    unavailable()
}

#[tauri::command]
pub async fn remote_apply(_prefs: RemoteAccessPreferences) -> Result<RemoteStatus, String> {
    Err("Remote access is unavailable in this build".to_string())
}

#[tauri::command]
pub async fn remote_retry() -> Result<RemoteStatus, String> {
    Err("Remote access is unavailable in this build".to_string())
}

/// No-op: there is nothing to stop when the feature is compiled out.
pub fn shutdown(_app: &tauri::AppHandle) {}
