//! Solayge Mobile — the Tauri 2 shell for the companion client.
//!
//! The mobile app is a control surface: it pairs with, authenticates to, and
//! reads/writes a desktop host through the `solayge-protocol` contract. It never
//! runs coding agents itself.
//!
//! This shell owns:
//!
//! * the installation's Ed25519 identity in platform secure storage;
//! * the QR/manual pairing handshake over the real Iroh transport, including the
//!   desktop-approval wait and persistence of the paired desktop;
//! * a single live, authenticated [`IrohRemoteClient`] that the React UI drives
//!   through the generic [`remote_call`] command.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_notification::NotificationExt;

use solayge_identity::{
    default_store, load_or_create_identity, prove, AuthRequest, AuthResponse, DeviceIdentity,
    FileStore, PairBeginResponse, PairProveIdentityResponse, PairingClient, PairingQrPayload,
    SecureStore, PROTOCOL_VERSION,
};
use solayge_network::{EndpointType, IrohConfig, WsClientConfig, WsRemoteClient};
use solayge_protocol::{Action, Envelope, MessageKind, ProtocolError, RemoteClient, WireError};

/// Keychain/DPAPI/file namespace for the phone identity.
const APP_ID: &str = "com.solayge.mobile";
/// Where the approved desktops are recorded on the phone.
const PAIRED_FILE: &str = "paired-desktops.json";
/// Where the user's notification preferences are persisted.
const NOTIFICATION_SETTINGS_FILE: &str = "notification-settings.json";
/// Where the monitor remembers what it has already notified about.
const NOTIFICATION_STATE_FILE: &str = "notification-state.json";
/// The Android channel notifications are delivered on. Created from the UI so
/// its importance can follow the user's sound preference.
const NOTIFICATION_CHANNEL: &str = "solayge";
/// Where the Android FCM service writes the device's registration token.
const PUSH_TOKEN_FILE: &str = "push-token.json";
/// How often the background monitor polls a connected desktop.
const NOTIFICATION_POLL_SECS: u64 = 20;

/// The protocol version this client speaks. Surfaced to the UI for diagnostics.
#[tauri::command]
fn protocol_version() -> u32 {
    PROTOCOL_VERSION
}

/// The installation's public identity, for diagnostics and the verification UI.
#[derive(Debug, Serialize)]
struct IdentityView {
    device_id: String,
    public_key: String,
    /// A short grouped-hex fingerprint of the public key itself.
    fingerprint: String,
}

/// The result of scanning a pairing QR: everything the onboarding UI shows and
/// the `pair.begin` request the transport will carry to the desktop.
#[derive(Debug, Serialize)]
struct ParsedPairing {
    desktop_id: String,
    desktop_public_key: String,
    desktop_fingerprint: String,
    pairing_id: String,
    expires_at: i64,
    /// The code the phone derives from the QR; the user compares it with the
    /// code the desktop shows.
    verification_code: String,
    device_id: String,
    /// Whether the QR carried an address the phone can actually dial.
    reachable: bool,
    /// The desktop's tailnet-only Serve authority, when configured.
    tailscale_host: Option<String>,
}

/// The manual-code entry point, for phones without a camera.
#[derive(Debug, Serialize)]
struct ManualPairing {
    device_id: String,
    manual_code: String,
}

/// A paired desktop, as the mobile client remembers it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PairedDesktop {
    /// The desktop's device id.
    id: String,
    name: String,
    platform: String,
    /// The desktop's Iroh endpoint id, used to dial it.
    endpoint_id: String,
    /// The desktop's tailnet-only Serve authority (`host.tailnet.ts.net[:port]`),
    /// used for the WebSocket fallback. Absent when the desktop did not
    /// advertise one at pairing time.
    #[serde(default)]
    tailscale_host: Option<String>,
    /// The pinned desktop public key, base64url.
    public_key: String,
    fingerprint: String,
    paired_at: i64,
}

/// The result of starting a pairing over the transport.
#[derive(Debug, Serialize)]
struct PairStart {
    pairing_id: String,
    device_id: String,
    desktop_id: String,
    /// The code the user compares with the desktop.
    verification_code: String,
}

