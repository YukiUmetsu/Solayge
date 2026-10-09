//! Desktop host: connected-device management and QR pairing.
//!
//! This is the desktop side of the mobile trust model. It owns the desktop's
//! stable Ed25519 identity (in the OS credential store), the registry of approved
//! phones, and the [`PairingServer`] state machine. The device registry is the
//! single authorization authority every transport — Iroh or Tailscale — must
//! consult, so revoking a device here invalidates it everywhere.
//!
//! Everything here is non-cryptographic glue over `solayge-identity`; no key
//! material is invented or duplicated.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tauri::State;

use solayge_identity::{
    default_store, load_or_create_identity, DeviceIdentity, DeviceRegistry, FileStore,
    PairingConfig, PairingServer, PairingState, PublicIdentity, Scope,
};

/// Keychain/DPAPI/file namespace for the desktop identity.
const APP_ID: &str = "com.solayge.desktop";
/// Where the approved-device registry is persisted.
const REGISTRY_FILE: &str = "connected-devices.json";
/// Where each device's push token is persisted, keyed by device id.
const PUSH_TOKENS_FILE: &str = "push-tokens.json";

/// One device's push registration. The token is an opaque FCM address; it is
/// only ever forwarded to the push gateway, never shown or logged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushToken {
    pub token: String,
    pub platform: String,
    pub updated_at: i64,
}

/// The desktop-side device manager, managed by Tauri.
///
/// The approved-device registry is a plain file, but the Ed25519 identity lives
/// in the OS credential store. That store can prompt (macOS asks for the login
/// password when the item's owning app changed, e.g. after a reinstall), so the
/// identity is loaded lazily: a fresh install that never uses mobile never
/// touches the keychain. The first use — pairing or the transport host — loads
/// it once.
pub struct DeviceManager {
    data_dir: PathBuf,
    registry: Arc<DeviceRegistry>,
    server: OnceLock<PairingServer>,
    /// Serializes first-use identity creation so two callers cannot each mint a
    /// different identity (and clobber the other's persisted key).
    init: Mutex<()>,
    /// Push tokens by device id, for background FCM delivery.
    push_tokens: Mutex<HashMap<String, PushToken>>,
}

impl DeviceManager {
    /// Open the manager over the app data directory. Reads only the registry
    /// file; the identity is loaded on first use ([`Self::ensure_server`]).
    pub fn new(data_dir: &Path) -> Result<Self> {
        let data_dir = data_dir.to_path_buf();
        std::fs::create_dir_all(&data_dir).context("creating the app data directory")?;
        let registry = Arc::new(DeviceRegistry::from_snapshot(read_registry(&data_dir)));
        let push_tokens = read_push_tokens(&data_dir);
        Ok(DeviceManager {
            data_dir,
            registry,
            server: OnceLock::new(),
            init: Mutex::new(()),
            push_tokens: Mutex::new(push_tokens),
        })
    }

    /// Build a manager over an explicit secure store, loading the identity at
    /// once. Tests use the encrypted file store so they never touch the
    /// developer's real keychain.
    #[cfg(test)]
    pub(crate) fn with_store(
        data_dir: &Path,
        store: &dyn solayge_identity::SecureStore,
    ) -> Result<Self> {
        let manager = DeviceManager::new(data_dir)?;
        let identity = load_or_create_identity(store).context("loading the desktop identity")?;
        manager.install_identity(identity);
        Ok(manager)
    }

    fn install_identity(&self, identity: DeviceIdentity) {
        let server =
            PairingServer::with_registry(identity, PairingConfig::default(), self.registry.clone());
        let _ = self.server.set(server);
    }

    /// Load and install the desktop identity on first use; a no-op afterwards.
    /// The native store is preferred, falling back to the encrypted file store
    /// so host functionality is preserved when the keychain is unavailable.
    fn ensure_server(&self) -> Result<&PairingServer> {
        if let Some(server) = self.server.get() {
            return Ok(server);
        }
        let _guard = self.init.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(server) = self.server.get() {
            return Ok(server);
        }
        let identity = match load_or_create_identity(default_store(APP_ID, &self.data_dir).as_ref()) {
            Ok(identity) => identity,
            Err(error) => {
                eprintln!("[devices] native identity store unavailable, using file store: {error}");
                load_or_create_identity(&FileStore::new(&self.data_dir, APP_ID))
                    .context("loading the desktop identity")?
            }
        };
        self.install_identity(identity);
        self.server.get().context("installing the desktop identity")
    }

    /// The shared registry, for the transport layer's per-request authorization.
    #[allow(dead_code)] // consumed by the transport layer (solayge-network)
    pub fn registry(&self) -> &Arc<DeviceRegistry> {
        &self.registry
    }

    fn persist(&self) -> Result<()> {
        write_registry(&self.data_dir, &self.registry.snapshot())
    }

