//! `ports.*`: listening TCP ports on a server with their processes, systemd units or
//! Docker containers, what they answer over HTTP(S), and the user's notes. Everything
//! comes from standard tools over SSH; nothing is installed on the server.

mod cert;
mod discover;
pub(crate) mod probe;

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use hostlane_protocol::{
    ErrorKind, FindFreeParams, FreePorts, ListeningPort, PortAnnotateParams, PortAnnotated,
    PortCheck, PortNote, PortParams, PortsList, PortsListParams, RpcError, ServiceInfo,
    ServiceKind,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::Daemon;
use crate::remote;
use crate::ssh_config::{Entry, Loaded};
use crate::util::now_ms;
use discover::{CgroupOwner, Container, Published};

const LIST_TIMEOUT: Duration = Duration::from_secs(20);
const LIST_MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_PIDS: usize = 2000;
const PROBE_CONCURRENCY: usize = 8;
const MAX_PROBED: usize = 64;
const MAX_NOTE_FIELD: usize = 100;
const MAX_NOTE: usize = 2000;

fn remote_failed(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::RemoteCommandFailed, message)
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    line.trim().chars().take(max).collect()
}

/// Every port on a host, without probing.
struct Scan {
    host: String,
    tool: String,
    ports: Vec<ListeningPort>,
    warnings: Vec<String>,
}

impl Scan {
    fn port(&self, port: u16) -> Option<&ListeningPort> {
        self.ports.iter().find(|p| p.port == port)
    }

    /// Listening or reserved by a note.
    fn taken(&self) -> BTreeSet<u16> {
        self.ports.iter().map(|p| p.port).collect()
    }
}

