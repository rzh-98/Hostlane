//! One function per JSON-RPC method. Handlers return a JSON value or a typed error;
//! framing and request parsing live in `server.rs`. Calls that touch a server are
//! appended to the audit log, after the policy has let them through.

use hostlane_protocol::{
    ApprovalsList, ApprovalsListParams, AuditEntry, AuditListParams, AuditOutcome, ClientInfo,
    ClientNameParams, ClientsConfigureParams, ConnectionsList, Decision, ErrorKind, Event,
    ExecList, ExecListParams, ExecParams, FileChangeParams, HostLabelParams, HostParams, HostsList,
    IdParams, JobParams, PolicyCheck, PolicyCheckParams, PolicyRules, PolicyStopParams,
    PolicyStopped, Risk, RpcError, TrustParams, TrustResult, TunnelParams, UNTRUSTED_MEMBER,
    method, untrusted_fields,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::Daemon;
use crate::exec;
use crate::files;
use crate::hosts;
use crate::policy::{self, GateOptions, require_person};
use crate::ports;
use crate::services;
use crate::ssh::Trust;
use crate::store::NewCall;
use crate::util::{now_ms, sha256_hex};

/// Policy, host and credential changes, recorded in the audit log along with the
/// gated methods.
const POLICY_CHANGES: &[&str] = &[
    method::HOSTS_LABEL,
    method::HOSTS_ADD,
    method::HOSTS_UPDATE,
    method::HOSTS_REMOVE,
    method::SECRETS_SET,
    method::SECRETS_DELETE,
    method::CLIENTS_CREATE,
    method::CLIENTS_REVOKE,
    method::CLIENTS_CONFIGURE,
    method::APPROVALS_RESOLVE,
    method::POLICY_STOP,
    method::POLICY_RESUME,
    method::POLICY_RULES_ADD,
    method::POLICY_RULES_DELETE,
];

/// Runs a call and lists the result fields that hold text from the server (S6).
pub(crate) async fn dispatch(
    daemon: &Daemon,
    client: &ClientInfo,
    name: &str,
    params: Value,
) -> Result<Value, RpcError> {
    let mut result = audited(daemon, client, name, params).await;
    let fields = untrusted_fields(name);
    if !fields.is_empty()
        && let Ok(Value::Object(object)) = &mut result
    {
        object.insert(UNTRUSTED_MEMBER.into(), json!(fields));
    }
    result
}

async fn audited(
    daemon: &Daemon,
    client: &ClientInfo,
    name: &str,
    mut params: Value,
) -> Result<Value, RpcError> {
    let gated = method::GATED.contains(&name);
    if !gated && !POLICY_CHANGES.contains(&name) {
        return run(daemon, client, name, params, None).await;
    }
    let options = if gated {
        GateOptions::take(&mut params)?
    } else {
        GateOptions::default()
    };
    let started_at = now_ms();
    let host = params
        .get("host")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let recorded_params = redact_params(&params);
    let (risk, approver, result) = if let Err(err) = services::validate(name, &params) {
        (None, None, Err(err))
    } else if gated {
        let gate = policy::authorize(daemon, client, name, &params, options).await;
        match gate.result {
            Ok(approver) => {
                let result = run(daemon, client, name, params, approver.clone()).await;
                (gate.risk, approver, result)
            }
            Err(err) => (gate.risk, None, Err(err)),
        }
    } else {
        (None, None, run(daemon, client, name, params, None).await)
    };
    let host = host.or_else(|| {
        let value = result.as_ref().ok()?;
        let host = value
            .get("host")
            .or_else(|| value.pointer("/tunnel/host"))
            .or_else(|| value.pointer("/job/host"))?;
        host.as_str().map(str::to_owned)
    });
    let (outcome, error, summary) = match &result {
        Ok(value) => (AuditOutcome::Ok, None, summarize(value)),
        Err(err) => (
            AuditOutcome::Error,
            Some(err.clone()),
            err.data.details.clone().unwrap_or(Value::Null),
        ),
    };
    let call = NewCall {
        started_at,
        finished_at: now_ms(),
        client: client.clone(),
        host,
        method: name.to_string(),
        params: recorded_params,
        outcome,
        error,
        result: summary,
        approver,
        risk,
    };
    let entry = call.entry(0);
    match daemon.store.append_call(call).await {
        Ok(id) => daemon.events.publish(Event::AuditAppended {
            entry: AuditEntry { id, ..entry },
        }),
        Err(err) => tracing::error!(%err, method = name, "cannot write the audit log"),
    }
    result
}

/// Audited params: file content and edits are replaced by their size; the diff of a
/// change is kept in the `file_changes` table instead.
pub(crate) fn redact_params(params: &Value) -> Value {
    let mut params = exec::redact_params(params);
    let Some(object) = params.as_object_mut() else {
        return params;
    };
    if object.contains_key("secret") {
        object.insert("secret".into(), json!("<redacted>"));
    }
    if let Some(content) = object.get("content").and_then(Value::as_str) {
        let summary = json!({ "bytes": content.len(), "sha256": sha256_hex(content.as_bytes()) });
        object.insert("content".into(), summary);
    }
    if let Some(edits) = object.get("edits").and_then(Value::as_array) {
        let summary = json!({ "count": edits.len() });
        object.insert("edits".into(), summary);
    }
    if let Some(diff) = object.get("diff").and_then(Value::as_str) {
        let summary = json!({ "bytes": diff.len() });
        object.insert("diff".into(), summary);
    }
    params
}

/// The audited result without bulky output or tokens: command output, file content
/// and new client tokens are dropped, listings, search results and port tables are
/// replaced by their counts.
fn summarize(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        for key in ["stdout", "stderr", "content", "data", "token"] {
            object.remove(key);
        }
        for (key, count) in [
            ("entries", "entry_count"),
            ("matches", "match_count"),
            ("ports", "port_count"),
        ] {
            let Some(items) = object.get(key).and_then(Value::as_array) else {
                continue;
            };
            if key == "ports" && items.iter().all(Value::is_number) {
                continue;
            }
            let len = items.len();
            object.remove(key);
            object.insert(count.into(), json!(len));
        }
    }
    value
}