/// The polled state of a pairing awaiting desktop approval.
#[derive(Debug, Serialize)]
struct PairStatus {
    /// `none` | `pending` | `approved` | `error`.
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    desktop: Option<PairedDesktop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// The current live connection, if any.
#[derive(Debug, Serialize)]
struct ConnectionStatus {
    connected: bool,
    authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    desktop_id: Option<String>,
    /// `iroh` | `tailscale_ws` — the transport the current session uses.
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<String>,
}

/// A typed remote failure the UI can switch on.
#[derive(Debug, Serialize)]
struct RemoteError {
    code: String,
    message: String,
}

impl From<ProtocolError> for RemoteError {
    fn from(error: ProtocolError) -> Self {
        RemoteError {
            code: error.code().as_str().to_string(),
            message: error.message().to_string(),
        }
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl From<RemoteError> for String {
    fn from(error: RemoteError) -> Self {
        error.message
    }
}

/// A live, maybe-authenticated connection to one desktop. The client is
/// transport-agnostic: the same authenticated application session is reused
/// whether it was established over Iroh or the Tailscale WebSocket fallback.
struct LiveSession {
    desktop_id: String,
    desktop_public_key: String,
    endpoint_id: String,
    tailscale_host: Option<String>,
    device_id: String,
    client: Arc<dyn RemoteClient>,
    authenticated: bool,
    /// `iroh` | `tailscale_ws`
    transport: String,
}

/// The user's notification preferences, mirrored by the Settings UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NotificationSettings {
    /// Master switch. Off until the user turns notifications on.
    enabled: bool,
    /// Agent questions and permission requests that need a decision.
    on_questions: bool,
    /// Tasks that finished successfully.
    on_completed: bool,
    /// Tasks that failed or errored.
    on_failed: bool,
    /// Whether the OS notification plays a sound.
    sound: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        NotificationSettings {
            enabled: false,
            on_questions: true,
            on_completed: true,
            on_failed: true,
            sound: true,
        }
    }
}

/// What the background monitor last saw, so it notifies only about new events.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct SeenState {
    approvals: Vec<String>,
    sessions: std::collections::HashMap<String, String>,
}

/// The mobile app's shared state.
struct MobileState {
    data_dir: PathBuf,
    store: Box<dyn SecureStore>,
    identity: Mutex<Option<DeviceIdentity>>,
    session: Mutex<Option<LiveSession>>,
    paired: Mutex<Vec<PairedDesktop>>,
    notifications: Mutex<NotificationSettings>,
}

impl MobileState {
    fn new(data_dir: PathBuf, store: Box<dyn SecureStore>) -> Self {
        let paired = read_paired(&data_dir);
        let notifications =
            read_json(&data_dir.join(NOTIFICATION_SETTINGS_FILE)).unwrap_or_default();
        MobileState {
            data_dir,
            store,
            identity: Mutex::new(None),
            session: Mutex::new(None),
            paired: Mutex::new(paired),
            notifications: Mutex::new(notifications),
        }
    }

    /// Load the identity once, cloning it for the caller. A read error is
    /// surfaced rather than replaced with a new identity.
    fn identity(&self) -> Result<DeviceIdentity, String> {
        let mut guard = self.lock_identity();
        if guard.is_none() {
            *guard = Some(load_or_create_identity(self.store.as_ref()).map_err(|e| e.to_string())?);
        }
        Ok(guard.as_ref().expect("just populated").clone())
    }

    fn lock_identity(&self) -> std::sync::MutexGuard<'_, Option<DeviceIdentity>> {
        self.identity.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn pairing_client(&self) -> Result<PairingClient, String> {
        let identity = self.identity()?;
        Ok(PairingClient::new(
            identity,
            "Solayge Mobile",
            std::env::consts::OS,
        ))
    }

    fn upsert_paired(&self, record: PairedDesktop) {
        {
            let mut guard = self.paired.lock().unwrap_or_else(|p| p.into_inner());
            guard.retain(|d| d.id != record.id);
            guard.push(record);
        }
        let snapshot = self.paired.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Err(error) = write_paired(&self.data_dir, &snapshot) {
            eprintln!("[mobile] could not persist paired desktops: {error}");
        }
    }

    fn forget(&self, id: &str) {
        {
            let mut guard = self.paired.lock().unwrap_or_else(|p| p.into_inner());
            guard.retain(|d| d.id != id);
        }
        let snapshot = self.paired.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let _ = write_paired(&self.data_dir, &snapshot);
        // Drop a live session for the desktop being forgotten.
        let mut session = self.session.lock().unwrap_or_else(|p| p.into_inner());
        if session.as_ref().is_some_and(|s| s.desktop_id == id) {
            *session = None;
        }
    }