async fn run_script(
    daemon: &Daemon,
    entry: &Entry,
    loaded: &Loaded,
    script: &str,
) -> Result<String, RpcError> {
    let (_lease, channel) = daemon.pool.open_channel(entry, loaded).await?;
    let out = remote::capture(channel, script, LIST_MAX_BYTES, None, LIST_TIMEOUT).await?;
    if out.timed_out {
        return Err(remote_failed(format!(
            "listing ports did not finish within {}s",
            LIST_TIMEOUT.as_secs()
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !stdout.lines().any(|line| line == "@@end") {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(remote_failed(format!(
            "listing ports failed: {}",
            first_line(&stderr, 300)
        ))
        .with_hint("the remote login shell must be POSIX-compatible (sh, bash, zsh)"));
    }
    Ok(stdout)
}

fn systemd_service(name: &str, user: bool) -> ServiceInfo {
    ServiceInfo {
        kind: ServiceKind::Systemd,
        name: name.to_string(),
        user_unit: user,
        container_id: None,
        image: None,
        container_port: None,
        compose_project: None,
        compose_service: None,
    }
}

async fn scan(daemon: &Daemon, entry: &Entry, loaded: &Loaded) -> Result<Scan, RpcError> {
    let host = entry.host.id.clone();
    let notes = daemon.store.port_notes(host.clone()).await?;
    let output = run_script(daemon, entry, loaded, discover::LIST_SCRIPT).await?;
    let mut warnings = Vec::new();
    let mut uid = None;
    let mut tool = String::new();
    let mut sockets = Vec::new();
    let mut lsof = None;
    let mut containers = Vec::new();
    for (header, body) in discover::sections(&output) {
        let (name, arg) = header.split_once(' ').unwrap_or((header.as_str(), ""));
        match name {
            "uid" => uid = arg.trim().parse::<u32>().ok(),
            "listen" => {
                tool = arg.to_string();
                sockets = match arg {
                    "ss" => discover::parse_ss(&body),
                    "netstat" => discover::parse_netstat_linux(&body),
                    "proc" => discover::parse_proc_net(&body),
                    _ => discover::parse_netstat_bsd(&body),
                };
            }
            "lsof" => lsof = Some(discover::parse_lsof(&body)),
            "docker" => match discover::parse_docker(&body) {
                Ok(list) => containers = list,
                Err(err) => warnings.push(format!("docker: {}", first_line(&err, 200))),
            },
            _ => {}
        }
    }
    if tool == "bsd" {
        tool = if lsof.is_some() {
            "netstat+lsof"
        } else {
            "netstat"
        }
        .into();
    }
    if let Some(lsof) = &lsof {
        for socket in sockets.iter_mut().filter(|s| s.owners.is_empty()) {
            socket.owners = lsof.get(&socket.port).cloned().unwrap_or_default();
        }
    }
    let mut groups = discover::group(sockets);

    let mut published: HashMap<u16, (usize, Published)> = HashMap::new();
    for (index, container) in containers.iter().enumerate() {
        for port in discover::parse_published(&container.ports) {
            published.entry(port.host_port).or_insert((index, port));
        }
    }
    for (port, (_, entry)) in &published {
        groups
            .entry(*port)
            .or_default()
            .addresses
            .insert(entry.address.clone());
    }

    let mut pids: Vec<u32> = groups
        .values()
        .flat_map(|g| g.owners.keys().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    pids.truncate(MAX_PIDS);
    let mut ps = HashMap::new();
    let mut cgroups: HashMap<u32, CgroupOwner> = HashMap::new();
    if !pids.is_empty() {
        match run_script(daemon, entry, loaded, &discover::process_script(&pids)).await {
            Ok(output) => {
                for (header, body) in discover::sections(&output) {
                    if header == "ps" {
                        ps = discover::parse_ps(&body);
                    } else if let Some(pid) = header.strip_prefix("cgroup ")
                        && let (Ok(pid), Some(owner)) =
                            (pid.trim().parse(), discover::parse_cgroup(&body))
                    {
                        cgroups.insert(pid, owner);
                    }
                }
            }
            Err(err) => warnings.push(format!("process details unavailable: {}", err.message)),
        }
    }

    let now = now_ms();
    let container_by_id = |id: &str| -> Option<&Container> {
        containers
            .iter()
            .find(|c| c.id.starts_with(id) || id.starts_with(&c.id))
    };
    let mut ports = Vec::new();
    for (port, group) in groups {
        let pids: Vec<u32> = group.owners.keys().copied().collect();
        let main = discover::main_pid(&pids, &ps);
        let process = main.map(|pid| {
            let name = group.owners.get(&pid).map(String::as_str).unwrap_or("");
            discover::process_info(pid, name, ps.get(&pid), now)
        });
        let cgroup = main
            .and_then(|pid| cgroups.get(&pid))
            .or_else(|| pids.iter().find_map(|pid| cgroups.get(pid)));
        let published_here = published.get(&port);
        let service = match cgroup {
            Some(CgroupOwner::Container(id)) => container_by_id(id)
                .map(|container| container.service(published_here.map(|(_, p)| p.container_port))),
            _ => None,
        }
        .or_else(|| {
            published_here.map(|(index, p)| containers[*index].service(Some(p.container_port)))
        })
        .or_else(|| match cgroup {
            Some(CgroupOwner::Unit { name, user }) => Some(systemd_service(name, *user)),
            _ => None,
        });
        ports.push(ListeningPort {
            port,
            listening: true,
            bind: discover::bind_scope(&group.addresses),
            addresses: group.addresses.into_iter().collect(),
            pids,
            process,
            service,
            protocol: None,
            http: None,
            probe_error: None,
            note: notes.get(&port).cloned(),
        });
    }
    for (port, note) in notes {
        if !ports.iter().any(|p| p.port == port) {
            ports.push(ListeningPort {
                port,
                listening: false,
                addresses: Vec::new(),
                bind: None,
                pids: Vec::new(),
                process: None,
                service: None,
                protocol: None,
                http: None,
                probe_error: None,
                note: Some(note),
            });
        }
    }
    ports.sort_by_key(|p| p.port);

    let hidden = ports
        .iter()
        .filter(|p| p.listening && p.pids.is_empty() && p.service.is_none())
        .count();
    if hidden > 0 && uid != Some(0) {
        warnings.push(format!(
            "{hidden} port(s) show no process: they belong to other users and the SSH user is not root"
        ));
    }
    Ok(Scan {
        host,
        tool,
        ports,
        warnings,
    })
}

/// Probes the listening ports in `ports` concurrently over one connection.
async fn probe_ports(
    daemon: &Daemon,
    entry: &Entry,
    loaded: &Loaded,
    ports: &mut [ListeningPort],
    server_name: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<(), RpcError> {
    let targets: Vec<usize> = (0..ports.len()).filter(|i| ports[*i].listening).collect();
    if targets.is_empty() {
        return Ok(());
    }
    if targets.len() > MAX_PROBED {
        warnings.push(format!(
            "only the first {MAX_PROBED} of {} ports were probed",
            targets.len()
        ));
    }
    let lease = daemon.pool.lease(entry, loaded).await?;
    let limit = Arc::new(Semaphore::new(PROBE_CONCURRENCY));
    let mut tasks = JoinSet::new();
    for index in targets.into_iter().take(MAX_PROBED) {
        let addresses: BTreeSet<String> = ports[index].addresses.iter().cloned().collect();
        let address = discover::probe_address(&addresses);
        let port = ports[index].port;
        let conn = lease.conn.clone();
        let limit = limit.clone();
        let server_name = server_name.map(str::to_owned);
        tasks.spawn(async move {
            let _permit = limit.acquire_owned().await;
            let outcome = probe::probe(
                &conn.handle,
                &address,
                port,
                server_name.as_deref(),
                now_ms(),
            )
            .await;
            (index, outcome)
        });
    }
    let mut denied = false;
    while let Some(joined) = tasks.join_next().await {
        let Ok((index, outcome)) = joined else {
            continue;
        };
        denied |= outcome.forwarding_denied;
        let port = &mut ports[index];
        port.protocol = outcome.protocol;
        port.http = outcome.http;
        port.probe_error = outcome.error;
    }
    if denied {
        warnings.push(
            "the SSH server does not allow TCP forwarding, so ports were not probed; set AllowTcpForwarding yes in sshd_config"
                .into(),
        );
    }
    Ok(())
}

pub async fn list(daemon: &Daemon, params: PortsListParams) -> Result<PortsList, RpcError> {
    let (entry, loaded) = daemon.resolve_host(&params.host).await?;
    let mut scan = scan(daemon, &entry, &loaded).await?;
    if params.probe.unwrap_or(true) {
        probe_ports(
            daemon,
            &entry,
            &loaded,
            &mut scan.ports,
            None,
            &mut scan.warnings,
        )
        .await?;
    }
    Ok(PortsList {
        host: scan.host,
        tool: scan.tool,
        ports: scan.ports,
        warnings: scan.warnings,
        scanned_at: now_ms(),
    })
}

fn idle_port(port: u16, note: Option<PortNote>) -> ListeningPort {
    ListeningPort {
        port,
        listening: false,
        addresses: Vec::new(),
        bind: None,
        pids: Vec::new(),
        process: None,
        service: None,
        protocol: None,
        http: None,
        probe_error: None,
        note,
    }
}

fn valid_port(port: u16) -> Result<u16, RpcError> {
    if port == 0 {
        return Err(invalid("`port` must be between 1 and 65535"));
    }
    Ok(port)
}

/// One port, always probed; `ports` holds exactly that port.
pub async fn inspect(daemon: &Daemon, params: PortParams) -> Result<PortsList, RpcError> {
    let port = valid_port(params.port)?;
    let (entry, loaded) = daemon.resolve_host(&params.host).await?;
    let scan = scan(daemon, &entry, &loaded).await?;
    let mut warnings = scan.warnings;
    let mut ports = vec![
        scan.ports
            .into_iter()
            .find(|p| p.port == port)
            .unwrap_or_else(|| idle_port(port, None)),
    ];
    let server_name = params
        .server_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    probe_ports(
        daemon,
        &entry,
        &loaded,
        &mut ports,
        server_name,
        &mut warnings,
    )
    .await?;
    Ok(PortsList {
        host: scan.host,
        tool: scan.tool,
        ports,
        warnings,
        scanned_at: now_ms(),
    })
}

fn note_label(note: &PortNote) -> String {
    [note.name.as_deref(), note.project.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" / ")
}

pub async fn check(daemon: &Daemon, params: PortParams) -> Result<PortCheck, RpcError> {
    let port = valid_port(params.port)?;
    let (entry, loaded) = daemon.resolve_host(&params.host).await?;
    let scan = scan(daemon, &entry, &loaded).await?;
    let mut warnings = scan.warnings.clone();
    let found = scan.port(port).cloned();
    let in_use = found.as_ref().is_some_and(|p| p.listening);
    if let Some(note) = found
        .as_ref()
        .filter(|p| !p.listening)
        .and_then(|p| p.note.as_ref())
    {
        warnings.push(format!(
            "port {port} is free but reserved by a note ({})",
            note_label(note)
        ));
    }
    let suggestion = if in_use {
        let taken = scan.taken();
        (port.saturating_add(1)..=u16::MAX).find(|p| !taken.contains(p))
    } else {
        None
    };
    Ok(PortCheck {
        host: scan.host,
        port,
        in_use,
        listener: found.filter(|p| p.listening),
        suggestion,
        warnings,
    })
}

pub fn free_ports(taken: &BTreeSet<u16>, from: u16, to: u16, count: usize) -> Vec<u16> {
    (from..=to)
        .filter(|p| !taken.contains(p))
        .take(count)
        .collect()
}

pub async fn find_free(daemon: &Daemon, params: FindFreeParams) -> Result<FreePorts, RpcError> {
    let from = params.from.unwrap_or(8000);
    let to = params.to.unwrap_or(9999);
    let count = params.count.unwrap_or(1);
    if from == 0 || from > to {
        return Err(invalid("`from` must be at least 1 and not above `to`"));
    }
    if !(1..=100).contains(&count) {
        return Err(invalid("`count` must be between 1 and 100"));
    }
    let (entry, loaded) = daemon.resolve_host(&params.host).await?;
    let scan = scan(daemon, &entry, &loaded).await?;
    let mut warnings = scan.warnings.clone();
    let ports = free_ports(&scan.taken(), from, to, count as usize);
    if ports.len() < count as usize {
        warnings.push(format!(
            "only {} free port(s) between {from} and {to}",
            ports.len()
        ));
    }
    Ok(FreePorts {
        host: scan.host,
        ports,
        warnings,
    })
}

fn note_field(value: Option<String>, field: &str, max: usize) -> Result<Option<String>, RpcError> {
    let value = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if value.as_ref().is_some_and(|v| v.chars().count() > max) {
        return Err(invalid(format!(
            "`{field}` is longer than {max} characters"
        )));
    }
    Ok(value)
}

pub async fn annotate(
    daemon: &Daemon,
    params: PortAnnotateParams,
) -> Result<PortAnnotated, RpcError> {
    let port = valid_port(params.port)?;
    let (entry, _) = daemon.resolve_host(&params.host).await?;
    let name = note_field(params.name, "name", MAX_NOTE_FIELD)?;
    let project = note_field(params.project, "project", MAX_NOTE_FIELD)?;
    let text = note_field(params.note, "note", MAX_NOTE)?;
    let note = (name.is_some() || project.is_some() || text.is_some()).then(|| PortNote {
        name,
        project,
        note: text,
        updated_at: now_ms(),
    });
    let host = entry.host.id;
    daemon
        .store
        .set_port_note(host.clone(), port, note.clone())
        .await?;
    Ok(PortAnnotated { host, port, note })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_ports_skip_taken_ones() {
        let taken: BTreeSet<u16> = [8000, 8001, 8003].into_iter().collect();
        assert_eq!(free_ports(&taken, 8000, 8010, 2), vec![8002, 8004]);
        assert_eq!(free_ports(&taken, 8000, 8001, 5), Vec::<u16>::new());
        assert_eq!(free_ports(&taken, 65535, 65535, 1), vec![65535]);
    }

    #[test]
    fn note_fields_are_trimmed_and_capped() {
        assert_eq!(note_field(Some("  ".into()), "name", 5).unwrap(), None);
        assert_eq!(
            note_field(Some(" api ".into()), "name", 5)
                .unwrap()
                .as_deref(),
            Some("api")
        );
        assert!(note_field(Some("toolong".into()), "name", 5).is_err());
    }
}
