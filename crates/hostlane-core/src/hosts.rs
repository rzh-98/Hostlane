//! Hosts added in Hostlane (`hosts.add`, `hosts.update`, `hosts.remove`) and the
//! passwords and passphrases saved for them (`secrets.*`).

use std::path::{Path, PathBuf};

use hostlane_protocol::{
    ErrorKind, Host, HostAddParams, HostRemoved, HostSource, RpcError, SavedSecret, SecretChanged,
    SecretDeleteParams, SecretKind, SecretSetParams, SecretsList,
};
use zeroize::Zeroizing;

use crate::Daemon;
use crate::catalog;
use crate::secrets::{self, key_path_id};
use crate::ssh;
use crate::ssh_config::{Entry, expand_tilde};
use crate::util::now_ms;

const MAX_SECRET_BYTES: usize = 4096;

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Checks and normalizes the params of `hosts.add` / `hosts.update`.
fn validate(mut host: HostAddParams) -> Result<HostAddParams, RpcError> {
    host.id = host.id.trim().to_string();
    host.hostname = host.hostname.trim().to_string();
    let id_ok = !host.id.is_empty()
        && host.id.len() <= 64
        && !host.id.starts_with('-')
        && host
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !id_ok {
        return Err(invalid(
            "`id` must be 1–64 letters, digits, `.`, `_` or `-`, not starting with `-`",
        ));
    }
    let word = |value: &str| !value.starts_with('-') && !value.chars().any(char::is_whitespace);
    if host.hostname.is_empty() || !word(&host.hostname) {
        return Err(invalid("`hostname` must be an address or DNS name"));
    }
    host.user = clean(host.user);
    if let Some(user) = &host.user
        && !word(user)
    {
        return Err(invalid("`user` must not contain spaces"));
    }
    if host.port == Some(0) {
        return Err(invalid("`port` must be 1–65535"));
    }
    host.proxy_jump = clean(host.proxy_jump);
    if let Some(jump) = &host.proxy_jump
        && !word(jump)
    {
        return Err(invalid("`proxy_jump` must not contain spaces"));
    }
    host.identity_file = clean(host.identity_file);
    if let Some(file) = &host.identity_file {
        let path = expand_tilde(file);
        if !path.is_absolute() {
            return Err(invalid(
                "`identity_file` must be an absolute path or start with ~/",
            ));
        }
        if !path.is_file() {
            return Err(invalid(format!("no key file at {}", path.display())));
        }
    }
    let mut tags: Vec<String> = Vec::new();
    for tag in host.tags.iter().map(|t| t.trim()) {
        if !tag.is_empty() && !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
    }
    host.tags = tags;
    Ok(host)
}

async fn find(daemon: &Daemon, id: &str) -> Result<Host, RpcError> {
    let loaded = daemon.load_hosts().await?;
    loaded
        .find(id)
        .map(|entry| entry.host.clone())
        .ok_or_else(|| catalog::not_found(id, &loaded))
}

fn require_manual(host: &Host) -> Result<(), RpcError> {
    if host.source == HostSource::Manual {
        Ok(())
    } else {
        Err(invalid(format!(
            "`{}` comes from ~/.ssh/config; edit the file to change it",
            host.id
        )))
    }
}

pub(crate) async fn add(daemon: &Daemon, params: HostAddParams) -> Result<Host, RpcError> {
    let host = validate(params)?;
    let loaded = daemon.load_hosts().await?;
    if loaded.find(&host.id).is_some() {
        return Err(invalid(format!(
            "a host named `{}` already exists",
            host.id
        )));
    }
    save(daemon, host).await
}

pub(crate) async fn update(daemon: &Daemon, params: HostAddParams) -> Result<Host, RpcError> {
    let host = validate(params)?;
    require_manual(&find(daemon, &host.id).await?)?;
    let saved = save(daemon, host).await?;
    daemon.pool.disconnect(&saved.id).await;
    daemon.reachability.invalidate(&saved.id).await;
    Ok(saved)
}

