//! `host add|update|remove`, `secret set|delete`, `secrets`, `logs`, `service` and `kill`.

use std::io::{IsTerminal, Read};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use hostlane_core::client::Client;
use hostlane_protocol::{
    ExecStatus, Host, HostAddParams, HostRemoved, HostsList, ProcessKillParams, SavedSecret,
    SecretChanged, SecretDeleteParams, SecretKind, SecretsList, ServiceCommandResult,
    ServiceControlParams, ServiceKind, ServiceLogsParams, method,
};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::policy::EnvArg;
use crate::{ago, print_json, print_table};

#[derive(Subcommand)]
pub enum ManageCommand {
    /// Add, change or remove hosts kept by Hostlane (outside ~/.ssh/config).
    #[command(subcommand)]
    Host(HostCommand),
    /// Save or delete a host's login password or a key's passphrase in the system
    /// keychain. The secret is read from the terminal or standard input, never from
    /// the command line.
    #[command(subcommand)]
    Secret(SecretCommand),
    /// List saved passwords and passphrases (never their values).
    Secrets,
    /// Show the last lines of a systemd unit's journal or a container's log.
    Logs {
        host: String,
        /// Unit name (`nginx`) or container name or id.
        name: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Number of lines (default 200, at most 10000).
        #[arg(short = 'n', long)]
        lines: Option<u32>,
        /// Only newer entries, e.g. 30s, 10m, 2h, 1d.
        #[arg(long)]
        since: Option<String>,
        /// Run with `sudo -n` (e.g. when the journal needs it).
        #[arg(long)]
        sudo: bool,
    },
    /// Start, stop or restart a systemd unit or a Docker container.
    Service {
        #[arg(value_enum)]
        action: ServiceAction,
        host: String,
        name: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Run with `sudo -n` (default for system units when the SSH user is not root).
        #[arg(long, conflicts_with = "no_sudo")]
        sudo: bool,
        /// Run without sudo.
        #[arg(long)]
        no_sudo: bool,
    },
    /// Send a signal to a process on a host.
    Kill {
        host: String,
        pid: u32,
        /// TERM (default), INT, HUP, KILL, QUIT, USR1 or USR2.
        #[arg(long, short = 's')]
        signal: Option<String>,
        /// Run with `sudo -n`.
        #[arg(long)]
        sudo: bool,
    },
}

#[derive(Args)]
pub struct TargetArgs {
    /// The name is a Docker container rather than a systemd unit.
    #[arg(long, conflicts_with = "user_unit")]
    docker: bool,
    /// A systemd user unit (`systemctl --user`).
    #[arg(long)]
    user_unit: bool,
}

