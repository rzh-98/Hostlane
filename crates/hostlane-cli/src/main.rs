use std::collections::BTreeMap;
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use hostlane_core::client::{Client, ClientError};
use hostlane_core::{Daemon, DaemonOptions, paths, server};
use hostlane_protocol::{
    AuditList, AuditListParams, AuditOutcome, ClientInfo, Connection, ConnectionState,
    ConnectionsList, DaemonStatus, Decision, ErrorKind, Event, ExecList, ExecListParams,
    ExecParams, ExecRecord, ExecResult, ExecStatus, HostKeyInfo, HostsList, OutputChunk,
    OutputReadParams, OutputStream, PolicyCheck, TrustResult, method,
};
use serde_json::{Value, json};

mod clients;
mod files;
mod jobs;
mod manage;
mod mcp;
mod policy;
mod ports;
mod service;
mod tunnels;

/// Lets your own agent operate remote servers over SSH, safely, from this machine.
#[derive(Parser)]
#[command(name = "hostlane", version, about)]
struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,

    /// Daemon socket path (defaults to the per-user location; env: HOSTLANE_SOCKET).
    #[arg(long, global = true, value_name = "PATH")]
    socket: Option<PathBuf>,

    /// When a call needs approval, fail at once with APPROVAL_REQUIRED and a ticket_id
    /// instead of waiting.
    #[arg(long = "async", global = true)]
    asynchronous: bool,

    /// Repeat a call that needed approval, using its ticket_id.
    #[arg(long, global = true, value_name = "TICKET_ID")]
    ticket: Option<String>,

    /// Seconds to wait for an approval (default 120).
    #[arg(long, global = true, value_name = "SECONDS")]
    approval_timeout: Option<u64>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show whether the daemon is running.
    Status,
    /// List known hosts (~/.ssh/config and hosts added in Hostlane) and their connection state.
    Hosts,
    /// Run a command on a host and print its output.
    ///
    /// Without --json, stdout and stderr are printed as they are and hostlane exits
    /// with the remote exit code (124 on timeout). With --json, it exits 0 whenever
    /// the command ran; read `exit_code` from the result.
    Exec {
        host: String,
        /// The command, run by the remote login shell. Quote it or put it after `--`.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
        /// Remote working directory (`~/…` is relative to the home directory).
        #[arg(long)]
        cwd: Option<String>,
        /// Environment variable for the command, as NAME=VALUE. Repeatable.
        #[arg(long = "env", value_name = "NAME=VALUE")]
        env: Vec<String>,
        /// Seconds before the command is stopped (default 120).
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<f64>,
        /// Bytes kept per stream; longer output keeps its head and tail (default 16384).
        #[arg(long, value_name = "BYTES")]
        max_output: Option<u64>,
        /// Send this process's standard input to the command.
        #[arg(long)]
        stdin: bool,
    },
    /// Read stored output of a command, by the output_id from `exec`.
    Output {
        output_id: String,
        #[arg(long, value_enum, default_value = "stdout")]
        stream: StreamArg,
        /// Byte offset; negative counts from the end.
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        offset: i64,
        /// Bytes to read (default 65536, at most 1 MiB).
        #[arg(long)]
        limit: Option<u64>,
    },
    #[command(flatten)]
    Files(files::FileCommand),
    #[command(flatten)]
    Ports(ports::PortCommand),
    #[command(flatten)]
    Tunnels(tunnels::TunnelCommand),
    #[command(flatten)]
    Policy(policy::PolicyCommand),
    #[command(flatten)]
    Manage(manage::ManageCommand),
    #[command(flatten)]
    Jobs(jobs::JobsCommand),
    #[command(flatten)]
    Clients(clients::ClientsCommand),
    /// Open the pooled connection to a host and show its host key.
    Connect { host: String },
    /// Trust a host's key after you have verified its fingerprint.
    Trust {
        host: String,
        /// Expected `SHA256:…` fingerprint. Without it, hostlane asks interactively.
        #[arg(long)]
        fingerprint: Option<String>,
    },
    /// Close the pooled connection to a host.
    Disconnect { host: String },
    /// Show pooled connections.
    Connections,
    /// List recent commands run through Hostlane.
    History {
        #[arg(long)]
        host: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Show the audit log.
    Audit {
        #[arg(long)]
        host: Option<String>,
        /// Client name, e.g. `cli` or `desktop`.
        #[arg(long)]
        client: Option<String>,
        /// Only calls assessed at this risk or higher.
        #[arg(long, value_enum)]
        min_risk: Option<policy::RiskArg>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Follow commands and their output live, like the desktop session monitor.
    Watch {
        #[arg(long)]
        host: Option<String>,
    },
    /// Send a raw JSON-RPC call to the daemon.
    Call {
        method: String,
        /// Params as a JSON value, e.g. '{"host":"web1"}'.
        params: Option<String>,
    },
    /// Serve Hostlane's tools to an agent over MCP (stdio).
    ///
    /// Add it to your agent's MCP settings as the command `hostlane mcp`. Starts the
    /// daemon if it is not running.
    Mcp,
    /// Manage the background daemon.
    #[command(subcommand)]
    Daemon(DaemonCommand),
}

#[derive(Clone, Copy, ValueEnum)]
enum StreamArg {
    Stdout,
    Stderr,
}

impl From<StreamArg> for OutputStream {
    fn from(value: StreamArg) -> Self {
        match value {
            StreamArg::Stdout => OutputStream::Stdout,
            StreamArg::Stderr => OutputStream::Stderr,
        }
    }
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Run the daemon in the foreground.
    Run,
    /// Start the daemon in the background if it is not running.
    Start,
    /// Ask the running daemon to exit.
    Stop,
    /// Print the socket path clients use.
    SocketPath,
    /// Run the daemon as a per-user service that starts at login (launchd on macOS,
    /// systemd --user on Linux). Restarts a daemon that is already running.
    Install {
        /// Print the service file instead of installing it.
        #[arg(long)]
        print: bool,
    },
    /// Remove the per-user service and stop the daemon it runs.
    Uninstall,
    /// Show whether the per-user service is installed.
    Service,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start tokio runtime");
    match runtime.block_on(run(cli)) {
        Ok(code) => code,
        Err(err) => {
            report_error(&err, json);
            ExitCode::FAILURE
        }
    }
}

/// Identifies this process to the daemon: `HOSTLANE_CLIENT` (default `cli`) and `HOSTLANE_SESSION`.
fn client_info() -> ClientInfo {
    let non_empty = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    ClientInfo {
        name: non_empty("HOSTLANE_CLIENT").unwrap_or_else(|| "cli".into()),
        session: non_empty("HOSTLANE_SESSION"),
        verified: false,
    }
}

/// The agent's client token (`HOSTLANE_TOKEN`), if any.
pub(crate) fn client_token() -> Option<String> {
    std::env::var("HOSTLANE_TOKEN").ok()
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let socket = cli.socket.clone().unwrap_or_else(paths::socket_path);
    let mut approval = serde_json::Map::new();
    if let Some(ticket) = &cli.ticket {
        approval.insert("ticket_id".into(), json!(ticket));
    }
    if cli.asynchronous {
        approval.insert("async".into(), json!(true));
    }
    if let Some(secs) = cli.approval_timeout {
        approval.insert("approval_timeout_secs".into(), json!(secs));
    }
    let info = client_info();
    let client = Client::new(&socket)
        .with_token(client_token())
        .with_approval(approval);
    let interactive = info.name == "cli"
        && !client.has_token()
        && std::io::stdin().is_terminal()
        && std::io::stderr().is_terminal();
    let waits = !cli.asynchronous && !interactive;
    let mut client = client.with_info(info);
    if interactive {
        client = client.with_approver(Arc::new(policy::ask));
    }
    let json = cli.json;
    match cli.command {
        Command::Status => status(&client, json).await?,
        Command::Hosts => hosts(&client, json).await?,
        Command::Exec {
            host,
            command,
            cwd,
            env,
            timeout,
            max_output,
            stdin,
        } => {
            let params = ExecParams {
                host,
                command: command.join(" "),
                cwd,
                env: parse_env(&env)?,
                stdin: if stdin { Some(read_stdin()?) } else { None },
                timeout_ms: timeout.map(|secs| (secs * 1000.0).round().max(1.0) as u64),
                max_output_bytes: max_output,
            };
            return exec(&client, params, json, waits && cli.ticket.is_none()).await;
        }
        Command::Output {
            output_id,
            stream,
            offset,
            limit,
        } => output(&client, output_id, stream.into(), offset, limit, json).await?,
        Command::Files(command) => files::run(&client, command, json).await?,
        Command::Ports(command) => ports::run(&client, command, json).await?,
        Command::Tunnels(command) => tunnels::run(&client, command, json).await?,
        Command::Policy(command) => policy::run(&client, command, json).await?,
        Command::Manage(command) => return manage::run(&client, command, json).await,
        Command::Jobs(command) => return jobs::run(&client, command, json).await,
        Command::Clients(command) => clients::run(&client, command, json).await?,
        Command::Connect { host } => connect(&client, &host, json).await?,
        Command::Trust { host, fingerprint } => trust(&client, &host, fingerprint, json).await?,
        Command::Disconnect { host } => {
            let reply = client
                .call(method::HOSTS_DISCONNECT, json!({ "host": host }))
                .await?;
            let text = if reply["disconnected"] == true {
                format!("Disconnected from {host}.")
            } else {
                format!("{host} was not connected.")
            };
            output_value(json, reply, &text);
        }
        Command::Connections => connections(&client, json).await?,
        Command::History { host, limit } => history(&client, host, limit, json).await?,
        Command::Audit {
            host,
            client: client_name,
            min_risk,
            limit,
        } => {
            audit(
                &client,
                host,
                client_name,
                min_risk.map(Into::into),
                limit,
                json,
            )
            .await?
        }
        Command::Watch { host } => watch(&client, host, json).await?,
        Command::Call { method, params } => {
            let params = match params {
                Some(text) => serde_json::from_str(&text).context("params must be valid JSON")?,
                None => Value::Null,
            };
            print_json(&client.call(&method, params).await?);
        }
        Command::Mcp => mcp::serve(socket, cli.socket).await?,
        Command::Daemon(DaemonCommand::Run) => daemon_run(socket).await?,
        Command::Daemon(DaemonCommand::Start) => daemon_start(&client, cli.socket, json).await?,
        Command::Daemon(DaemonCommand::Stop) => {
            client.call(method::DAEMON_SHUTDOWN, Value::Null).await?;
            output_value(json, json!({ "stopped": true }), "Hostlane daemon stopped.");
        }
        Command::Daemon(DaemonCommand::Install { print }) => {
            daemon_install(&client, cli.socket, print, json).await?
        }
        Command::Daemon(DaemonCommand::Uninstall) => {
            let removed = service::uninstall()?;
            let text = if removed {
                "Removed the Hostlane service; the daemon is stopped."
            } else {
                "No Hostlane service is installed."
            };
            output_value(json, json!({ "uninstalled": removed }), text);
        }
        Command::Daemon(DaemonCommand::Service) => {
            let status = service::status();
            let text = match (&status.path, &status.binary) {
                _ if !status.supported => {
                    "A background service is not supported on this system.".to_string()
                }
                (Some(path), Some(binary)) if status.installed => {
                    format!("Installed: {}\nRuns: {binary}", path.display())
                }
                (Some(path), _) if status.installed => format!("Installed: {}", path.display()),
                _ => "Not installed. Run `hostlane daemon install`.".to_string(),
            };
            output_value(json, serde_json::to_value(&status)?, &text);
        }
        Command::Daemon(DaemonCommand::SocketPath) => {
            output_value(
                json,
                json!({ "socket_path": socket }),
                &socket.display().to_string(),
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn parse_env(pairs: &[String]) -> Result<Option<BTreeMap<String, String>>> {
    if pairs.is_empty() {
        return Ok(None);
    }
    let mut env = BTreeMap::new();
    for pair in pairs {
        let (name, value) = pair
            .split_once('=')
            .ok_or_else(|| anyhow!("--env expects NAME=VALUE, got `{pair}`"))?;
        env.insert(name.to_string(), value.to_string());
    }
    Ok(Some(env))
}

fn read_stdin() -> Result<String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("cannot read standard input (it must be UTF-8 text)")?;
    Ok(input)
}

async fn status(client: &Client, json: bool) -> Result<()> {
    let status: DaemonStatus = client.call_as(method::DAEMON_STATUS, Value::Null).await?;
    if json {
        print_json(&status);
    } else {
        println!("Hostlane daemon {} is running", status.version);
        println!("  pid       {}", status.pid);
        println!("  uptime    {}s", status.uptime_ms / 1000);
        println!("  socket    {}", status.socket_path);
        println!("  data dir  {}", status.data_dir);
    }
    Ok(())
}

fn state_label(connection: Option<&Connection>) -> String {
    let Some(connection) = connection else {
        return "-".into();
    };
    match connection.state {
        ConnectionState::Connected => match connection.latency_ms {
            Some(ms) => format!("connected {ms}ms"),
            None => "connected".into(),
        },
        ConnectionState::Connecting => "connecting".into(),
        ConnectionState::Disconnected => "-".into(),
        ConnectionState::Failed => connection
            .error
            .as_ref()
            .map(|e| kind_name(e.kind()).to_lowercase())
            .unwrap_or_else(|| "failed".into()),
    }
}

async fn hosts(client: &Client, json: bool) -> Result<()> {
    let list: HostsList = client.call_as(method::HOSTS_LIST, Value::Null).await?;
    if json {
        print_json(&list);
        return Ok(());
    }
    for warning in &list.warnings {
        eprintln!("warning: {warning}");
    }
    if list.hosts.is_empty() {
        println!("No hosts found. Add entries to ~/.ssh/config or run `hostlane host add`.");
        return Ok(());
    }
    let connections: ConnectionsList = client
        .call_as(method::CONNECTIONS_LIST, Value::Null)
        .await
        .unwrap_or(ConnectionsList {
            connections: Vec::new(),
        });
    let rows: Vec<[String; 6]> = list
        .hosts
        .iter()
        .map(|h| {
            let connection = connections.connections.iter().find(|c| c.host == h.id);
            [
                h.alias.clone(),
                h.hostname.clone(),
                h.user.clone().unwrap_or_else(|| "-".into()),
                h.port.to_string(),
                h.proxy_jump.clone().unwrap_or_else(|| "-".into()),
                state_label(connection),
            ]
        })
        .collect();
    print_table(
        ["HOST", "HOSTNAME", "USER", "PORT", "JUMP", "STATUS"],
        &rows,
    );
    Ok(())
}

async fn exec(client: &Client, params: ExecParams, json: bool, waits: bool) -> Result<ExitCode> {
    let timeout_ms = params.timeout_ms;
    if waits && !json {
        let check = json!({ "method": method::EXEC_RUN, "params": &params });
        if let Ok(check) = client
            .call_as::<PolicyCheck>(method::POLICY_CHECK, check)
            .await
            && check.decision == Decision::Approve
        {
            eprintln!(
                "hostlane: {} needs approval ({}); waiting for an answer in the Hostlane app…",
                policy::label(check.risk),
                check
                    .reasons
                    .first()
                    .map(String::as_str)
                    .unwrap_or("policy rule")
            );
        }
    }
    let result: ExecResult = client
        .call_as(method::EXEC_RUN, serde_json::to_value(&params)?)
        .await?;
    if json {
        print_untrusted(method::EXEC_RUN, &result);
        return Ok(ExitCode::SUCCESS);
    }
    let mut stdout = std::io::stdout();
    stdout.write_all(result.stdout.as_bytes())?;
    stdout.flush()?;
    eprint!("{}", result.stderr);
    if result.truncated {
        eprintln!(
            "hostlane: output was shortened; read all of it with `hostlane output {}` (add --stream stderr for stderr)",
            result.output_id
        );
    }
    let code = match result.status {
        ExecStatus::Cancelled => {
            eprintln!("hostlane: the command was terminated by the emergency stop");
            130
        }
        ExecStatus::TimedOut => {
            let secs = timeout_ms.unwrap_or(120_000) as f64 / 1000.0;
            eprintln!("hostlane: the command timed out after {secs}s and was stopped");
            124
        }
        _ => match (result.exit_code, &result.signal) {
            (Some(code), _) => code.clamp(0, 255) as u8,
            (None, Some(signal)) => {
                eprintln!("hostlane: the command was killed by SIG{signal}");
                255
            }
            (None, None) => 255,
        },
    };
    Ok(ExitCode::from(code))
}

async fn output(
    client: &Client,
    output_id: String,
    stream: OutputStream,
    offset: i64,
    limit: Option<u64>,
    json: bool,
) -> Result<()> {
    let params = OutputReadParams {
        output_id: output_id.clone(),
        stream,
        offset: Some(offset),
        limit,
    };
    let chunk: OutputChunk = client
        .call_as(method::OUTPUT_READ, serde_json::to_value(&params)?)
        .await?;
    if json {
        print_untrusted(method::OUTPUT_READ, &chunk);
        return Ok(());
    }
    let mut stdout = std::io::stdout();
    stdout.write_all(chunk.data.as_bytes())?;
    stdout.flush()?;
    if chunk.next_offset < chunk.total_bytes {
        eprintln!(
            "hostlane: {} more bytes; continue with `hostlane output {output_id} --offset {}`",
            chunk.total_bytes - chunk.next_offset,
            chunk.next_offset
        );
    }
    if chunk.capped {
        eprintln!("hostlane: the stream was longer than the storage cap; the rest was not kept");
    }
    Ok(())
}

fn describe_key(key: &HostKeyInfo) -> String {
    format!("{} {}", key.key_type, key.fingerprint)
}

async fn connect(client: &Client, host: &str, json: bool) -> Result<()> {
    let connection: Connection = client
        .call_as(method::HOSTS_CONNECT, json!({ "host": host }))
        .await?;
    if json {
        print_json(&connection);
        return Ok(());
    }
    println!("Connected to {host}.");
    if let Some(key) = &connection.host_key {
        println!("  host key  {}", describe_key(key));
    }
    if let Some(ms) = connection.latency_ms {
        println!("  latency   {ms} ms");
    }
    Ok(())
}

/// The host key the server presents now, read from a `HOST_KEY_UNKNOWN` error.
async fn presented_key(client: &Client, host: &str) -> Result<Option<HostKeyInfo>> {
    match client
        .call(method::HOSTS_CONNECT, json!({ "host": host }))
        .await
    {
        Ok(_) => Ok(None),
        Err(ClientError::Rpc(err)) if err.kind() == ErrorKind::HostKeyUnknown => Ok(err
            .data
            .details
            .and_then(|details| serde_json::from_value(details).ok())),
        Err(err) => Err(err.into()),
    }
}

async fn trust(client: &Client, host: &str, fingerprint: Option<String>, json: bool) -> Result<()> {
    let fingerprint = match fingerprint {
        Some(fingerprint) => fingerprint,
        None => {
            if json || !std::io::stdin().is_terminal() {
                bail!(
                    "pass --fingerprint SHA256:… (get it from the HOST_KEY_UNKNOWN error and verify it with the server's owner)"
                );
            }
            let Some(key) = presented_key(client, host).await? else {
                println!("{host} is already trusted.");
                return Ok(());
            };
            println!("{host} presents this host key:");
            println!("  {}", describe_key(&key));
            println!(
                "Compare it with the server's own fingerprint, e.g. `ssh-keygen -lf /etc/ssh/ssh_host_*_key.pub` on the server."
            );
            print!("Trust this key? [y/N] ");
            std::io::stdout().flush()?;
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
                bail!("not trusted");
            }
            key.fingerprint
        }
    };
    let result: TrustResult = client
        .call_as(
            method::HOSTS_TRUST,
            json!({ "host": host, "fingerprint": fingerprint }),
        )
        .await?;
    if json {
        print_json(&result);
    } else if result.added {
        println!(
            "Trusted {host} ({}); recorded in {}.",
            describe_key(&result.host_key),
            result.known_hosts_path
        );
    } else {
        println!(
            "{host} was already trusted ({}).",
            describe_key(&result.host_key)
        );
    }
    Ok(())
}

async fn connections(client: &Client, json: bool) -> Result<()> {
    let list: ConnectionsList = client
        .call_as(method::CONNECTIONS_LIST, Value::Null)
        .await?;
    if json {
        print_json(&list);
        return Ok(());
    }
    if list.connections.is_empty() {
        println!("No connections yet.");
        return Ok(());
    }
    let rows: Vec<[String; 5]> = list
        .connections
        .iter()
        .map(|c| {
            [
                c.host.clone(),
                state_label(Some(c)),
                c.active_channels.to_string(),
                c.last_used_at.map(ago).unwrap_or_else(|| "-".into()),
                c.host_key
                    .as_ref()
                    .map(|k| k.fingerprint.clone())
                    .unwrap_or_else(|| "-".into()),
            ]
        })
        .collect();
    print_table(["HOST", "STATUS", "CHANNELS", "USED", "HOST KEY"], &rows);
    Ok(())
}

fn exec_outcome(record: &ExecRecord) -> String {
    match record.status {
        ExecStatus::Running => "running".into(),
        ExecStatus::Exited => match (record.exit_code, &record.signal) {
            (Some(code), _) => format!("exit {code}"),
            (None, Some(signal)) => format!("SIG{signal}"),
            _ => "exited".into(),
        },
        ExecStatus::TimedOut => "timed out".into(),
        ExecStatus::Failed => "failed".into(),
        ExecStatus::Interrupted => "interrupted".into(),
        ExecStatus::Cancelled => "cancelled".into(),
    }
}

pub(crate) fn one_line(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max || text.lines().nth(1).is_some() {
        out.push('…');
    }
    out
}

async fn history(client: &Client, host: Option<String>, limit: u32, json: bool) -> Result<()> {
    let params = ExecListParams {
        host,
        before: None,
        limit: Some(limit),
    };
    let list: ExecList = client
        .call_as(method::EXEC_LIST, serde_json::to_value(&params)?)
        .await?;
    if json {
        print_json(&list);
        return Ok(());
    }
    if list.execs.is_empty() {
        println!("No commands yet.");
        return Ok(());
    }
    let rows: Vec<[String; 6]> = list
        .execs
        .iter()
        .map(|e| {
            [
                ago(e.started_at),
                e.host.clone(),
                e.client.name.clone(),
                exec_outcome(e),
                e.id.clone(),
                one_line(&e.command, 60),
            ]
        })
        .collect();
    print_table(
        ["WHEN", "HOST", "CLIENT", "RESULT", "OUTPUT ID", "COMMAND"],
        &rows,
    );
    Ok(())
}

async fn audit(
    client: &Client,
    host: Option<String>,
    client_name: Option<String>,
    min_risk: Option<hostlane_protocol::Risk>,
    limit: u32,
    json: bool,
) -> Result<()> {
    let params = AuditListParams {
        host,
        client: client_name,
        min_risk,
        limit: Some(limit),
        ..Default::default()
    };
    let list: AuditList = client
        .call_as(method::AUDIT_LIST, serde_json::to_value(&params)?)
        .await?;
    if json {
        print_json(&list);
        return Ok(());
    }
    if list.entries.is_empty() {
        println!("The audit log is empty.");
        return Ok(());
    }
    let rows: Vec<[String; 7]> = list
        .entries
        .iter()
        .map(|e| {
            let outcome = match e.outcome {
                AuditOutcome::Ok => "ok".to_string(),
                AuditOutcome::Interrupted => "interrupted".to_string(),
                AuditOutcome::Error => e
                    .error_kind
                    .map(|k| kind_name(k).to_lowercase())
                    .unwrap_or_else(|| "error".into()),
            };
            let detail = ["command", "path"]
                .iter()
                .find_map(|key| e.params.get(*key).and_then(Value::as_str))
                .map(|text| one_line(text, 60))
                .or_else(|| {
                    let port = e.params.get("port").and_then(Value::as_u64)?;
                    Some(format!("port {port}"))
                })
                .or_else(|| {
                    let port = e.params.get("remote_port").and_then(Value::as_u64)?;
                    Some(format!("remote port {port}"))
                })
                .or_else(|| {
                    e.params
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            let outcome = match &e.approver {
                Some(approver) => format!("{outcome} ({approver})"),
                None => outcome,
            };
            [
                ago(e.started_at),
                e.client.name.clone(),
                e.host.clone().unwrap_or_else(|| "-".into()),
                e.method.clone(),
                e.risk.map(policy::label).unwrap_or_else(|| "-".into()),
                outcome,
                detail,
            ]
        })
        .collect();
    print_table(
        [
            "WHEN", "CLIENT", "HOST", "METHOD", "RISK", "OUTCOME", "DETAIL",
        ],
        &rows,
    );
    if list.has_more {
        eprintln!("(more entries; raise --limit to see them)");
    }
    Ok(())
}

async fn watch(client: &Client, host: Option<String>, json: bool) -> Result<()> {
    let mut subscription = client.subscribe().await?;
    if !json {
        eprintln!("Watching commands; press Ctrl-C to stop.");
    }
    let mut hosts_by_exec: BTreeMap<String, String> = BTreeMap::new();
    while let Some(event) = subscription.next().await? {
        let event_host = match &event {
            Event::ExecStarted { exec } | Event::ExecFinished { exec } => Some(exec.host.clone()),
            Event::ExecOutput { id, .. } => hosts_by_exec.get(id).cloned(),
            Event::Connection { connection } => Some(connection.host.clone()),
            Event::FileChanged { change } => Some(change.host.clone()),
            Event::AuditAppended { entry } => entry.host.clone(),
            Event::Tunnel { tunnel } => Some(tunnel.host.clone()),
            Event::ApprovalRequested { approval } | Event::ApprovalUpdated { approval } => {
                approval.host.clone()
            }
            Event::Job { job } => Some(job.host.clone()),
            Event::TunnelRemoved { .. } | Event::Lagged { .. } | Event::Policy { .. } => None,
        };
        if let (Some(filter), Some(event_host)) = (&host, &event_host)
            && filter != event_host
        {
            continue;
        }
        if json {
            println!("{}", serde_json::to_string(&event)?);
            continue;
        }
        match event {
            Event::ExecStarted { exec } => {
                hosts_by_exec.insert(exec.id.clone(), exec.host.clone());
                let cwd = exec.cwd.map(|c| format!(" (in {c})")).unwrap_or_default();
                println!(
                    "[{}] {} $ {}{cwd}",
                    exec.host, exec.client.name, exec.command
                );
            }
            Event::ExecOutput { stream, data, .. } => match stream {
                OutputStream::Stdout => {
                    print!("{data}");
                    std::io::stdout().flush()?;
                }
                OutputStream::Stderr => eprint!("{data}"),
            },
            Event::ExecFinished { exec } => {
                hosts_by_exec.remove(&exec.id);
                let duration = exec
                    .finished_at
                    .map(|end| format!(" in {} ms", end.saturating_sub(exec.started_at)))
                    .unwrap_or_default();
                println!("[{}] {}{duration}", exec.host, exec_outcome(&exec));
            }
            Event::ApprovalRequested { approval } => {
                println!(
                    "[{}] {} needs approval ({}): {} · {}",
                    approval.host.as_deref().unwrap_or("-"),
                    approval.client.name,
                    policy::label(approval.risk),
                    approval.summary,
                    approval.id
                );
            }
            Event::Policy { status } if status.stopped => {
                println!("(emergency stop is on)");
            }
            Event::Connection { .. }
            | Event::AuditAppended { .. }
            | Event::Tunnel { .. }
            | Event::TunnelRemoved { .. }
            | Event::ApprovalUpdated { .. }
            | Event::Policy { .. } => {}
            Event::FileChanged { change } => {
                let what = if change.deleted {
                    "removed".to_string()
                } else {
                    format!("+{} -{}", change.added_lines, change.removed_lines)
                };
                println!(
                    "[{}] {} {:?} {} ({what}) · {}",
                    change.host, change.client.name, change.operation, change.path, change.id
                );
            }
            Event::Job { job } if job.state != hostlane_protocol::JobState::Running => {
                println!(
                    "[{}] job {} ended: {:?}{}",
                    job.host,
                    job.id,
                    job.state,
                    job.exit_code
                        .map(|c| format!(" (exit {c})"))
                        .unwrap_or_default()
                );
            }
            Event::Job { job } if job.checked_at == job.started_at => {
                println!(
                    "[{}] {} started job {}: {}",
                    job.host, job.client.name, job.id, job.command
                );
            }
            Event::Job { .. } => {}
            Event::Lagged { skipped } => eprintln!("(missed {skipped} events)"),
        }
    }
    Ok(())
}

async fn daemon_run(socket: PathBuf) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,russh=warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    let daemon = Arc::new(Daemon::open(DaemonOptions::from_env(socket))?);
    server::serve(daemon).await?;
    Ok(())
}

async fn daemon_start(client: &Client, socket: Option<PathBuf>, json: bool) -> Result<()> {
    let (started, status) = ensure_daemon(client, socket).await?;
    if started {
        output_value(
            json,
            json!({ "started": true, "pid": status.pid }),
            &format!("Hostlane daemon started (pid {}).", status.pid),
        );
    } else {
        output_value(
            json,
            json!({ "started": false, "pid": status.pid }),
            &format!("Hostlane daemon is already running (pid {}).", status.pid),
        );
    }
    Ok(())
}

async fn daemon_install(
    client: &Client,
    socket: Option<PathBuf>,
    print: bool,
    json: bool,
) -> Result<()> {
    let definition = service::Definition::current(socket.as_deref())?;
    if print {
        let content = if cfg!(target_os = "macos") {
            service::launchd_plist(&definition)
        } else {
            service::systemd_unit(&definition)
        };
        output_value(json, json!({ "content": content }), content.trim_end());
        return Ok(());
    }
    let installed = service::install(client, &definition).await?;
    let mut text = format!(
        "Installed the Hostlane service ({}); the daemon now starts at login.",
        installed.path.display()
    );
    if installed.restarted {
        text.push_str("\nThe daemon that was running was restarted under the service.");
    }
    output_value(
        json,
        json!({
            "installed": true,
            "path": installed.path,
            "binary": definition.binary,
            "restarted": installed.restarted,
        }),
        &text,
    );
    Ok(())
}

/// Starts the daemon in the background unless it is running; `true` if it was started.
pub(crate) async fn ensure_daemon(
    client: &Client,
    socket: Option<PathBuf>,
) -> Result<(bool, DaemonStatus)> {
    if let Ok(status) = client
        .call_as::<DaemonStatus>(method::DAEMON_STATUS, Value::Null)
        .await
    {
        return Ok((false, status));
    }
    let log_path = paths::log_path();
    if service::start().unwrap_or(false) {
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(status) = client
                .call_as::<DaemonStatus>(method::DAEMON_STATUS, Value::Null)
                .await
            {
                return Ok((true, status));
            }
        }
        bail!(
            "the Hostlane service did not come up within 5s; see {}",
            log_path.display()
        );
    }

    if let Some(dir) = log_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("cannot open {}", log_path.display()))?;

    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.args(["daemon", "run"]);
    if let Some(socket) = socket {
        command.arg("--socket").arg(socket);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().context("failed to spawn daemon")?;

    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Ok(status) = client
            .call_as::<DaemonStatus>(method::DAEMON_STATUS, Value::Null)
            .await
        {
            return Ok((true, status));
        }
    }
    bail!(
        "daemon did not come up within 5s; see {}",
        log_path.display()
    )
}

/// "12s ago", "5m ago", "3h ago", "2d ago".
pub(crate) fn ago(unix_ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let secs = now.saturating_sub(unix_ms) / 1000;
    match secs {
        0..60 => format!("{secs}s ago"),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

fn kind_name(kind: ErrorKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "ERROR".into())
}

pub(crate) fn output_value(json: bool, value: Value, text: &str) {
    if json {
        print_json(&value);
    } else {
        println!("{text}");
    }
}

/// Prints a result as JSON with the `untrusted` member listing its server-provided
/// fields (S6), which the typed result drops.
pub(crate) fn print_untrusted(method: &str, value: &impl serde::Serialize) {
    let mut value = serde_json::to_value(value).expect("serializable");
    let fields = hostlane_protocol::untrusted_fields(method);
    if let Value::Object(object) = &mut value
        && !fields.is_empty()
    {
        object.insert(hostlane_protocol::UNTRUSTED_MEMBER.into(), json!(fields));
    }
    print_json(&value);
}

fn print_json(value: &impl serde::Serialize) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serializable")
    );
}

fn print_table<const N: usize>(headers: [&str; N], rows: &[[String; N]]) {
    let mut widths = headers.map(|h| h.chars().count());
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        println!("{}", padded.join("  ").trim_end());
    };
    line(headers.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
}

/// Errors go to stderr as text, or to stdout as JSON so agents can parse them.
fn report_error(err: &anyhow::Error, json: bool) {
    let (kind, hint, details) = match err.downcast_ref::<ClientError>() {
        Some(ClientError::Rpc(rpc)) => (
            serde_json::to_value(rpc.kind()).unwrap_or(Value::Null),
            rpc.data.hint.clone(),
            rpc.data.details.clone(),
        ),
        Some(ClientError::NotRunning(_)) => (
            json!("DAEMON_NOT_RUNNING"),
            Some("run `hostlane daemon start`".to_string()),
            None,
        ),
        _ => (json!("ERROR"), None, None),
    };
    if json {
        let mut error = json!({ "kind": kind, "message": err.to_string(), "hint": hint });
        if let Some(details) = details {
            error["details"] = details;
        }
        print_json(&json!({ "error": error }));
    } else {
        eprintln!("error: {err:#}");
        if let Some(hint) = hint {
            eprintln!("hint: {hint}");
        }
    }
}
