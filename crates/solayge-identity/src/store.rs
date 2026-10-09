//! Secure key storage.
//!
//! [`SecureStore`] is the crate's single key-value abstraction. Platform
//! backends keep the private identity seed out of plaintext application state:
//!
//! * [`AppleKeychain`] — the Apple keychain (macOS and iOS).
//! * [`AndroidKeystoreStore`] — Android Keystore-wrapped values.
//! * [`WindowsCredential`] — Windows DPAPI, per user.
//! * [`FileStore`] — an AES-256-GCM file fallback for desktops without a native
//!   credential store.
//! * [`MemoryStore`] — in-memory, for tests.
//!
//! All backends are reached through [`default_store`], so the rest of the
//! application never grows a second key-store abstraction.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rand::RngCore;

use crate::error::{IdentityError, Result};
use crate::identity::DeviceIdentity;

#[cfg(target_os = "android")]
#[allow(unsafe_code)] // JNI/ndk interop; the crate keeps every other module unsafe-free.
mod android;

#[cfg(target_os = "android")]
pub use android::AndroidKeystoreStore;

/// The store key under which the device identity seed is kept.
pub const IDENTITY_KEY: &str = "device-identity-ed25519-v1";

const NONCE_LEN: usize = 12;

/// A minimal, thread-safe secret store.
///
/// Implementations must be safe to share across threads and must not silently
/// return `Ok(None)` for a transient backend failure: [`load_or_create_identity`]
/// relies on that distinction to avoid minting a new identity when the real one
/// is merely unreadable.
pub trait SecureStore: Send + Sync {
    /// Read a value, or `None` when the key is absent.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    /// Create or replace a value.
    fn set(&self, key: &str, value: &[u8]) -> Result<()>;
    /// Remove a value. Absent keys are not an error.
    fn delete(&self, key: &str) -> Result<()>;
    /// A stable backend name for diagnostics.
    fn backend_name(&self) -> &'static str;
}

/// Choose the best store for the current platform.
pub fn default_store(app_id: &str, data_dir: &Path) -> Box<dyn SecureStore> {
    let _ = (app_id, data_dir);

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        Box::new(AppleKeychain::new(app_id))
    }

    #[cfg(target_os = "android")]
    {
        Box::new(AndroidKeystoreStore::new(data_dir, app_id))
    }

    #[cfg(target_os = "windows")]
    {
        Box::new(WindowsCredential::new(data_dir, app_id))
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "android",
        target_os = "windows"
    )))]
    {
        Box::new(FileStore::new(data_dir, app_id))
    }
}

/// Load the stable device identity, generating and persisting one on first run.
///
/// A read error is surfaced rather than treated as "no identity", so a transient
/// keychain failure can never replace a user's identity (and therefore their
/// pairings) with a new one.
pub fn load_or_create_identity(store: &dyn SecureStore) -> Result<DeviceIdentity> {
    match store.get(IDENTITY_KEY)? {
        Some(bytes) => DeviceIdentity::from_secret_bytes(&bytes),
        None => {
            let identity = DeviceIdentity::generate();
            store_identity(store, &identity)?;
            Ok(identity)
        }
    }
}

/// Persist a device identity's secret seed.
pub fn store_identity(store: &dyn SecureStore, identity: &DeviceIdentity) -> Result<()> {
    store.set(IDENTITY_KEY, identity.secret_bytes().as_ref())
}

// ---- in-memory (tests) ----

/// A process-local store. Never use for a real identity: it does not persist.
#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        MemoryStore::default()
    }
}

impl SecureStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .values
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(key)
            .cloned())
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.values
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key.to_string(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.values
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "memory"
    }
}

// ---- encrypted file fallback ----

/// An AES-256-GCM encrypted file store, with the random key in a sibling file
/// readable only by the user.
pub struct FileStore {
    dir: PathBuf,
    service: String,
}

