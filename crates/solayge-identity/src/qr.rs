//! The QR pairing payload.
//!
//! The QR carries **only public, nonsecret** bootstrap data: the protocol
//! version, the desktop's public identity, a one-time pairing id, the desktop's
//! per-pairing nonce, and optional reachability hints. It never contains a
//! private key, a reusable bearer token, or any secret that would let a scanner
//! pair without the desktop's explicit approval and both sides' identity proofs.

use serde::{Deserialize, Serialize};

use crate::error::{IdentityError, Result};
use crate::identity::{decode_b64, encode_b64, DeviceId, PublicIdentity};
use crate::{NONCE_LEN, PROTOCOL_VERSION};

/// Hard cap on an encoded QR payload. QR versions used by the scanner hold a few
/// kilobytes; keeping the payload well under that prevents oversized codes.
pub const MAX_QR_BYTES: usize = 1024;

/// The maximum length of a bearer-free reachability hint string.
const MAX_HINT_LEN: usize = 256;

/// Nonsecret hints that help a phone reach the desktop. Possession of any of
/// these is never sufficient to control the desktop.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityHints {
    /// The desktop's Iroh endpoint id, when one is published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iroh_endpoint_id: Option<String>,
    /// A tailnet-only hostname (`https://device.tailnet.ts.net`), if configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tailscale_host: Option<String>,
    /// Explicit relay URLs, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relay_urls: Vec<String>,
}

impl ReachabilityHints {
    fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("iroh_endpoint_id", self.iroh_endpoint_id.as_deref()),
            ("tailscale_host", self.tailscale_host.as_deref()),
        ] {
            if let Some(value) = value {
                if value.is_empty() || value.len() > MAX_HINT_LEN {
                    return Err(IdentityError::InvalidField(format!(
                        "{name} must be 1..={MAX_HINT_LEN} characters"
                    )));
                }
            }
        }
        if self.relay_urls.len() > 8 {
            return Err(IdentityError::InvalidField(
                "at most 8 relay URLs may be advertised".into(),
            ));
        }
        for url in &self.relay_urls {
            if url.is_empty() || url.len() > MAX_HINT_LEN {
                return Err(IdentityError::InvalidField(
                    "relay URL length out of range".into(),
                ));
            }
        }
        Ok(())
    }
}

/// The payload encoded into the pairing QR code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingQrPayload {
    /// Protocol version the desktop speaks.
    pub protocol_version: u32,
    /// The desktop's derived device id.
    pub desktop_id: String,
    /// The desktop's public key, URL-safe base64.
    pub desktop_public_key: String,
    /// The one-time pairing id. Single-use and short-lived.
    pub pairing_id: String,
    /// The desktop's per-pairing nonce, URL-safe base64.
    pub desktop_nonce: String,
    /// Unix seconds after which the pairing is refused.
    pub expires_at: i64,
    /// Nonsecret reachability/bootstrap hints.
    #[serde(default)]
    pub hints: ReachabilityHints,
}

impl PairingQrPayload {
    /// Build a payload for an offer.
    pub fn new(
        desktop: &PublicIdentity,
        pairing_id: impl Into<String>,
        desktop_nonce: &[u8; NONCE_LEN],
        expires_at: i64,
        hints: ReachabilityHints,
    ) -> Self {
        PairingQrPayload {
            protocol_version: PROTOCOL_VERSION,
            desktop_id: desktop.device_id().to_string(),
            desktop_public_key: desktop.to_base64(),
            pairing_id: pairing_id.into(),
            desktop_nonce: encode_b64(desktop_nonce),
            expires_at,
            hints,
        }
    }

    /// The desktop public identity.
    pub fn desktop_identity(&self) -> Result<PublicIdentity> {
        let identity = PublicIdentity::from_base64(&self.desktop_public_key)?;
        if identity.device_id().as_str() != self.desktop_id {
            return Err(IdentityError::InvalidField(
                "desktop_id does not match desktop_public_key".into(),
            ));
        }
        Ok(identity)
    }

    /// The decoded desktop nonce.
    pub fn desktop_nonce_bytes(&self) -> Result<[u8; NONCE_LEN]> {
        decode_b64(&self.desktop_nonce)?
            .try_into()
            .map_err(|_| IdentityError::InvalidEncoding(format!("nonce must be {NONCE_LEN} bytes")))
    }

