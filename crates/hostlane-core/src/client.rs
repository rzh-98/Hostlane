//! Client for the daemon socket, used by the CLI and (later) the MCP server.

use std::io;
use std::path::{Path, PathBuf};

use hostlane_protocol::{Request, Response, RpcError};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Hostlane daemon is not running (no socket at {0})")]
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
}

impl Client {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Sends one request on a fresh connection and returns the raw `result`.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let request = Request::new(1, method, params);
        let line =
            serde_json::to_string(&request).map_err(|e| ClientError::Protocol(e.to_string()))?;
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

    #[cfg(unix)]
    async fn round_trip(&self, line: &str) -> Result<String, ClientError> {
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
        let mut reply = String::new();
        BufReader::new(reader).read_line(&mut reply).await?;
        if reply.is_empty() {
            return Err(ClientError::Protocol(
                "connection closed without a response".into(),
            ));
        }
        Ok(reply)
    }

    #[cfg(not(unix))]
    async fn round_trip(&self, _line: &str) -> Result<String, ClientError> {
        Err(ClientError::NotRunning(
            self.socket_path.display().to_string(),
        ))
    }
}