    fn lock_session(&self) -> std::sync::MutexGuard<'_, Option<LiveSession>> {
        self.session.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn paired_list(&self) -> Vec<PairedDesktop> {
        self.paired
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn notification_settings(&self) -> NotificationSettings {
        self.notifications
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn set_notification_settings(&self, settings: NotificationSettings) {
        {
            let mut guard = self.notifications.lock().unwrap_or_else(|p| p.into_inner());
            *guard = settings.clone();
        }
        if let Err(error) = write_json(&self.data_dir.join(NOTIFICATION_SETTINGS_FILE), &settings) {
            eprintln!("[mobile] could not persist notification settings: {error}");
        }
    }

    /// The live, authenticated client and the desktop it belongs to, if any.
    fn live_client(&self) -> Option<(String, Arc<dyn RemoteClient>)> {
        let guard = self.session.lock().unwrap_or_else(|p| p.into_inner());
        let session = guard.as_ref()?;
        if !session.authenticated {
            return None;
        }
        Some((session.desktop_id.clone(), session.client.clone()))
    }
}

/// Resolve the identity secure store: the OS keychain/keystore when usable, else
/// the encrypted file store. Probing here means the first `identity()` call
/// cannot fail because a platform store is unavailable.
fn identity_store(data_dir: &Path) -> Box<dyn SecureStore> {
    let os_store = default_store(APP_ID, data_dir);
    if load_or_create_identity(os_store.as_ref()).is_ok() {
        os_store
    } else {
        Box::new(FileStore::new(data_dir, APP_ID))
    }
}

/// The production client configuration: public n0 discovery and relay. A direct
/// path is preferred when available; the relay carries the traffic otherwise.
fn client_config() -> IrohConfig {
    IrohConfig::production(EndpointType::MobileClient)
}

#[tauri::command]
fn mobile_identity(state: State<'_, MobileState>) -> Result<IdentityView, String> {
    let identity = state.identity()?;
    let public = identity.public();
    Ok(IdentityView {
        device_id: identity.device_id().to_string(),
        public_key: public.to_base64(),
        fingerprint: public.short_fingerprint(),
    })
}

#[tauri::command]
fn parse_pairing_qr(state: State<'_, MobileState>, qr: String) -> Result<ParsedPairing, String> {
    let payload = PairingQrPayload::from_qr_string(&qr).map_err(|e| e.to_string())?;
    let client = state.pairing_client()?;
    let transcript = client.expect_from_qr(&payload).map_err(|e| e.to_string())?;
    let desktop = payload.desktop_identity().map_err(|e| e.to_string())?;
    let reachable = payload
        .hints
        .iroh_endpoint_id
        .as_deref()
        .is_some_and(|id| !id.is_empty());

    Ok(ParsedPairing {
        desktop_id: payload.desktop_id.clone(),
        desktop_public_key: payload.desktop_public_key.clone(),
        desktop_fingerprint: desktop.short_fingerprint(),
        pairing_id: payload.pairing_id.clone(),
        expires_at: payload.expires_at,
        verification_code: transcript.verification_code().to_string(),
        device_id: client.device_id().to_string(),
        reachable,
        tailscale_host: payload.hints.tailscale_host.clone(),
    })
}

#[tauri::command]
fn manual_pairing_request(
    state: State<'_, MobileState>,
    code: String,
) -> Result<ManualPairing, String> {
    let trimmed = code.trim().to_uppercase();
    if trimmed.is_empty() {
        return Err("Enter the pairing code shown on your desktop".into());
    }
    let client = state.pairing_client()?;
    // Validate that the code is at least the right shape before continuing.
    client
        .begin_request_manual(&trimmed)
        .map_err(|e| e.to_string())?;
    Ok(ManualPairing {
        device_id: client.device_id().to_string(),
        manual_code: trimmed,
    })
}

/// Dial the desktop in a scanned QR, run `pair.begin` + `pair.prove_identity`,
/// and keep the connection open so [`pair_poll`] can wait for approval.
#[tauri::command]
fn pair_with_qr(state: State<'_, MobileState>, qr: String) -> Result<PairStart, String> {
    let payload = PairingQrPayload::from_qr_string(&qr).map_err(|e| e.to_string())?;
    let iroh_id = payload
        .hints
        .iroh_endpoint_id
        .clone()
        .filter(|id| !id.is_empty());
    let tailscale_host = payload.hints.tailscale_host.clone();
    if iroh_id.is_none() && tailscale_host.is_none() {
        return Err("This QR code does not include a connection address. Make sure the \
                    desktop's mobile host is enabled and running, then show a new code."
            .to_string());
    }

    let identity = state.identity()?;
    let mut pairing = PairingClient::new(
        identity.clone(),
        "Solayge Mobile",
        std::env::consts::OS,
    );
    let begin = pairing
        .begin_request_for_qr(&payload)
        .map_err(|e| e.to_string())?;

    let (client, transport) = dial_for_pairing(iroh_id.clone(), tailscale_host.clone())?;

    let response: PairBeginResponse =
        exchange_typed(client.as_ref(), Action::PairBegin, to_value(&begin)?)
            .map_err(|e| format!("The desktop refused the pairing: {e}"))?;

    // Verifies the desktop's proof and that both sides derive the same code.
    let verified = pairing
        .accept_begin(&response)
        .map_err(|e| format!("The desktop's identity proof did not verify: {e}"))?;
    let proof = pairing
        .prove_identity_request(&response)
        .map_err(|e| e.to_string())?;
    let _: PairProveIdentityResponse =
        exchange_typed(client.as_ref(), Action::PairProveIdentity, to_value(&proof)?)
            .map_err(|e| format!("The desktop refused this phone's identity proof: {e}"))?;

    *state.lock_session() = Some(LiveSession {
        desktop_id: response.desktop_id.clone(),
        desktop_public_key: response.desktop_public_key.clone(),
        endpoint_id: iroh_id.unwrap_or_default(),
        tailscale_host,
        device_id: identity.device_id().to_string(),
        client,
        authenticated: false,
        transport: transport.to_string(),
    });

    Ok(PairStart {
        pairing_id: response.pairing_id,
        device_id: verified.device_id.to_string(),
        desktop_id: response.desktop_id,
        verification_code: verified.verification_code.to_string(),
    })
}

/// Poll for desktop approval by attempting connection authentication. Approval
/// on the desktop registers this phone, which makes the signed proof succeed.
#[tauri::command]
fn pair_poll(state: State<'_, MobileState>) -> Result<PairStatus, String> {
    let (
        client,
        desktop_id,
        desktop_public_key,
        endpoint_id,
        tailscale_host,
        device_id,
        authenticated,
    ) = {
        let guard = state.lock_session();
        match guard.as_ref() {
            Some(session) => (
                session.client.clone(),
                session.desktop_id.clone(),
                session.desktop_public_key.clone(),
                session.endpoint_id.clone(),
                session.tailscale_host.clone(),
                session.device_id.clone(),
                session.authenticated,
            ),
            None => return Ok(PairStatus { state: "none".into(), desktop: None, message: None }),
        }
    };
    if authenticated {
        return Ok(PairStatus {
            state: "approved".into(),
            desktop: state.paired_list().into_iter().find(|d| d.id == desktop_id),
            message: None,
        });
    }

    let identity = state.identity()?;
    let mut auth = AuthRequest {
        device_id: device_id.clone(),
        desktop_id: desktop_id.clone(),
        protocol_version: PROTOCOL_VERSION,
        host_nonce: None,
        proof: None,
        transport_binding: None,
    };

    // Leg 1: request a challenge. Before approval this still succeeds.
    let challenge = match exchange_typed::<AuthResponse>(
        client.as_ref(),
        Action::ConnectionAuthenticate,
        to_value(&auth)?,
    ) {
        Ok(challenge) => challenge,
        Err(error) if error.code == "transport" => {
            return Ok(PairStatus {
                state: "error".into(),
                desktop: None,
                message: Some(error.message),
            })
        }
        Err(_) => {
            return Ok(PairStatus { state: "pending".into(), desktop: None, message: None })
        }
    };
    if challenge.authenticated {
        return Ok(PairStatus { state: "pending".into(), desktop: None, message: None });
    }
    let Some(nonce) = challenge.host_nonce else {
        return Ok(PairStatus { state: "pending".into(), desktop: None, message: None });
    };

    // Leg 2: prove possession of the key pairing pinned for this device.
    auth.host_nonce = Some(nonce);
    auth.proof = Some(prove(&identity, &auth).map_err(|e| e.to_string())?);
    match exchange_typed::<AuthResponse>(
        client.as_ref(),
        Action::ConnectionAuthenticate,
        to_value(&auth)?,
    ) {
        Ok(result) if result.authenticated => {}
        Ok(_) => return Ok(PairStatus { state: "pending".into(), desktop: None, message: None }),
        Err(error) if error.code == "transport" => {
            return Ok(PairStatus {
                state: "error".into(),
                desktop: None,
                message: Some(error.message),
            })
        }
        // Not registered yet: still waiting for the human to approve.
        Err(_) => return Ok(PairStatus { state: "pending".into(), desktop: None, message: None }),
    }

    // Approved. Read the host metadata for a friendly name and persist. A grant
    // without `sessions.read` cannot read metadata, so fall back to a generic
    // name rather than leaving the wizard stuck.
    let metadata = exchange_typed::<solayge_protocol::HostMetadata>(
        client.as_ref(),
        Action::HostsGetMetadata,
        Value::Null,
    )
    .ok();
    let record = PairedDesktop {
        id: desktop_id.clone(),
        name: metadata
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "Solayge Desktop".to_string()),
        platform: metadata
            .as_ref()
            .map(|m| m.platform.clone())
            .unwrap_or_default(),
        endpoint_id,
        tailscale_host,
        public_key: desktop_public_key.clone(),
        fingerprint: solayge_identity::PublicIdentity::from_base64(&desktop_public_key)
            .map(|p| p.short_fingerprint())
            .unwrap_or_default(),
        paired_at: now_secs(),
    };
    state.upsert_paired(record.clone());
    if let Some(session) = state.lock_session().as_mut() {
        session.authenticated = true;
    }
    Ok(PairStatus { state: "approved".into(), desktop: Some(record), message: None })
}

/// Stop waiting on a pairing and drop the provisional connection.
#[tauri::command]
fn pair_cancel(state: State<'_, MobileState>) {
    let mut session = state.lock_session();
    if session.as_ref().is_some_and(|s| !s.authenticated) {
        *session = None;
    }
}

/// Approved desktops, oldest pairing first.
#[tauri::command]
fn paired_desktops(state: State<'_, MobileState>) -> Vec<PairedDesktopView> {
    let mut list: Vec<PairedDesktop> = state.paired_list();
    list.sort_by_key(|d| (d.paired_at, d.id.clone()));
    list.into_iter().map(PairedDesktopView::from).collect()
}

#[derive(Debug, Serialize)]
struct PairedDesktopView {
    id: String,
    name: String,
    platform: String,
    fingerprint: String,
}

impl From<PairedDesktop> for PairedDesktopView {
    fn from(record: PairedDesktop) -> Self {
        PairedDesktopView {
            id: record.id,
            name: record.name,
            platform: record.platform,
            fingerprint: record.fingerprint,
        }
    }
}

/// Forget a paired desktop and close any live session with it.
#[tauri::command]
fn forget_desktop(state: State<'_, MobileState>, desktop_id: String) {
    state.forget(&desktop_id);
}

/// The current notification preferences.
#[tauri::command]
fn notification_settings(state: State<'_, MobileState>) -> NotificationSettings {
    state.notification_settings()
}

/// Persist notification preferences. The background monitor reads them on its
/// next tick, so changes take effect without restarting the app.
#[tauri::command]
fn set_notification_settings(
    state: State<'_, MobileState>,
    settings: NotificationSettings,
) -> Result<(), String> {
    state.set_notification_settings(settings);
    Ok(())
}

/// The device's FCM token, written by the Android `FirebaseMessagingService`.
/// `None` until Firebase has issued one (or on non-Android targets).
#[derive(Debug, Serialize)]
struct PushTokenView {
    token: String,
    platform: String,
}

#[tauri::command]
fn push_token(state: State<'_, MobileState>) -> Option<PushTokenView> {
    let value: Value = read_json(&state.data_dir.join(PUSH_TOKEN_FILE))?;
    let token = value.get("token")?.as_str()?.trim().to_string();
    if token.is_empty() {
        return None;
    }
    Some(PushTokenView {
        token,
        platform: value
            .get("platform")
            .and_then(Value::as_str)
            .unwrap_or("android")
            .to_string(),
    })
}

/// A notification tap the user made while the app was closed. Reading it
/// consumes it, so the app navigates once.
#[tauri::command]
fn take_push_tap(state: State<'_, MobileState>) -> Option<Value> {
    let path = state.data_dir.join("push-tap.json");
    let value: Value = read_json(&path)?;
    let _ = std::fs::remove_file(&path);
    Some(value)
}

/// Dial and authenticate a previously paired desktop.
///
/// In `automatic` mode (the default) Iroh is tried first and the Tailscale
/// WebSocket backup is tried only when Iroh is unreachable and the desktop
/// advertised a Serve endpoint. An authorization denial is never retried or
/// fallen back from: a revoked phone stays denied on every transport.
#[tauri::command]
fn connect_desktop(
    state: State<'_, MobileState>,
    desktop_id: String,
    mode: Option<String>,
) -> Result<ConnectionStatus, String> {
    let record = state
        .paired_list()
        .into_iter()
        .find(|d| d.id == desktop_id)
        .ok_or_else(|| "This desktop is not paired with this phone".to_string())?;
    let identity = state.identity()?;
    let mode = mode.unwrap_or_else(|| "automatic".to_string());

    let candidates = transport_candidates(&mode);

    let mut outcomes: Vec<String> = Vec::new();
    for transport in candidates {
        let built: Result<(Arc<dyn RemoteClient>, String), String> = match transport {
            "iroh" => solayge_network::connect_by_endpoint_id(client_config(), &record.endpoint_id)
                .map(|client| (Arc::new(client) as Arc<dyn RemoteClient>, "iroh".to_string()))
                .map_err(|error| format!("Iroh: {error}")),
            "tailscale" => match record.tailscale_host.as_deref() {
                Some(host) => WsRemoteClient::connect(WsClientConfig::wss(wss_url(host)))
                    .map(|client| (Arc::new(client) as Arc<dyn RemoteClient>, "tailscale_ws".to_string()))
                    .map_err(|error| format!("Tailscale: {error}")),
                None => Err(
                    "Tailscale: this desktop has not advertised a Tailscale endpoint".to_string(),
                ),
            },
            other => Err(format!("unknown transport '{other}'")),
        };

        match built {
            Ok((client, label)) => match authenticate_client(client.as_ref(), &identity, &record.id)
            {
                Ok(()) => {
                    *state.lock_session() = Some(LiveSession {
                        desktop_id: record.id.clone(),
                        desktop_public_key: record.public_key.clone(),
                        endpoint_id: record.endpoint_id.clone(),
                        tailscale_host: record.tailscale_host.clone(),
                        device_id: identity.device_id().to_string(),
                        client,
                        authenticated: true,
                        transport: label.clone(),
                    });
                    return Ok(ConnectionStatus {
                        connected: true,
                        authenticated: true,
                        desktop_id: Some(desktop_id),
                        transport: Some(label),
                    });
                }
                Err(error) => {
                    // Revocation/invalid proof is terminal; never fall back.
                    if error.code == "not_authorized" {
                        return Err(error.message);
                    }
                    outcomes.push(format!("{label}: {}", error.message));
                }
            },
            Err(error) => outcomes.push(error),
        }
    }

    Err(format!(
        "Could not connect to {}: {}",
        record.name,
        outcomes.join("; ")
    ))
}

/// Run the same challenge-response handshake the Iroh pairing flow uses, over
/// whichever transport the client speaks. A successful handshake returns
/// nothing; revocations and bad proofs surface as `not_authorized`.
fn authenticate_client(
    client: &dyn RemoteClient,
    identity: &DeviceIdentity,
    desktop_id: &str,
) -> Result<(), RemoteError> {
    let mut auth = AuthRequest {
        device_id: identity.device_id().to_string(),
        desktop_id: desktop_id.to_string(),
        protocol_version: PROTOCOL_VERSION,
        host_nonce: None,
        proof: None,
        transport_binding: None,
    };
    let payload = serde_json::to_value(&auth).map_err(internal_error)?;
    let challenge: AuthResponse =
        exchange_typed(client, Action::ConnectionAuthenticate, payload)?;
    let nonce = challenge.host_nonce.ok_or_else(|| RemoteError {
        code: "transport".into(),
        message: "The desktop did not issue an authentication challenge".into(),
    })?;
    auth.host_nonce = Some(nonce);
    auth.proof = Some(prove(identity, &auth).map_err(internal_error)?);
    let payload = serde_json::to_value(&auth).map_err(internal_error)?;
    let result: AuthResponse = exchange_typed(client, Action::ConnectionAuthenticate, payload)?;
    if !result.authenticated {
        return Err(RemoteError {
            code: "not_authorized".into(),
            message: "The desktop did not accept this phone's identity proof".into(),
        });
    }
    Ok(())
}

fn internal_error(error: impl std::fmt::Display) -> RemoteError {
    RemoteError {
        code: "internal".into(),
        message: error.to_string(),
    }
}

/// The transports to try, in order, for a user's mode preference. Unknown modes
/// use the automatic order: Iroh primary, Tailscale WSS fallback.
fn transport_candidates(mode: &str) -> Vec<&'static str> {
    match mode {
        "iroh" => vec!["iroh"],
        "tailscale" => vec!["tailscale"],
        _ => vec!["iroh", "tailscale"],
    }
}

