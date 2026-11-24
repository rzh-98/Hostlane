//! `services.*` and `processes.kill`: commands Hostlane builds from typed params and
//! runs like `exec.run`, so they appear in the session monitor and the policy rates
//! the exact command that will run.

use hostlane_protocol::{
    ClientInfo, ErrorKind, ExecParams, ProcessKillParams, RpcError, ServiceCommandResult,
    ServiceControlParams, ServiceKind, ServiceLogsParams, method,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Daemon;
use crate::exec;
use crate::ssh::local_user;
use crate::util::sh_quote;

const DEFAULT_LINES: u32 = 200;
const MAX_LINES: u32 = 10_000;
const SIGNALS: &[&str] = &["TERM", "INT", "HUP", "KILL", "QUIT", "USR1", "USR2"];

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

fn parse<T: DeserializeOwned>(params: &Value) -> Result<T, RpcError> {
    serde_json::from_value(params.clone()).map_err(|err| invalid(format!("invalid params: {err}")))
}

/// Unit and container names: letters, digits and `@._:-`, not starting with `-`.
fn check_name(name: &str) -> Result<&str, RpcError> {
    let ok = !name.is_empty()
        && name.len() <= 256
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._:-".contains(c));
    if ok {
        Ok(name)
    } else {
        Err(invalid(format!(
            "`{name}` is not a valid unit or container name"
        )))
    }
}

/// `10m`, `2h`, `1d` → (`journalctl --since`, `docker logs --since`) values.
fn since(value: &str) -> Result<(String, String), RpcError> {
    let value = value.trim();
    let (number, unit) = value.split_at(value.len().saturating_sub(1));
    let count: u64 = number
        .parse()
        .map_err(|_| invalid("`since` must look like 30s, 10m, 2h or 1d"))?;
    let (journal, docker) = match unit {
        "s" => (format!("-{count}s"), format!("{count}s")),
        "m" => (format!("-{count}min"), format!("{count}m")),
        "h" => (format!("-{count}h"), format!("{count}h")),
        "d" => (format!("-{count}d"), format!("{}h", count * 24)),
        _ => return Err(invalid("`since` must look like 30s, 10m, 2h or 1d")),
    };
    Ok((journal, docker))
}

fn with_sudo(command: String, sudo: bool) -> String {
    if sudo {
        format!("sudo -n {command}")
    } else {
        command
    }
}

/// The command a call runs, given the SSH login user.
pub(crate) fn command(name: &str, params: &Value, login_user: &str) -> Result<String, RpcError> {
    match name {
        method::SERVICES_LOGS => {
            let params: ServiceLogsParams = parse(params)?;
            let unit = sh_quote(check_name(&params.name)?);
            let lines = params.lines.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
            let since = params.since.as_deref().map(since).transpose()?;
            let command = match params.kind {
                ServiceKind::Systemd => {
                    let mut command = String::from("journalctl");
                    if params.user_unit {
                        command.push_str(" --user");
                    }
                    command.push_str(&format!(" -u {unit} -n {lines} --no-pager -o short-iso"));
                    if let Some((journal, _)) = &since {
                        command.push_str(&format!(" --since {journal}"));
                    }
                    command
                }
                ServiceKind::Docker => {
                    let mut command = format!("docker logs --tail {lines} --timestamps");
                    if let Some((_, docker)) = &since {
                        command.push_str(&format!(" --since {docker}"));
                    }
                    command.push_str(&format!(" {unit} 2>&1"));
                    command
                }
            };
            Ok(with_sudo(command, params.sudo.unwrap_or(false)))
        }
        method::SERVICES_START | method::SERVICES_STOP | method::SERVICES_RESTART => {
            let params: ServiceControlParams = parse(params)?;
            let target = sh_quote(check_name(&params.name)?);
            let action = name.trim_start_matches("services.");
            let (command, default_sudo) = match params.kind {
                ServiceKind::Systemd if params.user_unit => {
                    (format!("systemctl --user {action} {target}"), false)
                }
                ServiceKind::Systemd => {
                    (format!("systemctl {action} {target}"), login_user != "root")
                }
                ServiceKind::Docker => (format!("docker {action} {target}"), false),
            };
            Ok(with_sudo(command, params.sudo.unwrap_or(default_sudo)))
        }
        method::PROCESSES_KILL => {
            let params: ProcessKillParams = parse(params)?;
            if params.pid <= 1 {
                return Err(invalid("`pid` must be greater than 1"));
            }
            let signal = params
                .signal
                .as_deref()
                .unwrap_or("TERM")
                .trim()
                .to_ascii_uppercase();
            let signal = signal.strip_prefix("SIG").unwrap_or(&signal);
            if !SIGNALS.contains(&signal) {
                return Err(invalid(format!(
                    "`signal` must be one of {}",
                    SIGNALS.join(", ")
                )));
            }
            Ok(with_sudo(
                format!("kill -{signal} {}", params.pid),
                params.sudo.unwrap_or(false),
            ))
        }
        _ => Err(invalid(format!("`{name}` is not a service method"))),
    }
}

/// Checks the params of a service method before the policy asks anyone to approve it,
/// so invalid calls fail at once. Other methods pass.
pub(crate) fn validate(name: &str, params: &Value) -> Result<(), RpcError> {
    match name {
        method::SERVICES_LOGS
        | method::SERVICES_START
        | method::SERVICES_STOP
        | method::SERVICES_RESTART
        | method::PROCESSES_KILL => command(name, params, "root").map(|_| ()),
        _ => Ok(()),
    }
}

/// The SSH login user of `host`, for deciding whether system units need sudo.
pub(crate) async fn login_user(daemon: &Daemon, host: &str) -> Result<String, RpcError> {
    let (entry, _) = daemon.resolve_host(host).await?;
    Ok(entry.host.user.clone().unwrap_or_else(local_user))
}

/// Recognizable failures and what to do about them.
fn hint(stderr: &str, stdout: &str) -> Option<String> {
    let text = format!("{stderr}\n{stdout}");
    if text.contains("sudo: a password is required")
        || text.contains("sudo: a terminal is required")
    {
        Some(
            "sudo on this host needs a password, which Hostlane cannot type; allow this command with NOPASSWD in sudoers, or run it yourself"
                .into(),
        )
    } else if text.contains("Interactive authentication required") || text.contains("Access denied")
    {
        Some(
            "this needs root: retry with sudo, or log in as a user allowed to manage the unit"
                .into(),
        )
    } else if text.contains("permission denied while trying to connect to the Docker daemon") {
        Some(
            "the SSH user cannot use Docker: add it to the docker group, or retry with sudo".into(),
        )
    } else if text.contains("No journal files were opened due to insufficient permissions") {
        Some("the SSH user cannot read the system journal: add it to the systemd-journal or adm group, or retry with sudo".into())
    } else {
        None
    }
}

pub(crate) async fn run(
    daemon: &Daemon,
    client: &ClientInfo,
    name: &str,
    params: Value,
    approver: Option<String>,
) -> Result<ServiceCommandResult, RpcError> {
    let host = params
        .get("host")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("`host` is required"))?
        .to_string();
    let (entry, loaded) = daemon.resolve_host(&host).await?;
    let user = entry.host.user.clone().unwrap_or_else(local_user);
    let command = command(name, &params, &user)?;
    let logs = name == method::SERVICES_LOGS;
    let exec_params = ExecParams {
        host,
        command: command.clone(),
        cwd: None,
        env: None,
        stdin: None,
        timeout_ms: Some(if logs { 30_000 } else { 120_000 }),
        max_output_bytes: Some(if logs { 256 * 1024 } else { 16 * 1024 }),
    };
    let result = exec::run(daemon, client, exec_params, &entry, &loaded, approver).await?;
    let hint = if result.exit_code == Some(0) {
        None
    } else {
        hint(&result.stderr, &result.stdout)
    };
    Ok(ServiceCommandResult {
        command,
        result,
        hint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_commands() {
        let logs =
            json!({ "host": "h", "kind": "systemd", "name": "nginx", "lines": 50, "since": "10m" });
        assert_eq!(
            command(method::SERVICES_LOGS, &logs, "deploy").unwrap(),
            "journalctl -u nginx -n 50 --no-pager -o short-iso --since -10min"
        );
        let docker = json!({ "host": "h", "kind": "docker", "name": "web-1", "since": "1d" });
        assert_eq!(
            command(method::SERVICES_LOGS, &docker, "deploy").unwrap(),
            "docker logs --tail 200 --timestamps --since 24h web-1 2>&1"
        );
        let restart = json!({ "host": "h", "kind": "systemd", "name": "nginx.service" });
        assert_eq!(
            command(method::SERVICES_RESTART, &restart, "deploy").unwrap(),
            "sudo -n systemctl restart nginx.service"
        );
        assert_eq!(
            command(method::SERVICES_RESTART, &restart, "root").unwrap(),
            "systemctl restart nginx.service"
        );
        let user_unit = json!({ "host": "h", "kind": "systemd", "name": "app", "user_unit": true });
        assert_eq!(
            command(method::SERVICES_STOP, &user_unit, "deploy").unwrap(),
            "systemctl --user stop app"
        );
        let container = json!({ "host": "h", "kind": "docker", "name": "web", "sudo": true });
        assert_eq!(
            command(method::SERVICES_START, &container, "deploy").unwrap(),
            "sudo -n docker start web"
        );
        let kill = json!({ "host": "h", "pid": 4242, "signal": "sigkill" });
        assert_eq!(
            command(method::PROCESSES_KILL, &kill, "deploy").unwrap(),
            "kill -KILL 4242"
        );
    }

    #[test]
    fn rejects_unsafe_input() {
        for name in ["", "-x", "a;rm -rf /", "a b", "$(id)", "a'b"] {
            let params = json!({ "host": "h", "kind": "systemd", "name": name });
            assert!(
                command(method::SERVICES_RESTART, &params, "u").is_err(),
                "{name}"
            );
        }
        let bad_signal = json!({ "host": "h", "pid": 10, "signal": "TERM; id" });
        assert!(command(method::PROCESSES_KILL, &bad_signal, "u").is_err());
        let init = json!({ "host": "h", "pid": 1 });
        assert!(command(method::PROCESSES_KILL, &init, "u").is_err());
        let since = json!({ "host": "h", "kind": "docker", "name": "w", "since": "1w; id" });
        assert!(command(method::SERVICES_LOGS, &since, "u").is_err());
    }
}
