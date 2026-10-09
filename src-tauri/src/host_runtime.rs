//! Process-global runtime facts about the mobile host listener.
//!
//! Kept dependency-light and always compiled so the pairing UI can read the
//! Iroh endpoint id whether or not the `mobile-network` feature is enabled. When
//! the listener is not running the id is simply `None`, and the QR carries no
//! reachability hint (the manual path still works on the same machine).

use std::sync::OnceLock;

static ENDPOINT_ID: OnceLock<String> = OnceLock::new();

/// Record the live listener's endpoint id. Called once when the host binds; a
/// second call is ignored.
pub fn set_endpoint_id(id: impl Into<String>) {
    let _ = ENDPOINT_ID.set(id.into());
}

/// The listener's endpoint id, if the host is running.
pub fn endpoint_id() -> Option<String> {
    ENDPOINT_ID.get().cloned()
}
