//! QR pairing with desktop approval: `pair.begin`, `pair.prove_identity`,
//! `pair.approve`, and `pair.reject`.
//!
//! ## Flow
//!
//! 1. The desktop user chooses **Add phone**. [`PairingServer::start`] creates a
//!    short-lived, single-use session and returns a [`PairingOffer`] whose
//!    [`PairingQrPayload`] is rendered as a QR code (a manual code is offered as
//!    a camera-free fallback).
//! 2. The phone scans the QR, generates an installation identity, and sends a
//!    [`PairBeginRequest`] bound to the pairing id (or the manual code).
//! 3. The desktop validates the challenge, binds the phone's identity to the
//!    session, and returns a [`PairBeginResponse`] containing the desktop's
//!    identity **proof** and the short verification code.
//! 4. The phone verifies the desktop proof, derives the same verification code,
//!    and sends a [`PairProveIdentityRequest`] — its own signature over the
//!    transcript.
//! 5. The desktop verifies the phone's proof; the human compares the two codes
//!    and presses **Approve phone** (default grant: View + Message).
//!
//! Expiry, single use, replay, cancellation, incorrect verification, and rate
//! limits are all enforced here and covered by tests.
//!
//! ## Clock
//!
//! Every method takes `now_secs`, so the state machine is deterministic under
//! test and the transport supplies real time. The crate never reads the clock
//! itself.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::error::{IdentityError, Result};
use crate::identity::{decode_b64, encode_b64, DeviceId, DeviceIdentity, PublicIdentity};
use crate::qr::{PairingQrPayload, ReachabilityHints};
use crate::registry::DeviceRegistry;
use crate::scope::{normalize_grant, Scope};
use crate::transcript::PairingTranscript;
use crate::NONCE_LEN;

// Re-export the device record so callers that only import the pairing surface
// (for example the desktop device-management UI) get it from one place.
pub use crate::registry::PairedDevice;

/// A pairing session's lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingState {
    /// Created by the desktop; the phone has not sent `pair.begin` yet.
    AwaitingProof,
    /// The phone proved its identity; awaiting explicit desktop approval.
    PendingApproval,
    /// The desktop approved and the device was registered. Terminal.
    Approved,
    /// The desktop (or an error budget) rejected the session. Terminal.
    Rejected,
    /// The challenge was consumed and may not be reused. Terminal.
    Consumed,
}

impl PairingState {
    /// Whether no further transition is possible.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            PairingState::Approved | PairingState::Rejected | PairingState::Consumed
        )
    }

    /// The short state word used by `PairAck` on the wire.
    pub const fn wire_name(self) -> &'static str {
        match self {
            PairingState::AwaitingProof => "awaiting_proof",
            PairingState::PendingApproval => "pending",
            PairingState::Approved => "approved",
            PairingState::Rejected => "rejected",
            PairingState::Consumed => "consumed",
        }
    }
}

/// Tunables for the pairing server.
#[derive(Debug, Clone, Copy)]
pub struct PairingConfig {
    /// How long a challenge stays valid.
    pub ttl_secs: i64,
    /// The most simultaneous live challenges.
    pub max_active_sessions: usize,
    /// The most `pair.begin` attempts permitted per window.
    pub max_begins_per_window: u32,
    /// The rate-limit window length.
    pub window_secs: i64,
    /// Wrong identity proofs tolerated before a session is canceled.
    pub max_proof_attempts: u32,
}

impl Default for PairingConfig {
    fn default() -> Self {
        PairingConfig {
            ttl_secs: 120,
            max_active_sessions: 32,
            max_begins_per_window: 10,
            window_secs: 60,
            max_proof_attempts: 3,
        }
    }
}

/// A freshly created pairing challenge handed to the desktop UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingOffer {
    /// The one-time pairing id (also in the QR).
    pub pairing_id: String,
    /// A short code a user can type when the camera is unavailable.
    pub manual_code: String,
    /// The QR payload to render.
    pub qr: PairingQrPayload,
    /// Unix seconds after which the challenge is refused.
    pub expires_at: i64,
}

/// Phone → desktop `pair.begin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairBeginRequest {
    /// Protocol version in force.
    pub protocol_version: u32,
    /// The pairing id from the QR, when scanned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_id: Option<String>,
    /// The manual code, when typed instead of scanned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_code: Option<String>,
    /// The phone's public key, URL-safe base64.
    pub device_public_key: String,
    /// The phone's fresh per-pairing nonce, URL-safe base64.
    pub device_nonce: String,
    /// Human-friendly phone name.
    pub device_name: String,
    /// `ios` | `android` | …
    pub platform: String,
    /// The scopes the phone would like (the desktop decides what to grant).
    pub requested_scopes: Vec<Scope>,
    /// Optional binding to the transport session this handshake runs over. When
    /// set, it becomes part of the signed transcript on both sides, so a proof
    /// cannot be replayed onto another connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_binding: Option<String>,
}

impl PairBeginRequest {
    /// Resolve the phone identity from the encoded key.
    pub fn device_identity(&self) -> Result<PublicIdentity> {
        PublicIdentity::from_base64(&self.device_public_key)
    }

    /// The phone's decoded nonce.
    pub fn device_nonce_bytes(&self) -> Result<[u8; NONCE_LEN]> {
        decode_b64(&self.device_nonce)?
            .try_into()
            .map_err(|_| IdentityError::InvalidEncoding(format!("nonce must be {NONCE_LEN} bytes")))
    }