async fn run(
    daemon: &Daemon,
    client: &ClientInfo,
    name: &str,
    params: Value,
    approver: Option<String>,
) -> Result<Value, RpcError> {
    match name {
        method::DAEMON_STATUS => to_value(daemon.status()),
        method::DAEMON_SHUTDOWN => {
            daemon.request_shutdown();
            Ok(Value::Null)
        }
        method::HOSTS_LIST => {
            let loaded = daemon.load_hosts().await?;
            let hosts = loaded.hosts();
            let manual = daemon
                .store
                .manual_hosts()
                .await?
                .into_iter()
                .map(|mut params| {
                    if let Some(host) = hosts.iter().find(|h| h.id == params.id) {
                        params.env = host.env;
                        params.tags = host.tags.clone();
                    }
                    params
                })
                .collect();
            to_value(HostsList {
                hosts,
                warnings: loaded.warnings,
                manual,
            })
        }
        method::HOSTS_CONNECT => {
            let params: HostParams = parse(params)?;
            let (entry, loaded) = daemon.resolve_host(&params.host).await?;
            daemon.pool.connect(&entry, &loaded, Trust::Strict).await?;
            connection(daemon, &params.host)
        }
        method::HOSTS_DISCONNECT => {
            let params: HostParams = parse(params)?;
            let disconnected = daemon.pool.disconnect(&params.host).await;
            Ok(json!({ "disconnected": disconnected }))
        }
        method::HOSTS_TRUST => {
            let params: TrustParams = parse(params)?;
            let fingerprint = params.fingerprint.trim().to_string();
            if !fingerprint.starts_with("SHA256:") {
                return Err(RpcError::new(
                    ErrorKind::InvalidParams,
                    "`fingerprint` must look like SHA256:…",
                ));
            }
            let (entry, loaded) = daemon.resolve_host(&params.host).await?;
            let (_, host_key, added) = daemon
                .pool
                .connect(&entry, &loaded, Trust::Accept(fingerprint.clone()))
                .await?;
            if host_key.fingerprint != fingerprint {
                return Err(RpcError::new(
                    ErrorKind::InvalidParams,
                    format!(
                        "`{}` is already trusted with a different key ({})",
                        params.host, host_key.fingerprint
                    ),
                )
                .with_details(host_key));
            }
            to_value(TrustResult {
                host_key,
                added,
                known_hosts_path: daemon
                    .pool
                    .locations()
                    .hostlane_known_hosts
                    .display()
                    .to_string(),
            })
        }
        method::CONNECTIONS_LIST => to_value(ConnectionsList {
            connections: daemon.pool.list(),
        }),
        method::EXEC_RUN => {
            let params: ExecParams = parse(params)?;
            let (entry, loaded) = daemon.resolve_host(&params.host).await?;
            to_value(exec::run(daemon, client, params, &entry, &loaded, approver).await?)
        }
        method::EXEC_LIST => {
            let params: ExecListParams = parse_or_default(params)?;
            to_value(ExecList {
                execs: daemon.store.list_execs(params).await?,
            })
        }
        method::OUTPUT_READ => to_value(exec::read_output(daemon, parse(params)?).await?),
        method::FILES_READ => to_value(files::read(daemon, parse(params)?).await?),
        method::FILES_LIST => to_value(files::list(daemon, parse(params)?).await?),
        method::FILES_SEARCH => to_value(files::search(daemon, parse(params)?).await?),
        method::FILES_WRITE => to_value(files::write(daemon, client, parse(params)?).await?),
        method::FILES_PATCH => to_value(files::patch(daemon, client, parse(params)?).await?),
        method::FILES_UPLOAD => to_value(files::upload(daemon, client, parse(params)?).await?),
        method::FILES_DOWNLOAD => to_value(files::download(daemon, parse(params)?).await?),
        method::FILES_CHANGE => {
            let params: FileChangeParams = parse(params)?;
            to_value(files::get_change(daemon, &params.change_id).await?)
        }
        method::FILES_REVERT => {
            let params: FileChangeParams = parse(params)?;
            to_value(files::revert(daemon, client, &params.change_id).await?)
        }
        method::PORTS_LIST => to_value(ports::list(daemon, parse(params)?).await?),
        method::PORTS_INSPECT => to_value(ports::inspect(daemon, parse(params)?).await?),
        method::PORTS_CHECK => to_value(ports::check(daemon, parse(params)?).await?),
        method::PORTS_FIND_FREE => to_value(ports::find_free(daemon, parse(params)?).await?),
        method::PORTS_ANNOTATE => to_value(ports::annotate(daemon, parse(params)?).await?),
        method::TUNNELS_LIST => to_value(daemon.tunnels.list()),
        method::TUNNELS_OPEN => to_value(daemon.tunnels.open(client, parse(params)?).await?),
        method::TUNNELS_CLOSE => to_value(daemon.tunnels.close(parse(params)?).await?),
        method::TUNNELS_START => {
            let params: TunnelParams = parse(params)?;
            to_value(daemon.tunnels.start(&params.id).await?)
        }
        method::TUNNELS_UPDATE => to_value(daemon.tunnels.update(parse(params)?).await?),
        method::TUNNELS_DELETE => {
            let params: TunnelParams = parse(params)?;
            to_value(daemon.tunnels.delete(&params.id).await?)
        }
        method::HOSTS_LABEL => {
            require_person(client, "change host labels")?;
            let params: HostLabelParams = parse(params)?;
            daemon.resolve_host(&params.host).await?;
            let mut tags: Vec<String> = Vec::new();
            for tag in params.tags.iter().map(|t| t.trim()) {
                if !tag.is_empty() && !tags.iter().any(|t| t == tag) {
                    tags.push(tag.to_string());
                }
            }
            daemon
                .store
                .set_host_labels(params.host.clone(), params.env, tags)
                .await?;
            let loaded = daemon.load_hosts().await?;
            let host = loaded
                .find(&params.host)
                .map(|entry| entry.host.clone())
                .ok_or_else(|| RpcError::new(ErrorKind::HostNotFound, "host vanished"))?;
            to_value(host)
        }
        method::HOSTS_ADD => {
            require_person(client, "add hosts")?;
            to_value(hosts::add(daemon, parse(params)?).await?)
        }
        method::HOSTS_UPDATE => {
            require_person(client, "change hosts")?;
            to_value(hosts::update(daemon, parse(params)?).await?)
        }
        method::HOSTS_REMOVE => {
            require_person(client, "remove hosts")?;
            let params: HostParams = parse(params)?;
            to_value(hosts::remove(daemon, &params.host).await?)
        }
        method::SECRETS_LIST => to_value(hosts::list_secrets(daemon).await?),
        method::SECRETS_SET => {
            require_person(client, "save passwords and passphrases")?;
            to_value(hosts::set_secret(daemon, parse(params)?).await?)
        }
        method::SECRETS_DELETE => {
            require_person(client, "delete saved passwords and passphrases")?;
            to_value(hosts::delete_secret(daemon, parse(params)?).await?)
        }
        method::SERVICES_LOGS
        | method::SERVICES_START
        | method::SERVICES_STOP
        | method::SERVICES_RESTART
        | method::PROCESSES_KILL => {
            to_value(services::run(daemon, client, name, params, approver).await?)
        }
        method::JOBS_START => to_value(daemon.jobs.start(client, parse(params)?, approver).await?),
        method::JOBS_LIST => to_value(daemon.jobs.list(parse_or_default(params)?).await?),
        method::JOBS_STATUS => to_value(daemon.jobs.status(parse(params)?).await?),
        method::JOBS_LOGS => to_value(daemon.jobs.logs(parse(params)?).await?),
        method::JOBS_CANCEL => {
            let params: JobParams = parse(params)?;
            to_value(daemon.jobs.cancel(&params.id).await?)
        }
        method::CLIENTS_LIST => to_value(daemon.clients.list().await?),
        method::CLIENTS_CREATE => {
            require_person(client, "register clients")?;
            let params: ClientNameParams = parse(params)?;
            to_value(daemon.clients.create(&params.name).await?)
        }
        method::CLIENTS_REVOKE => {
            require_person(client, "revoke client tokens")?;
            let params: ClientNameParams = parse(params)?;
            daemon.clients.revoke(&params.name).await?;
            Ok(json!({ "revoked": params.name }))
        }
        method::CLIENTS_CONFIGURE => {
            require_person(client, "change client token settings")?;
            let params: ClientsConfigureParams = parse(params)?;
            to_value(daemon.clients.configure(params.require_token).await?)
        }
        method::APPROVALS_LIST => {
            let params: ApprovalsListParams = parse_or_default(params)?;
            to_value(ApprovalsList {
                approvals: daemon.policy.approvals().list(params.pending_only),
            })
        }
        method::APPROVALS_GET => {
            let params: IdParams = parse(params)?;
            let approval = daemon.policy.approvals().get(&params.id).ok_or_else(|| {
                RpcError::new(
                    ErrorKind::ApprovalNotFound,
                    format!("no approval `{}`", params.id),
                )
            })?;
            to_value(approval)
        }
        method::APPROVALS_RESOLVE => {
            require_person(client, "decide approvals")?;
            to_value(daemon.policy.resolve(parse(params)?, client).await?)
        }
        method::POLICY_STATUS => to_value(daemon.policy.status()),
        method::POLICY_STOP => {
            let params: PolicyStopParams = parse_or_default(params)?;
            let denied_approvals = daemon.policy.stop(client, params.reason).await?;
            let (closed_tunnels, cancelled_jobs) = tokio::join!(
                daemon.tunnels.close_agent_tunnels(),
                daemon.jobs.cancel_agent_jobs()
            );
            to_value(PolicyStopped {
                status: daemon.policy.status(),
                closed_tunnels,
                denied_approvals,
                cancelled_jobs,
            })
        }
        method::POLICY_RESUME => {
            require_person(client, "resume after an emergency stop")?;
            to_value(daemon.policy.resume().await?)
        }
        method::POLICY_CHECK => {
            let mut params: PolicyCheckParams = parse(params)?;
            if !method::GATED.contains(&params.method.as_str()) {
                return to_value(PolicyCheck {
                    decision: Decision::Allow,
                    risk: Risk::L0,
                    reasons: Vec::new(),
                    matched: "not gated".into(),
                    stopped: false,
                });
            }
            GateOptions::take(&mut params.params)?;
            to_value(policy::check(daemon, client, params).await)
        }
        method::POLICY_RULES_LIST => to_value(PolicyRules {
            rules: daemon.policy.rules(),
        }),
        method::POLICY_RULES_ADD => {
            require_person(client, "change policy rules")?;
            to_value(
                daemon
                    .policy
                    .add_rule(parse(params)?, client.name.clone())
                    .await?,
            )
        }
        method::POLICY_RULES_DELETE => {
            require_person(client, "change policy rules")?;
            let params: IdParams = parse(params)?;
            daemon.policy.delete_rule(&params.id).await?;
            Ok(json!({ "deleted": params.id }))
        }
        method::AUDIT_LIST => {
            let params: AuditListParams = parse_or_default(params)?;
            to_value(daemon.store.list_calls(params).await?)
        }
        method::EVENTS_SUBSCRIBE => Err(RpcError::new(
            ErrorKind::InvalidRequest,
            "events.subscribe must be the only request on its connection",
        )),
        _ => Err(RpcError::new(
            ErrorKind::MethodNotFound,
            format!("unknown method `{name}`"),
        )
        .with_hint(format!("available methods: {}", method::ALL.join(", ")))),
    }
}

