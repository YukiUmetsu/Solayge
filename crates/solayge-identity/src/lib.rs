//! `solayge-identity` — per-device cryptographic identity, secure key storage,
//! and the QR pairing protocol for Solayge remote clients.
//!
//! This crate is the single source of truth for *who a device is* and *whether a
//! device is trusted*. It deliberately contains no transport: the mobile client
//! and the desktop host both speak [`solayge_protocol::Envelope`] frames, and
//! this crate turns the pairing actions (`pair.begin`, `pair.prove_identity`,
//! `pair.approve`, `pair.reject`) into an explicit, testable state machine.
//!
//! ## Design rules
//!
//! * **No invented cryptography.** Ed25519 comes from the audited
//!   [`ed25519_dalek`] crate; fingerprints are SHA-256 over a canonical,
//!   domain-separated transcript.
//! * **The QR code is a bootstrap hint, not trust.** It carries only public data
//!   (protocol version, desktop public identity, a one-time pairing id, and
//!   nonsecret reachability hints). Trust is conferred by the phone proving
//!   possession of its private key, the desktop proving possession of its own,
//!   a human-compared verification code, and explicit desktop approval.
//! * **Single-use, short-lived, rate-limited challenges.** A pairing id can be
//!   consumed once and expires quickly; repeated attempts are throttled.
//! * **Revocation is transport-independent.** [`DeviceRegistry`] is the one
//!   authority a host consults on every reconnect and every request.
//!
//! ## Entry points
//!
//! * [`DeviceIdentity`] / [`PublicIdentity`] — a stable Ed25519 application key.
//! * [`SecureStore`] and [`default_store`] — OS-native key storage.
//! * [`PairingServer`] (desktop) / [`PairingClient`] (phone).
//! * [`PairingQrPayload`] — the QR payload.
//! * [`DeviceRegistry`] — authorized devices and revocation.
//!
//! ```
//! use solayge_identity::{DeviceIdentity, PairingClient, PairingConfig, PairingServer};
//!
//! let desktop = DeviceIdentity::generate();
//! let server = PairingServer::new(desktop, PairingConfig::default());
//!
//! // Desktop: Settings → Connected devices → Add phone.
//! let offer = server.start(1_000).unwrap();
//!
//! // Phone: scan the QR, build a begin request.
//! let phone = DeviceIdentity::generate();
//! let mut client = PairingClient::new(phone, "Pixel 9", "android");
//! let begin = client.begin_request(&offer).unwrap();
//!
//! // Desktop validates the request and returns its own identity proof.
//! let response = server.handle_begin(begin, 1_001).unwrap();
//! let verified = client.accept_begin(&response).unwrap();
//! assert_eq!(verified.verification_code.digits().len(), 6);
//!
//! // Phone proves possession of its private key.
//! let proof = client.prove_identity_request(&response).unwrap();
//! server.handle_prove_identity(proof, 1_002).unwrap();
//!
//! // The desktop user approves, defaulting to View + Message.
//! let device = server.approve(&offer.pairing_id, None, 1_003).unwrap();
//! assert!(server.registry().is_authorized(&device.device_id));
//! ```
#![deny(unsafe_code)]

pub mod error;
pub mod fingerprint;
pub mod identity;
pub mod auth;
pub mod pairing;
pub mod qr;
pub mod registry;
pub mod scope;
pub mod store;
pub mod transcript;

pub use error::{IdentityError, Result};
pub use auth::{
    auth_message, prove, verify, AuthRequest, AuthResponse, ChallengeStore, AUTH_CONTEXT, AUTH_TTL_SECS,
};
pub use fingerprint::{fingerprint_bytes, format_fingerprint, verification_code, VerificationCode};
pub use identity::{DeviceId, DeviceIdentity, PublicIdentity};
pub use pairing::{
    PairBeginRequest, PairBeginResponse, PairDecisionRequest, PairDecisionResponse,
    PairDecisionState, PairProveIdentityRequest, PairProveIdentityResponse, PairedDevice,
    PairingClient, PairingConfig, PairingOffer, PairingServer, PairingState, VerifiedPairing,
};
pub use qr::{PairingQrPayload, ReachabilityHints};
pub use registry::DeviceRegistry;
pub use scope::Scope;
pub use store::{
    default_store, load_or_create_identity, store_identity, FileStore, MemoryStore, SecureStore,
    IDENTITY_KEY,
};
pub use transcript::PairingTranscript;

/// The protocol version this crate's pairing frames are built for.
pub use solayge_protocol::PROTOCOL_VERSION;

/// Length of an Ed25519 public key, in bytes.
pub const PUBLIC_KEY_LEN: usize = 32;

/// Length of an Ed25519 signature, in bytes.
pub const SIGNATURE_LEN: usize = 64;

/// Length of a pairing nonce, in bytes.
pub const NONCE_LEN: usize = 32;