/// Build the `wss://` URL for a Serve authority (`host` or `host:port`).
fn wss_url(authority: &str) -> String {
    let trimmed = authority.trim().trim_end_matches('/');
    let normalized = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("wss://"))
        .unwrap_or(trimmed);
    format!("wss://{normalized}/")
}

/// Choose a transport for pairing: Iroh first, then Tailscale when it is the
/// only advertised address or Iroh could not be dialed.
fn dial_for_pairing(
    iroh_id: Option<String>,
    tailscale_host: Option<String>,
) -> Result<(Arc<dyn RemoteClient>, &'static str), String> {
    if let Some(id) = iroh_id {
        match solayge_network::connect_by_endpoint_id(client_config(), &id) {
            Ok(client) => return Ok((Arc::new(client), "iroh")),
            Err(error) => {
                if tailscale_host.is_none() {
                    return Err(format!("Could not reach the desktop: {error}"));
                }
                eprintln!("[mobile] Iroh pairing failed, trying Tailscale: {error}");
            }
        }
    }
    let host = tailscale_host.ok_or_else(|| {
        "This QR code does not include a connection address. Make sure the desktop's mobile \
         host is enabled and running, then show a new code."
            .to_string()
    })?;
    WsRemoteClient::connect(WsClientConfig::wss(wss_url(&host)))
        .map(|client| (Arc::new(client) as Arc<dyn RemoteClient>, "tailscale_ws"))
        .map_err(|error| format!("Could not reach the desktop over Tailscale: {error}"))
}