fn connection(daemon: &Daemon, host: &str) -> Result<Value, RpcError> {
    let connection = daemon
        .pool
        .list()
        .into_iter()
        .find(|c| c.host == host)
        .ok_or_else(|| RpcError::new(ErrorKind::Internal, "connection state missing"))?;
    to_value(connection)
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    serde_json::from_value(params)
        .map_err(|err| RpcError::new(ErrorKind::InvalidParams, format!("invalid params: {err}")))
}

fn parse_or_default<T: DeserializeOwned + Default>(params: Value) -> Result<T, RpcError> {
    if params.is_null() {
        Ok(T::default())
    } else {
        parse(params)
    }
}

fn to_value(value: impl Serialize) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_drops_file_content() {
        let params = json!({ "host": "h", "path": "/a", "content": "secret", "edits": [{}, {}] });
        let recorded = redact_params(&params);
        assert_eq!(recorded["content"]["bytes"], 6);
        assert_eq!(recorded["edits"]["count"], 2);
        assert!(!recorded.to_string().contains("secret"));

        let secret =
            redact_params(&json!({ "host": "h", "kind": "password", "secret": "hunter2" }));
        assert_eq!(secret["secret"], "<redacted>");
        assert!(!secret.to_string().contains("hunter2"));

        let result = json!({ "content": "x", "sha256": "ab", "matches": [1, 2] });
        let summary = summarize(&result);
        assert!(summary.get("content").is_none());
        assert_eq!(summary["match_count"], 2);
        assert_eq!(summary["sha256"], "ab");

        let ports = summarize(&json!({ "ports": [{ "port": 22 }] }));
        assert_eq!(ports["port_count"], 1);
        let free = summarize(&json!({ "ports": [8000, 8001] }));
        assert_eq!(free["ports"], json!([8000, 8001]));

        let created = summarize(&json!({ "client": { "name": "a" }, "token": "hlt_x" }));
        assert!(created.get("token").is_none());
        let logs = summarize(&json!({ "job": { "host": "h" }, "data": "out" }));
        assert!(logs.get("data").is_none());
    }
}
