//! Pairing verification fingerprints and short human codes.
//!
//! A QR code only carries public bootstrap data; on its own it does **not**
//! confer trust. After the phone sends its identity in `pair.begin`, both sides
//! derive the same short code from the full pairing transcript — both device
//! public identities and both nonces — and the human compares them. If the
//! values differ, one side is talking to a different peer and must abort.
//!
//! The derivation is deliberately simple and auditable: SHA-256 over a
//! domain-separated, length-prefixed canonical encoding of the transcript. It
//! is a fingerprint, not a MAC; the security comes from the mutual Ed25519
//! proofs over the same transcript.

use sha2::{Digest, Sha256};

use crate::transcript::PairingTranscript;

/// Domain separator for the transcript fingerprint.
pub const FINGERPRINT_CONTEXT: &[u8] = b"solayge/pairing/fingerprint/v1";

/// Number of decimal digits in a verification code.
pub const CODE_DIGITS: usize = 6;

/// A short, human-comparable code derived from the pairing transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationCode(String);

impl VerificationCode {
    /// The full code including the grouping space, e.g. `"123 456"`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The bare digits, without the grouping space.
    pub fn digits(&self) -> String {
        self.0.replace(' ', "")
    }
}

impl std::fmt::Display for VerificationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The 32-byte transcript fingerprint both sides must agree on.
pub fn fingerprint_bytes(transcript: &PairingTranscript) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(FINGERPRINT_CONTEXT);
    hasher.update(transcript.canonical_bytes());
    hasher.finalize().into()
}

/// The short verification code derived from the transcript fingerprint.
pub fn verification_code(transcript: &PairingTranscript) -> VerificationCode {
    let hash = fingerprint_bytes(transcript);
    let value = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) % 1_000_000;
    let digits = format!("{value:0CODE_DIGITS$}");
    VerificationCode(format!("{} {}", &digits[..3], &digits[3..]))
}

/// Render a fingerprint as uppercase hex, grouped in fours by default for
/// readability in a list or dialog.
pub fn format_fingerprint(hash: &[u8; 32]) -> String {
    hex_grouped(hash, 4)
}

/// Hex-encode `bytes` and insert a space every `group` hex characters.
pub(crate) fn hex_grouped(bytes: &[u8], group: usize) -> String {
    let group = group.max(1);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let mut out = String::with_capacity(hex.len() + hex.len() / group);
    for (i, ch) in hex.chars().enumerate() {
        if i > 0 && i % group == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out.to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceIdentity, NONCE_LEN};

    fn transcript(
        pairing_id: &str,
        desktop_nonce: [u8; NONCE_LEN],
        device_nonce: [u8; NONCE_LEN],
        desktop: &DeviceIdentity,
        device: &DeviceIdentity,
    ) -> PairingTranscript {
        PairingTranscript {
            protocol_version: crate::PROTOCOL_VERSION,
            pairing_id: pairing_id.to_string(),
            transport_binding: None,
            desktop: desktop.public(),
            device: device.public(),
            desktop_nonce,
            device_nonce,
        }
    }

    #[test]
    fn the_same_transcript_always_yields_the_same_code() {
        let desktop = DeviceIdentity::generate();
        let device = DeviceIdentity::generate();
        let t = transcript("pair_1", [1u8; 32], [2u8; 32], &desktop, &device);
        assert_eq!(verification_code(&t), verification_code(&t.clone()));
        assert_eq!(fingerprint_bytes(&t), fingerprint_bytes(&t));
    }

    #[test]
    fn the_code_is_six_digits_and_grouped() {
        let desktop = DeviceIdentity::generate();
        let device = DeviceIdentity::generate();
        let t = transcript("pair_1", [1u8; 32], [2u8; 32], &desktop, &device);
        let code = verification_code(&t);
        assert_eq!(code.as_str().len(), CODE_DIGITS + 1);
        assert_eq!(code.as_str().chars().filter(|c| *c == ' ').count(), 1);
        assert_eq!(code.digits().len(), CODE_DIGITS);
        assert!(code.digits().chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn every_transcript_input_changes_the_fingerprint() {
        let desktop = DeviceIdentity::generate();
        let device = DeviceIdentity::generate();
        let other = DeviceIdentity::generate();
        let base = transcript("pair_1", [1u8; 32], [2u8; 32], &desktop, &device);

        let different_id = transcript("pair_2", [1u8; 32], [2u8; 32], &desktop, &device);
        let different_desktop = transcript("pair_1", [9u8; 32], [2u8; 32], &desktop, &device);
        let different_device_nonce = transcript("pair_1", [1u8; 32], [7u8; 32], &desktop, &device);
        let different_device = transcript("pair_1", [1u8; 32], [2u8; 32], &desktop, &other);
        let different_desktop_key = transcript("pair_1", [1u8; 32], [2u8; 32], &other, &device);
        let mut different_binding = base.clone();
        different_binding.transport_binding = Some("iroh:endpoint-a".to_string());

        for changed in [
            different_id,
            different_desktop,
            different_device_nonce,
            different_device,
            different_desktop_key,
            different_binding,
        ] {
            assert_ne!(
                fingerprint_bytes(&base),
                fingerprint_bytes(&changed),
                "a changed transcript input must change the fingerprint"
            );
        }
    }

    #[test]
    fn fingerprints_are_rendered_as_grouped_uppercase_hex() {
        let hash = [0xabu8; 32];
        let rendered = format_fingerprint(&hash);
        assert!(rendered.starts_with("ABAB ABAB"));
        assert!(!rendered.contains("ab"));
    }
}