#[tauri::command]
fn disconnect(state: State<'_, MobileState>) {
    *state.lock_session() = None;
}

#[tauri::command]
fn connection_status(state: State<'_, MobileState>) -> ConnectionStatus {
    match state.lock_session().as_ref() {
        Some(session) => ConnectionStatus {
            connected: session.client.is_connected(),
            authenticated: session.authenticated,
            desktop_id: Some(session.desktop_id.clone()),
            transport: Some(session.transport.clone()),
        },
        None => ConnectionStatus {
            connected: false,
            authenticated: false,
            desktop_id: None,
            transport: None,
        },
    }
}

/// Forward one protocol action to the live desktop. This is the UI's only write
/// path; the React `RemoteClient` maps its methods onto it.
#[tauri::command]
fn remote_call(
    state: State<'_, MobileState>,
    action: String,
    payload: Option<Value>,
    session_id: Option<String>,
    command_id: Option<String>,
) -> Result<Value, RemoteError> {
    let (client, desktop_id, authenticated) = {
        let guard = state.lock_session();
        match guard.as_ref() {
            Some(session) => (
                session.client.clone(),
                session.desktop_id.clone(),
                session.authenticated,
            ),
            None => {
                return Err(RemoteError {
                    code: "transport".into(),
                    message: "Not connected to a desktop".into(),
                })
            }
        }
    };
    if !authenticated {
        return Err(RemoteError {
            code: "not_authorized".into(),
            message: "This phone is not authenticated with the desktop".into(),
        });
    }

    let action = Action::parse(&action).ok_or_else(|| RemoteError {
        code: "unknown_action".into(),
        message: format!("unknown action: {action}"),
    })?;
    let mut envelope = Envelope::command(action, payload.unwrap_or(Value::Null));
    envelope.desktop_id = Some(desktop_id);
    if let Some(session_id) = session_id.filter(|s| !s.is_empty()) {
        envelope.session_id = Some(session_id);
    }
    if let Some(command_id) = command_id.filter(|c| !c.is_empty()) {
        envelope.command_id = Some(command_id);
    }

    let response = client.exchange(envelope).map_err(RemoteError::from)?;
    if response.kind == MessageKind::Error {
        let wire: WireError = response.payload_as().map_err(RemoteError::from)?;
        return Err(RemoteError { code: wire.code, message: wire.message });
    }
    Ok(response.payload)
}