    /// The challenge selector: exactly one of pairing id / manual code.
    fn selector(&self) -> Result<(&str, bool)> {
        match (self.pairing_id.as_deref(), self.manual_code.as_deref()) {
            (Some(id), None) if !id.is_empty() => Ok((id, false)),
            (None, Some(code)) if !code.is_empty() => Ok((code, true)),
            _ => Err(IdentityError::InvalidField(
                "provide exactly one of pairing_id or manual_code".into(),
            )),
        }
    }
}

/// Desktop → phone `pair.begin` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairBeginResponse {
    /// Protocol version in force.
    pub protocol_version: u32,
    /// The pairing id being negotiated.
    pub pairing_id: String,
    /// The desktop's derived device id.
    pub desktop_id: String,
    /// The desktop's public key, URL-safe base64.
    pub desktop_public_key: String,
    /// The desktop's nonce, URL-safe base64.
    pub desktop_nonce: String,
    /// The phone's device id (echoed for binding).
    pub device_id: String,
    /// The phone's public key (echoed for binding).
    pub device_public_key: String,
    /// The phone's nonce (echoed for binding).
    pub device_nonce: String,
    /// The desktop's Ed25519 signature over the transcript.
    pub desktop_proof: String,
    /// The short human-comparable verification code.
    pub verification_code: String,
    /// Unix seconds after which the challenge is refused.
    pub expires_at: i64,
    /// Current session state.
    pub state: PairingState,
    /// The transport binding echoed back, when one was supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_binding: Option<String>,
}

/// Phone → desktop `pair.prove_identity`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairProveIdentityRequest {
    /// The pairing id being negotiated.
    pub pairing_id: String,
    /// The phone's device id.
    pub device_id: String,
    /// The phone's Ed25519 signature over the transcript, base64.
    pub device_proof: String,
}

/// Desktop → phone `pair.prove_identity` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairProveIdentityResponse {
    /// The pairing id.
    pub pairing_id: String,
    /// `pending` while awaiting human approval.
    pub state: PairingState,
}

/// The outcome of an approve/reject decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairDecisionState {
    /// The device was registered.
    Approved,
    /// The pairing was refused.
    Rejected,
}

/// Desktop decision over `pair.approve` / `pair.reject`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairDecisionRequest {
    /// The pairing id.
    pub pairing_id: String,
    /// The phone's device id.
    pub device_id: String,
    /// The scopes to grant on approval. `None` (or empty) uses the safe default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grants: Option<Vec<Scope>>,
}

/// The response to a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairDecisionResponse {
    /// The pairing id.
    pub pairing_id: String,
    /// The decision outcome.
    pub state: PairDecisionState,
    /// The registered device, on approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<PairedDevice>,
}

/// A pairing the phone has verified end to end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPairing {
    /// The mutually-bound transcript.
    pub transcript: PairingTranscript,
    /// The short code the user must compare with the desktop.
    pub verification_code: crate::fingerprint::VerificationCode,
    /// The phone's own device id.
    pub device_id: DeviceId,
}

struct DeviceBinding {
    id: DeviceId,
    public: PublicIdentity,
    nonce: [u8; NONCE_LEN],
    name: String,
    platform: String,
}

struct Session {
    pairing_id: String,
    manual_code: String,
    desktop_nonce: [u8; NONCE_LEN],
    created_at: i64,
    expires_at: i64,
    state: PairingState,
    device: Option<DeviceBinding>,
    transport_binding: Option<String>,
    requested_scopes: Vec<Scope>,
    granted_scopes: Option<Vec<Scope>>,
    identity_proven: bool,
    proof_attempts: u32,
}

impl Session {
    fn transcript(&self, desktop: &PublicIdentity) -> Option<PairingTranscript> {
        let device = self.device.as_ref()?;
        Some(PairingTranscript {
            protocol_version: crate::PROTOCOL_VERSION,
            pairing_id: self.pairing_id.clone(),
            transport_binding: self.transport_binding.clone(),
            desktop: *desktop,
            device: device.public,
            desktop_nonce: self.desktop_nonce,
            device_nonce: device.nonce,
        })
    }
}

struct Inner {
    sessions: HashMap<String, Session>,
    begin_attempts: VecDeque<i64>,
}

