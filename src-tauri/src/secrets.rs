//! Encrypted-at-rest storage for per-project environment variables.
//!
//! Two backends, chosen per account in Settings:
//!
//! - `keychain` — the OS-native secret store: macOS Keychain via the built-in
//!   `security` tool, or Windows Credential-Manager-equivalent DPAPI. Values
//!   never touch `state.json`.
//! - `file` — an AES-256-GCM encrypted JSON file in the app data directory,
//!   with the random key kept in a sibling file readable only by the user.
//!
//! `state.json` only ever holds the variable *names* (`ProjectEnvVar`); every
//! value lives in the store keyed by `<project path> + <variable name>`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rand::RngCore;

/// Keychain service name shared by every item this app writes.
pub const SERVICE: &str = "solayge";

const ENV_SEP: char = '\u{1f}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    Keychain,
    File,
}

impl StoreKind {
    /// Resolve the configured value, falling back to the OS-native store when
    /// one is available and otherwise to the encrypted file store.
    pub fn parse(configured: Option<&str>) -> StoreKind {
        match configured.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("file") => StoreKind::File,
            Some("keychain") => StoreKind::Keychain,
            _ if native_available() => StoreKind::Keychain,
            _ => StoreKind::File,
        }
    }
}

/// Whether this platform ships a native secret store we know how to drive.
pub fn native_available() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// A resolved secret store rooted at the app data directory.
#[derive(Debug, Clone)]
pub struct Secrets {
    data_dir: PathBuf,
    kind: StoreKind,
}

impl Secrets {
    pub fn new(data_dir: &Path, kind: StoreKind) -> Self {
        Secrets {
            data_dir: data_dir.to_path_buf(),
            kind,
        }
    }

    fn account(project: &str, key: &str) -> String {
        format!("{project}{ENV_SEP}{key}")
    }

    pub fn get(&self, project: &str, key: &str) -> Result<Option<String>> {
        let account = Self::account(project, key);
        match self.kind {
            StoreKind::Keychain => keychain_get(&self.data_dir, &account),
            StoreKind::File => file_get(&self.data_dir, &account),
        }
    }

    pub fn set(&self, project: &str, key: &str, value: &str) -> Result<()> {
        let account = Self::account(project, key);
        match self.kind {
            StoreKind::Keychain => keychain_set(&self.data_dir, &account, value),
            StoreKind::File => file_set(&self.data_dir, &account, value),
        }
    }

    pub fn delete(&self, project: &str, key: &str) -> Result<()> {
        let account = Self::account(project, key);
        match self.kind {
            StoreKind::Keychain => keychain_delete(&self.data_dir, &account),
            StoreKind::File => file_delete(&self.data_dir, &account),
        }
    }

    /// Values for `keys`, in order, skipping any that are not set.
    pub fn get_many(&self, project: &str, keys: &[String]) -> Result<Vec<(String, String)>> {
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            if let Some(v) = self.get(project, k)? {
                out.push((k.clone(), v));
            }
        }
        Ok(out)
    }

    /// Copy every `(project, key)` secret from one store to another, removing
    /// it from the source. Used when the account switches stores. No-op when
    /// the stores are the same.
    ///
    /// Two-phase on purpose: nothing is deleted from the source until *every*
    /// value has been written to the destination. A failure partway through
    /// (a locked keychain, a full disk) therefore leaves the config's current
    /// store intact instead of stranding secrets in the store it does not point
    /// at.
    pub fn migrate(
        data_dir: &Path,
        from: StoreKind,
        to: StoreKind,
        entries: &[(String, String)],
    ) -> Result<()> {
        if from == to {
            return Ok(());
        }
        let src = Secrets::new(data_dir, from);
        let dst = Secrets::new(data_dir, to);
        migrate_entries(
            entries,
            |project, key| src.get(project, key),
            |project, key, value| dst.set(project, key, value),
            |project, key| {
                let _ = src.delete(project, key);
            },
        )
    }
}

