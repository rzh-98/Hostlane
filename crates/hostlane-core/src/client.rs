//! Client for the daemon socket, used by the CLI and (later) the MCP server.

use std::io;
use std::path::{Path, PathBuf};

use hostlane_protocol::{ClientInfo, Event, Notification, Request, Response, RpcError, method};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Hostlane daemon is not running (nothing is listening on {0})")]
    NotRunning(String),
    #[error("daemon connection failed: {0}")]
    Io(#[from] io::Error),
    #[error("invalid response from daemon: {0}")]
    Protocol(String),
    #[error(transparent)]
    Rpc(#[from] RpcError),
}

pub struct Client {
    socket_path: PathBuf,
    info: Option<ClientInfo>,
}

/// An open `events.subscribe` connection.
pub struct Subscription {
    reader: Box<dyn tokio::io::AsyncBufRead + Unpin + Send>,
    _writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
}

impl Subscription {
    /// Next event, or `None` when the daemon closes the connection.
    pub async fn next(&mut self) -> Result<Option<Event>, ClientError> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line).await? == 0 {
                return Ok(None);
            }
            let Ok(notification) = serde_json::from_str::<Notification>(&line) else {
                continue;
            };
            if notification.method == method::EVENT_NOTIFICATION
                && let Ok(event) = serde_json::from_value(notification.params)
            {
                return Ok(Some(event));
            }
        }
    }
}

impl Client {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            info: None,
        }
    }

    /// Identifies this client in the audit log and session monitor.
    pub fn with_info(mut self, info: ClientInfo) -> Self {
        self.info = Some(info);
        self
    }

    fn request(&self, method: &str, params: Value) -> Result<String, ClientError> {
        let mut request = Request::new(1, method, params);
        request.client = self.info.clone();
        serde_json::to_string(&request).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Sends one request on a fresh connection and returns the raw `result`.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let line = self.request(method, params)?;
        let reply = self.round_trip(&line).await?;
        let response: Response =
            serde_json::from_str(&reply).map_err(|e| ClientError::Protocol(e.to_string()))?;
        match (response.result, response.error) {
            (_, Some(error)) => Err(error.into()),
            (Some(result), None) => Ok(result),
            (None, None) => Ok(Value::Null),
        }
    }

    pub async fn call_as<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, ClientError> {
        let value = self.call(method, params).await?;
        serde_json::from_value(value).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    /// Opens a connection that receives every daemon event from now on.
    pub async fn subscribe(&self) -> Result<Subscription, ClientError> {
        let line = self.request(method::EVENTS_SUBSCRIBE, Value::Null)?;
        let (mut reader, writer) = self.send(&line).await?;
        let reply = read_reply(&mut reader).await?;
        let response: Response =
            serde_json::from_str(&reply).map_err(|e| ClientError::Protocol(e.to_string()))?;
        if let Some(error) = response.error {
            return Err(error.into());
        }
        Ok(Subscription {
            reader,
            _writer: writer,
        })
    }

    #[cfg(unix)]
    async fn send(
        &self,
        line: &str,
    ) -> Result<
        (
            Box<dyn tokio::io::AsyncBufRead + Unpin + Send>,
            Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        ),
        ClientError,
    > {
        let stream = tokio::net::UnixStream::connect(&self.socket_path)
            .await
            .map_err(|err| match err.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                    ClientError::NotRunning(self.socket_path.display().to_string())
                }
                _ => ClientError::Io(err),
            })?;
        let (reader, mut writer) = stream.into_split();
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok((Box::new(BufReader::new(reader)), Box::new(writer)))
    }

    #[cfg(not(unix))]
    async fn send(
        &self,
        _line: &str,
    ) -> Result<
        (
            Box<dyn tokio::io::AsyncBufRead + Unpin + Send>,
            Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        ),
        ClientError,
    > {
        Err(ClientError::NotRunning(
            self.socket_path.display().to_string(),
        ))
    }

    async fn round_trip(&self, line: &str) -> Result<String, ClientError> {
        let (mut reader, _writer) = self.send(line).await?;
        read_reply(&mut reader).await
    }
}

async fn read_reply(
    reader: &mut (dyn tokio::io::AsyncBufRead + Unpin + Send),
) -> Result<String, ClientError> {
    let mut reply = String::new();
    reader.read_line(&mut reply).await?;
    if reply.is_empty() {
        return Err(ClientError::Protocol(
            "connection closed without a response".into(),
        ));
    }
    Ok(reply)
}
