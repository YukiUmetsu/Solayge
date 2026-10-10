//! The host-side registry of authorized devices and revocations.
//!
//! This is the one authority every transport consults. A device is trusted only
//! while it is present and not revoked; revocation immediately drops it and
//! every later authorization check — on any transport — fails until the device
//! is paired again through a fresh, human-approved pairing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{IdentityError, Result};
use crate::identity::DeviceId;
use crate::scope::Scope;

/// A device the desktop has approved and stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    /// The derived device id.
    pub device_id: String,
    /// The human-friendly name claimed by the phone.
    pub name: String,
    /// `ios` | `android` | `macos` | `windows` | `linux` | …
    pub platform: String,
    /// The pinned public identity, URL-safe base64.
    pub public_key: String,
    /// The scopes the desktop granted.
    pub scopes: Vec<Scope>,
    /// Unix seconds when pairing was approved.
    pub paired_at: i64,
    /// Unix seconds of the last authorized connection, if any.
    #[serde(default)]
    pub last_seen: Option<i64>,
    /// Whether a transport session is currently active. Transient: it is not
    /// restored from persisted state, because a restart means no session is open.
    #[serde(default)]
    pub online: bool,
}

/// A serializable snapshot of the registry, for the host's persistence layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySnapshot {
    /// The approved devices.
    pub devices: Vec<PairedDevice>,
    /// The ids revoked since pairing.
    #[serde(default)]
    pub revoked: Vec<String>,
}

struct Inner {
    devices: BTreeMap<String, PairedDevice>,
    revoked: BTreeSet<String>,
    revision: u64,
}

