//! Local socket server: newline-delimited JSON-RPC, one task per connection.

use std::io;
use std::path::Path;
use std::sync::Arc;

use hostlane_protocol::{ErrorKind, Request, Response, RpcError};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::Daemon;
use crate::handlers;

/// Largest request line accepted, so a misbehaving client cannot exhaust memory.
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("another Hostlane daemon is already listening on {0}")]
    AlreadyRunning(String),
    #[error("local sockets are not supported on this platform yet")]
    Unsupported,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Handles one request line and returns the response line (without the newline).
pub async fn handle_line(daemon: &Daemon, line: &str) -> String {
    let response = match serde_json::from_str::<Request>(line) {
        Ok(request) if request.jsonrpc != "2.0" => Response::err(
            request.id,
            RpcError::new(ErrorKind::InvalidRequest, "jsonrpc must be \"2.0\""),
        ),
        Ok(request) => {
            let id = request.id.clone();
            match handlers::dispatch(daemon, &request.method, request.params).await {
                Ok(result) => Response::ok(id, result),
                Err(error) => Response::err(id, error),
            }
        }
        Err(err) => Response::err(
            Value::Null,
            RpcError::new(ErrorKind::ParseError, format!("invalid request: {err}")),
        ),
    };
    serde_json::to_string(&response).expect("responses always serialize")
}

async fn serve_stream<S>(daemon: Arc<Daemon>, stream: S) -> io::Result<()>
where
    S: tokio::io::AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        let read = (&mut reader)
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_line(&mut line)
            .await?;
        if read == 0 {
            return Ok(());
        }
        if read > MAX_LINE_BYTES {
            let error = RpcError::new(ErrorKind::InvalidRequest, "request line too long");
            write_line(
                &mut writer,
                &serde_json::to_string(&Response::err(Value::Null, error))?,
            )
            .await?;
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response = handle_line(&daemon, trimmed).await;
        write_line(&mut writer, &response).await?;
    }
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, line: &str) -> io::Result<()> {
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

/// Runs the daemon until `daemon.shutdown`, Ctrl-C or SIGTERM.
#[cfg(unix)]
pub async fn serve(daemon: Arc<Daemon>) -> Result<(), ServeError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use tokio::net::{UnixListener, UnixStream};

    let path = daemon.socket_path.clone();
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    if path.exists() {
        if UnixStream::connect(&path).await.is_ok() {
            return Err(ServeError::AlreadyRunning(path.display().to_string()));
        }
        std::fs::remove_file(&path)?;
    }

    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!(socket = %path.display(), "daemon listening");

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let daemon = daemon.clone();
                    tokio::spawn(async move {
                        if let Err(err) = serve_stream(daemon, stream).await {
                            tracing::debug!(%err, "connection closed with error");
                        }
                    });
                }
                Err(err) => break Err(err.into()),
            },
            _ = daemon.shutdown.notified() => break Ok(()),
            _ = tokio::signal::ctrl_c() => break Ok(()),
            _ = sigterm.recv() => break Ok(()),
        }
    };

    remove_socket(&path);
    tracing::info!("daemon stopped");
    result
}

#[cfg(not(unix))]
pub async fn serve(_daemon: Arc<Daemon>) -> Result<(), ServeError> {
    Err(ServeError::Unsupported)
}

fn remove_socket(path: &Path) {
    if let Err(err) = std::fs::remove_file(path)
        && err.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(%err, "could not remove socket file");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daemon() -> Daemon {
        Daemon::new("/tmp/hostlane-test.sock".into()).with_ssh_config("/nonexistent/config".into())
    }

    fn parse(line: &str) -> Value {
        serde_json::from_str(line).unwrap()
    }

    #[tokio::test]
    async fn answers_status_with_matching_id() {
        let reply = parse(
            &handle_line(
                &daemon(),
                r#"{"jsonrpc":"2.0","id":7,"method":"daemon.status"}"#,
            )
            .await,
        );
        assert_eq!(reply["id"], 7);
        assert_eq!(
            reply["result"]["protocol"],
            hostlane_protocol::PROTOCOL_VERSION
        );
    }

    #[tokio::test]
    async fn reports_typed_errors() {
        let reply =
            parse(&handle_line(&daemon(), r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#).await);
        assert_eq!(reply["error"]["data"]["kind"], "METHOD_NOT_FOUND");
        assert_eq!(reply["error"]["code"], -32601);

        let reply = parse(&handle_line(&daemon(), "not json").await);
        assert_eq!(reply["error"]["data"]["kind"], "PARSE_ERROR");
    }

    #[tokio::test]
    async fn lists_no_hosts_without_config() {
        let reply = parse(
            &handle_line(
                &daemon(),
                r#"{"jsonrpc":"2.0","id":2,"method":"hosts.list"}"#,
            )
            .await,
        );
        assert_eq!(reply["result"]["hosts"], serde_json::json!([]));
    }
}