async fn save(daemon: &Daemon, host: HostAddParams) -> Result<Host, RpcError> {
    let id = host.id.clone();
    let (env, tags) = (host.env, host.tags.clone());
    daemon.store.save_manual_host(host).await?;
    daemon.store.set_host_labels(id.clone(), env, tags).await?;
    find(daemon, &id).await
}

pub(crate) async fn remove(daemon: &Daemon, id: &str) -> Result<HostRemoved, RpcError> {
    require_manual(&find(daemon, id).await?)?;
    let mut deleted_tunnels = Vec::new();
    for tunnel in daemon.tunnels.list().tunnels {
        if tunnel.host == id {
            deleted_tunnels.extend(daemon.tunnels.delete(&tunnel.id).await?.closed);
        }
    }
    daemon.pool.disconnect(id).await;
    daemon.reachability.set(daemon, id.into(), false).await?;
    for check in daemon
        .health
        .list()
        .await
        .into_iter()
        .filter(|check| check.host == id)
    {
        daemon
            .health
            .set(
                daemon,
                hostlane_protocol::HealthParams {
                    host: id.into(),
                    port: check.port,
                    interval_secs: 0,
                },
            )
            .await?;
    }
    daemon
        .store
        .set_setting(format!("hostnote:{id}"), None)
        .await?;
    daemon.store.delete_manual_host(id.to_string()).await?;
    let account = secrets::account(SecretKind::Password, id);
    let deleted_password = daemon.secrets.delete(&account).await.unwrap_or(false);
    daemon.store.forget_secret(account).await?;
    Ok(HostRemoved {
        host: id.to_string(),
        deleted_tunnels,
        deleted_password,
    })
}

pub(crate) async fn list_secrets(daemon: &Daemon) -> Result<SecretsList, RpcError> {
    Ok(SecretsList {
        secrets: daemon.store.secrets().await?,
        store: daemon.secrets.store_name().into(),
    })
}

fn is_encrypted(path: &Path) -> bool {
    matches!(
        russh::keys::load_secret_key(path, None),
        Err(russh::keys::Error::KeyIsEncrypted)
    )
}

/// The key file a passphrase is for: `key_path`, or the host's first encrypted key.
fn passphrase_key(
    daemon: &Daemon,
    entry: &Entry,
    key_path: Option<&str>,
) -> Result<PathBuf, RpcError> {
    if let Some(path) = key_path {
        let path = expand_tilde(path.trim());
        if !path.is_file() {
            return Err(invalid(format!("no key file at {}", path.display())));
        }
        return Ok(path);
    }
    ssh::identity_files(entry, &daemon.pool.locations().ssh_dir)
        .into_iter()
        .find(|path| is_encrypted(path))
        .ok_or_else(|| {
            invalid(format!(
                "`{}` has no passphrase-protected key file; pass key_path",
                entry.host.id
            ))
        })
}

pub(crate) async fn set_secret(
    daemon: &Daemon,
    params: SecretSetParams,
) -> Result<SecretChanged, RpcError> {
    let secret = Zeroizing::new(params.secret.clone());
    drop(params.secret);
    if secret.is_empty() || secret.len() > MAX_SECRET_BYTES {
        return Err(invalid("the secret must be 1–4096 bytes"));
    }
    let (entry, _) = daemon.resolve_host(&params.host).await?;
    let (account, saved) = match params.kind {
        SecretKind::Password => (
            secrets::account(SecretKind::Password, &entry.host.id),
            SavedSecret {
                kind: SecretKind::Password,
                host: Some(entry.host.id.clone()),
                key_path: None,
                saved_at: now_ms(),
            },
        ),
        SecretKind::Passphrase => {
            let path = passphrase_key(daemon, &entry, params.key_path.as_deref())?;
            if !is_encrypted(&path) {
                return Err(invalid(format!(
                    "{} is not protected by a passphrase",
                    path.display()
                )));
            }
            if russh::keys::load_secret_key(&path, Some(secret.as_str())).is_err() {
                return Err(invalid(format!("wrong passphrase for {}", path.display())));
            }
            let id = key_path_id(&path);
            (
                secrets::account(SecretKind::Passphrase, &id),
                SavedSecret {
                    kind: SecretKind::Passphrase,
                    host: None,
                    key_path: Some(id),
                    saved_at: now_ms(),
                },
            )
        }
    };
    let existed = daemon.secrets.get(&account).await?.is_some();
    daemon.secrets.set(&account, secret).await?;
    daemon.store.record_secret(account, saved.clone()).await?;
    Ok(SecretChanged {
        secret: saved,
        existed,
    })
}

