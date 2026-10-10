//! The pairing transcript: the exact bytes both sides sign and fingerprint.
//!
//! Binding the proof and the verification code to *both* public identities and
//! *both* nonces defeats channel confusion and active interposition: an attacker
//! who relays messages between two honest peers cannot make them agree on a
//! code, because the two sides' transcripts differ by the nonces each chose.

use serde::{Deserialize, Serialize};

use crate::identity::{DeviceId, PublicIdentity};
use crate::NONCE_LEN;

/// Domain separator for the canonical transcript encoding.
const TRANSCRIPT_CONTEXT: &[u8] = b"solayge/pairing/transcript/v1";

/// Everything both sides bind a pairing proof and fingerprint to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingTranscript {
    /// Protocol version in force.
    pub protocol_version: u32,
    /// The one-time pairing id from the QR / manual code.
    pub pairing_id: String,
    /// An optional binding to a specific transport session (for example an Iroh
    /// connection id). When both peers set the same value it is part of the
    /// signed bytes, so a proof cannot be replayed onto a different channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_binding: Option<String>,
    /// The desktop's public identity.
    pub desktop: PublicIdentity,
    /// The phone's public identity.
    pub device: PublicIdentity,
    /// The desktop's fresh per-pairing nonce.
    pub desktop_nonce: [u8; NONCE_LEN],
    /// The phone's fresh per-pairing nonce.
    pub device_nonce: [u8; NONCE_LEN],
}

impl PairingTranscript {
    /// The desktop device id.
    pub fn desktop_id(&self) -> DeviceId {
        self.desktop.device_id()
    }

    /// The phone device id.
    pub fn device_id(&self) -> DeviceId {
        self.device.device_id()
    }

    /// A canonical, unambiguous encoding. Every field is length-prefixed so that
    /// no two different transcripts can produce the same byte string.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(256);
        out.extend_from_slice(TRANSCRIPT_CONTEXT);
        out.extend_from_slice(&self.protocol_version.to_be_bytes());
        push_field(&mut out, self.pairing_id.as_bytes());
        match &self.transport_binding {
            Some(binding) => {
                out.push(1);
                push_field(&mut out, binding.as_bytes());
            }
            None => out.push(0),
        }
        push_field(&mut out, self.desktop.as_bytes());
        push_field(&mut out, self.device.as_bytes());
        push_field(&mut out, &self.desktop_nonce);
        push_field(&mut out, &self.device_nonce);
        out
    }

    /// The transcript fingerprint. See [`crate::fingerprint`].
    pub fn fingerprint(&self) -> [u8; 32] {
        crate::fingerprint::fingerprint_bytes(self)
    }

    /// The short human-comparable verification code.
    pub fn verification_code(&self) -> crate::fingerprint::VerificationCode {
        crate::fingerprint::verification_code(self)
    }
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u32).to_be_bytes());
    out.extend_from_slice(field);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceIdentity;

    fn sample() -> PairingTranscript {
        let desktop = DeviceIdentity::generate();
        let device = DeviceIdentity::generate();
        PairingTranscript {
            protocol_version: crate::PROTOCOL_VERSION,
            pairing_id: "pair_abc".to_string(),
            transport_binding: None,
            desktop: desktop.public(),
            device: device.public(),
            desktop_nonce: [3u8; 32],
            device_nonce: [4u8; 32],
        }
    }

    #[test]
    fn canonical_encoding_is_deterministic() {
        let t = sample();
        assert_eq!(t.canonical_bytes(), t.canonical_bytes());
    }

    #[test]
    fn canonical_encoding_frames_adjacent_fields() {
        // "ab" + "c" must not encode the same as "a" + "bc".
        let mut first = sample();
        first.pairing_id = "ab".into();
        let mut second = sample();
        second.pairing_id = "a".into();
        assert_ne!(first.canonical_bytes(), second.canonical_bytes());
    }

    #[test]
    fn device_ids_come_from_the_transcript_keys() {
        let t = sample();
        assert_eq!(t.desktop_id(), t.desktop.device_id());
        assert_eq!(t.device_id(), t.device.device_id());
        assert_eq!(t.verification_code(), t.verification_code());
    }
}