fn exchange_typed<T: serde::de::DeserializeOwned>(
    client: &dyn RemoteClient,
    action: Action,
    payload: Value,
) -> Result<T, RemoteError> {
    let response = client
        .exchange(Envelope::command(action, payload))
        .map_err(RemoteError::from)?;
    match response.kind {
        MessageKind::Error => {
            let wire: WireError = response.payload_as().map_err(RemoteError::from)?;
            Err(RemoteError { code: wire.code, message: wire.message })
        }
        _ => response.payload_as::<T>().map_err(|e| RemoteError {
            code: "invalid_payload".into(),
            message: e.to_string(),
        }),
    }
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn paired_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PAIRED_FILE)
}

fn read_paired(data_dir: &Path) -> Vec<PairedDesktop> {
    std::fs::read_to_string(paired_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_paired(data_dir: &Path, records: &[PairedDesktop]) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let bytes = serde_json::to_vec_pretty(records)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(paired_path(data_dir))?;
        file.write_all(&bytes)?;
        file.flush()?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(paired_path(data_dir), &bytes)?;
    }
    Ok(())
}

/// Read a JSON file, falling back to the type's default when missing or corrupt.
fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write a JSON file atomically enough for a best-effort preference/state store.
fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, &bytes)
}

/// Start the background notification monitor. It keeps polling the connected
/// desktop from the Rust process, so notifications are produced even while the
/// webview is throttled or suspended by the OS. The thread lives for the whole
/// app process.
fn start_notification_monitor(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("solayge-mobile-notify".to_string())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(NOTIFICATION_POLL_SECS));
            poll_notifications(&app);
        });
}