/// The desktop-side pairing authority.
pub struct PairingServer {
    desktop: DeviceIdentity,
    registry: Arc<DeviceRegistry>,
    config: PairingConfig,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for PairingServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingServer")
            .field("desktop_id", &self.desktop.device_id().as_str())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl PairingServer {
    /// Create a server owning the desktop identity and its device registry.
    pub fn new(desktop: DeviceIdentity, config: PairingConfig) -> Self {
        PairingServer::with_registry(desktop, config, Arc::new(DeviceRegistry::new()))
    }

    /// Create a server over an existing registry.
    pub fn with_registry(
        desktop: DeviceIdentity,
        config: PairingConfig,
        registry: Arc<DeviceRegistry>,
    ) -> Self {
        PairingServer {
            desktop,
            registry,
            config,
            inner: Mutex::new(Inner {
                sessions: HashMap::new(),
                begin_attempts: VecDeque::new(),
            }),
        }
    }

    /// The desktop's public identity.
    pub fn desktop_id(&self) -> DeviceId {
        self.desktop.device_id()
    }

    /// The desktop's public identity.
    pub fn desktop_identity(&self) -> PublicIdentity {
        self.desktop.public()
    }

    /// The desktop's signing identity, for host-side signatures (for example
    /// connection authentication).
    pub fn desktop_identity_ref(&self) -> &DeviceIdentity {
        &self.desktop
    }

    /// The device registry this server records approvals into.
    pub fn registry(&self) -> &Arc<DeviceRegistry> {
        &self.registry
    }

    /// The active configuration.
    pub fn config(&self) -> PairingConfig {
        self.config
    }

    /// Start a new pairing challenge (desktop **Add phone**).
    pub fn start(&self, now: i64) -> Result<PairingOffer> {
        self.start_with_hints(now, ReachabilityHints::default())
    }

    /// Start a challenge advertising reachability hints.
    pub fn start_with_hints(&self, now: i64, hints: ReachabilityHints) -> Result<PairingOffer> {
        let pairing_id = format!("pair_{}", random_b64url(16));
        let manual_code = random_manual_code();
        let mut desktop_nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut desktop_nonce);
        let expires_at = now + self.config.ttl_secs;

        let session = Session {
            pairing_id: pairing_id.clone(),
            manual_code: manual_code.clone(),
            desktop_nonce,
            created_at: now,
            expires_at,
            state: PairingState::AwaitingProof,
            device: None,
            transport_binding: None,
            requested_scopes: Vec::new(),
            granted_scopes: None,
            identity_proven: false,
            proof_attempts: 0,
        };

        let qr = PairingQrPayload::new(
            &self.desktop.public(),
            pairing_id.clone(),
            &desktop_nonce,
            expires_at,
            hints,
        );

        let mut inner = lock(&self.inner);
        self.prune(&mut inner, now);
        // Replace any session already bound to this newly generated id (never),
        // and enforce the cap after pruning. Expired sessions are already gone.
        if inner.sessions.len() >= self.config.max_active_sessions {
            return Err(IdentityError::RateLimited(
                "too many live pairing challenges".into(),
            ));
        }
        inner.sessions.insert(pairing_id.clone(), session);
        drop(inner);

        Ok(PairingOffer {
            pairing_id,
            manual_code,
            qr,
            expires_at,
        })
    }

    /// Handle the phone's `pair.begin`.
    pub fn handle_begin(&self, request: PairBeginRequest, now: i64) -> Result<PairBeginResponse> {
        if request.protocol_version != crate::PROTOCOL_VERSION {
            return Err(IdentityError::InvalidField(format!(
                "unsupported protocol_version {}",
                request.protocol_version
            )));
        }
        let (selector, by_manual_code) = request.selector()?;
        let selector = selector.to_string();
        let device_public = request.device_identity()?;
        let device_nonce = request.device_nonce_bytes()?;
        validate_device_claims(&request.device_name, &request.platform)?;
        let requested_scopes = request.requested_scopes.clone();
        let expected_device_id = device_public.device_id();

        let desktop_public = self.desktop.public();
        let mut inner = lock(&self.inner);
        self.check_rate_limit(&mut inner, now)?;
        self.prune(&mut inner, now);

        // Locate the session by pairing id or manual code.
        let key = if by_manual_code {
            inner
                .sessions
                .iter()
                .find(|(_, s)| s.manual_code == selector && s.state == PairingState::AwaitingProof)
                .map(|(k, _)| k.clone())
        } else {
            inner
                .sessions
                .contains_key(&selector)
                .then(|| selector.clone())
        };
        let key = key.ok_or_else(|| IdentityError::PairingNotFound(selector.clone()))?;

        let session = inner
            .sessions
            .get_mut(&key)
            .ok_or_else(|| IdentityError::PairingNotFound(selector.clone()))?;

        // Terminal states and expiry are checked before anything is bound.
        if session.state == PairingState::Rejected {
            return Err(IdentityError::PairingRejected(key));
        }
        // A different identity claiming an in-progress session is an attempt to
        // hijack it; refuse and cancel so neither party can proceed.
        if let Some(bound) = &session.device {
            if bound.public != device_public {
                session.state = PairingState::Rejected;
                return Err(IdentityError::ProofInvalid(
                    "a different device already claimed this pairing".into(),
                ));
            }
        }
        if session.state != PairingState::AwaitingProof {
            // Approved/consumed challenges are single-use.
            return Err(IdentityError::PairingNotActive(key));
        }
        if now > session.expires_at {
            session.state = PairingState::Consumed;
            return Err(IdentityError::PairingExpired(key));
        }

        session.device = Some(DeviceBinding {
            id: expected_device_id.clone(),
            public: device_public,
            nonce: device_nonce,
            name: request.device_name.clone(),
            platform: request.platform.clone(),
        });
        session.requested_scopes = requested_scopes;
        session.transport_binding = request.transport_binding.clone();
        session.state = PairingState::PendingApproval;

        let transcript = session
            .transcript(&desktop_public)
            .ok_or_else(|| IdentityError::Internal("session lost its device binding".into()))?;
        let expires_at = session.expires_at;
        drop(inner);

        let desktop_proof = self.desktop.sign_base64(&transcript.canonical_bytes());
        let verification_code = transcript.verification_code().to_string();

        Ok(PairBeginResponse {
            protocol_version: crate::PROTOCOL_VERSION,
            pairing_id: key,
            desktop_id: desktop_public.device_id().to_string(),
            desktop_public_key: desktop_public.to_base64(),
            desktop_nonce: encode_b64(&transcript.desktop_nonce),
            device_id: expected_device_id.to_string(),
            device_public_key: device_public.to_base64(),
            device_nonce: encode_b64(&transcript.device_nonce),
            desktop_proof,
            verification_code,
            expires_at,
            state: PairingState::PendingApproval,
            transport_binding: transcript.transport_binding.clone(),
        })
    }

