//! `ports`, `port`, `port-check`, `free-port`, `port-note`.

use anyhow::Result;
use clap::Subcommand;
use hostlane_core::client::Client;
use hostlane_protocol::{
    BindScope, FindFreeParams, FreePorts, ListeningPort, PortAnnotateParams, PortAnnotated,
    PortCheck, PortParams, PortsList, PortsListParams, ServiceKind, method,
};

use crate::{ago, print_json, print_table};

#[derive(Subcommand)]
pub enum PortCommand {
    /// List listening TCP ports with their process, service, HTTP answer and note.
    Ports {
        host: String,
        /// Skip the HTTP/HTTPS probe (faster).
        #[arg(long)]
        no_probe: bool,
    },
    /// Show one port in detail, probing it over HTTP/HTTPS.
    Port {
        host: String,
        port: u16,
        /// Host header and TLS SNI, e.g. the site's domain.
        #[arg(long)]
        server_name: Option<String>,
    },
    /// Check whether a port is in use; suggests the nearest free one if so.
    PortCheck { host: String, port: u16 },
    /// Find free ports. Ports with a note count as reserved.
    FreePort {
        host: String,
        /// First port to consider (default 8000).
        #[arg(long)]
        from: Option<u16>,
        /// Last port to consider (default 9999).
        #[arg(long)]
        to: Option<u16>,
        /// How many ports to return (default 1, at most 100).
        #[arg(long)]
        count: Option<u32>,
    },
    /// Name a port and its project, or clear the note by giving no fields.
    PortNote {
        host: String,
        port: u16,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
}

fn scope(bind: Option<BindScope>) -> &'static str {
    match bind {
        Some(BindScope::Loopback) => "loopback",
        Some(BindScope::Private) => "private",
        Some(BindScope::Public) => "public",
        Some(BindScope::All) => "all",
        None => "-",
    }
}

fn process(port: &ListeningPort) -> String {
    match &port.process {
        Some(process) => format!("{} ({})", process.name, process.pid),
        None if port.listening => "?".into(),
        None => "-".into(),
    }
}

fn service(port: &ListeningPort) -> String {
    let Some(service) = &port.service else {
        return String::new();
    };
    match service.kind {
        ServiceKind::Systemd if service.user_unit => format!("{} (user)", service.name),
        ServiceKind::Systemd => service.name.clone(),
        ServiceKind::Docker => match service.container_port {
            Some(inner) => format!("docker {} :{inner}", service.name),
            None => format!("docker {}", service.name),
        },
    }
}

fn answer(port: &ListeningPort) -> String {
    if let Some(http) = &port.http {
        let mut parts = vec![format!("{} {}", http.scheme, http.status)];
        if let Some(location) = &http.location {
            parts.push(format!("-> {location}"));
        }
        if let Some(title) = &http.title {
            parts.push(title.chars().take(40).collect());
        }
        if let Some(days) = http.tls.as_ref().and_then(|t| t.days_left) {
            parts.push(if days < 0 {
                "cert expired".into()
            } else {
                format!("cert {days}d")
            });
        }
        return parts.join(" ");
    }
    if let Some(protocol) = &port.protocol {
        return protocol.clone();
    }
    port.probe_error.clone().unwrap_or_default()
}

fn note(port: &ListeningPort) -> String {
    let Some(note) = &port.note else {
        return String::new();
    };
    [note.name.as_deref(), note.project.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" / ")
}

fn print_ports(list: &PortsList) {
    let rows: Vec<[String; 6]> = list
        .ports
        .iter()
        .map(|port| {
            [
                port.port.to_string(),
                if port.listening {
                    scope(port.bind).to_string()
                } else {
                    "free".into()
                },
                process(port),
                service(port),
                answer(port),
                note(port),
            ]
        })
        .collect();
    print_table(
        ["PORT", "BIND", "PROCESS", "SERVICE", "ANSWER", "NOTE"],
        &rows,
    );
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
}