/// The store-agnostic half of [`Secrets::migrate`]: read every value from the
/// source, write it all to the destination, and only then remove the originals.
/// Taking the operations as closures keeps the ordering rule testable without a
/// real keychain or a second data directory.
fn migrate_entries(
    entries: &[(String, String)],
    mut get: impl FnMut(&str, &str) -> Result<Option<String>>,
    mut set: impl FnMut(&str, &str, &str) -> Result<()>,
    mut remove: impl FnMut(&str, &str),
) -> Result<()> {
    let mut copied: Vec<(String, String)> = Vec::with_capacity(entries.len());
    for (project, key) in entries {
        let Some(value) = get(project, key)? else {
            continue;
        };
        set(project, key, &value)?;
        copied.push((project.clone(), key.clone()));
    }
    // Every write succeeded; the source copies are now safe to drop.
    for (project, key) in &copied {
        remove(project, key);
    }
    Ok(())
}

// ---- encrypted file store ----

fn secrets_path(data_dir: &Path) -> PathBuf {
    data_dir.join("secrets.json")
}

fn key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("secrets.key")
}

/// Create/overwrite a file with owner-only permissions from the start, so there
/// is no window where the key or ciphertext is world-readable.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.flush()?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)?;
    }
    // Also tighten a pre-existing file that was created with looser modes.
    restrict_permissions(path);
    Ok(())
}

fn load_key(data_dir: &Path) -> Result<[u8; 32]> {
    let path = key_path(data_dir);
    match std::fs::read(&path) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            Ok(key)
        }
        // Never silently rotate a key: that would make stored secrets
        // undecryptable. Surface the corruption instead.
        Ok(_) => Err(anyhow!(
            "the secret key file is corrupt; refusing to overwrite {}",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut key = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut key);
            std::fs::create_dir_all(data_dir)?;
            write_private(&path, &key)?;
            Ok(key)
        }
        Err(e) => Err(anyhow!("could not read the secret key: {e}")),
    }
}

fn load_map(data_dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(secrets_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_map(data_dir: &Path, map: &BTreeMap<String, String>) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    write_private(&secrets_path(data_dir), &serde_json::to_vec_pretty(map)?)?;
    Ok(())
}

fn encrypt(data_dir: &Path, plain: &str) -> Result<String> {
    let key = load_key(data_dir)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow!("bad key: {e}"))?;
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .map_err(|e| anyhow!("encrypt failed: {e}"))?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ciphertext);
    Ok(B64.encode(blob))
}

fn decrypt(data_dir: &Path, encoded: &str) -> Result<String> {
    let blob = B64.decode(encoded).map_err(|e| anyhow!("bad ciphertext: {e}"))?;
    if blob.len() < 13 {
        return Err(anyhow!("ciphertext too short"));
    }
    let (nonce, ciphertext) = blob.split_at(12);
    let key = load_key(data_dir)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow!("bad key: {e}"))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow!("could not decrypt secret"))?;
    String::from_utf8(plain).map_err(|e| anyhow!("secret is not text: {e}"))
}

fn file_get(data_dir: &Path, account: &str) -> Result<Option<String>> {
    let map = load_map(data_dir);
    let Some(encoded) = map.get(account) else {
        return Ok(None);
    };
    Ok(Some(decrypt(data_dir, encoded)?))
}

fn file_set(data_dir: &Path, account: &str, value: &str) -> Result<()> {
    let mut map = load_map(data_dir);
    map.insert(account.to_string(), encrypt(data_dir, value)?);
    save_map(data_dir, &map)
}

