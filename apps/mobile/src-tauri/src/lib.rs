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

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{Manager, State};

use solayge_identity::{
    default_store, load_or_create_identity, prove, AuthRequest, AuthResponse, DeviceIdentity,
    FileStore, PairBeginResponse, PairProveIdentityResponse, PairingClient, PairingQrPayload,
    SecureStore, PROTOCOL_VERSION,
};
use solayge_network::{EndpointType, IrohConfig, IrohRemoteClient};
use solayge_protocol::{Action, Envelope, MessageKind, ProtocolError, RemoteClient, WireError};

/// Keychain/DPAPI/file namespace for the phone identity.
const APP_ID: &str = "com.solayge.mobile";
/// Where the approved desktops are recorded on the phone.
const PAIRED_FILE: &str = "paired-desktops.json";

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

/// A live, maybe-authenticated connection to one desktop.
struct LiveSession {
    desktop_id: String,
    desktop_public_key: String,
    endpoint_id: String,
    device_id: String,
    client: Arc<IrohRemoteClient>,
    authenticated: bool,
}

/// The mobile app's shared state.
struct MobileState {
    data_dir: PathBuf,
    store: Box<dyn SecureStore>,
    identity: Mutex<Option<DeviceIdentity>>,
    session: Mutex<Option<LiveSession>>,
    paired: Mutex<Vec<PairedDesktop>>,
}

impl MobileState {
    fn new(data_dir: PathBuf, store: Box<dyn SecureStore>) -> Self {
        let paired = read_paired(&data_dir);
        MobileState {
            data_dir,
            store,
            identity: Mutex::new(None),
            session: Mutex::new(None),
            paired: Mutex::new(paired),
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
    let endpoint_id = payload
        .hints
        .iroh_endpoint_id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            "This QR code does not include a connection address. Make sure the desktop's \
             mobile host is enabled and running, then show a new code."
                .to_string()
        })?;

    let identity = state.identity()?;
    let mut pairing = PairingClient::new(
        identity.clone(),
        "Solayge Mobile",
        std::env::consts::OS,
    );
    let begin = pairing
        .begin_request_for_qr(&payload)
        .map_err(|e| e.to_string())?;

    let client = solayge_network::connect_by_endpoint_id(client_config(), &endpoint_id)
        .map_err(|e| format!("Could not reach the desktop: {e}"))?;
    let client = Arc::new(client);

    let response: PairBeginResponse =
        exchange_typed(&client, Action::PairBegin, to_value(&begin)?)
            .map_err(|e| format!("The desktop refused the pairing: {e}"))?;

    // Verifies the desktop's proof and that both sides derive the same code.
    let verified = pairing
        .accept_begin(&response)
        .map_err(|e| format!("The desktop's identity proof did not verify: {e}"))?;
    let proof = pairing
        .prove_identity_request(&response)
        .map_err(|e| e.to_string())?;
    let _: PairProveIdentityResponse =
        exchange_typed(&client, Action::PairProveIdentity, to_value(&proof)?)
            .map_err(|e| format!("The desktop refused this phone's identity proof: {e}"))?;

    *state.lock_session() = Some(LiveSession {
        desktop_id: response.desktop_id.clone(),
        desktop_public_key: response.desktop_public_key.clone(),
        endpoint_id,
        device_id: identity.device_id().to_string(),
        client,
        authenticated: false,
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
    let (client, desktop_id, desktop_public_key, endpoint_id, device_id, authenticated) = {
        let guard = state.lock_session();
        match guard.as_ref() {
            Some(session) => (
                session.client.clone(),
                session.desktop_id.clone(),
                session.desktop_public_key.clone(),
                session.endpoint_id.clone(),
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
        &client,
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
    match exchange_typed::<AuthResponse>(&client, Action::ConnectionAuthenticate, to_value(&auth)?) {
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
        &client,
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

/// Dial and authenticate a previously paired desktop.
#[tauri::command]
fn connect_desktop(
    state: State<'_, MobileState>,
    desktop_id: String,
) -> Result<ConnectionStatus, String> {
    let record = state
        .paired_list()
        .into_iter()
        .find(|d| d.id == desktop_id)
        .ok_or_else(|| "This desktop is not paired with this phone".to_string())?;
    let identity = state.identity()?;

    let client = solayge_network::connect_by_endpoint_id(client_config(), &record.endpoint_id)
        .map_err(|e| format!("Could not reach {}: {e}", record.name))?;
    let client = Arc::new(client);

    // Authenticate this connection with the pinned key.
    let mut auth = AuthRequest {
        device_id: identity.device_id().to_string(),
        desktop_id: record.id.clone(),
        protocol_version: PROTOCOL_VERSION,
        host_nonce: None,
        proof: None,
        transport_binding: None,
    };
    let challenge: AuthResponse =
        exchange_typed(&client, Action::ConnectionAuthenticate, to_value(&auth)?)?;
    let nonce = challenge
        .host_nonce
        .ok_or_else(|| "The desktop did not issue an authentication challenge".to_string())?;
    auth.host_nonce = Some(nonce);
    auth.proof = Some(prove(&identity, &auth).map_err(|e| e.to_string())?);
    let result: AuthResponse =
        exchange_typed(&client, Action::ConnectionAuthenticate, to_value(&auth)?)?;
    if !result.authenticated {
        return Err("The desktop did not accept this phone's identity proof".into());
    }

    *state.lock_session() = Some(LiveSession {
        desktop_id: record.id.clone(),
        desktop_public_key: record.public_key,
        endpoint_id: record.endpoint_id,
        device_id: identity.device_id().to_string(),
        client,
        authenticated: true,
    });
    Ok(ConnectionStatus {
        connected: true,
        authenticated: true,
        desktop_id: Some(desktop_id),
    })
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
        },
        None => ConnectionStatus { connected: false, authenticated: false, desktop_id: None },
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
    client: &IrohRemoteClient,
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
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let store = identity_store(&data_dir);
            app.manage(MobileState::new(data_dir, store));
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
            connect_desktop,
            disconnect,
            connection_status,
            remote_call,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Solayge Mobile");
}
