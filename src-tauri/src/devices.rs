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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use tauri::State;

use solayge_identity::{
    default_store, load_or_create_identity, DeviceIdentity, DeviceRegistry, FileStore,
    PairingConfig, PairingServer, PairingState, PublicIdentity, Scope,
};

/// Keychain/DPAPI/file namespace for the desktop identity.
const APP_ID: &str = "com.solayge.desktop";
/// Where the approved-device registry is persisted.
const REGISTRY_FILE: &str = "connected-devices.json";

/// The desktop-side device manager, managed by Tauri.
pub struct DeviceManager {
    data_dir: PathBuf,
    registry: Arc<DeviceRegistry>,
    server: PairingServer,
}

impl DeviceManager {
    /// Load the desktop identity and the approved-device registry, creating both
    /// on first run. If the native credential store is unavailable the encrypted
    /// file store is used instead, so host functionality is preserved.
    pub fn new(data_dir: &Path) -> Result<Self> {
        let data_dir = data_dir.to_path_buf();
        std::fs::create_dir_all(&data_dir).context("creating the app data directory")?;

        let identity = match load_or_create_identity(default_store(APP_ID, &data_dir).as_ref()) {
            Ok(identity) => identity,
            Err(error) => {
                eprintln!("[devices] native identity store unavailable, using file store: {error}");
                load_or_create_identity(&FileStore::new(&data_dir, APP_ID))
                    .context("loading the desktop identity")?
            }
        };

        DeviceManager::with_identity(data_dir, identity)
    }

    /// Build a manager over an explicit secure store. Tests use the encrypted
    /// file store so they never touch the developer's real keychain.
    #[cfg(test)]
    pub(crate) fn with_store(
        data_dir: &Path,
        store: &dyn solayge_identity::SecureStore,
    ) -> Result<Self> {
        let data_dir = data_dir.to_path_buf();
        std::fs::create_dir_all(&data_dir).context("creating the app data directory")?;
        let identity = load_or_create_identity(store).context("loading the desktop identity")?;
        DeviceManager::with_identity(data_dir, identity)
    }

    fn with_identity(data_dir: PathBuf, identity: DeviceIdentity) -> Result<Self> {
        let registry = Arc::new(DeviceRegistry::from_snapshot(read_registry(&data_dir)));
        let server =
            PairingServer::with_registry(identity, PairingConfig::default(), registry.clone());

        Ok(DeviceManager {
            data_dir,
            registry,
            server,
        })
    }

    /// The shared registry, for the transport layer's per-request authorization.
    #[allow(dead_code)] // consumed by the transport layer (solayge-network)
    pub fn registry(&self) -> &Arc<DeviceRegistry> {
        &self.registry
    }

    fn persist(&self) -> Result<()> {
        write_registry(&self.data_dir, &self.registry.snapshot())
    }

    fn desktop_public(&self) -> PublicIdentity {
        self.server.desktop_identity()
    }

    /// The pairing authority, for the transport host handler.
    pub(crate) fn pairing_server(&self) -> &PairingServer {
        &self.server
    }

    /// The desktop's own device id, as a string.
    pub(crate) fn desktop_id_string(&self) -> String {
        self.server.desktop_id().to_string()
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
pub fn desktop_identity(manager: State<'_, DeviceManager>) -> DesktopIdentityView {
    let public = manager.desktop_public();
    DesktopIdentityView {
        desktop_id: public.device_id().to_string(),
        fingerprint: public.short_fingerprint(),
    }
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
#[tauri::command]
pub fn pairing_start(manager: State<'_, DeviceManager>) -> Result<PairingOfferView, String> {
    // Advertise the Iroh endpoint id when the listener is running, so the phone
    // can dial this desktop. The hint carries no secret; trust still requires
    // approve-on-desktop and both sides' identity proofs.
    let hints = solayge_identity::ReachabilityHints {
        iroh_endpoint_id: crate::host_runtime::endpoint_id(),
        ..Default::default()
    };
    let reachable = hints.iroh_endpoint_id.is_some();
    let offer = manager
        .server
        .start_with_hints(now(), hints)
        .map_err(|e| e.to_string())?;
    let qr_string = offer.qr.to_qr_string().map_err(|e| e.to_string())?;
    let desktop = manager.desktop_public();
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
    let state = manager.server.session_state(&pairing_id)?;
    let device = manager.server.pending_device(&pairing_id);
    Some(PairingStatusView {
        state: state.wire_name().to_string(),
        verification_code: manager.server.verification_code(&pairing_id),
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
        .server
        .approve(&pairing_id, grants, now())
        .map_err(|e| e.to_string())?;
    manager.persist().map_err(|e| e.to_string())?;
    Ok(DeviceManager::device_view(&device))
}

/// Reject an in-flight pairing.
#[tauri::command]
pub fn pairing_reject(manager: State<'_, DeviceManager>, pairing_id: String) -> Result<(), String> {
    manager
        .server
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
    Ok(())
}

// A small accessor used by tests and the transport layer.
#[allow(dead_code)] // consumed by the transport layer (solayge-network) and tests
impl DeviceManager {
    /// The current session state, if the id is known.
    pub fn state_of(&self, pairing_id: &str) -> Option<PairingState> {
        self.server.session_state(pairing_id)
    }

    /// Touch a device's `last_seen` after an authorized connection.
    pub fn touch(&self, device_id: &str, when: i64) -> Result<()> {
        self.registry
            .authorize(device_id, when)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        self.persist()
    }

    /// The signed-in desktop identity (for host-side signing).
    pub fn identity(&self) -> &DeviceIdentity {
        self.server.desktop_identity_ref()
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
        let offer = manager.server.start(now()).unwrap();

        // A phone completes pairing.
        let phone = DeviceIdentity::generate();
        let mut client = solayge_identity::PairingClient::new(phone, "Pixel", "android");
        let begin = client.begin_request(&offer).unwrap();
        let response = manager.server.handle_begin(begin, now() + 1).unwrap();
        client.accept_begin(&response).unwrap();
        let proof = client.prove_identity_request(&response).unwrap();
        manager
            .server
            .handle_prove_identity(proof, now() + 2)
            .unwrap();
        let device = manager
            .server
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
}
