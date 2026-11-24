//! Opening one authenticated SSH connection: TCP (or a jump channel), host key
//! verification against known_hosts, then authentication: keys in the SSH agent, key
//! files (passphrases from the keychain), then a saved password (`password` or
//! `keyboard-interactive`).

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hostlane_protocol::{AuthFailure, ErrorKind, HostKeyInfo, RpcError, SecretKind};
use russh::MethodKind;
use russh::client::{self, Handle};
use russh::client::{AuthResult, KeyboardInteractiveAuthResponse};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{Algorithm, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::known_hosts::{self, Check};
use crate::secrets::{self, Secrets};
use crate::ssh_config::{Entry, expand_tilde};

pub type SshHandle = Handle<ClientHandler>;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_KEYS: &[&str] = &["id_ed25519", "id_ecdsa", "id_rsa"];

/// Where to look for keys and known hosts.
#[derive(Debug, Clone)]
pub struct Locations {
    /// Directory of the ssh_config in use (normally `~/.ssh`): default keys and known_hosts.
    pub ssh_dir: PathBuf,
    /// Hostlane's own known_hosts, the only one it writes.
    pub hostlane_known_hosts: PathBuf,
    /// Passwords and key passphrases.
    pub secrets: Arc<Secrets>,
}

#[derive(Debug, Clone)]
pub enum Trust {
    /// Accept only keys already in a known_hosts file.
    Strict,
    /// Also accept an unknown key with exactly this fingerprint, and record it.
    Accept(String),
}

pub struct Established {
    pub handle: SshHandle,
    pub host_key: HostKeyInfo,
    /// Time to open the TCP connection (or jump channel).
    pub transport_ms: u64,
    /// The key was added to Hostlane's known_hosts.
    pub learned: bool,
}

type Seen = Arc<Mutex<Option<(HostKeyInfo, Check, PublicKey)>>>;

pub struct ClientHandler {
    host: String,
    name: String,
    files: Vec<PathBuf>,
    trust: Trust,
    seen: Seen,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => {
                PublicKey::new(cert.public_key().clone(), "")
            }
        };
        let check = known_hosts::check(&self.name, &key, &self.files);
        let info = HostKeyInfo {
            host: self.host.clone(),
            known_hosts_name: self.name.clone(),
            key_type: key.algorithm().as_str().to_string(),
            fingerprint: known_hosts::fingerprint(&key),
            recorded_at: match &check {
                Check::Changed { at } | Check::Revoked { at } => Some(at.clone()),
                _ => None,
            },
        };
        let accept = match (&check, &self.trust) {
            (Check::Known, _) => true,
            (Check::Unknown, Trust::Accept(expected)) => *expected == info.fingerprint,
            _ => false,
        };
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some((info, check, key));
        Ok(accept)
    }
}

/// known_hosts files consulted for `entry`, Hostlane's own last.
pub fn known_hosts_files(entry: &Entry, locations: &Locations) -> Vec<PathBuf> {
    let mut files = if entry.options.user_known_hosts_files.is_empty() {
        vec![
            locations.ssh_dir.join("known_hosts"),
            locations.ssh_dir.join("known_hosts2"),
        ]
    } else {
        entry.options.user_known_hosts_files.clone()
    };
    files.push(locations.hostlane_known_hosts.clone());
    files
}

/// Name the host key is looked up and recorded under (`HostKeyAlias` wins, without a port).
pub fn known_hosts_name(entry: &Entry) -> String {
    match &entry.options.host_key_alias {
        Some(alias) => alias.to_ascii_lowercase(),
        None => known_hosts::entry_name(&entry.host.hostname, entry.host.port),
    }
}

fn same_family(a: &Algorithm, b: &Algorithm) -> bool {
    matches!((a, b), (Algorithm::Rsa { .. }, Algorithm::Rsa { .. })) || a == b
}

fn client_config(recorded: &[Algorithm]) -> client::Config {
    let mut config = client::Config {
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    };
    if !recorded.is_empty() {
        let defaults = config.preferred.key.to_vec();
        let (mut keys, rest): (Vec<_>, Vec<_>) = defaults
            .into_iter()
            .partition(|alg| recorded.iter().any(|r| same_family(r, alg)));
        keys.extend(rest);
        config.preferred.key = Cow::Owned(keys);
    }
    config
}

fn unreachable(host: &str, err: impl std::fmt::Display) -> RpcError {
    RpcError::new(
        ErrorKind::HostUnreachable,
        format!("cannot connect to `{host}`: {err}"),
    )
    .with_hint("check that the host is up and HostName/Port in ~/.ssh/config are right")
}