/// One monitor tick: read the desktop, notify about anything new, remember it.
fn poll_notifications(app: &AppHandle) {
    let state = app.state::<MobileState>();
    // When FCM is configured, the Android push service owns background delivery
    // (and suppresses itself in the foreground). Running this local poller too
    // would double-notify and keep waking the radio, so it stands down.
    if state.data_dir.join(PUSH_TOKEN_FILE).exists() {
        return;
    }
    let settings = state.notification_settings();
    if !settings.enabled {
        return;
    }
    let Some((desktop_id, client)) = state.live_client() else {
        return;
    };
    let sessions = match client.list_sessions() {
        Ok(sessions) => sessions,
        Err(_) => return,
    };
    let approvals = match client.list_approvals() {
        Ok(approvals) => approvals,
        Err(_) => return,
    };

    let path = state.data_dir.join(NOTIFICATION_STATE_FILE);
    let mut all: std::collections::HashMap<String, SeenState> = read_json(&path).unwrap_or_default();
    let first_observation = !all.contains_key(&desktop_id);
    let mut seen = all.remove(&desktop_id).unwrap_or_default();

    if !first_observation {
        for approval in &approvals {
            if settings.on_questions && !seen.approvals.iter().any(|id| id == &approval.id) {
                let title = if approval.kind == solayge_protocol::ApprovalKind::Question {
                    "Agent has a question"
                } else {
                    "Permission needed"
                };
                show_notification(app, &settings, title, &approval.title);
            }
        }
        for session in &sessions {
            let Some(previous) = seen.sessions.get(&session.id) else {
                continue;
            };
            if previous == &session.status {
                continue;
            }
            if settings.on_completed
                && matches!(session.status.as_str(), "succeeded" | "completed")
            {
                show_notification(app, &settings, "Task completed", &session.title);
            } else if settings.on_failed && session.status == "failed" {
                show_notification(app, &settings, "Task failed", &session.title);
            }
        }
    }

    seen.approvals = approvals.iter().map(|a| a.id.clone()).collect();
    seen.sessions = sessions
        .iter()
        .map(|s| (s.id.clone(), s.status.clone()))
        .collect();
    all.insert(desktop_id, seen);
    if let Err(error) = write_json(&path, &all) {
        eprintln!("[mobile] could not persist notification state: {error}");
    }
}