pub(crate) async fn delete_secret(
    daemon: &Daemon,
    params: SecretDeleteParams,
) -> Result<SecretChanged, RpcError> {
    let saved = match params.kind {
        SecretKind::Password => SavedSecret {
            kind: SecretKind::Password,
            host: Some(params.host.clone()),
            key_path: None,
            saved_at: now_ms(),
        },
        SecretKind::Passphrase => {
            let key_path = match params.key_path.as_deref() {
                Some(path) => key_path_id(&expand_tilde(path.trim())),
                None => {
                    let (entry, _) = daemon.resolve_host(&params.host).await?;
                    let files: Vec<String> =
                        ssh::identity_files(&entry, &daemon.pool.locations().ssh_dir)
                            .iter()
                            .map(|path| key_path_id(path))
                            .collect();
                    daemon
                        .store
                        .secrets()
                        .await?
                        .into_iter()
                        .filter_map(|s| s.key_path)
                        .find(|path| files.contains(path))
                        .ok_or_else(|| {
                            invalid(format!(
                                "no passphrase is saved for the keys of `{}`",
                                params.host
                            ))
                        })?
                }
            };
            SavedSecret {
                kind: SecretKind::Passphrase,
                host: None,
                key_path: Some(key_path),
                saved_at: now_ms(),
            }
        }
    };
    let subject = saved
        .host
        .clone()
        .or_else(|| saved.key_path.clone())
        .unwrap_or_default();
    let account = secrets::account(saved.kind, &subject);
    let in_store = daemon.secrets.delete(&account).await?;
    let in_index = daemon.store.forget_secret(account).await?;
    Ok(SecretChanged {
        secret: saved,
        existed: in_store || in_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_hosts() {
        let ok = validate(HostAddParams {
            id: " web-1 ".into(),
            hostname: "203.0.113.10".into(),
            user: Some(" ".into()),
            tags: vec!["a".into(), " a ".into(), "".into(), "b".into()],
            ..HostAddParams::default()
        })
        .unwrap();
        assert_eq!(ok.id, "web-1");
        assert_eq!(ok.user, None);
        assert_eq!(ok.tags, vec!["a", "b"]);
        for (id, hostname) in [
            ("", "h"),
            ("a b", "h"),
            ("-x", "h"),
            ("ok", ""),
            ("ok", "-oProxyCommand=x"),
            ("ok", "a b"),
        ] {
            let params = HostAddParams {
                id: id.into(),
                hostname: hostname.into(),
                ..HostAddParams::default()
            };
            assert!(validate(params).is_err(), "{id:?} {hostname:?}");
        }
        let missing_key = HostAddParams {
            id: "k".into(),
            hostname: "h".into(),
            identity_file: Some("/nonexistent/key".into()),
            ..HostAddParams::default()
        };
        assert!(validate(missing_key).is_err());
        let zero_port = HostAddParams {
            id: "p".into(),
            hostname: "h".into(),
            port: Some(0),
            ..HostAddParams::default()
        };
        assert!(validate(zero_port).is_err());
    }
}