impl FileStore {
    /// A store rooted at `dir` for `service` (used as the file-name prefix).
    pub fn new(dir: &Path, service: &str) -> Self {
        FileStore {
            dir: dir.to_path_buf(),
            service: sanitize(service),
        }
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join(format!("{}.key", self.service))
    }

    fn map_path(&self) -> PathBuf {
        self.dir.join(format!("{}.store.json", self.service))
    }

    fn load_key(&self) -> Result<[u8; 32]> {
        let path = self.key_path();
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                Ok(key)
            }
            // Never rotate a key that exists but is unreadable: it would make
            // stored secrets undecryptable.
            Ok(_) => Err(IdentityError::Storage(format!(
                "corrupt key file {}",
                path.display()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut key = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut key);
                std::fs::create_dir_all(&self.dir)
                    .map_err(|e| IdentityError::Storage(e.to_string()))?;
                write_private(&path, &key)?;
                Ok(key)
            }
            Err(e) => Err(IdentityError::Storage(e.to_string())),
        }
    }

    fn load_map(&self) -> Result<BTreeMap<String, String>> {
        match std::fs::read_to_string(self.map_path()) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                IdentityError::Storage(format!("corrupt store {}: {e}", self.map_path().display()))
            }),
            // A missing file is a genuinely empty store. Any other read error,
            // and any parse error, is surfaced so `load_or_create_identity` can
            // never mistake an unreadable identity for a brand-new install.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(IdentityError::Storage(e.to_string())),
        }
    }

    fn save_map(&self, map: &BTreeMap<String, String>) -> Result<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| IdentityError::Storage(e.to_string()))?;
        let bytes = serde_json::to_vec_pretty(map)
            .map_err(|e| IdentityError::Storage(format!("serialize store: {e}")))?;
        write_private(&self.map_path(), &bytes)?;
        Ok(())
    }

    fn encrypt(&self, plain: &[u8]) -> Result<String> {
        let key = self.load_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| IdentityError::Storage(format!("bad key: {e}")))?;
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), plain)
            .map_err(|_| IdentityError::Storage("encryption failed".into()))?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ciphertext);
        Ok(B64.encode(blob))
    }

    fn decrypt(&self, encoded: &str) -> Result<Vec<u8>> {
        let blob = B64
            .decode(encoded)
            .map_err(|e| IdentityError::Storage(format!("bad ciphertext: {e}")))?;
        if blob.len() < NONCE_LEN + 1 {
            return Err(IdentityError::Storage("ciphertext too short".into()));
        }
        let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
        let key = self.load_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| IdentityError::Storage(format!("bad key: {e}")))?;
        cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| IdentityError::Storage("could not decrypt value".into()))
    }
}

impl SecureStore for FileStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let map = self.load_map()?;
        match map.get(key) {
            Some(encoded) => Ok(Some(self.decrypt(encoded)?)),
            None => Ok(None),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        let mut map = self.load_map()?;
        map.insert(key.to_string(), self.encrypt(value)?);
        self.save_map(&map)
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut map = self.load_map()?;
        if map.remove(key).is_some() {
            self.save_map(&map)?;
        }
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "encrypted-file"
    }
}

