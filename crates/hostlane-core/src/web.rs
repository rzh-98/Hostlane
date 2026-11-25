use crate::{Daemon, ports, remote, util::sh_quote};
use hostlane_protocol::{
    BindScope, DomainRoute, ErrorKind, Exposure, HostNote, HostNoteParams, HostProfile,
    PortsListParams, RpcError, WebReport,
};
use std::time::Duration;
mod routes;

const PROFILE: &str = r#"export LC_ALL=C
uname -srm
printf 'cpu='; getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo unknown
printf 'memory='; if test -r /proc/meminfo; then awk '/^MemTotal:/ {print $2, $3}' /proc/meminfo; else sysctl -n hw.memsize 2>/dev/null | awk '{print $1, "bytes"}'; fi
printf 'disk='; df -Pk / | awk 'END {print $4 " KiB available of " $2 " KiB"}'
for tool in tmux rg docker git; do if command -v "$tool" >/dev/null 2>&1; then echo "tool=$tool"; fi; done"#;

const INVENTORY: &str = r#"export LC_ALL=C
for file in /etc/nginx/nginx.conf /etc/nginx/conf.d/*.conf /etc/nginx/sites-enabled/* /etc/caddy/Caddyfile; do
  test -f "$file" && test -r "$file" || continue
  case "$file" in /etc/caddy/*) echo '@@caddy';; *) echo '@@nginx';; esac
  cat "$file" || exit 1
  echo
done
echo '@@firewall'
if status=$(ufw status verbose 2>/dev/null) && printf '%s\n' "$status" | grep -q '^Status: active'; then echo ufw; printf '%s\n' "$status"
elif firewall-cmd --state >/dev/null 2>&1; then echo firewalld; firewall-cmd --list-all-zones 2>/dev/null
elif rules=$(nft list ruleset 2>/dev/null); then echo nftables; printf '%s\n' "$rules"
elif rules=$(iptables -S INPUT 2>/dev/null); then echo iptables; printf '%s\n' "$rules"
else echo unavailable; fi
echo '@@end'"#;

async fn collect(
    daemon: &Daemon,
    host: &str,
    script: &str,
    cap: usize,
) -> Result<String, RpcError> {
    let (entry, loaded) = daemon.resolve_host(host).await?;
    let (_lease, channel) = daemon.pool.open_channel(&entry, &loaded).await?;
    let result = remote::capture(
        channel,
        &format!("sh -c {}", sh_quote(script)),
        cap,
        None,
        Duration::from_secs(15),
    )
    .await?;
    if result.timed_out || result.truncated || result.exit_code != Some(0) {
        return Err(RpcError::new(
            ErrorKind::RemoteCommandFailed,
            "inventory incomplete (failed, timed out or exceeded the output limit)",
        ));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

pub async fn profile(daemon: &Daemon, host: String) -> Result<HostProfile, RpcError> {
    let output = collect(daemon, &host, PROFILE, 8192).await?;
    let value = |key: &str| {
        output
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .filter(|v| !v.is_empty())
            .unwrap_or("unknown")
            .to_owned()
    };
    Ok(HostProfile {
        host,
        system: output.lines().next().unwrap_or("unknown").into(),
        cpu: value("cpu="),
        memory: value("memory="),
        disk: value("disk="),
        tools: output
            .lines()
            .filter_map(|line| line.strip_prefix("tool=").map(str::to_owned))
            .collect(),
    })
}

pub async fn note(daemon: &Daemon, params: HostNoteParams) -> Result<HostNote, RpcError> {
    daemon.resolve_host(&params.host).await?;
    if params.content.len() > 16 * 1024 {
        return Err(RpcError::new(
            ErrorKind::InvalidParams,
            "host note exceeds 16 KiB",
        ));
    }
    let key = format!("hostnote:{}", params.host);
    daemon
        .store
        .set_setting(key, (!params.content.is_empty()).then_some(params.content))
        .await?;
    export(daemon, params.host).await
}

pub async fn export(daemon: &Daemon, host: String) -> Result<HostNote, RpcError> {
    let (entry, _) = daemon.resolve_host(&host).await?;
    let content = daemon
        .store
        .setting(&format!("hostnote:{host}"))
        .await?
        .unwrap_or_default();
    let markdown = format!(
        "# {}\n\n- Host: {}\n- SSH port: {}\n\n## Notes\n\n{}\n",
        host, entry.host.hostname, entry.host.port, content
    );
    Ok(HostNote {
        host,
        content,
        markdown,
    })
}

fn exposure(bind: Option<BindScope>, port: u16, firewall: &str, rules: &str) -> &'static str {
    match bind {
        Some(BindScope::Loopback) => "loopback",
        Some(BindScope::Private) => "private bind (routing and firewall may restrict access)",
        Some(_) => {
            let ufw_allow = firewall == "ufw"
                && rules.lines().any(|line| {
                    let mut words = line.split_whitespace();
                    words.next().is_some_and(|value| {
                        value == port.to_string() || value == format!("{port}/tcp")
                    }) && words.next() == Some("ALLOW")
                        && words.next() == Some("IN")
                        && words.next() == Some("Anywhere")
                });
            let tcp_port = format!("{port}/tcp");
            let firewalld_allow = firewall == "firewalld"
                && rules
                    .split('\n')
                    .scan(false, |active, line| {
                        if !line.starts_with(' ') {
                            *active = line.ends_with(" (active)");
                        }
                        Some(
                            *active
                                && line.trim().strip_prefix("ports:").is_some_and(|list| {
                                    list.split_whitespace().any(|entry| entry == tcp_port)
                                }),
                        )
                    })
                    .any(|allowed| allowed);
            let iptables_allow = firewall == "iptables"
                && rules.lines().any(|line| {
                    line.trim() == format!("-A INPUT -p tcp -m tcp --dport {port} -j ACCEPT")
                });
            let nft_allow = firewall == "nftables"
                && rules.split('}').any(|chain| {
                    chain.contains("hook input ")
                        && chain
                            .lines()
                            .any(|line| line.trim() == format!("tcp dport {port} accept"))
                });
            if ufw_allow
                || firewalld_allow
                || iptables_allow
                || nft_allow
                || (firewall == "nftables" && rules.trim().is_empty())
                || (firewall == "iptables" && rules.trim() == "-P INPUT ACCEPT")
            {
                "potentially public"
            } else {
                "non-loopback; external reachability unknown"
            }
        }
        None => "unknown",
    }
}

pub async fn report(daemon: &Daemon, host: String) -> Result<WebReport, RpcError> {
    let ports = ports::list(
        daemon,
        PortsListParams {
            host: host.clone(),
            probe: Some(false),
        },
    )
    .await?;
    let output = collect(daemon, &host, INVENTORY, 512 * 1024).await?;
    let mut sections: Vec<(String, String)> = Vec::new();
    for line in output.lines() {
        if matches!(line, "@@nginx" | "@@caddy" | "@@firewall" | "@@end") {
            sections.push((line[2..].into(), String::new()));
        } else if let Some((_, body)) = sections.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    let (firewall, rules) = sections
        .iter()
        .rev()
        .find(|(kind, _)| kind == "firewall")
        .and_then(|(_, body)| body.split_once('\n'))
        .unwrap_or(("unavailable", ""));
    let routes = routes::parse(&sections);
    let listeners = ports
        .ports
        .into_iter()
        .filter(|entry| entry.listening)
        .map(|entry| Exposure {
            port: entry.port,
            addresses: entry.addresses,
            assessment: exposure(entry.bind, entry.port, firewall, rules).into(),
        })
        .collect();
    let mut warnings = ports.warnings;
    warnings.push("Host-local estimate only; cloud rules, NAT, zone order and external reachability are not verified. Only readable standard Nginx/Caddy files and static targets are mapped.".into());
    Ok(WebReport {
        host,
        firewall: firewall.into(),
        listeners,
        routes,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uncertain_firewall_is_not_treated_as_open() {
        for (kind, rules) in [
            ("ufw", "8080/tcp ALLOW IN Anywhere"),
            ("firewalld", "public (active)\n  ports: 8080/tcp"),
            (
                "nftables",
                "chain input {\n type filter hook input priority 0; policy drop;\n tcp dport 8080 accept\n}",
            ),
            (
                "iptables",
                "-P INPUT DROP\n-A INPUT -p tcp -m tcp --dport 8080 -j ACCEPT",
            ),
        ] {
            assert_eq!(
                exposure(Some(BindScope::Loopback), 8080, kind, rules),
                "loopback"
            );
            assert_eq!(
                exposure(Some(BindScope::All), 8080, kind, rules),
                "potentially public"
            );
            assert!(exposure(Some(BindScope::All), 8443, kind, rules).contains("unknown"));
        }
        assert!(exposure(Some(BindScope::Public), 443, "unavailable", "").contains("unknown"));
    }
}