/// Thread-safe registry of paired devices and revocations.
pub struct DeviceRegistry {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for DeviceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = lock(&self.inner);
        f.debug_struct("DeviceRegistry")
            .field("devices", &inner.devices.len())
            .field("revoked", &inner.revoked.len())
            .finish()
    }
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        DeviceRegistry::new()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl DeviceRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        DeviceRegistry {
            inner: Mutex::new(Inner {
                devices: BTreeMap::new(),
                revoked: BTreeSet::new(),
                revision: 0,
            }),
        }
    }

    /// Rebuild a registry from a persisted snapshot. `online` is reset because a
    /// restart means no session can be open.
    pub fn from_snapshot(snapshot: RegistrySnapshot) -> Self {
        let mut devices = BTreeMap::new();
        for mut device in snapshot.devices {
            device.online = false;
            devices.insert(device.device_id.clone(), device);
        }
        DeviceRegistry {
            inner: Mutex::new(Inner {
                devices,
                revoked: snapshot.revoked.into_iter().collect(),
                revision: 0,
            }),
        }
    }

    /// Export the persistable state.
    pub fn snapshot(&self) -> RegistrySnapshot {
        let inner = lock(&self.inner);
        RegistrySnapshot {
            devices: inner.devices.values().cloned().collect(),
            revoked: inner.revoked.iter().cloned().collect(),
        }
    }

    /// Store an approved device. A fresh approval clears any prior revocation.
    pub fn register(&self, device: PairedDevice) -> Result<()> {
        let id = DeviceId::new(device.device_id.clone());
        if !id.is_valid() {
            return Err(IdentityError::InvalidField(format!(
                "not a valid device id: {}",
                device.device_id
            )));
        }
        let mut inner = lock(&self.inner);
        inner.revoked.remove(&device.device_id);
        inner.devices.insert(device.device_id.clone(), device);
        inner.revision += 1;
        Ok(())
    }

    /// Whether a device id is registered at all.
    pub fn is_registered(&self, device_id: &str) -> bool {
        lock(&self.inner).devices.contains_key(device_id)
    }

    /// Whether a device id was explicitly revoked.
    pub fn is_revoked(&self, device_id: &str) -> bool {
        lock(&self.inner).revoked.contains(device_id)
    }

    /// Whether a device may currently act. This is the check a host performs on
    /// every connection and every request; it fails for revoked and unknown
    /// devices alike.
    pub fn is_authorized(&self, device_id: &str) -> bool {
        let inner = lock(&self.inner);
        inner.devices.contains_key(device_id) && !inner.revoked.contains(device_id)
    }

    /// Authorize a device for an application session and record the visit.
    pub fn authorize(&self, device_id: &str, now: i64) -> Result<PairedDevice> {
        let mut inner = lock(&self.inner);
        if inner.revoked.contains(device_id) {
            return Err(IdentityError::DeviceRevoked(device_id.to_string()));
        }
        match inner.devices.get_mut(device_id) {
            Some(device) => {
                device.last_seen = Some(now);
                device.online = true;
                Ok(device.clone())
            }
            None => Err(IdentityError::UnknownDevice(device_id.to_string())),
        }
    }

    /// Mark a device offline without revoking it.
    pub fn set_offline(&self, device_id: &str) {
        if let Some(device) = lock(&self.inner).devices.get_mut(device_id) {
            device.online = false;
        }
    }

    /// The full authorized-device list, oldest pairing first.
    pub fn list(&self) -> Vec<PairedDevice> {
        let mut devices: Vec<PairedDevice> = lock(&self.inner).devices.values().cloned().collect();
        devices.sort_by_key(|d| (d.paired_at, d.device_id.clone()));
        devices
    }

    /// Look up one device.
    pub fn get(&self, device_id: &str) -> Option<PairedDevice> {
        lock(&self.inner).devices.get(device_id).cloned()
    }

    /// The pinned public key for a device id.
    pub fn public_key_of(&self, device_id: &str) -> Option<String> {
        lock(&self.inner)
            .devices
            .get(device_id)
            .map(|d| d.public_key.clone())
    }

    /// Revoke a device. Returns whether a device was present. Revocation is
    /// idempotent and also marks previously unknown ids as revoked, so a device
    /// that tries to authenticate after being revoked cannot slip through.
    pub fn revoke(&self, device_id: &str) -> bool {
        let mut inner = lock(&self.inner);
        let removed = inner.devices.remove(device_id).is_some();
        let inserted = inner.revoked.insert(device_id.to_string());
        if removed || inserted {
            inner.revision += 1;
        }
        removed
    }

    /// A monotonically increasing counter bumped on every register/revoke, so a
    /// long-lived session can cheaply detect that its authorization changed.
    pub fn revision(&self) -> u64 {
        lock(&self.inner).revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceIdentity, Scope};

    fn device(identity: &DeviceIdentity, name: &str) -> PairedDevice {
        PairedDevice {
            device_id: identity.device_id().to_string(),
            name: name.to_string(),
            platform: "ios".into(),
            public_key: identity.public().to_base64(),
            scopes: Scope::default_phone(),
            paired_at: 10,
            last_seen: None,
            online: false,
        }
    }

    #[test]
    fn an_approved_device_authorizes_and_records_last_seen() {
        let registry = DeviceRegistry::new();
        let identity = DeviceIdentity::generate();
        registry.register(device(&identity, "iPhone")).unwrap();

        assert!(registry.is_authorized(&identity.device_id().to_string()));
        let authorized = registry
            .authorize(&identity.device_id().to_string(), 99)
            .unwrap();
        assert_eq!(authorized.last_seen, Some(99));
        assert!(authorized.online);
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn a_revoked_device_fails_every_reconnect_until_repaired() {
        let registry = DeviceRegistry::new();
        let identity = DeviceIdentity::generate();
        let id = identity.device_id().to_string();
        registry.register(device(&identity, "iPhone")).unwrap();

        assert!(registry.revoke(&id));
        assert!(registry.is_revoked(&id));
        assert!(!registry.is_authorized(&id));

        // Every reconnect attempt fails, with a revocation error.
        for _ in 0..5 {
            assert_eq!(
                registry.authorize(&id, 100).unwrap_err(),
                IdentityError::DeviceRevoked(id.clone())
            );
        }
        // A revoked device that was never stored is still refused.
        let unknown = DeviceIdentity::generate().device_id().to_string();
        assert!(!registry.is_authorized(&unknown));
        assert!(matches!(
            registry.authorize(&unknown, 1),
            Err(IdentityError::UnknownDevice(_))
        ));

        // Only a fresh approval re-registers it, clearing the revocation.
        registry
            .register(device(&identity, "iPhone (re-paired)"))
            .unwrap();
        assert!(!registry.is_revoked(&id));
        assert!(registry.authorize(&id, 101).is_ok());
    }

    #[test]
    fn snapshots_round_trip_and_reset_online() {
        let registry = DeviceRegistry::new();
        let identity = DeviceIdentity::generate();
        let mut d = device(&identity, "Pixel");
        d.online = true;
        d.last_seen = Some(50);
        registry.register(d).unwrap();
        let other = DeviceIdentity::generate();
        registry.revoke(&other.device_id().to_string());

        let snapshot = registry.snapshot();
        let restored = DeviceRegistry::from_snapshot(snapshot);
        assert!(!restored.list()[0].online, "online is transient");
        assert_eq!(restored.list()[0].last_seen, Some(50));
        assert!(restored.is_revoked(&other.device_id().to_string()));
    }

    #[test]
    fn revision_advances_on_register_and_revoke() {
        let registry = DeviceRegistry::new();
        let before = registry.revision();
        let identity = DeviceIdentity::generate();
        registry.register(device(&identity, "iPhone")).unwrap();
        let after_register = registry.revision();
        assert!(after_register > before);
        registry.revoke(&identity.device_id().to_string());
        assert!(registry.revision() > after_register);
    }
}
