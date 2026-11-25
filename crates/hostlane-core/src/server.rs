//! Local socket server: newline-delimited JSON-RPC, one task per connection.

use std::io;
use std::path::Path;
use std::sync::Arc;

use hostlane_protocol::{
    ClientInfo, ErrorKind, Event, Notification, Request, Response, RpcError, Subscribed, method,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::broadcast::error::RecvError;

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

fn parse_request(line: &str) -> Result<Request, Box<Response>> {
    match serde_json::from_str::<Request>(line) {
        Ok(request) if request.jsonrpc != "2.0" => Err(Box::new(Response::err(
            request.id,
            RpcError::new(ErrorKind::InvalidRequest, "jsonrpc must be \"2.0\""),
        ))),
        Ok(request) => Ok(request),
        Err(err) => Err(Box::new(Response::err(
            Value::Null,
            RpcError::new(ErrorKind::ParseError, format!("invalid request: {err}")),
        ))),
    }
}

async fn handle_request(daemon: &Daemon, request: Request) -> Response {
    let client = request.client.unwrap_or_default();
    let client = match daemon
        .clients
        .authenticate(client, request.token.as_deref(), &request.method)
        .await
    {
        Ok(client) => client,
        Err(error) => return Response::err(request.id, error),
    };
    match handlers::dispatch(daemon, &client, &request.method, request.params).await {
        Ok(result) => Response::ok(request.id, result),
        Err(error) => Response::err(request.id, error),
    }
}

fn to_line(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).expect("messages always serialize")
}

/// Handles one request line and returns the response line (without the newline).
pub async fn handle_line(daemon: &Daemon, line: &str) -> String {
    let response = match parse_request(line) {
        Ok(request) => handle_request(daemon, request).await,
        Err(response) => *response,
    };
    to_line(&response)
}

/// Streams events until the client hangs up. The client sends nothing more.
async fn subscribe<R, W>(
    daemon: &Daemon,
    client: Option<ClientInfo>,
    id: Value,
    reader: &mut R,
    writer: &mut W,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut events = daemon.events.subscribe();
    let _listener = client
        .as_ref()
        .filter(|client| crate::policy::is_desktop(client))
        .map(|_| daemon.policy.desktop_listener());
    let ack = serde_json::to_value(Subscribed { subscribed: true }).unwrap_or(Value::Null);
    write_line(writer, &to_line(&Response::ok(id, ack))).await?;
    let mut sink = [0u8; 256];
    loop {
        let event = tokio::select! {
            received = events.recv() => match received {
                Ok(event) => event,
                Err(RecvError::Lagged(skipped)) => Event::Lagged { skipped },
                Err(RecvError::Closed) => return Ok(()),
            },
            read = reader.read(&mut sink) => match read {
                Ok(0) | Err(_) => return Ok(()),
                Ok(_) => continue,
            },
        };
        let params = serde_json::to_value(&event).unwrap_or(Value::Null);
        let notification = Notification::new(method::EVENT_NOTIFICATION, params);
        write_line(writer, &to_line(&notification)).await?;
    }
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
        let response = match parse_request(trimmed) {
            Ok(request) if request.method == method::EVENTS_SUBSCRIBE => {
                let client = request.client.unwrap_or_default();
                match daemon
                    .clients
                    .authenticate(client, request.token.as_deref(), &request.method)
                    .await
                {
                    Ok(client) => {
                        return subscribe(
                            &daemon,
                            Some(client),
                            request.id,
                            &mut reader,
                            &mut writer,
                        )
                        .await;
                    }
                    Err(error) => Response::err(request.id, error),
                }
            }
            Ok(request) => handle_request(&daemon, request).await,
            Err(response) => *response,
        };
        write_line(&mut writer, &to_line(&response)).await?;
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
    daemon.start().await;
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
    daemon.close().await;
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
        let data_dir = std::env::temp_dir().join(format!(
            "hostlane-server-test-{}-{}",
            std::process::id(),
            crate::util::new_id("t")
        ));
        Daemon::open(crate::DaemonOptions {
            socket_path: data_dir.join("hostlane.sock"),
            data_dir,
            ssh_config_path: "/nonexistent/config".into(),
        })
        .unwrap()
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