fn file_delete(data_dir: &Path, account: &str) -> Result<()> {
    let mut map = load_map(data_dir);
    if map.remove(account).is_some() {
        save_map(data_dir, &map)?;
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

// ---- macOS keychain ----

#[cfg(target_os = "macos")]
fn keychain_set(_data_dir: &Path, account: &str, value: &str) -> Result<()> {
    // `security` has no stdin form for the password here, so the value is
    // passed as an argument. That is visible only to processes running as the
    // same user, and the item itself is protected by the login keychain.
    let status = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            SERVICE,
            "-a",
            account,
            "-w",
            value,
        ])
        .status()
        .context("running security add-generic-password")?;
    if !status.success() {
        return Err(anyhow!("security add-generic-password failed ({status})"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn keychain_get(_data_dir: &Path, account: &str) -> Result<Option<String>> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"])
        .output()
        .context("running security find-generic-password")?;
    if !out.status.success() {
        return Ok(None);
    }
    let mut value = String::from_utf8_lossy(&out.stdout).to_string();
    if value.ends_with('\n') {
        value.pop();
    }
    Ok(Some(value))
}

#[cfg(target_os = "macos")]
fn keychain_delete(_data_dir: &Path, account: &str) -> Result<()> {
    let _ = std::process::Command::new("security")
        .args(["delete-generic-password", "-s", SERVICE, "-a", account])
        .output();
    Ok(())
}

// ---- Windows DPAPI ----

#[cfg(target_os = "windows")]
fn windows_path(data_dir: &Path) -> PathBuf {
    data_dir.join("secrets.win.json")
}

#[cfg(target_os = "windows")]
fn windows_load(data_dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(windows_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn windows_save(data_dir: &Path, map: &BTreeMap<String, String>) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(windows_path(data_dir), serde_json::to_vec_pretty(map)?)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn dpapi(protect: bool, input: &str) -> Result<String> {
    use std::io::Write as _;
    let script = if protect {
        "$i=[Console]::In.ReadToEnd();$b=[Text.Encoding]::UTF8.GetBytes($i);\
         $p=[Security.Cryptography.ProtectedData]::Protect($b,$null,'CurrentUser');\
         [Console]::Out.Write([Convert]::ToBase64String($p))"
    } else {
        "$i=[Console]::In.ReadToEnd();\
         $p=[Convert]::FromBase64String($i);\
         $b=[Security.Cryptography.ProtectedData]::Unprotect($p,$null,'CurrentUser');\
         [Console]::Out.Write([Text.Encoding]::UTF8.GetString($b))"
    };
    let mut child = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("running powershell for DPAPI")?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no stdin"))?
        .write_all(input.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(anyhow!("DPAPI operation failed ({})", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn keychain_set(data_dir: &Path, account: &str, value: &str) -> Result<()> {
    let mut map = windows_load(data_dir);
    map.insert(account.to_string(), dpapi(true, value)?);
    windows_save(data_dir, &map)
}

#[cfg(target_os = "windows")]
fn keychain_get(data_dir: &Path, account: &str) -> Result<Option<String>> {
    let map = windows_load(data_dir);
    let Some(encoded) = map.get(account) else {
        return Ok(None);
    };
    Ok(Some(dpapi(false, encoded)?))
}

#[cfg(target_os = "windows")]
fn keychain_delete(data_dir: &Path, account: &str) -> Result<()> {
    let mut map = windows_load(data_dir);
    if map.remove(account).is_some() {
        windows_save(data_dir, &map)?;
    }
    Ok(())
}

// ---- other platforms ----

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn keychain_set(_data_dir: &Path, _account: &str, _value: &str) -> Result<()> {
    Err(anyhow!(
        "no native secret store on this platform; choose the encrypted file store"
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn keychain_get(_data_dir: &Path, _account: &str) -> Result<Option<String>> {
    Err(anyhow!(
        "no native secret store on this platform; choose the encrypted file store"
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn keychain_delete(_data_dir: &Path, _account: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("solayge-secrets-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn file_store_round_trips_and_fails_closed() {
        let dir = temp_dir();
        let store = Secrets::new(&dir, StoreKind::File);
        assert_eq!(store.get("/p", "TOKEN").unwrap(), None);
        store.set("/p", "TOKEN", "s3cr3t").unwrap();
        store.set("/p", "EMPTY", "").unwrap();
        assert_eq!(store.get("/p", "TOKEN").unwrap().as_deref(), Some("s3cr3t"));
        assert_eq!(store.get("/p", "EMPTY").unwrap().as_deref(), Some(""));

        // The value is not written in the clear.
        let raw = std::fs::read_to_string(secrets_path(&dir)).unwrap();
        assert!(!raw.contains("s3cr3t"));

        store.delete("/p", "TOKEN").unwrap();
        assert_eq!(store.get("/p", "TOKEN").unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_store_isolates_projects() {
        let dir = temp_dir();
        let store = Secrets::new(&dir, StoreKind::File);
        store.set("/a", "K", "one").unwrap();
        store.set("/b", "K", "two").unwrap();
        assert_eq!(store.get("/a", "K").unwrap().as_deref(), Some("one"));
        assert_eq!(store.get("/b", "K").unwrap().as_deref(), Some("two"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_many_skips_missing() {
        let dir = temp_dir();
        let store = Secrets::new(&dir, StoreKind::File);
        store.set("/p", "A", "1").unwrap();
        let keys = vec!["A".to_string(), "B".to_string()];
        assert_eq!(
            store.get_many("/p", &keys).unwrap(),
            vec![("A".to_string(), "1".to_string())]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_key_is_not_overwritten() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(key_path(&dir), b"short").unwrap();
        let store = Secrets::new(&dir, StoreKind::File);
        assert!(store.set("/p", "K", "v").is_err());
        assert_eq!(std::fs::read(key_path(&dir)).unwrap(), b"short");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn key_and_ciphertext_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir();
        let store = Secrets::new(&dir, StoreKind::File);
        store.set("/p", "K", "v").unwrap();
        for path in [key_path(&dir), secrets_path(&dir)] {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{path:?} should be 0600");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_moves_every_value_once_all_writes_succeed() {
        let source: Vec<(String, String)> = vec![
            ("/p".into(), "A".into()),
            ("/p".into(), "B".into()),
            ("/q".into(), "C".into()),
        ];
        let mut destination: Vec<(String, String)> = Vec::new();
        let mut removed: Vec<(String, String)> = Vec::new();

        migrate_entries(
            &source,
            |p, k| {
                Ok(source
                    .iter()
                    .find(|(ap, ak)| ap == p && ak == k)
                    .map(|(_, v)| v.clone()))
            },
            |p, k, v| {
                destination.push((format!("{p}/{k}"), v.to_string()));
                Ok(())
            },
            |p, k| removed.push((p.to_string(), k.to_string())),
        )
        .unwrap();

        assert_eq!(destination.len(), 3, "every value reached the destination");
        assert_eq!(removed.len(), 3, "and every original was dropped");
    }

    #[test]
    fn migration_keeps_the_source_when_a_write_fails() {
        // A destination that fails on the second write must not delete anything
        // from the source: the settings still point at the old store, so a
        // stranded value would be unreachable.
        let source: Vec<(String, String)> = vec![
            ("/p".into(), "A".into()),
            ("/p".into(), "B".into()),
            ("/p".into(), "C".into()),
        ];
        let mut writes = 0usize;
        let mut removed: Vec<(String, String)> = Vec::new();

        let result = migrate_entries(
            &source,
            |p, k| {
                Ok(source
                    .iter()
                    .find(|(ap, ak)| ap == p && ak == k)
                    .map(|(_, v)| v.clone()))
            },
            |_, _, _| {
                writes += 1;
                if writes == 2 {
                    Err(anyhow!("destination unavailable"))
                } else {
                    Ok(())
                }
            },
            |p, k| removed.push((p.to_string(), k.to_string())),
        );

        assert!(result.is_err(), "the failure is surfaced");
        assert!(removed.is_empty(), "nothing was deleted from the source");
    }

    #[test]
    fn migration_skips_values_the_source_does_not_have() {
        let mut writes: Vec<String> = Vec::new();
        let mut removed: Vec<String> = Vec::new();
        migrate_entries(
            &[("/p".into(), "MISSING".into())],
            |_, _| Ok(None),
            |_, k, _| {
                writes.push(k.to_string());
                Ok(())
            },
            |_, k| removed.push(k.to_string()),
        )
        .unwrap();
        assert!(writes.is_empty());
        assert!(removed.is_empty());
    }
}