/// Write with owner-only permissions from the start (no world-readable window).
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| IdentityError::Storage(e.to_string()))?;
        file.write_all(bytes)
            .map_err(|e| IdentityError::Storage(e.to_string()))?;
        file.flush()
            .map_err(|e| IdentityError::Storage(e.to_string()))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes).map_err(|e| IdentityError::Storage(e.to_string()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ---- Apple keychain (macOS + iOS) ----

/// The Apple keychain generic-password store. Works on macOS and iOS.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub struct AppleKeychain {
    service: String,
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
impl AppleKeychain {
    /// A store using `app_id` as the keychain service.
    pub fn new(app_id: &str) -> Self {
        AppleKeychain {
            service: format!("{app_id}.identity"),
        }
    }

    fn account(key: &str) -> String {
        format!("solayge::{key}")
    }
}

/// Apple status code for "no such keychain item" (`errSecItemNotFound`).
#[cfg(any(target_os = "macos", target_os = "ios"))]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

#[cfg(any(target_os = "macos", target_os = "ios"))]
impl SecureStore for AppleKeychain {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        use security_framework::passwords::{generic_password, PasswordOptions};
        let options = PasswordOptions::new_generic_password(&self.service, &Self::account(key));
        match generic_password(options) {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(IdentityError::Storage(format!("keychain read: {e}"))),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        security_framework::passwords::set_generic_password(
            &self.service,
            &Self::account(key),
            value,
        )
        .map_err(|e| IdentityError::Storage(format!("keychain write: {e}")))
    }

    fn delete(&self, key: &str) -> Result<()> {
        use security_framework::passwords::{
            delete_generic_password, generic_password, PasswordOptions,
        };
        // Confirm existence first so a missing item is a no-op, not an error.
        if generic_password(PasswordOptions::new_generic_password(
            &self.service,
            &Self::account(key),
        ))
        .is_err()
        {
            return Ok(());
        }
        delete_generic_password(&self.service, &Self::account(key))
            .map_err(|e| IdentityError::Storage(format!("keychain delete: {e}")))
    }

    fn backend_name(&self) -> &'static str {
        "apple-keychain"
    }
}

// ---- Windows DPAPI ----

/// A Windows Credential Protection (DPAPI) store, scoped to the current user.
#[cfg(target_os = "windows")]
pub struct WindowsCredential {
    dir: PathBuf,
    service: String,
}

#[cfg(target_os = "windows")]
impl WindowsCredential {
    /// A store rooted at `dir` for `service`.
    pub fn new(dir: &Path, service: &str) -> Self {
        WindowsCredential {
            dir: dir.to_path_buf(),
            service: sanitize(service),
        }
    }

    fn map_path(&self) -> PathBuf {
        self.dir.join(format!("{}.win.store.json", self.service))
    }

    fn load_map(&self) -> Result<BTreeMap<String, String>> {
        match std::fs::read_to_string(self.map_path()) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                IdentityError::Storage(format!("corrupt store {}: {e}", self.map_path().display()))
            }),
            // A missing file is a genuinely empty store; surface every other
            // read/parse failure rather than treating it as "no identity".
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(IdentityError::Storage(e.to_string())),
        }
    }

    fn save_map(&self, map: &BTreeMap<String, String>) -> Result<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| IdentityError::Storage(e.to_string()))?;
        let bytes = serde_json::to_vec_pretty(map)
            .map_err(|e| IdentityError::Storage(format!("serialize store: {e}")))?;
        std::fs::write(self.map_path(), bytes).map_err(|e| IdentityError::Storage(e.to_string()))
    }
}

#[cfg(target_os = "windows")]
impl SecureStore for WindowsCredential {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match self.load_map()?.get(key) {
            Some(encoded) => {
                let plain = dpapi(false, encoded)?;
                Ok(Some(
                    B64.decode(plain)
                        .map_err(|e| IdentityError::Storage(e.to_string()))?,
                ))
            }
            None => Ok(None),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        let mut map = self.load_map()?;
        map.insert(key.to_string(), dpapi(true, &B64.encode(value))?);
        self.save_map(&map)
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut map = self.load_map()?;
        if map.remove(key).is_some() {
            self.save_map(&map)?;
        }
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "windows-dpapi"
    }
}