impl TargetArgs {
    fn kind(&self) -> ServiceKind {
        if self.docker {
            ServiceKind::Docker
        } else {
            ServiceKind::Systemd
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

#[derive(Subcommand)]
pub enum HostCommand {
    /// Add a host without editing ~/.ssh/config.
    Add {
        /// Name used in other commands; letters, digits, `.`, `_` and `-`.
        id: String,
        /// Address or DNS name.
        hostname: String,
        #[command(flatten)]
        settings: HostArgs,
    },
    /// Change a host added in Hostlane. Options not given keep their value.
    Update {
        id: String,
        /// New address or DNS name.
        #[arg(long)]
        hostname: Option<String>,
        #[command(flatten)]
        settings: HostArgs,
        /// Stop using the key file set with --key.
        #[arg(long, conflicts_with = "key")]
        no_key: bool,
        /// Remove the jump host.
        #[arg(long, conflicts_with = "jump")]
        no_jump: bool,
    },
    /// Remove a host added in Hostlane, with its saved tunnels and password.
    Remove { id: String },
}

#[derive(Args)]
pub struct HostArgs {
    /// Login user (default: your local user name).
    #[arg(long)]
    user: Option<String>,
    /// SSH port (default 22).
    #[arg(long)]
    port: Option<u16>,
    /// Jump host, as in ProxyJump: a host id or user@host:port.
    #[arg(long)]
    jump: Option<String>,
    /// Private key file; only this key is offered.
    #[arg(long, value_name = "PATH")]
    key: Option<String>,
    /// Environment label (`none` removes it).
    #[arg(long, value_enum)]
    env: Option<EnvArg>,
    /// Tag. Repeatable; replaces the host's tags.
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[derive(Subcommand)]
pub enum SecretCommand {
    /// Save a password or passphrase (asked for on the terminal).
    Set {
        host: String,
        #[command(flatten)]
        kind: KindArgs,
        /// Key file of the passphrase (default: the host's first encrypted key).
        #[arg(long, value_name = "PATH")]
        key: Option<String>,
        /// Read the secret from standard input instead of the terminal.
        #[arg(long)]
        stdin: bool,
    },
    /// Delete a saved password or passphrase.
    Delete {
        host: String,
        #[command(flatten)]
        kind: KindArgs,
        #[arg(long, value_name = "PATH")]
        key: Option<String>,
    },
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct KindArgs {
    /// The host's login password.
    #[arg(long)]
    password: bool,
    /// The passphrase of the host's private key.
    #[arg(long)]
    passphrase: bool,
}

impl KindArgs {
    fn kind(&self) -> SecretKind {
        if self.passphrase {
            SecretKind::Passphrase
        } else {
            SecretKind::Password
        }
    }
}

fn describe(secret: &SavedSecret) -> String {
    match (&secret.host, &secret.key_path, secret.kind) {
        (_, Some(path), _) => format!("passphrase of {path}"),
        (Some(host), None, _) => format!("password of {host}"),
        (None, None, SecretKind::Password) => "password".into(),
        (None, None, SecretKind::Passphrase) => "passphrase".into(),
    }
}

fn read_secret(prompt: &str, from_stdin: bool) -> Result<Zeroizing<String>> {
    let secret = if from_stdin {
        let mut secret = Zeroizing::new(String::new());
        std::io::stdin()
            .read_to_string(&mut secret)
            .context("cannot read the secret from standard input")?;
        while secret.ends_with(['\n', '\r']) {
            secret.pop();
        }
        secret
    } else {
        if !std::io::stdin().is_terminal() {
            bail!("no terminal to ask for the secret; pass --stdin and pipe it in");
        }
        Zeroizing::new(rpassword::prompt_password(prompt)?)
    };
    if secret.is_empty() {
        bail!("the secret is empty");
    }
    Ok(secret)
}

fn print_host(host: &Host, verb: &str) {
    let user = host
        .user
        .as_deref()
        .map(|u| format!("{u}@"))
        .unwrap_or_default();
    println!("{verb} {}: {user}{}:{}", host.id, host.hostname, host.port);
}

pub async fn run(client: &Client, command: ManageCommand, json: bool) -> Result<ExitCode> {
    match command {
        ManageCommand::Host(command) => host(client, command, json).await?,
        ManageCommand::Secret(command) => secret(client, command, json).await?,
        ManageCommand::Secrets => {
            let list: SecretsList = client.call_as(method::SECRETS_LIST, Value::Null).await?;
            if json {
                print_json(&list);
            } else if list.secrets.is_empty() {
                println!("No saved passwords or passphrases (store: {}).", list.store);
            } else {
                let rows: Vec<[String; 2]> = list
                    .secrets
                    .iter()
                    .map(|s| [describe(s), ago(s.saved_at)])
                    .collect();
                print_table(["SECRET", "SAVED"], &rows);
                println!("Store: {}", list.store);
            }
        }
        ManageCommand::Logs {
            host,
            name,
            target,
            lines,
            since,
            sudo,
        } => {
            let params = ServiceLogsParams {
                host,
                kind: target.kind(),
                name,
                user_unit: target.user_unit,
                lines,
                since,
                sudo: sudo.then_some(true),
            };
            return command_result(client, method::SERVICES_LOGS, &params, json).await;
        }
        ManageCommand::Service {
            action,
            host,
            name,
            target,
            sudo,
            no_sudo,
        } => {
            let params = ServiceControlParams {
                host,
                kind: target.kind(),
                name,
                user_unit: target.user_unit,
                sudo: match (sudo, no_sudo) {
                    (true, _) => Some(true),
                    (_, true) => Some(false),
                    _ => None,
                },
            };
            let method = match action {
                ServiceAction::Start => method::SERVICES_START,
                ServiceAction::Stop => method::SERVICES_STOP,
                ServiceAction::Restart => method::SERVICES_RESTART,
            };
            return command_result(client, method, &params, json).await;
        }
        ManageCommand::Kill {
            host,
            pid,
            signal,
            sudo,
        } => {
            let params = ProcessKillParams {
                host,
                pid,
                signal,
                sudo: sudo.then_some(true),
            };
            return command_result(client, method::PROCESSES_KILL, &params, json).await;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Prints a `services.*` result like `exec`: output as is, exit code passed on.
async fn command_result(
    client: &Client,
    method: &str,
    params: &impl serde::Serialize,
    json: bool,
) -> Result<ExitCode> {
    let reply: ServiceCommandResult = client
        .call_as(method, serde_json::to_value(params)?)
        .await?;
    if json {
        crate::print_untrusted(method, &reply);
        return Ok(ExitCode::SUCCESS);
    }
    let result = &reply.result;
    print!("{}", result.stdout);
    eprint!("{}", result.stderr);
    if let Some(hint) = &reply.hint {
        eprintln!("hint: {hint}");
    }
    let code = match (result.status, result.exit_code) {
        (ExecStatus::TimedOut, _) => 124,
        (ExecStatus::Cancelled, _) => 130,
        (_, Some(code)) => code.clamp(0, 255) as u8,
        _ => 255,
    };
    if code != 0 {
        eprintln!("hostlane: `{}` exited with {code}", reply.command);
    }
    Ok(ExitCode::from(code))
}

async fn host(client: &Client, command: HostCommand, json: bool) -> Result<()> {
    match command {
        HostCommand::Add {
            id,
            hostname,
            settings,
        } => {
            let params = HostAddParams {
                id,
                hostname,
                user: settings.user,
                port: settings.port,
                proxy_jump: settings.jump,
                identity_file: settings.key,
                env: settings.env.and_then(EnvArg::env),
                tags: settings.tags,
            };
            let host: Host = client
                .call_as(method::HOSTS_ADD, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&host);
            } else {
                print_host(&host, "Added");
                println!(
                    "Run `hostlane connect {}` to check and trust its host key.",
                    host.id
                );
            }
        }
        HostCommand::Update {
            id,
            hostname,
            settings,
            no_key,
            no_jump,
        } => {
            let list: HostsList = client.call_as(method::HOSTS_LIST, Value::Null).await?;
            let Some(mut params) = list.manual.into_iter().find(|h| h.id == id) else {
                if list.hosts.iter().any(|h| h.id == id) {
                    bail!("`{id}` comes from ~/.ssh/config; edit the file to change it");
                }
                bail!("no host `{id}` was added in Hostlane");
            };
            if let Some(hostname) = hostname {
                params.hostname = hostname;
            }
            if settings.user.is_some() {
                params.user = settings.user;
            }
            if settings.port.is_some() {
                params.port = settings.port;
            }
            if no_jump {
                params.proxy_jump = None;
            } else if settings.jump.is_some() {
                params.proxy_jump = settings.jump;
            }
            if no_key {
                params.identity_file = None;
            } else if settings.key.is_some() {
                params.identity_file = settings.key;
            }
            if let Some(env) = settings.env {
                params.env = env.env();
            }
            if !settings.tags.is_empty() {
                params.tags = settings.tags;
            }
            let host: Host = client
                .call_as(method::HOSTS_UPDATE, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&host);
            } else {
                print_host(&host, "Updated");
            }
        }
        HostCommand::Remove { id } => {
            let removed: HostRemoved = client
                .call_as(method::HOSTS_REMOVE, json!({ "host": id }))
                .await?;
            if json {
                print_json(&removed);
            } else {
                println!("Removed {}.", removed.host);
                if !removed.deleted_tunnels.is_empty() {
                    println!("Deleted tunnels: {}", removed.deleted_tunnels.join(", "));
                }
                if removed.deleted_password {
                    println!("Deleted its saved password.");
                }
            }
        }
    }
    Ok(())
}

async fn secret(client: &Client, command: SecretCommand, json: bool) -> Result<()> {
    match command {
        SecretCommand::Set {
            host,
            kind,
            key,
            stdin,
        } => {
            let kind = kind.kind();
            let prompt = match (kind, &key) {
                (SecretKind::Password, _) => format!("Password for {host}: "),
                (SecretKind::Passphrase, Some(path)) => format!("Passphrase for {path}: "),
                (SecretKind::Passphrase, None) => format!("Passphrase for the key of {host}: "),
            };
            let secret = read_secret(&prompt, stdin)?;
            let mut params = json!({ "host": host, "kind": kind, "secret": secret.as_str() });
            if let Some(key) = key {
                params["key_path"] = json!(key);
            }
            let changed: SecretChanged = client.call_as(method::SECRETS_SET, params).await?;
            if json {
                print_json(&changed);
            } else {
                let verb = if changed.existed { "Replaced" } else { "Saved" };
                println!("{verb} the {}.", describe(&changed.secret));
            }
        }
        SecretCommand::Delete { host, kind, key } => {
            let params = SecretDeleteParams {
                host,
                kind: kind.kind(),
                key_path: key,
            };
            let changed: SecretChanged = client
                .call_as(method::SECRETS_DELETE, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&changed);
            } else if changed.existed {
                println!("Deleted the {}.", describe(&changed.secret));
            } else {
                println!("No {} was saved.", describe(&changed.secret));
            }
        }
    }
    Ok(())
}