/// Emit one OS notification, honoring the sound preference.
fn show_notification(
    app: &AppHandle,
    settings: &NotificationSettings,
    title: &str,
    body: &str,
) {
    let mut builder = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .channel_id(NOTIFICATION_CHANNEL);
    if settings.sound {
        builder = builder.sound("default");
    }
    if let Err(error) = builder.show() {
        eprintln!("[mobile] notification was not shown: {error}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    // The official barcode scanner is mobile-only (its crate is
    // `#![cfg(mobile)]`), so it is registered only when building for
    // Android/iOS. On a desktop dev build the dependency compiles to an empty
    // crate and would not expose `init` at all.
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());

    builder
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let store = identity_store(&data_dir);
            app.manage(MobileState::new(data_dir, store));
            start_notification_monitor(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            protocol_version,
            mobile_identity,
            parse_pairing_qr,
            manual_pairing_request,
            pair_with_qr,
            pair_poll,
            pair_cancel,
            paired_desktops,
            forget_desktop,
            notification_settings,
            set_notification_settings,
            push_token,
            take_push_tap,
            connect_desktop,
            disconnect,
            connection_status,
            remote_call,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Solayge Mobile");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wss_url_normalizes_a_serve_authority() {
        assert_eq!(
            wss_url("macbook-pro.berylline-goblin.ts.net"),
            "wss://macbook-pro.berylline-goblin.ts.net/"
        );
        assert_eq!(
            wss_url("macbook-pro.berylline-goblin.ts.net:8443"),
            "wss://macbook-pro.berylline-goblin.ts.net:8443/"
        );
        assert_eq!(wss_url("https://host.tailnet.ts.net/"), "wss://host.tailnet.ts.net/");
        assert_eq!(wss_url("wss://host.tailnet.ts.net/"), "wss://host.tailnet.ts.net/");
        assert_eq!(wss_url("  host.ts.net/  "), "wss://host.ts.net/");
    }

    #[test]
    fn transport_candidates_order_iroh_primary_then_tailscale_fallback() {
        assert_eq!(transport_candidates("automatic"), vec!["iroh", "tailscale"]);
        assert_eq!(transport_candidates("something-else"), vec!["iroh", "tailscale"]);
        assert_eq!(transport_candidates("iroh"), vec!["iroh"]);
        assert_eq!(transport_candidates("tailscale"), vec!["tailscale"]);
    }

    #[test]
    fn a_paired_desktop_without_a_tailscale_host_still_loads() {
        // Older records predate the fallback hint; the field must default.
        let json = r#"[{
            "id": "desk_1",
            "name": "MacBook Pro",
            "platform": "macos",
            "endpoint_id": "abc",
            "public_key": "key",
            "fingerprint": "fp",
            "paired_at": 1
        }]"#;
        let records: Vec<PairedDesktop> = serde_json::from_str(json).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].tailscale_host.is_none());
    }
}