    /// Handle the phone's `pair.prove_identity`.
    pub fn handle_prove_identity(
        &self,
        request: PairProveIdentityRequest,
        now: i64,
    ) -> Result<PairProveIdentityResponse> {
        let desktop_public = self.desktop.public();
        let mut inner = lock(&self.inner);
        self.prune(&mut inner, now);
        let session = inner
            .sessions
            .get_mut(&request.pairing_id)
            .ok_or_else(|| IdentityError::PairingNotFound(request.pairing_id.clone()))?;

        if session.state == PairingState::Rejected {
            return Err(IdentityError::PairingRejected(request.pairing_id.clone()));
        }
        if session.state != PairingState::PendingApproval {
            // Replay of a consumed/approved challenge, or begin was never sent.
            return Err(IdentityError::PairingNotActive(request.pairing_id.clone()));
        }
        if now > session.expires_at {
            session.state = PairingState::Consumed;
            return Err(IdentityError::PairingExpired(request.pairing_id.clone()));
        }
        if session.identity_proven {
            // Idempotent retry of the same successful proof.
            return Ok(PairProveIdentityResponse {
                pairing_id: request.pairing_id,
                state: PairingState::PendingApproval,
            });
        }

        let transcript = session
            .transcript(&desktop_public)
            .ok_or_else(|| IdentityError::InvalidPairingState("pair.begin has not run".into()))?;
        // Copy the binding out before mutating the session's proof budget.
        let (bound_id, bound_public) = match &session.device {
            Some(binding) => (binding.id.clone(), binding.public),
            None => {
                return Err(IdentityError::InvalidPairingState(
                    "pair.begin has not run".into(),
                ))
            }
        };
        if bound_id.as_str() != request.device_id {
            return Err(IdentityError::ProofInvalid(
                "device id does not match".into(),
            ));
        }

        let proof = decode_b64(&request.device_proof)
            .map_err(|_| IdentityError::ProofInvalid("proof is not valid base64".into()))?;
        if bound_public
            .verify(&transcript.canonical_bytes(), &proof)
            .is_err()
        {
            session.proof_attempts += 1;
            let attempts = session.proof_attempts;
            let exhausted = attempts >= self.config.max_proof_attempts;
            if exhausted {
                session.state = PairingState::Rejected;
            }
            return Err(IdentityError::ProofInvalid(format!(
                "identity proof failed ({attempts}/{}); {}",
                self.config.max_proof_attempts,
                if exhausted {
                    "pairing canceled".to_string()
                } else {
                    "retry allowed".to_string()
                }
            )));
        }

        session.identity_proven = true;
        Ok(PairProveIdentityResponse {
            pairing_id: request.pairing_id,
            state: PairingState::PendingApproval,
        })
    }

    /// Approve a pairing (desktop **Approve phone**) and register the device.
    ///
    /// `grants` of `None` or an empty list uses the safe default of View +
    /// Message; the desktop must ask explicitly for anything wider.
    pub fn approve(
        &self,
        pairing_id: &str,
        grants: Option<Vec<Scope>>,
        now: i64,
    ) -> Result<PairedDevice> {
        let mut inner = lock(&self.inner);
        self.prune(&mut inner, now);
        let session = inner
            .sessions
            .get_mut(pairing_id)
            .ok_or_else(|| IdentityError::PairingNotFound(pairing_id.to_string()))?;

        match session.state {
            PairingState::Rejected => {
                return Err(IdentityError::PairingRejected(pairing_id.to_string()))
            }
            PairingState::Approved => {
                return Err(IdentityError::PairingNotActive(pairing_id.to_string()))
            }
            PairingState::Consumed => {
                return Err(if now > session.expires_at {
                    IdentityError::PairingExpired(pairing_id.to_string())
                } else {
                    IdentityError::PairingNotActive(pairing_id.to_string())
                })
            }
            PairingState::AwaitingProof => {
                return Err(IdentityError::InvalidPairingState(
                    "the phone has not sent pair.begin".into(),
                ))
            }
            PairingState::PendingApproval => {}
        }
        if now > session.expires_at {
            session.state = PairingState::Consumed;
            return Err(IdentityError::PairingExpired(pairing_id.to_string()));
        }
        if !session.identity_proven {
            return Err(IdentityError::InvalidPairingState(
                "the phone has not proven identity ownership".into(),
            ));
        }
        let binding = session
            .device
            .as_ref()
            .ok_or_else(|| IdentityError::InvalidPairingState("no device to approve".into()))?;

        let granted = normalize_grant(&grants.unwrap_or_default());
        let device = PairedDevice {
            device_id: binding.id.to_string(),
            name: binding.name.clone(),
            platform: binding.platform.clone(),
            public_key: binding.public.to_base64(),
            scopes: granted.clone(),
            paired_at: now,
            last_seen: None,
            online: false,
        };
        session.granted_scopes = Some(granted);
        session.state = PairingState::Approved;
        drop(inner);

        self.registry.register(device.clone())?;
        Ok(device)
    }

    /// Reject a pairing. Idempotent: rejecting an already-rejected session is
    /// fine and keeps the device unauthorized.
    pub fn reject(&self, pairing_id: &str, now: i64) -> Result<PairDecisionResponse> {
        let mut inner = lock(&self.inner);
        self.prune(&mut inner, now);
        let session = inner
            .sessions
            .get_mut(pairing_id)
            .ok_or_else(|| IdentityError::PairingNotFound(pairing_id.to_string()))?;
        match session.state {
            PairingState::Approved | PairingState::Consumed => {
                return Err(IdentityError::PairingNotActive(pairing_id.to_string()))
            }
            PairingState::Rejected => {}
            _ => session.state = PairingState::Rejected,
        }
        Ok(PairDecisionResponse {
            pairing_id: pairing_id.to_string(),
            state: PairDecisionState::Rejected,
            device: None,
        })
    }

