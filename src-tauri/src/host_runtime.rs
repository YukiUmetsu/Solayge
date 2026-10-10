//! Process-global runtime facts about the mobile host listeners.
//!
//! Kept dependency-light and always compiled so the pairing UI can read the
//! Iroh endpoint id and the Tailscale Serve authority whether or not the
//! `mobile-network` feature is enabled. When a listener is not running the
//! corresponding value is `None`, and the QR carries no reachability hint.

use std::sync::{OnceLock, RwLock};

static ENDPOINT_ID: OnceLock<String> = OnceLock::new();
static TAILSCALE_AUTHORITY: RwLock<Option<String>> = RwLock::new(None);

/// Record the live Iroh listener's endpoint id. Called once when the host binds;
/// a second call is ignored.
pub fn set_endpoint_id(id: impl Into<String>) {
    let _ = ENDPOINT_ID.set(id.into());
}

/// The Iroh listener's endpoint id, if the host is running.
pub fn endpoint_id() -> Option<String> {
    ENDPOINT_ID.get().cloned()
}

/// Record the Tailscale Serve authority (`host.tailnet.ts.net` or
/// `host.tailnet.ts.net:port`) the phone should dial for the WSS fallback.
pub fn set_tailscale_authority(authority: impl Into<String>) {
    if let Ok(mut guard) = TAILSCALE_AUTHORITY.write() {
        *guard = Some(authority.into());
    }
}

/// Clear the advertised Serve authority (for example when the backup is turned
/// off or Serve is stopped).
pub fn clear_tailscale_authority() {
    if let Ok(mut guard) = TAILSCALE_AUTHORITY.write() {
        *guard = None;
    }
}

/// The advertised Tailscale Serve authority, if the backup is live.
pub fn tailscale_authority() -> Option<String> {
    TAILSCALE_AUTHORITY
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}