fn host_key_error(info: HostKeyInfo, check: &Check, trust: &Trust) -> RpcError {
    let host = info.host.clone();
    match check {
        Check::Unknown => match trust {
            Trust::Accept(expected) => RpcError::new(
                ErrorKind::InvalidParams,
                format!(
                    "`{host}` presented {} {}, not {expected}",
                    info.key_type, info.fingerprint
                ),
            )
            .with_hint("the fingerprint must match the key the server presents; nothing was trusted"),
            Trust::Strict => RpcError::new(
                ErrorKind::HostKeyUnknown,
                format!(
                    "the host key of `{host}` is not trusted yet: {} {}",
                    info.key_type, info.fingerprint
                ),
            )
            .with_hint(format!(
                "confirm this fingerprint with the server's owner, then run `hostlane trust {host} --fingerprint {}`",
                info.fingerprint
            )),
        },
        Check::Changed { at } => RpcError::new(
            ErrorKind::HostKeyChanged,
            format!(
                "the host key of `{host}` has changed: the server presented {} {}, but {at} records a different key; someone may be intercepting the connection",
                info.key_type, info.fingerprint
            ),
        )
        .with_hint(format!(
            "do not connect unless you know the server's keys were replaced; if so, remove the old entry at {at} and connect again"
        )),
        Check::Revoked { at } => RpcError::new(
            ErrorKind::HostKeyChanged,
            format!(
                "the host key of `{host}` ({}) is marked @revoked at {at}",
                info.fingerprint
            ),
        )
        .with_hint("the server is using a revoked key; contact its owner"),
        Check::Known => unreachable(&host, "handshake failed"),
    }
    .with_details(info)
}

/// Connects directly, or over `stream` (a `direct-tcpip` channel through a jump host).
pub async fn connect<S>(
    entry: &Entry,
    locations: &Locations,
    trust: Trust,
    stream: Option<S>,
) -> Result<Established, RpcError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let host = entry.host.id.clone();
    let name = known_hosts_name(entry);
    let files = known_hosts_files(entry, locations);
    let config = Arc::new(client_config(&known_hosts::recorded_algorithms(
        &name, &files,
    )));
    let seen: Seen = Arc::default();
    let handler = ClientHandler {
        host: host.clone(),
        name: name.clone(),
        files,
        trust: trust.clone(),
        seen: seen.clone(),
    };
    let timeout = entry
        .options
        .connect_timeout_secs
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT);

    let attempt = async {
        let started = Instant::now();
        let handshake = match stream {
            Some(stream) => {
                let transport_ms = started.elapsed().as_millis() as u64;
                (
                    client::connect_stream(config, stream, handler).await,
                    transport_ms,
                )
            }
            None => {
                let tcp =
                    tokio::net::TcpStream::connect((entry.host.hostname.as_str(), entry.host.port))
                        .await
                        .map_err(|err| unreachable(&host, err))?;
                let transport_ms = started.elapsed().as_millis() as u64;
                let _ = tcp.set_nodelay(true);
                (
                    client::connect_stream(config, tcp, handler).await,
                    transport_ms,
                )
            }
        };
        let (result, transport_ms) = handshake;
        let seen = seen.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut handle = match result {
            Ok(handle) => handle,
            Err(err) => {
                return Err(match seen {
                    Some((info, check, _)) if !matches!(check, Check::Known) => {
                        host_key_error(info, &check, &trust)
                    }
                    _ => unreachable(&host, err),
                });
            }
        };
        let Some((host_key, check, key)) = seen else {
            return Err(unreachable(&host, "the server did not present a host key"));
        };
        let learned = matches!(check, Check::Unknown);
        if learned {
            known_hosts::append(&locations.hostlane_known_hosts, &name, &key).map_err(|err| {
                RpcError::new(
                    ErrorKind::Internal,
                    format!(
                        "cannot write {}: {err}",
                        locations.hostlane_known_hosts.display()
                    ),
                )
            })?;
        }
        authenticate(&mut handle, entry, locations).await?;
        Ok(Established {
            handle,
            host_key,
            transport_ms,
            learned,
        })
    };

    tokio::time::timeout(timeout, attempt)
        .await
        .map_err(|_| unreachable(&host, format!("no answer within {}s", timeout.as_secs())))?
}

pub(crate) fn local_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".into())
}