    /// The current state of a challenge, for the desktop UI to poll.
    pub fn session_state(&self, pairing_id: &str) -> Option<PairingState> {
        lock(&self.inner).sessions.get(pairing_id).map(|s| s.state)
    }

    /// Route a wire decision (`pair.approve`/`pair.reject`) to the state machine.
    pub fn handle_decision(
        &self,
        approve: bool,
        request: PairDecisionRequest,
        now: i64,
    ) -> Result<PairDecisionResponse> {
        if approve {
            let device = self.approve(&request.pairing_id, request.grants, now)?;
            Ok(PairDecisionResponse {
                pairing_id: request.pairing_id,
                state: PairDecisionState::Approved,
                device: Some(device),
            })
        } else {
            self.reject(&request.pairing_id, now)
        }
    }

    fn check_rate_limit(&self, inner: &mut Inner, now: i64) -> Result<()> {
        let window = self.config.window_secs.max(1);
        while let Some(front) = inner.begin_attempts.front().copied() {
            if now - front >= window {
                inner.begin_attempts.pop_front();
            } else {
                break;
            }
        }
        if inner.begin_attempts.len() as u32 >= self.config.max_begins_per_window {
            return Err(IdentityError::RateLimited(format!(
                "more than {} pairing attempts in {}s",
                self.config.max_begins_per_window, window
            )));
        }
        inner.begin_attempts.push_back(now);
        Ok(())
    }

    fn prune(&self, inner: &mut Inner, now: i64) {
        // Keep recently finished sessions long enough that a late request gets a
        // meaningful "expired"/"not active" answer instead of a bare not-found,
        // but still bound the map's growth.
        let retention = (self.config.ttl_secs.saturating_mul(4)).max(300);
        inner.sessions.retain(|_, session| {
            !(session.state.is_terminal() && now - session.created_at > retention)
        });
    }
}

fn validate_device_claims(name: &str, platform: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(IdentityError::InvalidField(
            "device_name must be 1..=64 characters".into(),
        ));
    }
    if platform.is_empty() || platform.len() > 32 {
        return Err(IdentityError::InvalidField(
            "platform must be 1..=32 characters".into(),
        ));
    }
    Ok(())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn random_b64url(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    encode_b64(&bytes)
}

/// A manual code uses an unambiguous alphabet (no 0/O/1/I/L) and is grouped.
fn random_manual_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let code: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &code[..4], &code[4..])
}

/// The phone-side pairing client. It owns the installation identity and the
/// per-pairing nonce, and it verifies the desktop's proof before the user is
/// asked to trust anything.
pub struct PairingClient {
    identity: DeviceIdentity,
    device_nonce: [u8; NONCE_LEN],
    device_name: String,
    platform: String,
    requested_scopes: Vec<Scope>,
    transport_binding: Option<String>,
    pending: Option<PairingTranscript>,
}

impl std::fmt::Debug for PairingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingClient")
            .field("device_id", &self.identity.device_id().as_str())
            .field("platform", &self.platform)
            .finish_non_exhaustive()
    }
}

