use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SecretKind {
    /// Login password of a host (also used for keyboard-interactive prompts).
    Password,
    /// Passphrase of a private key file, shared by every host that uses the key.
    Passphrase,
}

/// Params of `secrets.set`. The secret goes to the system keychain; it is never
/// returned by any method or written to the audit log.
#[derive(Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretSetParams {
    pub host: String,
    pub kind: SecretKind,
    pub secret: String,
    /// For a passphrase: the key file. Defaults to the host's first key file that
    /// needs a passphrase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key_path: Option<String>,
}

impl std::fmt::Debug for SecretSetParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretSetParams")
            .field("host", &self.host)
            .field("kind", &self.kind)
            .field("secret", &"<redacted>")
            .field("key_path", &self.key_path)
            .finish()
    }
}

/// Params of `secrets.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretDeleteParams {
    pub host: String,
    pub kind: SecretKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key_path: Option<String>,
}

/// A secret Hostlane has saved (the secret itself stays in the keychain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SavedSecret {
    pub kind: SecretKind,
    /// Host of a password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
    /// Absolute path of the key file of a passphrase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key_path: Option<String>,
    #[ts(type = "number")]
    pub saved_at: u64,
}

/// Result of `secrets.list`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretsList {
    pub secrets: Vec<SavedSecret>,
    /// Where secrets are kept: `keychain` (macOS Keychain, Secret Service, Windows
    /// Credential Manager) or `file` (`HOSTLANE_SECRET_STORE=file`, for tests only).
    pub store: String,
}

/// Result of `secrets.set` and `secrets.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretChanged {
    pub secret: SavedSecret,
    /// `secrets.delete`: whether anything was there.
    #[serde(default)]
    pub existed: bool,
}
