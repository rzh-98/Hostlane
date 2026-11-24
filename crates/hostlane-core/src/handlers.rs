//! One function per JSON-RPC method. Handlers return a JSON value or a typed error;
//! framing and request parsing live in `server.rs`.

use hostlane_protocol::{ErrorKind, HostsList, RpcError, method};
use serde::Serialize;
use serde_json::Value;

use crate::{Daemon, ssh_config};

pub(crate) async fn dispatch(
    daemon: &Daemon,
    name: &str,
    _params: Value,
) -> Result<Value, RpcError> {
    match name {
        method::DAEMON_STATUS => to_value(daemon.status()),
        method::DAEMON_SHUTDOWN => {
            daemon.request_shutdown();
            Ok(Value::Null)
        }
        method::HOSTS_LIST => hosts_list(daemon).await,
        _ => Err(RpcError::new(
            ErrorKind::MethodNotFound,
            format!("unknown method `{name}`"),
        )
        .with_hint(format!("available methods: {}", method::ALL.join(", ")))),
    }
}

async fn hosts_list(daemon: &Daemon) -> Result<Value, RpcError> {
    let path = daemon.ssh_config_path.clone();
    let loaded = tokio::task::spawn_blocking(move || ssh_config::load(&path))
        .await
        .map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))?;
    to_value(HostsList {
        hosts: loaded.hosts,
        warnings: loaded.warnings,
    })
}

fn to_value(value: impl Serialize) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))
}
