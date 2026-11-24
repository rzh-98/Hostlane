use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use hostlane_core::client::{Client, ClientError};
use hostlane_core::{Daemon, paths, server};
use hostlane_protocol::{DaemonStatus, HostsList, method};
use serde_json::{Value, json};

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

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show whether the daemon is running.
    Status,
    /// List known hosts (from ~/.ssh/config for now).
    Hosts,
    /// Send a raw JSON-RPC call to the daemon.
    Call {
        method: String,
        /// Params as a JSON value, e.g. '{"host":"web1"}'.
        params: Option<String>,
    },
    /// Manage the background daemon.
    #[command(subcommand)]
    Daemon(DaemonCommand),
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start tokio runtime");
    match runtime.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            report_error(&err, json);
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let socket = cli.socket.clone().unwrap_or_else(paths::socket_path);
    let client = Client::new(&socket);
    match cli.command {
        Command::Status => status(&client, cli.json).await,
        Command::Hosts => hosts(&client, cli.json).await,
        Command::Call { method, params } => {
            let params = match params {
                Some(text) => serde_json::from_str(&text).context("params must be valid JSON")?,
                None => Value::Null,
            };
            print_json(&client.call(&method, params).await?);
            Ok(())
        }
        Command::Daemon(DaemonCommand::Run) => daemon_run(socket).await,
        Command::Daemon(DaemonCommand::Start) => daemon_start(&client, cli.socket, cli.json).await,
        Command::Daemon(DaemonCommand::Stop) => {
            client.call(method::DAEMON_SHUTDOWN, Value::Null).await?;
            output(
                cli.json,
                json!({ "stopped": true }),
                "Hostlane daemon stopped.",
            );
            Ok(())
        }
        Command::Daemon(DaemonCommand::SocketPath) => {
            output(
                cli.json,
                json!({ "socket_path": socket }),
                &socket.display().to_string(),
            );
            Ok(())
        }
    }
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
        println!("No hosts found. Add entries to ~/.ssh/config.");
        return Ok(());
    }
    let rows: Vec<[String; 5]> = list
        .hosts
        .iter()
        .map(|h| {
            [
                h.alias.clone(),
                h.hostname.clone(),
                h.user.clone().unwrap_or_else(|| "-".into()),
                h.port.to_string(),
                h.proxy_jump.clone().unwrap_or_else(|| "-".into()),
            ]
        })
        .collect();
    print_table(["HOST", "HOSTNAME", "USER", "PORT", "JUMP"], &rows);
    Ok(())
}

async fn daemon_run(socket: PathBuf) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let daemon = Arc::new(Daemon::new(socket));
    server::serve(daemon).await?;
    Ok(())
}

async fn daemon_start(client: &Client, socket: Option<PathBuf>, json: bool) -> Result<()> {
    if let Ok(status) = client
        .call_as::<DaemonStatus>(method::DAEMON_STATUS, Value::Null)
        .await
    {
        output(
            json,
            json!({ "started": false, "pid": status.pid }),
            &format!("Hostlane daemon is already running (pid {}).", status.pid),
        );
        return Ok(());
    }

    let log_path = paths::log_path();
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
            output(
                json,
                json!({ "started": true, "pid": status.pid }),
                &format!("Hostlane daemon started (pid {}).", status.pid),
            );
            return Ok(());
        }
    }
    bail!(
        "daemon did not come up within 5s; see {}",
        log_path.display()
    )
}

fn output(json: bool, value: Value, text: &str) {
    if json {
        print_json(&value);
    } else {
        println!("{text}");
    }
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
    let (kind, hint) = match err.downcast_ref::<ClientError>() {
        Some(ClientError::Rpc(rpc)) => (
            serde_json::to_value(rpc.kind()).unwrap_or(Value::Null),
            rpc.data.hint.clone(),
        ),
        Some(ClientError::NotRunning(_)) => (
            json!("DAEMON_NOT_RUNNING"),
            Some("run `hostlane daemon start`".to_string()),
        ),
        _ => (json!("ERROR"), None),
    };
    if json {
        print_json(&json!({
            "error": { "kind": kind, "message": err.to_string(), "hint": hint }
        }));
    } else {
        eprintln!("error: {err:#}");
        if let Some(hint) = hint {
            eprintln!("hint: {hint}");
        }
    }
}
