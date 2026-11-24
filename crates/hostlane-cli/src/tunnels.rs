//! `tunnels` and `tunnel open|close|start|update|delete`.

use anyhow::{Result, bail};
use clap::Subcommand;
use hostlane_core::client::Client;
use hostlane_protocol::{
    Tunnel, TunnelCloseParams, TunnelOpenParams, TunnelOpened, TunnelParams, TunnelState,
    TunnelUpdateParams, TunnelsClosed, TunnelsList, method,
};

use crate::{print_json, print_table};

#[derive(Subcommand)]
pub enum TunnelCommand {
    /// List tunnels with their state, local address and traffic.
    Tunnels,
    /// Open, close and manage local port forwards (like `ssh -L`).
    #[command(subcommand)]
    Tunnel(TunnelAction),
}

#[derive(Subcommand)]
pub enum TunnelAction {
    /// Forward a local port to a port on (or reachable from) the host.
    ///
    /// Unsaved tunnels opened by agents close after --ttl seconds (default 3600) or
    /// when `tunnel close --session` runs.
    Open {
        host: String,
        remote_port: u16,
        /// Local port (default: the remote port if free, else a nearby free port).
        #[arg(long)]
        local_port: Option<u16>,
        /// Address to connect to from the server (default 127.0.0.1).
        #[arg(long)]
        remote_host: Option<String>,
        /// Local address to listen on (default 127.0.0.1).
        #[arg(long)]
        bind: Option<String>,
        /// Required with a non-loopback --bind; other machines can then use the tunnel.
        #[arg(long)]
        allow_public: bool,
        #[arg(long)]
        name: Option<String>,
        /// Keep the tunnel and restore it when the daemon starts.
        #[arg(long)]
        save: bool,
        /// With --save: do not start it with the daemon.
        #[arg(long)]
        no_auto_start: bool,
        /// Seconds before an unsaved tunnel closes (max 86400).
        #[arg(long, value_name = "SECONDS")]
        ttl: Option<u64>,
    },
    /// Close a tunnel, or every temporary tunnel of a session (default: $HOSTLANE_SESSION).
    ///
    /// Saved tunnels are stopped and kept; use `delete` to remove them.
    Close {
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        session: Option<String>,
    },
    /// Start a stopped or failed tunnel.
    Start { id: String },
    /// Change a tunnel; a running tunnel moves to its new local port.
    Update {
        id: String,
        /// An empty name clears it.
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        local_port: Option<u16>,
        #[arg(long)]
        remote_host: Option<String>,
        #[arg(long)]
        remote_port: Option<u16>,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        allow_public: bool,
        /// Save a temporary tunnel (it no longer expires).
        #[arg(long, conflicts_with = "unsave")]
        save: bool,
        /// Stop saving it; it is closed when the daemon exits.
        #[arg(long)]
        unsave: bool,
        #[arg(long)]
        auto_start: Option<bool>,
    },
    /// Stop a tunnel and forget it, including its saved configuration.
    Delete { id: String },
}

fn state(tunnel: &Tunnel) -> &'static str {
    match tunnel.state {
        TunnelState::Active => "active",
        TunnelState::Reconnecting => "reconnecting",
        TunnelState::Failed => "failed",
        TunnelState::Stopped => "stopped",
    }
}

fn bytes(count: u64) -> String {
    match count {
        0..1024 => format!("{count} B"),
        1024..1_048_576 => format!("{:.1} KiB", count as f64 / 1024.0),
        1_048_576..1_073_741_824 => format!("{:.1} MiB", count as f64 / 1_048_576.0),
        _ => format!("{:.1} GiB", count as f64 / 1_073_741_824.0),
    }
}

