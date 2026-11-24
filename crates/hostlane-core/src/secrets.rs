//! Passwords and key passphrases, kept in the system keychain (macOS Keychain, the
//! Secret Service on Linux, Windows Credential Manager) under the service `Hostlane`.
//!
//! Only the daemon reads them, to authenticate. No method returns a secret; the
//! `secrets` table records which ones exist so lists do not touch the keychain.
//! `HOSTLANE_SECRET_STORE=file` keeps them in `<data dir>/secrets.json` (0600) instead,
//! for tests and machines without a keychain.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use hostlane_protocol::{ErrorKind, RpcError, SecretKind};
use zeroize::Zeroizing;

pub const STORE_ENV: &str = "HOSTLANE_SECRET_STORE";
const SERVICE: &str = "Hostlane";

pub type Secret = Zeroizing<String>;

#[derive(Debug)]
enum Backend {
    Keychain,
    File(PathBuf),
}

#[derive(Debug)]
pub struct Secrets {
    backend: Backend,
    /// Serializes file reads and writes.
    file_lock: Mutex<()>,
}

/// Keychain account of a host's password or a key file's passphrase.
pub fn account(kind: SecretKind, subject: &str) -> String {
    match kind {
        SecretKind::Password => format!("password/{subject}"),
        SecretKind::Passphrase => format!("passphrase/{subject}"),
    }
}

/// How a key file is named in the keychain: its absolute, canonical path.
pub fn key_path_id(path: &std::path::Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn store_error(err: impl std::fmt::Display) -> RpcError {
    RpcError::new(
        ErrorKind::Internal,
        format!("cannot use the system keychain: {err}"),
    )
    .with_hint(format!(
        "on Linux, Hostlane needs a Secret Service (GNOME Keyring or KWallet) that is unlocked; without one, use ssh-agent, or set {STORE_ENV}=file to keep secrets in a plain file"
    ))
}

impl Secrets {
    /// The keychain, or the file in `data_dir` when `HOSTLANE_SECRET_STORE=file`.
    pub fn from_env(data_dir: &std::path::Path) -> Self {
        let backend = match std::env::var(STORE_ENV).as_deref() {
            Ok("file") => Backend::File(data_dir.join("secrets.json")),
            _ => Backend::Keychain,
        };
        Self {
            backend,
            file_lock: Mutex::new(()),
        }
    }

    #[cfg(test)]
    pub fn file(path: PathBuf) -> Self {
        Self {
            backend: Backend::File(path),
            file_lock: Mutex::new(()),
        }
    }

    pub fn store_name(&self) -> &'static str {
        match self.backend {
            Backend::Keychain => "keychain",
            Backend::File(_) => "file",
        }
    }

    pub async fn get(&self, account: &str) -> Result<Option<Secret>, RpcError> {
        match &self.backend {
            Backend::Keychain => {
                let account = account.to_string();
                tokio::task::spawn_blocking(move || {
                    let entry = keyring::Entry::new(SERVICE, &account).map_err(store_error)?;
                    match entry.get_password() {
                        Ok(secret) => Ok(Some(Zeroizing::new(secret))),
                        Err(keyring::Error::NoEntry) => Ok(None),
                        Err(err) => Err(store_error(err)),
                    }
                })
                .await
                .map_err(store_error)?
            }
            Backend::File(path) => {
                let _guard = self.file_lock.lock().unwrap_or_else(|e| e.into_inner());
                Ok(read_file(path)?.remove(account).map(Zeroizing::new))
            }
        }
    }

    pub async fn set(&self, account: &str, secret: Secret) -> Result<(), RpcError> {
        match &self.backend {
            Backend::Keychain => {
                let account = account.to_string();
                tokio::task::spawn_blocking(move || {
                    keyring::Entry::new(SERVICE, &account)
                        .and_then(|entry| entry.set_password(&secret))
                        .map_err(store_error)
                })
                .await
                .map_err(store_error)?
            }
            Backend::File(path) => {
                let _guard = self.file_lock.lock().unwrap_or_else(|e| e.into_inner());
                let mut map = read_file(path)?;
                map.insert(account.to_string(), secret.to_string());
                write_file(path, &map)
            }
        }
    }

    /// Removes a secret; returns whether it existed.
    pub async fn delete(&self, account: &str) -> Result<bool, RpcError> {
        match &self.backend {
            Backend::Keychain => {
                let account = account.to_string();
                tokio::task::spawn_blocking(move || {
                    let entry = keyring::Entry::new(SERVICE, &account).map_err(store_error)?;
                    match entry.delete_credential() {
                        Ok(()) => Ok(true),
                        Err(keyring::Error::NoEntry) => Ok(false),
                        Err(err) => Err(store_error(err)),
                    }
                })
                .await
                .map_err(store_error)?
            }
            Backend::File(path) => {
                let _guard = self.file_lock.lock().unwrap_or_else(|e| e.into_inner());
                let mut map = read_file(path)?;
                let existed = map.remove(account).is_some();
                write_file(path, &map)?;
                Ok(existed)
            }
        }
    }
}

fn read_file(path: &PathBuf) -> Result<BTreeMap<String, String>, RpcError> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&Zeroizing::new(bytes)).map_err(store_error),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(err) => Err(store_error(err)),
    }
}

fn write_file(path: &PathBuf, map: &BTreeMap<String, String>) -> Result<(), RpcError> {
    let bytes = Zeroizing::new(serde_json::to_vec_pretty(map).map_err(store_error)?);
    let tmp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&tmp).map_err(store_error)?;
    file.write_all(&bytes).map_err(store_error)?;
    file.sync_all().map_err(store_error)?;
    std::fs::rename(&tmp, path).map_err(store_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn file_store_round_trip() {
        let dir = std::env::temp_dir().join(format!("hl-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secrets.json");
        let secrets = Secrets::file(path.clone());
        let account = account(SecretKind::Password, "web");
        assert_eq!(account, "password/web");
        assert!(secrets.get(&account).await.unwrap().is_none());
        secrets
            .set(&account, Zeroizing::new("hunter2".into()))
            .await
            .unwrap();
        assert_eq!(
            secrets.get(&account).await.unwrap().unwrap().as_str(),
            "hunter2"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(secrets.delete(&account).await.unwrap());
        assert!(!secrets.delete(&account).await.unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }
}
