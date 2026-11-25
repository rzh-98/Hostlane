use anyhow::{Result, ensure};
use clap::Subcommand;
use hostlane_core::client::Client;
use hostlane_protocol::method;
use serde_json::{Value, json};
use std::io::Read;

#[derive(Subcommand)]
pub enum WebCommand {
    /// Show OS, CPU count, RAM, root disk and installed tools.
    Profile { host: String },
    /// Save a Markdown note from stdin (empty input clears it).
    Note { host: String },
    /// Write the host's context to stdout as Markdown.
    Export { host: String },
    /// Show static proxy routes and estimates of listener exposure.
    Report { host: String },
    /// Check HTTP(S) on server loopback every 10–86400 seconds; 0 removes the check.
    Monitor {
        host: String,
        port: u16,
        seconds: u32,
    },
    /// List configured checks and their latest results.
    Health,
    /// Temporarily forward a server loopback port (0 = automatic) to local loopback.
    Reverse {
        host: String,
        remote_port: u16,
        local_port: u16,
    },
    /// Close one reverse listener and its active streams.
    Unforward { host: String, remote_port: u16 },
    /// List active reverse forwards.
    Forwards,
}

pub async fn run(client: &Client, command: WebCommand, json_mode: bool) -> Result<()> {
    let (method, params) = match command {
        WebCommand::Profile { host } => (method::HOSTS_PROFILE, json!({"host":host})),
        WebCommand::Note { host } => {
            let mut content = String::new();
            std::io::stdin().take(16385).read_to_string(&mut content)?;
            ensure!(content.len() <= 16384, "host note exceeds 16 KiB");
            (method::HOSTS_NOTE, json!({"host":host,"content":content}))
        }
        WebCommand::Export { host } => (method::HOSTS_EXPORT, json!({"host":host})),
        WebCommand::Report { host } => (method::WEB_REPORT, json!({"host":host})),
        WebCommand::Monitor {
            host,
            port,
            seconds,
        } => (
            method::HEALTH_SET,
            json!({"host":host,"port":port,"interval_secs":seconds}),
        ),
        WebCommand::Health => (method::HEALTH_LIST, Value::Null),
        WebCommand::Reverse {
            host,
            remote_port,
            local_port,
        } => (
            method::REVERSE_OPEN,
            json!({"host":host,"remote_port":remote_port,"local_port":local_port}),
        ),
        WebCommand::Unforward { host, remote_port } => (
            method::REVERSE_CLOSE,
            json!({"host":host,"port":remote_port}),
        ),
        WebCommand::Forwards => (method::REVERSE_LIST, Value::Null),
    };
    let result = client.call(method, params).await?;
    if !json_mode && method == method::HOSTS_EXPORT {
        print!("{}", result["markdown"].as_str().unwrap_or_default());
    } else {
        crate::print_json(&result);
    }
    Ok(())
}
