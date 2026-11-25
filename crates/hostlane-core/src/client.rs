//! Client for the daemon socket, used by the CLI and (later) the MCP server.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hostlane_protocol::{
    Approval, ClientInfo, ErrorKind, Event, Notification, Request, Response, RpcError, method,
};
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

/// What a person answered when asked about an approval in the client itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Approve,
    Deny,
    /// Leave it to the Hostlane app (or `hostlane approve`) and wait.
    Wait,
}

/// Asks a person about a call that needs approval; runs on a blocking thread.
pub type Approver = Arc<dyn Fn(&Approval) -> Answer + Send + Sync>;

pub struct Client {
    socket_path: PathBuf,
    info: Option<ClientInfo>,
    token: Option<String>,
    approval: serde_json::Map<String, Value>,
    approver: Option<Approver>,
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
            token: None,
            approval: serde_json::Map::new(),
            approver: None,
        }
    }

    /// Lets a person at this client decide approvals for its own calls (an interactive
    /// CLI), instead of only waiting for the app. Ignored with `async` or `ticket_id`.
    pub fn with_approver(mut self, approver: Approver) -> Self {
        self.approver = Some(approver);
        self
    }

    /// Approval options (`ticket_id`, `async`, `approval_timeout_secs`) added to the
    /// params of every call the policy checks (see [`method::GATED`]).
    pub fn with_approval(mut self, options: serde_json::Map<String, Value>) -> Self {
        self.approval = options;
        self
    }

    /// Identifies this client in the audit log and session monitor.
    pub fn with_info(mut self, info: ClientInfo) -> Self {
        self.info = Some(info);
        self
    }

    /// Client token from `clients.create`, sent with every request (`HOSTLANE_TOKEN`).
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        self
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn request(&self, method: &str, mut params: Value) -> Result<String, ClientError> {
        if method::GATED.contains(&method)
            && let Some(object) = params.as_object_mut()
        {
            for (key, value) in &self.approval {
                object.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
        let mut request = Request::new(1, method, params);
        request.client = self.info.clone();
        request.token = self.token.clone();
        serde_json::to_string(&request).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Sends one request on a fresh connection and returns the raw `result`.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        match &self.approver {
            Some(approver)
                if method::GATED.contains(&method)
                    && !self.approval.contains_key("async")
                    && !self.approval.contains_key("ticket_id") =>
            {
                self.call_asking(method, params, approver.clone()).await
            }
            _ => self.call_once(method, params).await,
        }
    }

    async fn call_asking(
        &self,
        method: &str,
        params: Value,
        approver: Approver,
    ) -> Result<Value, ClientError> {
        let with = |key: &str, value: Value| {
            let mut params = params.clone();
            if let Some(object) = params.as_object_mut() {
                object.insert(key.into(), value);
            }
            params
        };
        let error = match self
            .call_once(method, with("async", Value::Bool(true)))
            .await
        {
            Err(ClientError::Rpc(error)) if error.data.kind == ErrorKind::ApprovalRequired => error,
            other => return other,
        };
        let Some(ticket) = error
            .data
            .details
            .as_ref()
            .and_then(|d| d.get("ticket_id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Err(error.into());
        };
        let approval = self
            .call_once(method::APPROVALS_GET, serde_json::json!({ "id": ticket }))
            .await?;
        let approval: Approval =
            serde_json::from_value(approval).map_err(|e| ClientError::Protocol(e.to_string()))?;
        let answer = tokio::task::spawn_blocking(move || approver(&approval))
            .await
            .unwrap_or(Answer::Wait);
        if answer != Answer::Wait {
            self.call_once(
                method::APPROVALS_RESOLVE,
                serde_json::json!({ "id": ticket, "approve": answer == Answer::Approve }),
            )
            .await?;
        }
        self.call_once(method, with("ticket_id", Value::String(ticket)))
            .await
    }

    async fn call_once(&self, method: &str, params: Value) -> Result<Value, ClientError> {
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

    #[cfg(any(unix, windows))]
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
        #[cfg(unix)]
        let stream = tokio::net::UnixStream::connect(&self.socket_path).await;
        #[cfg(windows)]
        let stream = crate::windows_pipe::connect(&self.socket_path).await;
        let stream = stream.map_err(|err| match err.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                ClientError::NotRunning(self.socket_path.display().to_string())
            }
            _ => ClientError::Io(err),
        })?;
        let (reader, mut writer) = tokio::io::split(stream);
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok((Box::new(BufReader::new(reader)), Box::new(writer)))
    }

    #[cfg(not(any(unix, windows)))]
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