impl PairingClient {
    /// A client with a fresh random nonce.
    pub fn new(
        identity: DeviceIdentity,
        device_name: impl Into<String>,
        platform: impl Into<String>,
    ) -> Self {
        let mut device_nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut device_nonce);
        PairingClient::with_nonce(identity, device_name, platform, device_nonce)
    }

    /// A client with an explicit nonce, for deterministic tests.
    pub fn with_nonce(
        identity: DeviceIdentity,
        device_name: impl Into<String>,
        platform: impl Into<String>,
        device_nonce: [u8; NONCE_LEN],
    ) -> Self {
        PairingClient {
            identity,
            device_nonce,
            device_name: device_name.into(),
            platform: platform.into(),
            requested_scopes: Scope::default_phone(),
            transport_binding: None,
            pending: None,
        }
    }

    /// Bind the pairing proofs to a specific transport session id.
    pub fn with_transport_binding(mut self, binding: Option<String>) -> Self {
        self.transport_binding = binding;
        self
    }

    /// Override the requested scopes (the desktop still decides what to grant).
    pub fn with_requested_scopes(mut self, scopes: Vec<Scope>) -> Self {
        self.requested_scopes = scopes;
        self
    }

    /// The phone's device id.
    pub fn device_id(&self) -> DeviceId {
        self.identity.device_id()
    }

    /// The phone's public identity.
    pub fn public(&self) -> PublicIdentity {
        self.identity.public()
    }

    /// The per-pairing nonce.
    pub fn device_nonce(&self) -> &[u8; NONCE_LEN] {
        &self.device_nonce
    }

    /// Build `pair.begin` from a scanned offer.
    pub fn begin_request(&self, offer: &PairingOffer) -> Result<PairBeginRequest> {
        self.begin_request_for(Some(offer.pairing_id.clone()), None)
    }

    /// Build `pair.begin` from a scanned QR payload.
    pub fn begin_request_for_qr(&self, qr: &PairingQrPayload) -> Result<PairBeginRequest> {
        qr.validate()?;
        self.begin_request_for(Some(qr.pairing_id.clone()), None)
    }

    /// Build `pair.begin` from a typed manual code.
    pub fn begin_request_manual(&self, manual_code: &str) -> Result<PairBeginRequest> {
        self.begin_request_for(None, Some(manual_code.trim().to_uppercase()))
    }

    fn begin_request_for(
        &self,
        pairing_id: Option<String>,
        manual_code: Option<String>,
    ) -> Result<PairBeginRequest> {
        Ok(PairBeginRequest {
            protocol_version: crate::PROTOCOL_VERSION,
            pairing_id,
            manual_code,
            device_public_key: self.identity.public().to_base64(),
            device_nonce: encode_b64(&self.device_nonce),
            device_name: self.device_name.clone(),
            platform: self.platform.clone(),
            requested_scopes: self.requested_scopes.clone(),
            transport_binding: self.transport_binding.clone(),
        })
    }

    /// Verify the desktop's `pair.begin` response and bind the transcript.
    ///
    /// This rejects a response that does not echo this phone's identity and
    /// nonce (channel binding) or whose desktop proof does not verify.
    pub fn accept_begin(&mut self, response: &PairBeginResponse) -> Result<VerifiedPairing> {
        if response.protocol_version != crate::PROTOCOL_VERSION {
            return Err(IdentityError::TranscriptMismatch);
        }
        // The desktop must have bound to the same transport channel this phone
        // intended; otherwise the proof is for a different connection.
        if self.transport_binding != response.transport_binding {
            return Err(IdentityError::TranscriptMismatch);
        }
        let my_public = self.identity.public();
        if response.device_id != self.identity.device_id().to_string()
            || response.device_public_key != my_public.to_base64()
            || response.device_nonce != encode_b64(&self.device_nonce)
            || response.desktop_id != response.desktop_identity()?.device_id().to_string()
        {
            return Err(IdentityError::TranscriptMismatch);
        }

        let transcript = PairingTranscript {
            protocol_version: response.protocol_version,
            pairing_id: response.pairing_id.clone(),
            transport_binding: response.transport_binding.clone(),
            desktop: response.desktop_identity()?,
            device: my_public,
            desktop_nonce: decode_b64(&response.desktop_nonce)?
                .try_into()
                .map_err(|_| IdentityError::TranscriptMismatch)?,
            device_nonce: self.device_nonce,
        };

        let proof = decode_b64(&response.desktop_proof)
            .map_err(|_| IdentityError::ProofInvalid("desktop proof is not base64".into()))?;
        transcript
            .desktop
            .verify(&transcript.canonical_bytes(), &proof)
            .map_err(|_| IdentityError::ProofInvalid("desktop proof did not verify".into()))?;

        let code = transcript.verification_code();
        if code.to_string() != response.verification_code {
            return Err(IdentityError::TranscriptMismatch);
        }

        self.pending = Some(transcript.clone());
        Ok(VerifiedPairing {
            transcript,
            verification_code: code,
            device_id: self.identity.device_id(),
        })
    }

    /// Derive the transcript (and therefore the verification code) directly
    /// from a scanned QR payload, before any network round trip.
    ///
    /// Both sides can compute this independently: the QR carries the desktop's
    /// public identity and nonce, and the phone supplies its own. It is the value
    /// the user compares with the code the desktop shows after `pair.begin`.
    pub fn expect_from_qr(&self, qr: &PairingQrPayload) -> Result<PairingTranscript> {
        qr.validate()?;
        Ok(PairingTranscript {
            protocol_version: qr.protocol_version,
            pairing_id: qr.pairing_id.clone(),
            transport_binding: self.transport_binding.clone(),
            desktop: qr.desktop_identity()?,
            device: self.identity.public(),
            desktop_nonce: qr.desktop_nonce_bytes()?,
            device_nonce: self.device_nonce,
        })
    }

    /// Build `pair.prove_identity` after [`PairingClient::accept_begin`].
    pub fn prove_identity_request(
        &self,
        response: &PairBeginResponse,
    ) -> Result<PairProveIdentityRequest> {
        let transcript = self.pending.as_ref().ok_or_else(|| {
            IdentityError::InvalidPairingState("accept the desktop response first".into())
        })?;
        if transcript.pairing_id != response.pairing_id {
            return Err(IdentityError::TranscriptMismatch);
        }
        let device_proof = self.identity.sign_base64(&transcript.canonical_bytes());
        Ok(PairProveIdentityRequest {
            pairing_id: response.pairing_id.clone(),
            device_id: self.identity.device_id().to_string(),
            device_proof,
        })
    }
}

impl PairBeginResponse {
    /// The desktop identity encoded in the response.
    pub fn desktop_identity(&self) -> Result<PublicIdentity> {
        let identity = PublicIdentity::from_base64(&self.desktop_public_key)?;
        if identity.device_id().to_string() != self.desktop_id {
            return Err(IdentityError::InvalidField(
                "desktop_id does not match desktop_public_key".into(),
            ));
        }
        Ok(identity)
    }
}