/// Key files tried for `entry`: its `IdentityFile`s, or the default keys in `ssh_dir`.
pub(crate) fn identity_files(entry: &Entry, ssh_dir: &Path) -> Vec<PathBuf> {
    if entry.host.identity_files.is_empty() {
        DEFAULT_KEYS
            .iter()
            .map(|name| ssh_dir.join(name))
            .filter(|path| path.exists())
            .collect()
    } else {
        entry
            .host
            .identity_files
            .iter()
            .map(PathBuf::from)
            .collect()
    }
}

#[cfg(unix)]
type AgentStream = tokio::net::UnixStream;
#[cfg(not(unix))]
type AgentStream = tokio::io::DuplexStream;

#[cfg(unix)]
async fn connect_agent(entry: &Entry) -> Option<AgentClient<AgentStream>> {
    let socket = match entry.options.identity_agent.as_deref() {
        Some(value) if value.eq_ignore_ascii_case("none") => return None,
        Some(value) if value != "SSH_AUTH_SOCK" => {
            let value = value
                .strip_prefix('$')
                .and_then(|var| std::env::var(var).ok())
                .unwrap_or_else(|| value.to_string());
            expand_tilde(&value)
        }
        _ => PathBuf::from(std::env::var_os("SSH_AUTH_SOCK")?),
    };
    AgentClient::connect_uds(socket).await.ok()
}

#[cfg(not(unix))]
async fn connect_agent(_entry: &Entry) -> Option<AgentClient<AgentStream>> {
    None
}

/// Remembers which methods the server still offers after a failed attempt.
fn note_failure(result: &AuthResult, methods: &mut Option<Vec<MethodKind>>) {
    if let AuthResult::Failure {
        remaining_methods, ..
    } = result
    {
        *methods = Some(remaining_methods.to_vec());
    }
}

async fn authenticate(
    handle: &mut SshHandle,
    entry: &Entry,
    locations: &Locations,
) -> Result<(), RpcError> {
    let user = entry.host.user.clone().unwrap_or_else(local_user);
    let files = identity_files(entry, &locations.ssh_dir);
    let file_keys: Vec<PublicKey> = files
        .iter()
        .filter_map(|path| {
            let mut pub_path = path.clone().into_os_string();
            pub_path.push(".pub");
            russh::keys::load_public_key(PathBuf::from(pub_path)).ok()
        })
        .collect();

    let rsa_hash = handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();
    let hash_for = |key: &PublicKey| match key.algorithm() {
        Algorithm::Rsa { .. } => rsa_hash,
        _ => None,
    };
    let mut methods: Option<Vec<MethodKind>> = None;

    let mut tried_agent = 0;
    let mut offered = Vec::new();
    if let Some(mut agent) = connect_agent(entry).await {
        let identities: Vec<PublicKey> = agent
            .request_identities()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|identity| match identity {
                AgentIdentity::PublicKey { key, .. } => Some(key),
                AgentIdentity::Certificate { .. } => None,
            })
            .collect();
        let (mut ordered, others): (Vec<_>, Vec<_>) = identities.into_iter().partition(|key| {
            file_keys
                .iter()
                .any(|file_key| file_key.key_data() == key.key_data())
        });
        if !entry.options.identities_only {
            ordered.extend(others);
        }
        for key in ordered {
            tried_agent += 1;
            offered.push(key.key_data().clone());
            let hash = hash_for(&key);
            match handle
                .authenticate_publickey_with(&user, key, hash, &mut agent)
                .await
            {
                Ok(result) if result.success() => return Ok(()),
                Ok(result) => note_failure(&result, &mut methods),
                Err(_) if handle.is_closed() => break,
                Err(_) => {}
            }
        }
    }

    let mut tried_files = 0;
    let mut encrypted = Vec::new();
    for path in &files {
        if handle.is_closed() {
            break;
        }
        let key = match load_key(path, &locations.secrets).await {
            Ok(Some(key)) => key,
            Ok(None) => continue,
            Err(()) => {
                encrypted.push(path.display().to_string());
                continue;
            }
        };
        if offered.contains(key.public_key().key_data()) {
            continue;
        }
        tried_files += 1;
        let hash = hash_for(key.public_key());
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
        match handle.authenticate_publickey(&user, key).await {
            Ok(result) if result.success() => return Ok(()),
            Ok(result) => note_failure(&result, &mut methods),
            Err(_) => {}
        }
    }

    let offers = |methods: &Option<Vec<MethodKind>>, kind: MethodKind| {
        methods.as_ref().is_none_or(|m| m.contains(&kind))
    };
    let mut password_saved = false;
    if !handle.is_closed()
        && (offers(&methods, MethodKind::Password)
            || offers(&methods, MethodKind::KeyboardInteractive))
    {
        let account = secrets::account(SecretKind::Password, &entry.host.id);
        match locations.secrets.get(&account).await {
            Ok(Some(password)) => {
                password_saved = true;
                if offers(&methods, MethodKind::Password) {
                    match handle.authenticate_password(&user, password.as_str()).await {
                        Ok(result) if result.success() => return Ok(()),
                        Ok(result) => note_failure(&result, &mut methods),
                        Err(_) => {}
                    }
                }
                if !handle.is_closed()
                    && offers(&methods, MethodKind::KeyboardInteractive)
                    && keyboard_interactive(handle, &user, &password, &mut methods).await
                {
                    return Ok(());
                }
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(host = %entry.host.id, %err, "cannot read the saved password")
            }
        }
    }

    let server_methods: Vec<String> = methods
        .iter()
        .flatten()
        .map(|m| <&str>::from(m).to_string())
        .collect();
    let accepts_password = server_methods
        .iter()
        .any(|m| m == "password" || m == "keyboard-interactive");
    let host = &entry.host.id;
    let target = format!("{user}@{host}");
    let hint = if !encrypted.is_empty() {
        format!(
            "{} need a passphrase: save it with `hostlane secret set {host} --passphrase` (or in the Hostlane app), or load the key into ssh-agent with `ssh-add`",
            encrypted.join(", ")
        )
    } else if password_saved && accepts_password {
        format!(
            "the server rejected the saved password; update it with `hostlane secret set {host} --password`"
        )
    } else if accepts_password && tried_agent + tried_files == 0 {
        format!(
            "no SSH keys were found, but the server accepts passwords: save one with `hostlane secret set {host} --password` (or in the Hostlane app), or add a key to ssh-agent"
        )
    } else if accepts_password {
        format!(
            "put one of these keys in ~/.ssh/authorized_keys of `{user}` on the server, or save a password with `hostlane secret set {host} --password`"
        )
    } else if tried_agent + tried_files == 0 {
        "no SSH keys found: add a key to ssh-agent (ssh-add) or set IdentityFile in ~/.ssh/config"
            .to_string()
    } else {
        format!(
            "make sure one of these keys is in ~/.ssh/authorized_keys of `{user}` on the server"
        )
    };
    Err(RpcError::new(
        ErrorKind::AuthFailed,
        format!(
            "authentication failed for {target}: the server rejected {tried_agent} agent key(s), {tried_files} key file(s){}",
            if password_saved { " and the saved password" } else { "" }
        ),
    )
    .with_hint(hint)
    .with_details(AuthFailure {
        host: host.clone(),
        user,
        server_methods,
        encrypted_keys: encrypted,
        tried_agent_keys: tried_agent,
        tried_key_files: tried_files,
        password_saved,
    }))
}