    fn persist_push_tokens(&self) -> Result<()> {
        let snapshot = self
            .push_tokens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        write_push_tokens(&self.data_dir, &snapshot)
    }

    /// Record a device's push token. Only called for an authenticated device.
    pub fn set_push_token(&self, device_id: &str, token: String, platform: String) -> Result<()> {
        {
            let mut guard = self.push_tokens.lock().unwrap_or_else(|p| p.into_inner());
            guard.insert(
                device_id.to_string(),
                PushToken {
                    token,
                    platform,
                    updated_at: now(),
                },
            );
        }
        self.persist_push_tokens()
    }

    /// Forget a device's push token, e.g. on explicit unregister or revocation.
    pub fn clear_push_token(&self, device_id: &str) -> Result<()> {
        {
            let mut guard = self.push_tokens.lock().unwrap_or_else(|p| p.into_inner());
            guard.remove(device_id);
        }
        self.persist_push_tokens()
    }

    /// The token registered for one device, if any.
    #[allow(dead_code)] // read by tests and future push diagnostics
    pub fn push_token(&self, device_id: &str) -> Option<PushToken> {
        self.push_tokens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(device_id)
            .cloned()
    }

    /// Every push token whose device is still authorized. A revoked device is
    /// excluded even if its token lingers.
    pub fn push_targets(&self) -> Vec<(String, PushToken)> {
        let guard = self.push_tokens.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .iter()
            .filter(|(device_id, _)| self.registry.is_authorized(device_id))
            .map(|(device_id, token)| (device_id.clone(), token.clone()))
            .collect()
    }

    fn desktop_public(&self) -> Result<PublicIdentity> {
        Ok(self.ensure_server()?.desktop_identity())
    }

    /// The pairing authority, for the transport host handler. Loads the identity
    /// on first call.
    pub(crate) fn pairing_server(&self) -> Result<&PairingServer> {
        self.ensure_server()
    }

    /// The desktop's own device id, as a string.
    #[allow(dead_code)] // read by the mobile-network host handler
    pub(crate) fn desktop_id_string(&self) -> Result<String> {
        Ok(self.ensure_server()?.desktop_id().to_string())
    }

    fn device_view(device: &solayge_identity::PairedDevice) -> DeviceView {
        DeviceView {
            device_id: device.device_id.clone(),
            name: device.name.clone(),
            platform: device.platform.clone(),
            scopes: device.scopes.iter().map(|s| s.to_string()).collect(),
            paired_at: device.paired_at,
            last_seen: device.last_seen,
            online: device.online,
            fingerprint: fingerprint_of(&device.public_key),
        }
    }
}

/// The desktop's public identity, for the Add-phone dialog header.
#[derive(Debug, Serialize)]
pub struct DesktopIdentityView {
    /// The derived desktop id.
    pub desktop_id: String,
    /// A short, grouped-hex fingerprint.
    pub fingerprint: String,
}

/// The live state of a pairing challenge, including the verification code the
/// phone derives once it has sent `pair.begin`.
#[derive(Debug, Serialize)]
pub struct PairingStatusView {
    /// `awaiting_proof` | `pending` | `approved` | `rejected` | `consumed`.
    pub state: String,
    /// The short code only once both nonces are bound; `None` before then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_code: Option<String>,
    /// The claimed phone name, once `pair.begin` arrived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    /// The claimed platform, once `pair.begin` arrived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_platform: Option<String>,
}

/// One approved phone, as shown in Settings → Connected devices.
#[derive(Debug, Serialize)]
pub struct DeviceView {
    /// The device id.
    pub device_id: String,
    /// The claimed device name.
    pub name: String,
    /// `ios` | `android` | …
    pub platform: String,
    /// Granted scopes, e.g. `["sessions.read", "sessions.message"]`.
    pub scopes: Vec<String>,
    /// Unix seconds when the device was approved.
    pub paired_at: i64,
    /// Unix seconds of the last authorized connection, if any.
    pub last_seen: Option<i64>,
    /// Whether a transport session is currently active.
    pub online: bool,
    /// A short fingerprint of the pinned public key.
    pub fingerprint: String,
}

/// A pairing offer for the Add-phone dialog: a QR payload and a manual code.
#[derive(Debug, Serialize)]
pub struct PairingOfferView {
    /// The one-time pairing id.
    pub pairing_id: String,
    /// The short manual fallback code.
    pub manual_code: String,
    /// Unix seconds after which the challenge is refused.
    pub expires_at: i64,
    /// The JSON string to render as a QR code.
    pub qr_string: String,
    /// The desktop's device id.
    pub desktop_id: String,
    /// A short fingerprint of the desktop's public key.
    pub desktop_fingerprint: String,
    /// Whether the Iroh listener advertised an address the phone can dial.
    pub reachable: bool,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn fingerprint_of(public_key_base64: &str) -> String {
    PublicIdentity::from_base64(public_key_base64)
        .map(|identity| identity.short_fingerprint())
        .unwrap_or_default()
}

fn registry_path(data_dir: &Path) -> PathBuf {
    data_dir.join(REGISTRY_FILE)
}

fn push_tokens_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PUSH_TOKENS_FILE)
}