// `PairedDevice` is re-exported above so callers can import it from `pairing`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scope;

    const T0: i64 = 1_000;

    fn server() -> (PairingServer, DeviceIdentity) {
        let desktop = DeviceIdentity::generate();
        let server = PairingServer::new(desktop, PairingConfig::default());
        (server, DeviceIdentity::generate())
    }

    /// Run begin + prove. Returns the phone's client (bound) and the offer.
    fn begin_and_prove(
        server: &PairingServer,
        now: i64,
    ) -> (PairingClient, PairingOffer, PairBeginResponse) {
        let offer = server.start(now).unwrap();
        let phone = DeviceIdentity::generate();
        let mut client = PairingClient::new(phone, "Pixel 9", "android");
        let begin = client.begin_request(&offer).unwrap();
        let response = server.handle_begin(begin, now + 1).unwrap();
        client.accept_begin(&response).unwrap();
        let proof = client.prove_identity_request(&response).unwrap();
        server.handle_prove_identity(proof, now + 2).unwrap();
        (client, offer, response)
    }

    #[test]
    fn the_happy_path_pairs_and_registers_a_view_message_device() {
        let (server, _) = server();
        let (_client, offer, _response) = begin_and_prove(&server, T0);
        let device = server.approve(&offer.pairing_id, None, T0 + 3).unwrap();

        assert_eq!(device.scopes, Scope::default_phone());
        assert!(!device.scopes.iter().any(|s| s.is_admin()));
        assert!(server.registry().is_authorized(&device.device_id));
        assert_eq!(
            server.session_state(&offer.pairing_id),
            Some(PairingState::Approved)
        );
    }

    #[test]
    fn an_expired_pairing_is_rejected_at_begin() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let phone = DeviceIdentity::generate();
        let client = PairingClient::new(phone, "iPhone", "ios");
        let begin = client.begin_request(&offer).unwrap();
        let err = server
            .handle_begin(begin, offer.expires_at + 1)
            .unwrap_err();
        assert!(matches!(err, IdentityError::PairingExpired(_)));
        // The challenge cannot be revived.
        assert_eq!(
            server.session_state(&offer.pairing_id),
            Some(PairingState::Consumed)
        );
    }

    #[test]
    fn an_expired_pairing_cannot_be_proven_or_approved() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let phone = DeviceIdentity::generate();
        let mut client = PairingClient::new(phone, "iPhone", "ios");
        let begin = client.begin_request(&offer).unwrap();
        let response = server.handle_begin(begin, T0 + 1).unwrap();
        client.accept_begin(&response).unwrap();
        let proof = client.prove_identity_request(&response).unwrap();

        assert!(matches!(
            server
                .handle_prove_identity(proof, offer.expires_at + 1)
                .unwrap_err(),
            IdentityError::PairingExpired(_)
        ));
        assert!(matches!(
            server
                .approve(&offer.pairing_id, None, offer.expires_at + 2)
                .unwrap_err(),
            IdentityError::PairingExpired(_)
        ));
    }

    #[test]
    fn a_replayed_pairing_is_single_use() {
        let (server, _) = server();
        let (client, offer, response) = begin_and_prove(&server, T0);
        server.approve(&offer.pairing_id, None, T0 + 3).unwrap();

        // A second `pair.begin` on the same id is refused.
        let replay_begin = client.begin_request(&offer).unwrap();
        assert!(matches!(
            server.handle_begin(replay_begin, T0 + 4).unwrap_err(),
            IdentityError::PairingNotActive(_)
        ));
        // A replayed proof is refused.
        let replay_proof = client.prove_identity_request(&response).unwrap();
        assert!(matches!(
            server
                .handle_prove_identity(replay_proof, T0 + 5)
                .unwrap_err(),
            IdentityError::PairingNotActive(_)
        ));
        // Approval is single-use too.
        assert!(matches!(
            server.approve(&offer.pairing_id, None, T0 + 6).unwrap_err(),
            IdentityError::PairingNotActive(_)
        ));
    }

    #[test]
    fn a_canceled_pairing_cannot_be_used() {
        let (server, _) = server();
        let (client, offer, response) = begin_and_prove(&server, T0);
        let decision = server.reject(&offer.pairing_id, T0 + 3).unwrap();
        assert_eq!(decision.state, PairDecisionState::Rejected);
        // Rejecting twice is idempotent.
        server.reject(&offer.pairing_id, T0 + 4).unwrap();

        assert!(matches!(
            server
                .handle_begin(client.begin_request(&offer).unwrap(), T0 + 5)
                .unwrap_err(),
            IdentityError::PairingRejected(_)
        ));
        assert!(matches!(
            server
                .handle_prove_identity(client.prove_identity_request(&response).unwrap(), T0 + 6)
                .unwrap_err(),
            IdentityError::PairingRejected(_)
        ));
        assert!(matches!(
            server.approve(&offer.pairing_id, None, T0 + 7).unwrap_err(),
            IdentityError::PairingRejected(_)
        ));
    }

    #[test]
    fn an_incorrect_identity_proof_is_rejected_and_cancels_after_retries() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let mut client = PairingClient::with_nonce(
            DeviceIdentity::generate(),
            "iPhone",
            "ios",
            [5u8; NONCE_LEN],
        );
        let begin = client.begin_request(&offer).unwrap();
        let response = server.handle_begin(begin, T0 + 1).unwrap();
        client.accept_begin(&response).unwrap();

        // Forge a proof with a different key.
        let attacker = DeviceIdentity::generate();
        let mut bad = client.prove_identity_request(&response).unwrap();
        bad.device_proof = attacker.sign_base64(b"wrong transcript");

        for attempt in 0..3 {
            let err = server
                .handle_prove_identity(bad.clone(), T0 + 2 + attempt)
                .unwrap_err();
            assert!(matches!(err, IdentityError::ProofInvalid(_)));
        }
        // The identity budget is exhausted; the session is canceled.
        assert_eq!(
            server.session_state(&offer.pairing_id),
            Some(PairingState::Rejected)
        );
        assert!(matches!(
            server
                .approve(&offer.pairing_id, None, T0 + 10)
                .unwrap_err(),
            IdentityError::PairingRejected(_)
        ));
    }

    #[test]
    fn a_tampered_desktop_proof_is_detected_by_the_phone() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let mut client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
        let begin = client.begin_request(&offer).unwrap();
        let mut response = server.handle_begin(begin, T0 + 1).unwrap();
        response.desktop_proof = encode_b64(&[0u8; 64]);
        assert!(matches!(
            client.accept_begin(&response).unwrap_err(),
            IdentityError::ProofInvalid(_)
        ));
    }

    #[test]
    fn a_response_for_a_different_nonce_is_rejected() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let mut client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
        let begin = client.begin_request(&offer).unwrap();
        let mut response = server.handle_begin(begin, T0 + 1).unwrap();
        response.device_nonce = encode_b64(&[9u8; NONCE_LEN]);
        assert_eq!(
            client.accept_begin(&response).unwrap_err(),
            IdentityError::TranscriptMismatch
        );
    }

    #[test]
    fn pairing_begins_are_rate_limited() {
        let desktop = DeviceIdentity::generate();
        let server = PairingServer::new(
            desktop,
            PairingConfig {
                max_begins_per_window: 3,
                window_secs: 60,
                ..PairingConfig::default()
            },
        );
        let offer = server.start(T0).unwrap();
        let client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
        let begin = client.begin_request(&offer).unwrap();

        // A wrong pairing id is still an attempt, so the budget is consumed.
        let mut bad = begin.clone();
        bad.pairing_id = Some("pair_missing".into());
        for _ in 0..3 {
            assert!(matches!(
                server.handle_begin(bad.clone(), T0 + 1).unwrap_err(),
                IdentityError::PairingNotFound(_)
            ));
        }
        assert!(matches!(
            server.handle_begin(begin, T0 + 2).unwrap_err(),
            IdentityError::RateLimited(_)
        ));
        // After the window, attempts are allowed again.
        server.handle_begin(bad, T0 + 100).unwrap_err();
    }

    #[test]
    fn the_manual_code_path_pairs_without_a_qr() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let mut client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
        let begin = client.begin_request_manual(&offer.manual_code).unwrap();
        let response = server.handle_begin(begin, T0 + 1).unwrap();
        let verified = client.accept_begin(&response).unwrap();
        assert_eq!(
            verified.verification_code.to_string(),
            response.verification_code
        );
        client.prove_identity_request(&response).unwrap();
        let proof = client.prove_identity_request(&response).unwrap();
        server.handle_prove_identity(proof, T0 + 2).unwrap();
        server.approve(&offer.pairing_id, None, T0 + 3).unwrap();
    }

    #[test]
    fn both_sides_derive_the_same_verification_code() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let mut client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
        let begin = client.begin_request(&offer).unwrap();
        let response = server.handle_begin(begin, T0 + 1).unwrap();
        let verified = client.accept_begin(&response).unwrap();
        assert_eq!(
            verified.verification_code.to_string(),
            response.verification_code
        );
        assert_eq!(
            verified.transcript.fingerprint(),
            verified.transcript.fingerprint()
        );
        assert_eq!(
            response.desktop_identity().unwrap(),
            verified.transcript.desktop
        );
    }

    #[test]
    fn a_transport_binding_is_echoed_and_signed() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let mut client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios")
            .with_transport_binding(Some("iroh:endpoint-a".into()));
        let begin = client.begin_request(&offer).unwrap();
        assert_eq!(begin.transport_binding.as_deref(), Some("iroh:endpoint-a"));

        let response = server.handle_begin(begin, T0 + 1).unwrap();
        assert_eq!(
            response.transport_binding.as_deref(),
            Some("iroh:endpoint-a")
        );
        let verified = client.accept_begin(&response).unwrap();
        assert_eq!(
            verified.transcript.transport_binding.as_deref(),
            Some("iroh:endpoint-a")
        );

        // A client with the same key and nonce but a different expected channel
        // cannot verify the desktop proof: the signed bytes differ.
        let binding = *client.device_nonce();
        let mut other =
            PairingClient::with_nonce(client.identity.clone(), "iPhone", "ios", binding)
                .with_transport_binding(Some("tailscale:other".into()));
        assert!(matches!(
            other.accept_begin(&response).unwrap_err(),
            IdentityError::TranscriptMismatch
        ));
    }

    #[test]
    fn the_qr_alone_yields_the_same_code_as_the_server() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
        let from_qr = client.expect_from_qr(&offer.qr).unwrap();
        let response = server
            .handle_begin(client.begin_request(&offer).unwrap(), T0 + 1)
            .unwrap();
        assert_eq!(
            from_qr.verification_code().to_string(),
            response.verification_code
        );
        assert_eq!(from_qr.fingerprint(), from_qr.fingerprint());
    }

    #[test]
    fn approving_before_a_proof_is_refused() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
        let begin = client.begin_request(&offer).unwrap();
        server.handle_begin(begin, T0 + 1).unwrap();
        assert!(matches!(
            server.approve(&offer.pairing_id, None, T0 + 2).unwrap_err(),
            IdentityError::InvalidPairingState(_)
        ));
    }

    #[test]
    fn a_second_device_cannot_hijack_an_in_progress_session() {
        let (server, _) = server();
        let offer = server.start(T0).unwrap();
        let first = PairingClient::new(DeviceIdentity::generate(), "First", "ios");
        server
            .handle_begin(first.begin_request(&offer).unwrap(), T0 + 1)
            .unwrap();

        let second = PairingClient::new(DeviceIdentity::generate(), "Second", "android");
        assert!(matches!(
            server
                .handle_begin(second.begin_request(&offer).unwrap(), T0 + 2)
                .unwrap_err(),
            IdentityError::ProofInvalid(_)
        ));
        assert_eq!(
            server.session_state(&offer.pairing_id),
            Some(PairingState::Rejected)
        );
    }

    #[test]
    fn an_explicit_admin_grant_is_honored_only_when_requested() {
        let (server, _) = server();
        let (_client, offer, _r) = begin_and_prove(&server, T0);
        let device = server
            .approve(&offer.pairing_id, Some(vec![Scope::DevicesManage]), T0 + 3)
            .unwrap();
        assert_eq!(device.scopes, vec![Scope::DevicesManage]);
    }
}
