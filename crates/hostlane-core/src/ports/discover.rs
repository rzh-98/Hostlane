//! Finding listening TCP sockets, their processes and services from the output of
//! standard tools. Everything here is a pure function over command output, so it is
//! tested with captured samples.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use hostlane_protocol::{BindScope, ProcessInfo, ServiceInfo, ServiceKind};
use serde::Deserialize;

/// Lists listening sockets with the best available tool, then running containers.
///
/// Linux: `ss`, else `netstat -tlnp`, else `/proc/net/tcp*`. Elsewhere (macOS, BSD):
/// `netstat -an` for the sockets and `lsof` for their processes.
pub const LIST_SCRIPT: &str = r#"PATH="$PATH:/usr/sbin:/sbin"; LC_ALL=C; export PATH LC_ALL
echo "@@uid $(id -u 2>/dev/null)"
if [ "$(uname -s 2>/dev/null)" = Linux ]; then
  if command -v ss >/dev/null 2>&1; then echo "@@listen ss"; ss -tlnp 2>&1
  elif command -v netstat >/dev/null 2>&1; then echo "@@listen netstat"; netstat -tlnp 2>&1
  else echo "@@listen proc"; cat /proc/net/tcp /proc/net/tcp6 2>/dev/null
  fi
else
  echo "@@listen bsd"; netstat -an -p tcp 2>&1
  if command -v lsof >/dev/null 2>&1; then echo "@@lsof"; lsof -nP -iTCP -sTCP:LISTEN -F pcLn +c 0 2>/dev/null; fi
fi
if command -v docker >/dev/null 2>&1; then echo "@@docker"; docker ps --no-trunc --format '{{json .}}' 2>&1; fi
echo "@@end"
"#;

/// Process details and cgroups for `pids`.
pub fn process_script(pids: &[u32]) -> String {
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let spaced = list.replace(',', " ");
    format!(
        r#"LC_ALL=C; export LC_ALL
echo "@@ps"
ps -ww -o pid= -o ppid= -o user:32= -o etime= -o args= -p {list} 2>/dev/null \
  || ps -ww -o pid= -o ppid= -o user= -o etime= -o args= -p {list} 2>/dev/null \
  || ps -o pid,ppid,user,etime,args 2>/dev/null
for p in {spaced}; do echo "@@cgroup $p"; cat /proc/$p/cgroup 2>/dev/null; done
echo "@@end"
"#
    )
}

/// Splits `@@name arg` sections into (header, body lines).
pub fn sections(output: &str) -> Vec<(String, Vec<&str>)> {
    let mut out: Vec<(String, Vec<&str>)> = Vec::new();
    for line in output.lines() {
        if let Some(header) = line.strip_prefix("@@") {
            out.push((header.trim().to_string(), Vec::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push(line);
        }
    }
    out
}

/// One listening socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Socket {
    pub address: String,
    pub port: u16,
    /// (pid, process name) pairs, when the tool shows them.
    pub owners: Vec<(u32, String)>,
}

/// Normalizes an address as printed by the tools: drops brackets and `%iface`,
/// unwraps IPv4-mapped IPv6, and spells wildcards `0.0.0.0` / `::` / `*`.
pub fn normalize_address(raw: &str) -> String {
    let trimmed = raw.trim_start_matches('[').trim_end_matches(']');
    let without_scope = trimmed.split('%').next().unwrap_or(trimmed);
    match without_scope.parse::<IpAddr>() {
        Ok(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => v6.to_string(),
        },
        Ok(ip) => ip.to_string(),
        Err(_) => without_scope.to_string(),
    }
}

/// `addr:port` with the port after the last `:`.
fn split_colon(text: &str) -> Option<(String, u16)> {
    let (address, port) = text.rsplit_once(':')?;
    Some((normalize_address(address), port.parse().ok()?))
}

/// `users:(("nginx",pid=1234,fd=6),("nginx",pid=1235,fd=6))`
fn parse_ss_users(text: &str) -> Vec<(u32, String)> {
    let mut owners = Vec::new();
    for part in text.split("((").skip(1).flat_map(|rest| rest.split("),(")) {
        let mut fields = part.split(',');
        let name = fields
            .next()
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        let pid = fields
            .find_map(|field| field.strip_prefix("pid="))
            .and_then(|pid| pid.trim_end_matches([')', ' ']).parse().ok());
        if let Some(pid) = pid {
            owners.push((pid, name));
        }
    }
    owners
}

/// `ss -tlnp`. Lines: `LISTEN 0 511 0.0.0.0:80 0.0.0.0:* users:((…))`.
pub fn parse_ss(lines: &[&str]) -> Vec<Socket> {
    lines
        .iter()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.first() != Some(&"LISTEN") || fields.len() < 5 {
                return None;
            }
            let (address, port) = split_colon(fields[3])?;
            let owners = parse_ss_users(&fields[5..].join(" "));
            Some(Socket {
                address,
                port,
                owners,
            })
        })
        .collect()
}