fn read_push_tokens(data_dir: &Path) -> HashMap<String, PushToken> {
    std::fs::read_to_string(push_tokens_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_push_tokens(data_dir: &Path, tokens: &HashMap<String, PushToken>) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let bytes = serde_json::to_vec_pretty(tokens)?;
    write_private(&push_tokens_path(data_dir), &bytes)?;
    Ok(())
}

fn read_registry(data_dir: &Path) -> solayge_identity::registry::RegistrySnapshot {
    std::fs::read_to_string(registry_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_registry(
    data_dir: &Path,
    snapshot: &solayge_identity::registry::RegistrySnapshot,
) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let bytes = serde_json::to_vec_pretty(snapshot)?;
    write_private(&registry_path(data_dir), &bytes)?;
    Ok(())
}

/// Write owner-only, matching the rest of the app's at-rest data.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.flush()?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

// ---- Tauri commands ----

/// The desktop's public identity.
#[tauri::command]
pub fn desktop_identity(manager: State<'_, DeviceManager>) -> Result<DesktopIdentityView, String> {
    let public = manager.desktop_public().map_err(|e| e.to_string())?;
    Ok(DesktopIdentityView {
        desktop_id: public.device_id().to_string(),
        fingerprint: public.short_fingerprint(),
    })
}

/// Every approved phone, oldest pairing first.
#[tauri::command]
pub fn devices_list(manager: State<'_, DeviceManager>) -> Vec<DeviceView> {
    manager
        .registry
        .list()
        .iter()
        .map(DeviceManager::device_view)
        .collect()
}

/// Start a new pairing challenge (desktop **Add phone**).
///
/// This is the first point at which the desktop actually needs its mobile
/// identity and transport key, so starting the listener here is what triggers
/// the OS credential-store prompt — only when the user adds a phone, never on
/// first launch.
#[tauri::command]
pub async fn pairing_start(
    app: tauri::AppHandle,
    manager: State<'_, DeviceManager>,
) -> Result<PairingOfferView, String> {
    #[cfg(feature = "mobile-network")]
    {
        crate::iroh_host::ensure_started(app.clone());
        // Let the listener publish its endpoint id so the QR carries a direct
        // dial hint; the fallback after the wait is the manual/relay path.
        for _ in 0..20 {
            if crate::host_runtime::endpoint_id().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    #[cfg(not(feature = "mobile-network"))]
    let _ = &app;

    // Advertise the Iroh endpoint id when the listener is running, so the phone
    // can dial this desktop. The hint carries no secret; trust still requires
    // approve-on-desktop and both sides' identity proofs.
    let hints = solayge_identity::ReachabilityHints {
        iroh_endpoint_id: crate::host_runtime::endpoint_id(),
        // Advertised only when the Tailscale backup is configured and running,
        // so a phone never tries an endpoint that does not exist.
        tailscale_host: crate::host_runtime::tailscale_authority(),
        ..Default::default()
    };
    let reachable = hints.iroh_endpoint_id.is_some();
    let offer = manager
        .pairing_server()
        .map_err(|e| e.to_string())?
        .start_with_hints(now(), hints)
        .map_err(|e| e.to_string())?;
    let qr_string = offer.qr.to_qr_string().map_err(|e| e.to_string())?;
    let desktop = manager.desktop_public().map_err(|e| e.to_string())?;
    Ok(PairingOfferView {
        pairing_id: offer.pairing_id,
        manual_code: offer.manual_code,
        expires_at: offer.expires_at,
        qr_string,
        desktop_id: desktop.device_id().to_string(),
        desktop_fingerprint: desktop.short_fingerprint(),
        reachable,
    })
}

/// The live state of a challenge, including the verification code once the phone
/// has begun pairing. `None` when the id is unknown.
#[tauri::command]
pub fn pairing_status(
    manager: State<'_, DeviceManager>,
    pairing_id: String,
) -> Option<PairingStatusView> {
    let server = manager.pairing_server().ok()?;
    let state = server.session_state(&pairing_id)?;
    let device = server.pending_device(&pairing_id);
    Some(PairingStatusView {
        state: state.wire_name().to_string(),
        verification_code: server.verification_code(&pairing_id),
        device_name: device.as_ref().map(|(name, _)| name.clone()),
        device_platform: device.as_ref().map(|(_, platform)| platform.clone()),
    })
}

/// Approve a phone. `scopes` of `None`/empty defaults to View + Message; the UI
/// must opt in explicitly to anything wider.
#[tauri::command]
pub fn pairing_approve(
    manager: State<'_, DeviceManager>,
    pairing_id: String,
    scopes: Option<Vec<String>>,
) -> Result<DeviceView, String> {
    let grants = match scopes {
        Some(list) => Some(Scope::parse_all(&list).map_err(|e| e.to_string())?),
        None => None,
    };
    let device = manager
        .pairing_server()
        .map_err(|e| e.to_string())?
        .approve(&pairing_id, grants, now())
        .map_err(|e| e.to_string())?;
    manager.persist().map_err(|e| e.to_string())?;
    Ok(DeviceManager::device_view(&device))
}

/// Reject an in-flight pairing.
#[tauri::command]
pub fn pairing_reject(manager: State<'_, DeviceManager>, pairing_id: String) -> Result<(), String> {
    manager
        .pairing_server()
        .map_err(|e| e.to_string())?
        .reject(&pairing_id, now())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Revoke a device. The registry immediately stops authorizing it, so every
/// transport and reconnect fails until it is paired again.
#[tauri::command]
pub fn device_revoke(manager: State<'_, DeviceManager>, device_id: String) -> Result<(), String> {
    manager.registry.revoke(&device_id);
    manager.persist().map_err(|e| e.to_string())?;
    // A revoked device must not keep receiving background pushes.
    manager
        .clear_push_token(&device_id)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// A small accessor used by tests and the transport layer.
#[allow(dead_code)] // consumed by the transport layer (solayge-network) and tests
impl DeviceManager {
    /// The current session state, if the id is known.
    pub fn state_of(&self, pairing_id: &str) -> Option<PairingState> {
        self.pairing_server().ok()?.session_state(pairing_id)
    }

    /// Touch a device's `last_seen` after an authorized connection.
    pub fn touch(&self, device_id: &str, when: i64) -> Result<()> {
        self.registry
            .authorize(device_id, when)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        self.persist()
    }

    /// The signed-in desktop identity (for host-side signing).
    pub fn identity(&self) -> Result<&DeviceIdentity> {
        Ok(self.ensure_server()?.desktop_identity_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "solayge-devices-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    #[test]
    fn pairing_and_revocation_persist_across_restarts() {
        let dir = temp_dir();
        let store = || solayge_identity::FileStore::new(&dir, "test-identity");
        let manager = DeviceManager::with_store(&dir, &store()).unwrap();
        let offer = manager.pairing_server().unwrap().start(now()).unwrap();

        // A phone completes pairing.
        let phone = DeviceIdentity::generate();
        let mut client = solayge_identity::PairingClient::new(phone, "Pixel", "android");
        let begin = client.begin_request(&offer).unwrap();
        let response = manager
            .pairing_server()
            .unwrap()
            .handle_begin(begin, now() + 1)
            .unwrap();
        client.accept_begin(&response).unwrap();
        let proof = client.prove_identity_request(&response).unwrap();
        manager
            .pairing_server()
            .unwrap()
            .handle_prove_identity(proof, now() + 2)
            .unwrap();
        let device = manager
            .pairing_server()
            .unwrap()
            .approve(&offer.pairing_id, None, now() + 3)
            .unwrap();
        manager.persist().unwrap();
        let id = device.device_id.clone();

        // Restart: the device is still known and authorized.
        drop(manager);
        let restarted = DeviceManager::with_store(&dir, &store()).unwrap();
        assert!(restarted.registry().is_authorized(&id));
        assert_eq!(restarted.registry().list().len(), 1);

        // Revoke: authorization stops immediately and stays stopped.
        restarted.registry().revoke(&id);
        restarted.persist().unwrap();
        assert!(!restarted.registry().is_authorized(&id));
        drop(restarted);

        let again = DeviceManager::with_store(&dir, &store()).unwrap();
        assert!(!again.registry().is_authorized(&id));
        assert!(again.registry().is_revoked(&id));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn push_tokens_persist_and_unknown_devices_are_not_targets() {
        let dir = temp_dir();
        let store = || solayge_identity::FileStore::new(&dir, "test-identity");
        let manager = DeviceManager::with_store(&dir, &store()).unwrap();
        manager
            .set_push_token("dev_x", "token-1".into(), "android".into())
            .unwrap();
        assert_eq!(manager.push_token("dev_x").unwrap().token, "token-1");
        // A device that is not authorized is never a push target.
        assert!(manager.push_targets().is_empty());
        drop(manager);

        let restarted = DeviceManager::with_store(&dir, &store()).unwrap();
        assert_eq!(restarted.push_token("dev_x").unwrap().token, "token-1");
        restarted.clear_push_token("dev_x").unwrap();
        assert!(restarted.push_token("dev_x").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
