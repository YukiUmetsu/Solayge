//! Application-layer connection authentication.
//!
//! Pairing pins a phone's Ed25519 public identity on the desktop. That alone
//! does not authorize a *connection*: an attacker who can reach the desktop over
//! Iroh could otherwise send `connection.authenticate` claiming any registered
//! device id. This module adds the missing proof — the phone signs a
//! host-issued, single-use challenge with the private key pairing already bound
//! to that device id.
//!
//! Flow:
//!
//! 1. Phone sends an [`AuthRequest`] with no `proof`.
//! 2. Host issues a fresh 32-byte nonce ([`ChallengeStore::issue`]) and returns
//!    it in [`AuthResponse::host_nonce`].
//! 3. Phone signs [`auth_message`] and sends the proof back.
//! 4. Host consumes the nonce (single use, short TTL) and verifies the signature
//!    against the public key pinned for that device id.
//!
//! Every request after authentication must be tied to the connection that
//! completed this handshake; the transport layer carries a per-connection
//! [`crate::DeviceId`] for that (see `solayge-network`). The challenge is never
//! reused across connections.
//!
//! No new cryptography is introduced: signatures are Ed25519 through
//! [`DeviceIdentity`] and [`PublicIdentity`].

use std::collections::HashMap;
use std::sync::Mutex;

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::error::{IdentityError, Result};
use crate::identity::{decode_b64, encode_b64, DeviceIdentity, PublicIdentity};
use crate::NONCE_LEN;

/// Domain separator for the canonical auth message.
pub const AUTH_CONTEXT: &[u8] = b"solayge/connection-auth/v1";

/// How long an issued challenge stays valid.
pub const AUTH_TTL_SECS: i64 = 120;

/// The phone → host authentication request. When `proof` is absent the host
/// treats this as a challenge request; when present it verifies the signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthRequest {
    /// The paired device id being claimed.
    pub device_id: String,
    /// The desktop id the phone believes it is talking to.
    pub desktop_id: String,
    /// The protocol version in force.
    pub protocol_version: u32,
    /// The host nonce echoed from the challenge, base64url.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_nonce: Option<String>,
    /// The signature over [`auth_message`], base64url. Absent on the challenge
    /// request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    /// Optional binding to the transport session, signed with the rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_binding: Option<String>,
}

/// The host → phone authentication response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResponse {
    /// Whether the device is now authenticated.
    pub authenticated: bool,
    /// The desktop's device id.
    pub desktop_id: String,
    /// A fresh challenge, present when a proof is still required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_nonce: Option<String>,
}

/// The exact bytes a device signs to prove possession of its key.
///
/// Length-prefixed and domain-separated so no two field combinations can collide,
/// mirroring the pairing transcript.
pub fn auth_message(
    protocol_version: u32,
    device_id: &str,
    desktop_id: &str,
    host_nonce: &str,
    transport_binding: Option<&str>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(192);
    out.extend_from_slice(AUTH_CONTEXT);
    out.extend_from_slice(&protocol_version.to_be_bytes());
    push_field(&mut out, device_id.as_bytes());
    push_field(&mut out, desktop_id.as_bytes());
    push_field(&mut out, host_nonce.as_bytes());
    match transport_binding {
        Some(binding) => {
            out.push(1);
            push_field(&mut out, binding.as_bytes());
        }
        None => out.push(0),
    }
    out
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u32).to_be_bytes());
    out.extend_from_slice(field);
}

/// The host-side, per-device challenge authority.
///
/// One outstanding challenge per device id; issuing a new one replaces the old,
/// and consuming removes it so a captured proof cannot be replayed. The store is
/// reachable before authentication, so it is bounded and prunes expired entries
/// on every issue to keep a spray of made-up device ids from growing memory.
#[derive(Default)]
pub struct ChallengeStore {
    inner: Mutex<HashMap<String, (String, i64)>>,
}

/// The most unexpired challenges held at once.
pub const MAX_PENDING_CHALLENGES: usize = 512;

impl ChallengeStore {
    pub fn new() -> Self {
        ChallengeStore::default()
    }

    /// Issue (or replace) a challenge for `device_id`, returning its base64url
    /// nonce.
    pub fn issue(&self, device_id: &str, now: i64) -> String {
        let mut bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let nonce = encode_b64(&bytes);
        let mut inner = self.lock();
        let expires = now + AUTH_TTL_SECS;
        // Drop anything already stale.
        inner.retain(|_, (_, deadline)| *deadline >= now);
        // Bound the map: evict the soonest-to-expire entry when full so a flood
        // of distinct ids cannot exhaust memory before authentication.
        if inner.len() >= MAX_PENDING_CHALLENGES {
            if let Some(oldest) = inner
                .iter()
                .min_by_key(|(_, (_, deadline))| *deadline)
                .map(|(id, _)| id.clone())
            {
                inner.remove(&oldest);
            }
        }
        inner.insert(device_id.to_string(), (nonce.clone(), expires));
        nonce
    }