/// Linux `netstat -tlnp`: `tcp 0 0 127.0.0.1:5432 0.0.0.0:* LISTEN 1234/postgres`.
pub fn parse_netstat_linux(lines: &[&str]) -> Vec<Socket> {
    lines
        .iter()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if !fields.first()?.starts_with("tcp") || fields.get(5) != Some(&"LISTEN") {
                return None;
            }
            let (address, port) = split_colon(fields[3])?;
            let owners = fields
                .get(6)
                .and_then(|owner| owner.split_once('/'))
                .and_then(|(pid, name)| Some((pid.parse().ok()?, name.to_string())))
                .into_iter()
                .collect();
            Some(Socket {
                address,
                port,
                owners,
            })
        })
        .collect()
}

/// BSD/macOS `netstat -an -p tcp`: `tcp46 0 0 *.5000 *.* LISTEN`; the port follows the last dot.
pub fn parse_netstat_bsd(lines: &[&str]) -> Vec<Socket> {
    lines
        .iter()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if !fields.first()?.starts_with("tcp") || fields.last() != Some(&"LISTEN") {
                return None;
            }
            let (address, port) = fields.get(3)?.rsplit_once('.')?;
            let address = match (address, fields[0]) {
                ("*", "tcp4") => "0.0.0.0".to_string(),
                ("*", "tcp6") => "::".to_string(),
                ("*", _) => "*".to_string(),
                (address, _) => normalize_address(address),
            };
            Some(Socket {
                address,
                port: port.parse().ok()?,
                owners: Vec::new(),
            })
        })
        .collect()
}

fn hex_ipv4(hex: &str) -> Option<Ipv4Addr> {
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(Ipv4Addr::from(value.swap_bytes()))
}

fn hex_ipv6(hex: &str) -> Option<Ipv6Addr> {
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for word in 0..4 {
        let value = u32::from_str_radix(&hex[word * 8..word * 8 + 8], 16).ok()?;
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.swap_bytes().to_be_bytes());
    }
    Some(Ipv6Addr::from(bytes))
}

/// `/proc/net/tcp` and `tcp6`: hex `local_address`, state `0A` is LISTEN.
pub fn parse_proc_net(lines: &[&str]) -> Vec<Socket> {
    lines
        .iter()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.get(3) != Some(&"0A") {
                return None;
            }
            let (address, port) = fields.get(1)?.split_once(':')?;
            let ip = if address.len() == 8 {
                IpAddr::V4(hex_ipv4(address)?)
            } else {
                IpAddr::V6(hex_ipv6(address)?)
            };
            Some(Socket {
                address: normalize_address(&ip.to_string()),
                port: u16::from_str_radix(port, 16).ok()?,
                owners: Vec::new(),
            })
        })
        .collect()
}