    /// Validate every field, including that the id matches the key.
    pub fn validate(&self) -> Result<()> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(IdentityError::InvalidField(format!(
                "unsupported protocol_version {} (expected {PROTOCOL_VERSION})",
                self.protocol_version
            )));
        }
        self.desktop_identity()?;
        if self.pairing_id.is_empty() || self.pairing_id.len() > 64 {
            return Err(IdentityError::InvalidField(
                "pairing_id must be 1..=64 characters".into(),
            ));
        }
        if !self
            .pairing_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(IdentityError::InvalidField(
                "pairing_id contains an unsupported character".into(),
            ));
        }
        self.desktop_nonce_bytes()?;
        if self.expires_at <= 0 {
            return Err(IdentityError::InvalidField(
                "expires_at must be a positive unix timestamp".into(),
            ));
        }
        self.hints.validate()?;
        Ok(())
    }

    /// Serialize to the string encoded in the QR image.
    pub fn to_qr_string(&self) -> Result<String> {
        self.validate()?;
        let encoded = serde_json::to_string(self)
            .map_err(|e| IdentityError::Internal(format!("QR serialization failed: {e}")))?;
        if encoded.len() > MAX_QR_BYTES {
            return Err(IdentityError::InvalidField(format!(
                "QR payload of {} bytes exceeds the {MAX_QR_BYTES}-byte limit",
                encoded.len()
            )));
        }
        Ok(encoded)
    }

    /// Parse and validate a scanned QR string.
    pub fn from_qr_string(input: &str) -> Result<Self> {
        if input.len() > MAX_QR_BYTES * 4 {
            return Err(IdentityError::InvalidField("QR input is too large".into()));
        }
        let payload: PairingQrPayload = serde_json::from_str(input)
            .map_err(|e| IdentityError::InvalidEncoding(format!("QR payload: {e}")))?;
        payload.validate()?;
        Ok(payload)
    }
}

/// A device id that a QR advertised; kept as a helper for callers.
pub fn qr_desktop_id(payload: &PairingQrPayload) -> Result<DeviceId> {
    Ok(payload.desktop_identity()?.device_id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceIdentity;

    fn payload() -> (DeviceIdentity, PairingQrPayload) {
        let desktop = DeviceIdentity::generate();
        let qr = PairingQrPayload::new(
            &desktop.public(),
            "pair_123",
            &[7u8; NONCE_LEN],
            2_000,
            ReachabilityHints {
                iroh_endpoint_id: Some("iroh-endpoint".into()),
                tailscale_host: Some("host.tailnet.ts.net".into()),
                relay_urls: vec!["https://relay.example".into()],
            },
        );
        (desktop, qr)
    }

    #[test]
    fn a_valid_payload_round_trips() {
        let (_desktop, qr) = payload();
        let encoded = qr.to_qr_string().unwrap();
        let decoded = PairingQrPayload::from_qr_string(&encoded).unwrap();
        assert_eq!(decoded, qr);
        assert_eq!(decoded.desktop_nonce_bytes().unwrap(), [7u8; NONCE_LEN]);
    }

    #[test]
    fn the_qr_never_contains_private_or_bearer_material() {
        let (_desktop, qr) = payload();
        let encoded = qr.to_qr_string().unwrap().to_lowercase();
        for forbidden in [
            "private", "secret", "seed", "bearer", "token", "password", "refresh",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "QR payload must not contain `{forbidden}`: {encoded}"
            );
        }
        // It carries only the advertised public fields.
        let value: serde_json::Value = serde_json::from_str(&qr.to_qr_string().unwrap()).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 7);
        for key in [
            "protocol_version",
            "desktop_id",
            "desktop_public_key",
            "pairing_id",
            "desktop_nonce",
            "expires_at",
            "hints",
        ] {
            assert!(object.contains_key(key), "missing QR field {key}");
        }
    }

    #[test]
    fn mismatched_id_or_key_is_rejected() {
        let (_desktop, mut qr) = payload();
        qr.desktop_id = "did_someoneelse".into();
        assert!(matches!(qr.validate(), Err(IdentityError::InvalidField(_))));

        let other = DeviceIdentity::generate();
        qr.desktop_id = other.device_id().to_string();
        qr.desktop_public_key = other.public().to_base64();
        // Now consistent again:
        qr.validate().unwrap();
    }

    #[test]
    fn wrong_protocol_version_is_rejected() {
        let (_desktop, mut qr) = payload();
        qr.protocol_version = PROTOCOL_VERSION + 1;
        assert!(matches!(qr.validate(), Err(IdentityError::InvalidField(_))));
        let encoded = serde_json::to_string(&qr).unwrap();
        assert!(PairingQrPayload::from_qr_string(&encoded).is_err());
    }

    #[test]
    fn bad_nonce_and_expiry_are_rejected() {
        let (_desktop, mut qr) = payload();
        qr.desktop_nonce = "AAAA".into();
        assert!(qr.validate().is_err());

        let (_desktop, mut qr) = payload();
        qr.expires_at = 0;
        assert!(qr.validate().is_err());
    }
}
