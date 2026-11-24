//! One function per JSON-RPC method. Handlers return a JSON value or a typed error;
//! framing and request parsing live in `server.rs`. Calls that touch a server are
//! appended to the audit log.

use hostlane_protocol::{
    AuditEntry, AuditListParams, AuditOutcome, ClientInfo, ConnectionsList, ErrorKind, Event,
    ExecList, ExecListParams, ExecParams, FileChangeParams, HostParams, HostsList, RpcError,
    TrustParams, TrustResult, TunnelParams, method,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::Daemon;
use crate::exec;
use crate::files;
use crate::ports;
use crate::ssh::Trust;
use crate::store::NewCall;
use crate::util::{now_ms, sha256_hex};

/// Methods recorded in the audit log.
const AUDITED: &[&str] = &[
    method::HOSTS_CONNECT,
    method::HOSTS_DISCONNECT,
    method::HOSTS_TRUST,
    method::EXEC_RUN,
    method::FILES_READ,
    method::FILES_LIST,
    method::FILES_SEARCH,
    method::FILES_WRITE,
    method::FILES_PATCH,
    method::FILES_UPLOAD,
    method::FILES_DOWNLOAD,
    method::FILES_REVERT,
    method::PORTS_LIST,
    method::PORTS_INSPECT,
    method::PORTS_CHECK,
    method::PORTS_FIND_FREE,
    method::PORTS_ANNOTATE,
    method::TUNNELS_OPEN,
    method::TUNNELS_CLOSE,
    method::TUNNELS_START,
    method::TUNNELS_UPDATE,
    method::TUNNELS_DELETE,
];

pub(crate) async fn dispatch(
    daemon: &Daemon,
    client: &ClientInfo,
    name: &str,
    params: Value,
) -> Result<Value, RpcError> {
    if !AUDITED.contains(&name) {
        return run(daemon, client, name, params).await;
    }
    let started_at = now_ms();
    let host = params
        .get("host")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let recorded_params = redact_params(&params);
    let result = run(daemon, client, name, params).await;
    let host = host.or_else(|| {
        let value = result.as_ref().ok()?;
        let host = value
            .get("host")
            .or_else(|| value.pointer("/tunnel/host"))?;
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
fn redact_params(params: &Value) -> Value {
    let mut params = exec::redact_params(params);
    let Some(object) = params.as_object_mut() else {
        return params;
    };
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

/// The audited result without bulky output: command output and file content are
/// dropped, listings, search results and port tables are replaced by their counts.
fn summarize(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("stdout");
        object.remove("stderr");
        object.remove("content");
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
) -> Result<Value, RpcError> {
    match name {
        method::DAEMON_STATUS => to_value(daemon.status()),
        method::DAEMON_SHUTDOWN => {
            daemon.request_shutdown();
            Ok(Value::Null)
        }
        method::HOSTS_LIST => {
            let loaded = daemon.load_hosts().await?;
            to_value(HostsList {
                hosts: loaded.hosts(),
                warnings: loaded.warnings,
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
            to_value(exec::run(daemon, client, params, &entry, &loaded).await?)
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

        let result = json!({ "content": "x", "sha256": "ab", "matches": [1, 2] });
        let summary = summarize(&result);
        assert!(summary.get("content").is_none());
        assert_eq!(summary["match_count"], 2);
        assert_eq!(summary["sha256"], "ab");

        let ports = summarize(&json!({ "ports": [{ "port": 22 }] }));
        assert_eq!(ports["port_count"], 1);
        let free = summarize(&json!({ "ports": [8000, 8001] }));
        assert_eq!(free["ports"], json!([8000, 8001]));
    }
}