/// Loads a private key, decrypting it with the passphrase saved for it. `Ok(None)`: no
/// usable key at `path`; `Err(())`: the key is encrypted and no working passphrase is saved.
async fn load_key(path: &Path, secrets: &Secrets) -> Result<Option<russh::keys::PrivateKey>, ()> {
    match russh::keys::load_secret_key(path, None) {
        Ok(key) => Ok(Some(key)),
        Err(russh::keys::Error::KeyIsEncrypted) => {
            let account = secrets::account(SecretKind::Passphrase, &secrets::key_path_id(path));
            let passphrase = match secrets.get(&account).await {
                Ok(Some(passphrase)) => passphrase,
                Ok(None) => return Err(()),
                Err(err) => {
                    tracing::warn!(key = %path.display(), %err, "cannot read the saved passphrase");
                    return Err(());
                }
            };
            russh::keys::load_secret_key(path, Some(passphrase.as_str()))
                .map(Some)
                .map_err(|_| ())
        }
        Err(_) => Ok(None),
    }
}

/// Answers keyboard-interactive prompts that hide their input with the password.
async fn keyboard_interactive(
    handle: &mut SshHandle,
    user: &str,
    password: &str,
    methods: &mut Option<Vec<MethodKind>>,
) -> bool {
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await;
    for _ in 0..5 {
        match response {
            Ok(KeyboardInteractiveAuthResponse::Success) => return true,
            Ok(KeyboardInteractiveAuthResponse::Failure {
                remaining_methods, ..
            }) => {
                *methods = Some(remaining_methods.to_vec());
                return false;
            }
            Ok(KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. }) => {
                let answers = prompts
                    .iter()
                    .map(|prompt| {
                        if prompt.echo {
                            String::new()
                        } else {
                            password.to_string()
                        }
                    })
                    .collect();
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await;
            }
            Err(_) => return false,
        }
    }
    false
}