fn print_detail(port: &ListeningPort) {
    let field = |name: &str, value: String| {
        if !value.is_empty() {
            println!("{name:<10} {value}");
        }
    };
    field("port", port.port.to_string());
    field(
        "state",
        if port.listening { "listening" } else { "free" }.into(),
    );
    field(
        "bind",
        format!("{} {}", scope(port.bind), port.addresses.join(" ")),
    );
    if let Some(process) = &port.process {
        field("process", format!("{} (pid {})", process.name, process.pid));
        field("user", process.user.clone().unwrap_or_default());
        field("command", process.command.clone().unwrap_or_default());
        field("started", process.started_at.map(ago).unwrap_or_default());
    }
    if port.pids.len() > 1 {
        let pids: Vec<String> = port.pids.iter().map(u32::to_string).collect();
        field("pids", pids.join(" "));
    }
    field("service", service(port));
    if let Some(service) = &port.service {
        field("image", service.image.clone().unwrap_or_default());
        let compose = [
            service.compose_project.as_deref(),
            service.compose_service.as_deref(),
        ];
        field(
            "compose",
            compose.into_iter().flatten().collect::<Vec<_>>().join("/"),
        );
    }
    field("protocol", port.protocol.clone().unwrap_or_default());
    if let Some(http) = &port.http {
        field(
            "http",
            format!("{} {} ({}ms)", http.scheme, http.status, http.latency_ms),
        );
        field("title", http.title.clone().unwrap_or_default());
        field("server", http.server.clone().unwrap_or_default());
        field("location", http.location.clone().unwrap_or_default());
        if let Some(tls) = &http.tls {
            field("tls", tls.version.clone().unwrap_or_default());
            field("cert", tls.subject.clone().unwrap_or_default());
            field("issuer", tls.issuer.clone().unwrap_or_default());
            field("names", tls.names.join(" "));
            if let Some(days) = tls.days_left {
                let mut text = format!("{days} days left");
                if tls.self_signed {
                    text.push_str(", self-signed");
                }
                field("expires", text);
            }
        }
    }
    field("error", port.probe_error.clone().unwrap_or_default());
    if let Some(note) = &port.note {
        field("name", note.name.clone().unwrap_or_default());
        field("project", note.project.clone().unwrap_or_default());
        field("note", note.note.clone().unwrap_or_default());
    }
}

pub async fn run(client: &Client, command: PortCommand, json: bool) -> Result<()> {
    match command {
        PortCommand::Ports { host, no_probe } => {
            let params = PortsListParams {
                host,
                probe: Some(!no_probe),
            };
            let list: PortsList = client
                .call_as(method::PORTS_LIST, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&list);
                return Ok(());
            }
            print_ports(&list);
            print_warnings(&list.warnings);
        }
        PortCommand::Port {
            host,
            port,
            server_name,
        } => {
            let params = PortParams {
                host,
                port,
                server_name,
            };
            let list: PortsList = client
                .call_as(method::PORTS_INSPECT, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&list);
                return Ok(());
            }
            for port in &list.ports {
                print_detail(port);
            }
            print_warnings(&list.warnings);
        }
        PortCommand::PortCheck { host, port } => {
            let params = PortParams {
                host,
                port,
                server_name: None,
            };
            let check: PortCheck = client
                .call_as(method::PORTS_CHECK, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&check);
                return Ok(());
            }
            match &check.listener {
                Some(listener) if check.in_use => {
                    let by = [process(listener), service(listener)]
                        .into_iter()
                        .filter(|s| !s.is_empty() && s != "?")
                        .collect::<Vec<_>>()
                        .join(", ");
                    let by = if by.is_empty() {
                        String::new()
                    } else {
                        format!(" by {by}")
                    };
                    println!("{} is in use{by}", check.port);
                }
                _ => println!("{} is free", check.port),
            }
            if let Some(suggestion) = check.suggestion {
                println!("nearest free port: {suggestion}");
            }
            print_warnings(&check.warnings);
        }
        PortCommand::FreePort {
            host,
            from,
            to,
            count,
        } => {
            let params = FindFreeParams {
                host,
                from,
                to,
                count,
            };
            let free: FreePorts = client
                .call_as(method::PORTS_FIND_FREE, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&free);
                return Ok(());
            }
            for port in &free.ports {
                println!("{port}");
            }
            print_warnings(&free.warnings);
        }
        PortCommand::PortNote {
            host,
            port,
            name,
            project,
            note,
        } => {
            let params = PortAnnotateParams {
                host,
                port,
                name,
                project,
                note,
            };
            let result: PortAnnotated = client
                .call_as(method::PORTS_ANNOTATE, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&result);
                return Ok(());
            }
            match result.note {
                Some(_) => println!("noted {}:{}", result.host, result.port),
                None => println!("cleared the note on {}:{}", result.host, result.port),
            }
        }
    }
    Ok(())
}