/// "in 12m", "in 3h", or "" when there is no expiry.
fn expiry(tunnel: &Tunnel) -> String {
    let Some(at) = tunnel.expires_at else {
        return if tunnel.saved {
            "saved".into()
        } else {
            String::new()
        };
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let secs = at.saturating_sub(now) / 1000;
    match secs {
        0..60 => format!("closes in {secs}s"),
        60..3600 => format!("closes in {}m", secs / 60),
        _ => format!("closes in {}h", secs / 3600),
    }
}

fn local(tunnel: &Tunnel) -> String {
    if tunnel.bind == "127.0.0.1" {
        tunnel.local_port.to_string()
    } else {
        format!("{}:{}", tunnel.bind, tunnel.local_port)
    }
}

fn print_tunnels(list: &TunnelsList) {
    let rows: Vec<[String; 8]> = list
        .tunnels
        .iter()
        .map(|t| {
            [
                t.id.clone(),
                t.name.clone().unwrap_or_default(),
                state(t).into(),
                local(t),
                format!("{}:{}:{}", t.host, t.remote_host, t.remote_port),
                format!("{}/{}", t.active_connections, t.total_connections),
                format!("↓{} ↑{}", bytes(t.bytes_in), bytes(t.bytes_out)),
                expiry(t),
            ]
        })
        .collect();
    print_table(
        [
            "ID", "NAME", "STATE", "LOCAL", "REMOTE", "CONNS", "TRAFFIC", "LIFETIME",
        ],
        &rows,
    );
}

fn print_opened(opened: &TunnelOpened, verb: &str) {
    let t = &opened.tunnel;
    println!(
        "{verb} {} → {}:{}:{} ({})",
        t.url,
        t.host,
        t.remote_host,
        t.remote_port,
        state(t)
    );
    let lifetime = expiry(t);
    if !lifetime.is_empty() {
        println!("id {} · {lifetime}", t.id);
    } else {
        println!("id {}", t.id);
    }
    if let Some(error) = &t.error {
        eprintln!("error: {}", error.message);
    }
    for warning in &opened.warnings {
        eprintln!("warning: {warning}");
    }
}

pub async fn run(client: &Client, command: TunnelCommand, json: bool) -> Result<()> {
    let action = match command {
        TunnelCommand::Tunnels => {
            let list: TunnelsList = client
                .call_as(method::TUNNELS_LIST, serde_json::Value::Null)
                .await?;
            if json {
                print_json(&list);
            } else if list.tunnels.is_empty() {
                println!("No tunnels. Open one with `hostlane tunnel open <host> <port>`.");
            } else {
                print_tunnels(&list);
            }
            return Ok(());
        }
        TunnelCommand::Tunnel(action) => action,
    };
    match action {
        TunnelAction::Open {
            host,
            remote_port,
            local_port,
            remote_host,
            bind,
            allow_public,
            name,
            save,
            no_auto_start,
            ttl,
        } => {
            let params = TunnelOpenParams {
                host,
                remote_port,
                remote_host,
                local_port,
                bind,
                allow_public,
                name,
                save,
                auto_start: save.then_some(!no_auto_start),
                ttl_secs: ttl,
            };
            let opened: TunnelOpened = client
                .call_as(method::TUNNELS_OPEN, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&opened);
            } else {
                print_opened(&opened, "forwarding");
            }
        }
        TunnelAction::Close { id, session } => {
            let session = session.or_else(|| {
                std::env::var("HOSTLANE_SESSION")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            });
            if id.is_none() && session.is_none() {
                bail!("give a tunnel id, or --session (or set HOSTLANE_SESSION)");
            }
            let params = TunnelCloseParams { id, session };
            let closed: TunnelsClosed = client
                .call_as(method::TUNNELS_CLOSE, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&closed);
            } else if closed.closed.is_empty() {
                println!("no tunnels to close");
            } else {
                println!("closed {}", closed.closed.join(", "));
            }
        }
        TunnelAction::Start { id } => {
            let opened: TunnelOpened = client
                .call_as(
                    method::TUNNELS_START,
                    serde_json::to_value(TunnelParams { id })?,
                )
                .await?;
            if json {
                print_json(&opened);
            } else {
                print_opened(&opened, "forwarding");
            }
        }
        TunnelAction::Update {
            id,
            name,
            local_port,
            remote_host,
            remote_port,
            bind,
            allow_public,
            save,
            unsave,
            auto_start,
        } => {
            let params = TunnelUpdateParams {
                id,
                name,
                local_port,
                remote_host,
                remote_port,
                bind,
                allow_public,
                save: if save {
                    Some(true)
                } else if unsave {
                    Some(false)
                } else {
                    None
                },
                auto_start,
            };
            let opened: TunnelOpened = client
                .call_as(method::TUNNELS_UPDATE, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&opened);
            } else {
                print_opened(&opened, "updated");
            }
        }
        TunnelAction::Delete { id } => {
            let closed: TunnelsClosed = client
                .call_as(
                    method::TUNNELS_DELETE,
                    serde_json::to_value(TunnelParams { id })?,
                )
                .await?;
            if json {
                print_json(&closed);
            } else {
                println!("deleted {}", closed.closed.join(", "));
            }
        }
    }
    Ok(())
}
