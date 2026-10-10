//! A stable per-device Solayge application identity.
//!
//! Every install generates one Ed25519 signing key the first time it runs and
//! keeps it in the platform secure store (see [`crate::store`]). The public half
//! is the device's application identity: it is what the peer pins after a
//! human-approved pairing, and what must not change between sessions.
//!
//! The private key never crosses the Rust/JS boundary and is never serialized to
//! the plaintext application state.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{IdentityError, Result};
use crate::{PUBLIC_KEY_LEN, SIGNATURE_LEN};

/// Decode a URL-safe, unpadded base64 string.
pub(crate) fn decode_b64(input: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(input.as_bytes())
        .map_err(|e| IdentityError::InvalidEncoding(format!("base64url: {e}")))
}

/// Encode bytes as URL-safe, unpadded base64.
pub(crate) fn encode_b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The public identity (device id) of a Solayge install.
///
/// The id is derived deterministically from the public key, so two sides that
/// know a public key can independently agree on the id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(String);

impl DeviceId {
    /// Wrap an already-derived id string. Used when decoding wire frames.
    pub fn new(id: impl Into<String>) -> Self {
        DeviceId(id.into())
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the string is a well-formed, derived Solayge device id.
    pub fn is_valid(&self) -> bool {
        self.0.starts_with("did_")
            && self.0.len() > 4
            && self
                .0
                .trim_start_matches("did_")
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An Ed25519 public identity: the verifying key plus its derived [`DeviceId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicIdentity {
    key: [u8; PUBLIC_KEY_LEN],
}

impl PublicIdentity {
    /// Validate raw public-key bytes and build the identity.
    pub fn from_bytes(bytes: [u8; PUBLIC_KEY_LEN]) -> Result<Self> {
        // Reject points that are not valid Ed25519 encodings up front, so a
        // malformed peer identity fails at the boundary rather than at signing.
        VerifyingKey::from_bytes(&bytes)
            .map_err(|e| IdentityError::InvalidKey(format!("not an Ed25519 public key: {e}")))?;
        Ok(PublicIdentity { key: bytes })
    }

    /// Parse a URL-safe base64 public identity.
    pub fn from_base64(encoded: &str) -> Result<Self> {
        let bytes = decode_b64(encoded)?;
        let array: [u8; PUBLIC_KEY_LEN] = bytes.try_into().map_err(|_| {
            IdentityError::InvalidKey(format!("expected {PUBLIC_KEY_LEN} key bytes"))
        })?;
        PublicIdentity::from_bytes(array)
    }

    /// The raw public-key bytes.
    pub fn as_bytes(&self) -> &[u8; PUBLIC_KEY_LEN] {
        &self.key
    }

    /// URL-safe base64 of the public key (the wire and QR representation).
    pub fn to_base64(&self) -> String {
        encode_b64(&self.key)
    }

    /// The stable device id derived from this key.
    pub fn device_id(&self) -> DeviceId {
        DeviceId(format!("did_{}", encode_b64(&self.key)))
    }

    /// The `ed25519_dalek` verifying key.
    fn verifying_key(&self) -> VerifyingKey {
        VerifyingKey::from_bytes(&self.key).expect("constructed from validated bytes")
    }

    /// Verify a signature over `message`.
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<()> {
        let signature =
            Signature::from_slice(signature).map_err(|_| IdentityError::InvalidSignature)?;
        self.verifying_key()
            .verify_strict(message, &signature)
            .map_err(|_| IdentityError::InvalidSignature)
    }

    /// A short, grouped hex fingerprint of the public key itself. This is the
    /// identity-only counterpart to the pairing fingerprint; pairing uses the
    /// transcript-bound value from [`crate::fingerprint`].
    pub fn short_fingerprint(&self) -> String {
        crate::fingerprint::hex_grouped(self.key.as_slice(), 8)
    }
}

/// A device's Ed25519 signing identity. The private key stays in this struct and
/// is zeroized on drop.
pub struct DeviceIdentity {
    signing: SigningKey,
}

impl std::fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("device_id", &self.public().device_id().as_str())
            .finish_non_exhaustive()
    }
}

impl Clone for DeviceIdentity {
    /// Duplicate the identity for a short-lived holder (for example a pairing
    /// client). The copy carries the same secret and is zeroized on drop; prefer
    /// passing a reference where possible.
    fn clone(&self) -> Self {
        DeviceIdentity::from_seed(&self.secret_bytes())
    }
}

impl DeviceIdentity {
    /// Generate a fresh identity from the OS CSPRNG.
    pub fn generate() -> Self {
        let signing = SigningKey::generate(&mut rand::rngs::OsRng);
        DeviceIdentity { signing }
    }

    /// Rebuild an identity from its 32-byte Ed25519 seed.
    pub fn from_seed(seed: &[u8; PUBLIC_KEY_LEN]) -> Self {
        DeviceIdentity {
            signing: SigningKey::from_bytes(seed),
        }
    }

    /// Rebuild an identity from stored secret bytes, rejecting wrong lengths.
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self> {
        let seed: [u8; PUBLIC_KEY_LEN] = bytes.try_into().map_err(|_| {
            IdentityError::InvalidKey("identity seed must be exactly 32 bytes".into())
        })?;
        Ok(DeviceIdentity::from_seed(&seed))
    }

    /// The secret seed to persist in a [`crate::SecureStore`]. Wrapped in
    /// [`Zeroizing`] so it is cleared from memory when the caller is done.
    pub fn secret_bytes(&self) -> Zeroizing<[u8; PUBLIC_KEY_LEN]> {
        Zeroizing::new(self.signing.to_bytes())
    }

    /// The public identity.
    pub fn public(&self) -> PublicIdentity {
        PublicIdentity {
            key: self.signing.verifying_key().to_bytes(),
        }
    }

    /// The stable device id.
    pub fn device_id(&self) -> DeviceId {
        self.public().device_id()
    }

    /// Sign a message, returning the raw 64-byte signature.
    pub fn sign(&self, message: &[u8]) -> [u8; SIGNATURE_LEN] {
        self.signing.sign(message).to_bytes()
    }

    /// Sign and encode as URL-safe base64.
    pub fn sign_base64(&self, message: &[u8]) -> String {
        encode_b64(&self.sign(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_is_stable_across_reloads() {
        let identity = DeviceIdentity::generate();
        let seed = identity.secret_bytes();
        let reloaded = DeviceIdentity::from_seed(&seed).public();
        assert_eq!(identity.public(), reloaded);
        assert_eq!(identity.device_id(), reloaded.device_id());
    }

    #[test]
    fn signatures_verify_and_tampering_is_rejected() {
        let identity = DeviceIdentity::generate();
        let message = b"pairing transcript";
        let signature = identity.sign(message);
        identity.public().verify(message, &signature).unwrap();

        assert_eq!(
            identity
                .public()
                .verify(b"different transcript", &signature)
                .unwrap_err(),
            IdentityError::InvalidSignature
        );

        let mut flipped = signature;
        flipped[0] ^= 0x01;
        assert_eq!(
            identity.public().verify(message, &flipped).unwrap_err(),
            IdentityError::InvalidSignature
        );
    }

    #[test]
    fn public_identities_round_trip_through_base64_and_reject_junk() {
        let identity = DeviceIdentity::generate();
        let encoded = identity.public().to_base64();
        let decoded = PublicIdentity::from_base64(&encoded).unwrap();
        assert_eq!(decoded, identity.public());

        assert!(matches!(
            PublicIdentity::from_base64("not!base64!!"),
            Err(IdentityError::InvalidEncoding(_))
        ));
        // 31 bytes is not a public key.
        assert!(matches!(
            PublicIdentity::from_base64(&encode_b64(&[0u8; 31])),
            Err(IdentityError::InvalidKey(_))
        ));
    }

    #[test]
    fn device_ids_are_derived_and_valid() {
        let identity = DeviceIdentity::generate();
        let id = identity.device_id();
        assert!(id.is_valid());
        assert!(id.as_str().starts_with("did_"));
        // A different identity yields a different id.
        assert_ne!(DeviceIdentity::generate().device_id(), id);
    }

    #[test]
    fn malformed_secret_lengths_are_rejected() {
        assert!(matches!(
            DeviceIdentity::from_secret_bytes(&[0u8; 31]),
            Err(IdentityError::InvalidKey(_))
        ));
    }
}
