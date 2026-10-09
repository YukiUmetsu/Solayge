//! Solayge Mobile — the Tauri 2 shell for the companion client.
//!
//! The mobile app is a control surface: it pairs with, authenticates to, and
//! reads/writes a desktop host through the `solayge-protocol` contract. It never
//! runs coding agents itself.
//!
//! This shell owns the installation's Ed25519 [identity](solayge_identity) in
//! platform secure storage and exposes the QR/manual pairing entry points to the
//! React onboarding flow. The transport itself (Iroh/Tailscale) is a separate
//! layer; these commands operate on the identity and pairing payloads only.
#![deny(unsafe_code)]

use std::sync::Mutex;

use serde::Serialize;
use tauri::{Manager, State};

use solayge_identity::{
    default_store, load_or_create_identity, DeviceIdentity, PairBeginRequest, PairingClient,
    PairingQrPayload, SecureStore, PROTOCOL_VERSION,
};

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
    begin_request: PairBeginRequest,
}

/// The manual-code entry point, for phones without a camera.
#[derive(Debug, Serialize)]
struct ManualPairing {
    device_id: String,
    manual_code: String,
    begin_request: PairBeginRequest,
}

/// The mobile app's shared state: the secure store and the lazily loaded
/// installation identity.
struct MobileState {
    store: Box<dyn SecureStore>,
    identity: Mutex<Option<DeviceIdentity>>,
}

impl MobileState {
    fn new(store: Box<dyn SecureStore>) -> Self {
        MobileState {
            store,
            identity: Mutex::new(None),
        }
    }

    /// Load the identity once, cloning it for the caller. A read error is
    /// surfaced rather than replaced with a new identity.
    fn identity(&self) -> Result<DeviceIdentity, String> {
        let mut guard = self
            .identity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = Some(load_or_create_identity(self.store.as_ref()).map_err(|e| e.to_string())?);
        }
        Ok(guard.as_ref().expect("just populated").clone())
    }
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
    let client = pairing_client(&state)?;
    let transcript = client.expect_from_qr(&payload).map_err(|e| e.to_string())?;
    let begin_request = client
        .begin_request_for_qr(&payload)
        .map_err(|e| e.to_string())?;
    let desktop = payload.desktop_identity().map_err(|e| e.to_string())?;

    Ok(ParsedPairing {
        desktop_id: payload.desktop_id.clone(),
        desktop_public_key: payload.desktop_public_key.clone(),
        desktop_fingerprint: desktop.short_fingerprint(),
        pairing_id: payload.pairing_id.clone(),
        expires_at: payload.expires_at,
        verification_code: transcript.verification_code().to_string(),
        device_id: client.device_id().to_string(),
        begin_request,
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
    let client = pairing_client(&state)?;
    let begin_request = client
        .begin_request_manual(&trimmed)
        .map_err(|e| e.to_string())?;
    Ok(ManualPairing {
        device_id: client.device_id().to_string(),
        manual_code: trimmed,
        begin_request,
    })
}

fn pairing_client(state: &State<'_, MobileState>) -> Result<PairingClient, String> {
    let identity = state.identity()?;
    Ok(PairingClient::new(
        identity,
        "Solayge Mobile",
        std::env::consts::OS,
    ))
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
            let store = default_store("com.solayge.mobile", &data_dir);
            app.manage(MobileState::new(store));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            protocol_version,
            mobile_identity,
            parse_pairing_qr,
            manual_pairing_request,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Solayge Mobile");
}