/// `lsof -F pcLn`: `p<pid>`, `c<command>`, `n<addr:port>` records. Returns owners by port.
pub fn parse_lsof(lines: &[&str]) -> HashMap<u16, Vec<(u32, String)>> {
    let mut owners: HashMap<u16, Vec<(u32, String)>> = HashMap::new();
    let mut pid = 0u32;
    let mut command = String::new();
    for line in lines {
        let Some(tag) = line.chars().next() else {
            continue;
        };
        let value = &line[1..];
        match tag {
            'p' => {
                pid = value.parse().unwrap_or(0);
                command.clear();
            }
            'c' => command = value.to_string(),
            'n' => {
                if let Some((_, port)) = split_colon(value) {
                    let entry = owners.entry(port).or_default();
                    if pid != 0 && !entry.iter().any(|(p, _)| *p == pid) {
                        entry.push((pid, command.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    owners
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsRow {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub elapsed_secs: Option<u64>,
    pub args: String,
}

/// `[[dd-]hh:]mm:ss`
pub fn parse_etime(text: &str) -> Option<u64> {
    let (days, rest) = match text.split_once('-') {
        Some((days, rest)) => (days.parse::<u64>().ok()?, rest),
        None => (0, text),
    };
    let mut secs = 0u64;
    for part in rest.split(':') {
        secs = secs * 60 + part.parse::<u64>().ok()?;
    }
    Some(days * 86_400 + secs)
}

/// `ps -o pid= -o ppid= -o user= -o etime= -o args=`. Lines that do not start with a
/// number (headers) are skipped.
pub fn parse_ps(lines: &[&str]) -> HashMap<u32, PsRow> {
    let mut rows = HashMap::new();
    for line in lines {
        let mut rest = line.trim_start();
        let mut next = || {
            let field = rest.split_whitespace().next()?;
            let start = rest.find(field)? + field.len();
            rest = rest[start..].trim_start();
            Some(field)
        };
        let (Some(pid), Some(ppid), Some(user), Some(etime)) = (next(), next(), next(), next())
        else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid.parse(), ppid.parse()) else {
            continue;
        };
        rows.insert(
            pid,
            PsRow {
                pid,
                ppid,
                user: user.to_string(),
                elapsed_secs: parse_etime(etime),
                args: rest.trim_end().to_string(),
            },
        );
    }
    rows
}

/// What a process's cgroup says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgroupOwner {
    Unit { name: String, user: bool },
    Container(String),
}

fn container_id(component: &str) -> Option<&str> {
    let id = component
        .strip_prefix("docker-")
        .and_then(|rest| rest.strip_suffix(".scope"))
        .unwrap_or(component);
    (id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}

/// `0::/system.slice/nginx.service`, `…/docker-<id>.scope`, `12:pids:/docker/<id>`.
pub fn parse_cgroup(lines: &[&str]) -> Option<CgroupOwner> {
    let mut unit: Option<CgroupOwner> = None;
    for line in lines {
        let path = line.splitn(3, ':').nth(2).unwrap_or_default();
        let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
        if let Some(id) = components.iter().rev().find_map(|c| container_id(c)) {
            return Some(CgroupOwner::Container(id.to_string()));
        }
        if unit.is_none()
            && let Some(index) = components.iter().rposition(|c| c.ends_with(".service"))
        {
            let user = components[..index]
                .iter()
                .any(|c| c.starts_with("user@") && c.ends_with(".service"));
            unit = Some(CgroupOwner::Unit {
                name: components[index].to_string(),
                user,
            });
        }
    }
    unit
}

/// A running container from `docker ps --format '{{json .}}'`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Container {
    #[serde(rename = "ID")]
    pub id: String,
    pub names: String,
    pub image: String,
    #[serde(default)]
    pub ports: String,
    #[serde(default)]
    pub labels: String,
}

impl Container {
    fn label(&self, key: &str) -> Option<String> {
        self.labels
            .split(',')
            .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    pub fn service(&self, container_port: Option<u16>) -> ServiceInfo {
        ServiceInfo {
            kind: ServiceKind::Docker,
            name: self.names.split(',').next().unwrap_or_default().to_string(),
            user_unit: false,
            container_id: Some(self.id.chars().take(12).collect()),
            image: Some(self.image.clone()),
            container_port,
            compose_project: self.label("com.docker.compose.project"),
            compose_service: self.label("com.docker.compose.service"),
        }
    }
}

/// A published TCP port: host address and port → container port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub address: String,
    pub host_port: u16,
    pub container_port: u16,
}

fn port_range(text: &str) -> Option<(u16, u16)> {
    match text.split_once('-') {
        Some((start, end)) => Some((start.parse().ok()?, end.parse().ok()?)),
        None => {
            let port = text.parse().ok()?;
            Some((port, port))
        }
    }
}

/// `0.0.0.0:8080->80/tcp, :::8080->80/tcp, 127.0.0.1:8000-8001->8000-8001/tcp, 9000/tcp`
pub fn parse_published(ports: &str) -> Vec<Published> {
    let mut out = Vec::new();
    for part in ports.split(',').map(str::trim) {
        let Some((host, container)) = part.split_once("->") else {
            continue;
        };
        let Some((container, "tcp")) = container.split_once('/') else {
            continue;
        };
        let Some((address, host_ports)) = host.rsplit_once(':') else {
            continue;
        };
        let (Some((host_start, host_end)), Some((container_start, _))) =
            (port_range(host_ports), port_range(container))
        else {
            continue;
        };
        let address = if address.is_empty() || address == "::" {
            "::".to_string()
        } else {
            normalize_address(address)
        };
        for offset in 0..=host_end.saturating_sub(host_start) {
            out.push(Published {
                address: address.clone(),
                host_port: host_start + offset,
                container_port: container_start + offset,
            });
        }
    }
    out
}

/// Running containers, or the reason `docker ps` failed.
pub fn parse_docker(lines: &[&str]) -> Result<Vec<Container>, String> {
    let mut containers = Vec::new();
    let mut errors = Vec::new();
    for line in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        match serde_json::from_str::<Container>(line) {
            Ok(container) => containers.push(container),
            Err(_) => errors.push(line.to_string()),
        }
    }
    if containers.is_empty() && !errors.is_empty() {
        return Err(errors.join(" "));
    }
    Ok(containers)
}

pub fn scope_of(address: &str) -> BindScope {
    let ip = match address {
        "*" => return BindScope::All,
        other => match other.parse::<IpAddr>() {
            Ok(ip) => ip,
            Err(_) => return BindScope::Public,
        },
    };
    if ip.is_unspecified() {
        return BindScope::All;
    }
    if ip.is_loopback() {
        return BindScope::Loopback;
    }
    let private = match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private() || v4.is_link_local() || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    };
    if private {
        BindScope::Private
    } else {
        BindScope::Public
    }
}

/// Sockets grouped by port.
#[derive(Debug, Default, Clone)]
pub struct PortGroup {
    pub addresses: BTreeSet<String>,
    /// pid → name from the socket tool.
    pub owners: BTreeMap<u32, String>,
}

pub fn group(sockets: Vec<Socket>) -> BTreeMap<u16, PortGroup> {
    let mut ports: BTreeMap<u16, PortGroup> = BTreeMap::new();
    for socket in sockets {
        let group = ports.entry(socket.port).or_default();
        group.addresses.insert(socket.address);
        for (pid, name) in socket.owners {
            group.owners.entry(pid).or_insert(name);
        }
    }
    ports
}

/// Widest scope among the addresses.
pub fn bind_scope(addresses: &BTreeSet<String>) -> Option<BindScope> {
    addresses.iter().map(|a| scope_of(a)).max()
}

/// The main process of a port: the one whose parent does not also hold the socket.
pub fn main_pid(pids: &[u32], ps: &HashMap<u32, PsRow>) -> Option<u32> {
    pids.iter()
        .copied()
        .find(|pid| ps.get(pid).is_none_or(|row| !pids.contains(&row.ppid)))
        .or_else(|| pids.first().copied())
}

pub fn process_info(pid: u32, name: &str, row: Option<&PsRow>, now_ms: u64) -> ProcessInfo {
    let fallback_name = || {
        row.and_then(|r| r.args.split_whitespace().next())
            .map(|arg0| arg0.rsplit('/').next().unwrap_or(arg0).to_string())
            .unwrap_or_default()
    };
    ProcessInfo {
        pid,
        name: if name.is_empty() {
            fallback_name()
        } else {
            name.to_string()
        },
        user: row.map(|r| r.user.clone()),
        command: row.map(|r| r.args.clone()).filter(|a| !a.is_empty()),
        started_at: row
            .and_then(|r| r.elapsed_secs)
            .map(|secs| now_ms.saturating_sub(secs * 1000)),
    }
}

/// Address to connect to when probing a port from the server itself.
pub fn probe_address(addresses: &BTreeSet<String>) -> String {
    let has = |predicate: &dyn Fn(&str) -> bool| addresses.iter().find(|a| predicate(a));
    if has(&|a| a == "127.0.0.1" || a == "0.0.0.0" || a == "*").is_some() {
        return "127.0.0.1".into();
    }
    if has(&|a| a == "::1" || a == "::").is_some() {
        return "::1".into();
    }
    addresses
        .iter()
        .next()
        .cloned()
        .unwrap_or_else(|| "127.0.0.1".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ss_output() {
        let lines = [
            "State  Recv-Q Send-Q Local Address:Port  Peer Address:Port Process",
            "LISTEN 0      511          0.0.0.0:80         0.0.0.0:*     users:((\"nginx\",pid=1235,fd=6),(\"nginx\",pid=1234,fd=6))",
            "LISTEN 0      4096   127.0.0.53%lo:53         0.0.0.0:*     users:((\"systemd-resolve\",pid=600,fd=14))",
            "LISTEN 0      511             [::]:80            [::]:*",
            "LISTEN 0      128                *:22               *:*",
            "LISTEN 0      128   [::ffff:127.0.0.1]:3000       *:*",
        ];
        let sockets = parse_ss(&lines);
        assert_eq!(sockets.len(), 5);
        assert_eq!(sockets[0].address, "0.0.0.0");
        assert_eq!(
            sockets[0].owners,
            vec![(1235, "nginx".into()), (1234, "nginx".into())]
        );
        assert_eq!(sockets[1].address, "127.0.0.53");
        assert_eq!((sockets[2].address.as_str(), sockets[2].port), ("::", 80));
        assert_eq!(sockets[3].address, "*");
        assert_eq!(sockets[4].address, "127.0.0.1");
    }

    #[test]
    fn netstat_outputs() {
        let linux = [
            "Active Internet connections (only servers)",
            "Proto Recv-Q Send-Q Local Address           Foreign Address         State       PID/Program name",
            "tcp        0      0 127.0.0.1:5432          0.0.0.0:*               LISTEN      1234/postgres",
            "tcp6       0      0 :::80                   :::*                    LISTEN      -",
        ];
        let sockets = parse_netstat_linux(&linux);
        assert_eq!(sockets[0].owners, vec![(1234, "postgres".into())]);
        assert_eq!((sockets[1].address.as_str(), sockets[1].port), ("::", 80));

        let bsd = [
            "tcp4       0      0  127.0.0.1.2222         *.*                    LISTEN",
            "tcp46      0      0  *.5000                 *.*                    LISTEN",
            "tcp6       0      0  ::1.631                *.*                    LISTEN",
            "tcp4       0      0  10.0.0.2.443           10.0.0.9.5555          ESTABLISHED",
        ];
        let sockets = parse_netstat_bsd(&bsd);
        assert_eq!(sockets.len(), 3);
        assert_eq!(
            (sockets[0].address.as_str(), sockets[0].port),
            ("127.0.0.1", 2222)
        );
        assert_eq!(sockets[1].address, "*");
        assert_eq!(sockets[2].address, "::1");
    }

    #[test]
    fn proc_net_tcp() {
        let lines = [
            "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode",
            "   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345",
            "   1: 0100007F:1F90 0100007F:D2A4 01 00000000:00000000 00:00000000 00000000  1000        0 0",
            "   0: 00000000000000000000000000000000:0050 00000000000000000000000000000000:0000 0A 0 0 0 0 0",
            "   1: 00000000000000000000000001000000:0277 00000000000000000000000000000000:0000 0A 0 0 0 0 0",
        ];
        let sockets = parse_proc_net(&lines);
        assert_eq!(sockets.len(), 3);
        assert_eq!(
            (sockets[0].address.as_str(), sockets[0].port),
            ("127.0.0.1", 8080)
        );
        assert_eq!((sockets[1].address.as_str(), sockets[1].port), ("::", 80));
        assert_eq!((sockets[2].address.as_str(), sockets[2].port), ("::1", 631));
    }

    #[test]
    fn lsof_records() {
        let lines = [
            "p765",
            "cControlCenter",
            "Lme",
            "f11",
            "n*:7000",
            "f12",
            "n*:7000",
            "p900",
            "cnode",
            "f20",
            "n127.0.0.1:3000",
            "f21",
            "n[::1]:3000",
        ];
        let owners = parse_lsof(&lines);
        assert_eq!(owners[&7000], vec![(765, "ControlCenter".into())]);
        assert_eq!(owners[&3000], vec![(900, "node".into())]);
    }

    #[test]
    fn ps_rows_and_elapsed_time() {
        let lines = [
            "  PID  PPID USER     ELAPSED COMMAND",
            " 1234     1 www-data 2-03:04:05 nginx: master process /usr/sbin/nginx -g daemon on;",
            " 1235  1234 www-data      01:02 nginx: worker process",
        ];
        let rows = parse_ps(&lines);
        assert_eq!(rows.len(), 2);
        let master = &rows[&1234];
        assert_eq!(master.user, "www-data");
        assert_eq!(
            master.elapsed_secs,
            Some(2 * 86_400 + 3 * 3600 + 4 * 60 + 5)
        );
        assert_eq!(
            master.args,
            "nginx: master process /usr/sbin/nginx -g daemon on;"
        );
        assert_eq!(main_pid(&[1235, 1234], &rows), Some(1234));
        assert_eq!(parse_etime("00:07"), Some(7));
        assert_eq!(parse_etime("x"), None);

        let info = process_info(1235, "", rows.get(&1235), 100_000);
        assert_eq!(info.name, "nginx:");
        assert_eq!(info.started_at, Some(100_000 - 62_000));
    }

    #[test]
    fn cgroups() {
        assert_eq!(
            parse_cgroup(&["0::/system.slice/nginx.service"]),
            Some(CgroupOwner::Unit {
                name: "nginx.service".into(),
                user: false
            })
        );
        assert_eq!(
            parse_cgroup(&[
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/api.service"
            ]),
            Some(CgroupOwner::Unit {
                name: "api.service".into(),
                user: true
            })
        );
        let id = "a".repeat(64);
        assert_eq!(
            parse_cgroup(&[&format!("0::/system.slice/docker-{id}.scope")]),
            Some(CgroupOwner::Container(id.clone()))
        );
        assert_eq!(
            parse_cgroup(&[&format!("12:pids:/docker/{id}")]),
            Some(CgroupOwner::Container(id))
        );
        assert_eq!(
            parse_cgroup(&["0::/user.slice/user-1000.slice/session-3.scope"]),
            None
        );
    }

    #[test]
    fn docker_containers_and_published_ports() {
        let line = r#"{"ID":"0123456789abcdef","Image":"nginx:1.27","Names":"app-web-1","Ports":"0.0.0.0:8080->80/tcp, :::8080->80/tcp, 127.0.0.1:9000-9001->9000-9001/tcp, 5432/tcp, 0.0.0.0:53->53/udp","Labels":"com.docker.compose.project=app,com.docker.compose.service=web"}"#;
        let containers = parse_docker(&[line]).unwrap();
        let published = parse_published(&containers[0].ports);
        assert_eq!(published.len(), 4);
        assert_eq!(published[0].host_port, 8080);
        assert_eq!(published[1].address, "::");
        assert_eq!(
            (published[3].host_port, published[3].container_port),
            (9001, 9001)
        );
        let service = containers[0].service(Some(80));
        assert_eq!(service.name, "app-web-1");
        assert_eq!(service.container_id.as_deref(), Some("0123456789ab"));
        assert_eq!(service.compose_project.as_deref(), Some("app"));
        assert_eq!(service.compose_service.as_deref(), Some("web"));

        let err = parse_docker(&[
            "permission denied while trying to connect to the Docker daemon socket",
        ]);
        assert!(err.unwrap_err().contains("permission denied"));
    }

    #[test]
    fn scopes_and_probe_targets() {
        assert_eq!(scope_of("127.0.0.1"), BindScope::Loopback);
        assert_eq!(scope_of("::1"), BindScope::Loopback);
        assert_eq!(scope_of("10.1.2.3"), BindScope::Private);
        assert_eq!(scope_of("100.100.1.1"), BindScope::Private);
        assert_eq!(scope_of("fd00::1"), BindScope::Private);
        assert_eq!(scope_of("8.8.8.8"), BindScope::Public);
        assert_eq!(scope_of("::"), BindScope::All);
        assert_eq!(scope_of("*"), BindScope::All);
        let set = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(
            bind_scope(&set(&["127.0.0.1", "10.0.0.1"])),
            Some(BindScope::Private)
        );
        assert_eq!(probe_address(&set(&["::"])), "::1");
        assert_eq!(probe_address(&set(&["0.0.0.0", "::"])), "127.0.0.1");
        assert_eq!(probe_address(&set(&["10.0.0.5"])), "10.0.0.5");
    }

    #[test]
    fn sections_split_output() {
        let parsed = sections("noise\n@@uid 0\n@@listen ss\na\nb\n@@end\n");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[1].0, "listen ss");
        assert_eq!(parsed[1].1, vec!["a", "b"]);
        assert!(process_script(&[1, 2]).contains("-p 1,2"));
    }
}