#[cfg(target_os = "windows")]
fn dpapi(protect: bool, input: &str) -> Result<String> {
    use std::io::Write as _;
    let script = if protect {
        "$i=[Console]::In.ReadToEnd();$b=[Text.Encoding]::UTF8.GetBytes($i);\
         $p=[Security.Cryptography.ProtectedData]::Protect($b,$null,'CurrentUser');\
         [Console]::Out.Write([Convert]::ToBase64String($p))"
    } else {
        "$i=[Console]::In.ReadToEnd();$p=[Convert]::FromBase64String($i);\
         $b=[Security.Cryptography.ProtectedData]::Unprotect($p,$null,'CurrentUser');\
         [Console]::Out.Write([Text.Encoding]::UTF8.GetString($b))"
    };
    let mut child = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| IdentityError::Storage(format!("powershell: {e}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| IdentityError::Storage("no stdin".into()))?
        .write_all(input.as_bytes())
        .map_err(|e| IdentityError::Storage(e.to_string()))?;
    let out = child
        .wait_with_output()
        .map_err(|e| IdentityError::Storage(e.to_string()))?;
    if !out.status.success() {
        return Err(IdentityError::Storage(format!(
            "DPAPI operation failed ({})",
            out.status
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ---- other platforms ----

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    target_os = "windows"
)))]
pub use FileStore as OtherDesktopStore;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "solayge-identity-store-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    fn exercise(store: &dyn SecureStore) {
        assert_eq!(store.get("k").unwrap(), None);
        store.set("k", b"secret").unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some(&b"secret"[..]));
        store.delete("k").unwrap();
        assert_eq!(store.get("k").unwrap(), None);
        // Deleting again is a no-op.
        store.delete("k").unwrap();
    }

    #[test]
    fn memory_store_round_trips() {
        let store = MemoryStore::new();
        exercise(&store);
        assert_eq!(store.backend_name(), "memory");
    }

    #[test]
    fn file_store_round_trips_and_encrypts_at_rest() {
        let dir = temp_dir();
        let store = FileStore::new(&dir, "com.solayge.test");
        exercise(&store);

        store.set("k", b"top-secret-value").unwrap();
        let raw = std::fs::read_to_string(store.map_path()).unwrap();
        assert!(
            !raw.contains("top-secret-value"),
            "file store must not write plaintext"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn file_store_writes_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir();
        let store = FileStore::new(&dir, "com.solayge.test");
        store.set("k", b"v").unwrap();
        for path in [store.key_path(), store.map_path()] {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{path:?} must be 0600");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_key_is_not_rotated() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileStore::new(&dir, "com.solayge.test");
        std::fs::write(store.key_path(), b"short").unwrap();
        assert!(store.set("k", b"v").is_err());
        assert_eq!(std::fs::read(store.key_path()).unwrap(), b"short");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_store_is_an_error_not_a_new_identity() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileStore::new(&dir, "com.solayge.test");
        load_or_create_identity(&store).unwrap();

        // Simulate a partial/failed write by corrupting the map file.
        std::fs::write(store.map_path(), b"{ not json").unwrap();

        // A read must fail loudly rather than look like an empty store...
        assert!(matches!(
            store.get(IDENTITY_KEY),
            Err(IdentityError::Storage(_))
        ));
        // ...and loading must never mint and persist a replacement identity.
        assert!(matches!(
            load_or_create_identity(&store),
            Err(IdentityError::Storage(_))
        ));
        assert_eq!(std::fs::read(store.map_path()).unwrap(), b"{ not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_an_identity_is_stable_and_never_regenerates_on_error() {
        let store = MemoryStore::new();
        let first = load_or_create_identity(&store).unwrap();
        let second = load_or_create_identity(&store).unwrap();
        assert_eq!(first.public(), second.public());
        assert_eq!(first.device_id(), second.device_id());
    }

    #[test]
    fn a_read_error_does_not_mint_a_new_identity() {
        struct Broken;
        impl SecureStore for Broken {
            fn get(&self, _key: &str) -> Result<Option<Vec<u8>>> {
                Err(IdentityError::Storage("backend down".into()))
            }
            fn set(&self, _key: &str, _value: &[u8]) -> Result<()> {
                Ok(())
            }
            fn delete(&self, _key: &str) -> Result<()> {
                Ok(())
            }
            fn backend_name(&self) -> &'static str {
                "broken"
            }
        }
        assert!(matches!(
            load_or_create_identity(&Broken),
            Err(IdentityError::Storage(_))
        ));
    }

    #[test]
    fn the_default_store_is_available_on_this_platform() {
        let dir = temp_dir();
        let store = default_store("com.solayge.test", &dir);
        assert!(!store.backend_name().is_empty());
    }
}