    /// Consume the outstanding challenge if it matches and has not expired.
    pub fn consume(&self, device_id: &str, nonce: &str, now: i64) -> bool {
        let mut inner = self.lock();
        match inner.get(device_id).cloned() {
            Some((expected, expires)) if expires >= now && expected == nonce => {
                inner.remove(device_id);
                true
            }
            Some(_) => {
                inner.remove(device_id);
                false
            }
            None => false,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (String, i64)>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Produce the proof a phone sends back for `request`, signing the canonical
/// auth message with its own identity.
pub fn prove(identity: &DeviceIdentity, request: &AuthRequest) -> Result<String> {
    let nonce = request
        .host_nonce
        .as_deref()
        .ok_or_else(|| IdentityError::InvalidField("host_nonce is required to prove".into()))?;
    if request.device_id != identity.device_id().to_string() {
        return Err(IdentityError::InvalidField(
            "the request device_id does not match this identity".into(),
        ));
    }
    let message = auth_message(
        request.protocol_version,
        &request.device_id,
        &request.desktop_id,
        nonce,
        request.transport_binding.as_deref(),
    );
    Ok(identity.sign_base64(&message))
}

/// Verify a phone's proof against the pinned public key for its device id.
///
/// The caller must have already consumed a matching challenge.
pub fn verify(public_key_base64: &str, request: &AuthRequest) -> Result<()> {
    let public = PublicIdentity::from_base64(public_key_base64)?;
    if public.device_id().as_str() != request.device_id {
        return Err(IdentityError::InvalidField(
            "public key does not derive the claimed device id".into(),
        ));
    }
    let nonce = request
        .host_nonce
        .as_deref()
        .ok_or_else(|| IdentityError::InvalidField("host_nonce is required".into()))?;
    let proof = decode_b64(request.proof.as_deref().unwrap_or(""))?;
    let message = auth_message(
        request.protocol_version,
        &request.device_id,
        &request.desktop_id,
        nonce,
        request.transport_binding.as_deref(),
    );
    public.verify(&message, &proof)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(identity: &DeviceIdentity, desktop: &DeviceIdentity) -> AuthRequest {
        AuthRequest {
            device_id: identity.device_id().to_string(),
            desktop_id: desktop.device_id().to_string(),
            protocol_version: crate::PROTOCOL_VERSION,
            host_nonce: None,
            proof: None,
            transport_binding: None,
        }
    }

    #[test]
    fn a_challenge_proof_round_trips() {
        let phone = DeviceIdentity::generate();
        let desktop = DeviceIdentity::generate();
        let store = ChallengeStore::new();

        let nonce = store.issue(&phone.device_id().to_string(), 1_000);
        let mut req = request(&phone, &desktop);
        req.host_nonce = Some(nonce.clone());
        let proof = prove(&phone, &req).unwrap();
        req.proof = Some(proof);

        assert!(store.consume(&phone.device_id().to_string(), &nonce, 1_001));
        verify(&phone.public().to_base64(), &req).unwrap();
    }

    #[test]
    fn a_challenge_is_single_use() {
        let store = ChallengeStore::new();
        let nonce = store.issue("did_x", 100);
        assert!(store.consume("did_x", &nonce, 101));
        assert!(!store.consume("did_x", &nonce, 101), "replay must fail");
    }

    #[test]
    fn an_expired_challenge_is_refused() {
        let store = ChallengeStore::new();
        let nonce = store.issue("did_x", 100);
        assert!(!store.consume("did_x", &nonce, 100 + AUTH_TTL_SECS + 1));
    }

    #[test]
    fn a_proof_from_another_key_is_rejected() {
        let phone = DeviceIdentity::generate();
        let attacker = DeviceIdentity::generate();
        let desktop = DeviceIdentity::generate();
        let store = ChallengeStore::new();
        let nonce = store.issue(&phone.device_id().to_string(), 1);

        let mut req = request(&phone, &desktop);
        req.host_nonce = Some(nonce.clone());
        req.proof = Some(attacker.sign_base64(&auth_message(
            req.protocol_version,
            &req.device_id,
            &req.desktop_id,
            &nonce,
            None,
        )));

        assert!(store.consume(&phone.device_id().to_string(), &nonce, 2));
        assert!(verify(&phone.public().to_base64(), &req).is_err());
    }

    #[test]
    fn a_tampered_desktop_id_is_rejected() {
        let phone = DeviceIdentity::generate();
        let desktop = DeviceIdentity::generate();
        let store = ChallengeStore::new();
        let nonce = store.issue(&phone.device_id().to_string(), 1);
        let mut req = request(&phone, &desktop);
        req.host_nonce = Some(nonce.clone());
        req.proof = Some(prove(&phone, &req).unwrap());
        // Swap the desktop id after signing.
        req.desktop_id = "did_someoneelse".to_string();
        assert!(verify(&phone.public().to_base64(), &req).is_err());
    }

    #[test]
    fn the_challenge_store_is_bounded_and_prunes_expired() {
        let store = ChallengeStore::new();
        // A flood of made-up device ids cannot grow the map without bound.
        for i in 0..(MAX_PENDING_CHALLENGES * 3) {
            store.issue(&format!("did_flood_{i}"), 1_000);
        }
        assert!(store.lock().len() <= MAX_PENDING_CHALLENGES);

        // Expired entries are dropped on the next issue.
        let nonce = store.issue("did_fresh", 1_000 + AUTH_TTL_SECS + 1);
        assert!(store.consume("did_fresh", &nonce, 1_000 + AUTH_TTL_SECS + 2));
    }
}
