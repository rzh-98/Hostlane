//! `exec.run`: runs one command on a pooled connection, streams its output to the
//! session monitor, stores the full output and returns head and tail.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hostlane_protocol::{
    ClientInfo, ErrorKind, Event, ExecParams, ExecRecord, ExecResult, ExecStatus, OutputChunk,
    OutputReadParams, OutputStream, RpcError,
};
use russh::{ChannelMsg, Sig};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::Daemon;
use crate::pool::Lease;
use crate::ssh_config::{Entry, Loaded};
use crate::util::{
    Utf8Chunker, new_id, now_ms, sh_path, sh_quote, utf8_prefix_len, utf8_skip_continuation,
};

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const MAX_TIMEOUT_MS: u64 = 6 * 60 * 60 * 1000;
pub const DEFAULT_MAX_OUTPUT: u64 = 16 * 1024;
const MIN_MAX_OUTPUT: u64 = 256;
const MAX_MAX_OUTPUT: u64 = 1024 * 1024;
/// Bytes per stream kept on disk for `output.read`.
const STORE_CAP: u64 = 64 * 1024 * 1024;
/// Bytes per stream pushed live to the session monitor.
const LIVE_CAP: u64 = 1024 * 1024;
const TERM_GRACE: Duration = Duration::from_secs(2);
const DEFAULT_READ_LIMIT: u64 = 64 * 1024;
const MAX_READ_LIMIT: u64 = 1024 * 1024;

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

fn stream_name(stream: OutputStream) -> &'static str {
    match stream {
        OutputStream::Stdout => "stdout",
        OutputStream::Stderr => "stderr",
    }
}

pub fn output_path(dir: &Path, id: &str, stream: OutputStream) -> PathBuf {
    dir.join(format!("{id}.{}", stream_name(stream)))
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The command line sent to the server. `cd` and `export` lines run first in the
/// remote login shell, then the command itself, unchanged.
pub fn build_command(params: &ExecParams) -> Result<String, RpcError> {
    let mut script = String::new();
    if let Some(cwd) = params.cwd.as_deref().filter(|c| !c.is_empty()) {
        script.push_str(&format!("cd {} || exit 1\n", sh_path(cwd)));
    }
    for (name, value) in params.env.iter().flatten() {
        if !valid_env_name(name) {
            return Err(invalid(format!(
                "invalid environment variable name `{name}`"
            )));
        }
        script.push_str(&format!("export {name}={}\n", sh_quote(value)));
    }
    script.push_str(&params.command);
    Ok(script)
}

fn signal_name(signal: &Sig) -> String {
    match signal {
        Sig::Custom(name) => name.clone(),
        other => format!("{other:?}"),
    }
}

/// Collects one output stream: head and tail in memory, everything (up to a cap) on disk.
struct Sink {
    stream: OutputStream,
    path: PathBuf,
    max: usize,
    head: Vec<u8>,
    tail: Vec<u8>,
    total: u64,
    file: Option<tokio::io::BufWriter<tokio::fs::File>>,
    file_failed: bool,
    live_sent: u64,
    chunker: Utf8Chunker,
}

impl Sink {
    fn new(stream: OutputStream, dir: &Path, id: &str, max: usize) -> Self {
        Self {
            stream,
            path: output_path(dir, id, stream),
            max,
            head: Vec::new(),
            tail: Vec::new(),
            total: 0,
            file: None,
            file_failed: false,
            live_sent: 0,
            chunker: Utf8Chunker::default(),
        }
    }

    async fn push(&mut self, data: &[u8], id: &str, daemon: &Daemon) {
        self.total += data.len() as u64;

        let room = self.max.saturating_sub(self.head.len());
        self.head.extend_from_slice(&data[..room.min(data.len())]);
        let half = self.max / 2;
        self.tail.extend_from_slice(data);
        if self.tail.len() > half * 2 {
            let excess = self.tail.len() - half;
            self.tail.drain(..excess);
        }

        let stored_before = self.total - data.len() as u64;
        if stored_before < STORE_CAP && !self.file_failed {
            let take = ((STORE_CAP - stored_before) as usize).min(data.len());
            if self.file.is_none() {
                match open_private(&self.path).await {
                    Ok(file) => self.file = Some(tokio::io::BufWriter::new(file)),
                    Err(err) => {
                        tracing::warn!(%err, path = %self.path.display(), "cannot store output");
                        self.file_failed = true;
                    }
                }
            }
            if let Some(file) = &mut self.file
                && let Err(err) = file.write_all(&data[..take]).await
            {
                tracing::warn!(%err, "cannot store output");
                self.file_failed = true;
            }
        }

        if self.live_sent < LIVE_CAP {
            let take = ((LIVE_CAP - self.live_sent) as usize).min(data.len());
            self.live_sent += take as u64;
            let text = self.chunker.push(&data[..take]);
            if !text.is_empty() {
                daemon.events.publish(Event::ExecOutput {
                    id: id.to_string(),
                    stream: self.stream,
                    data: text,
                });
            }
        }
    }

    async fn finish(&mut self, id: &str, daemon: &Daemon) {
        if let Some(file) = &mut self.file {
            let _ = file.flush().await;
        }
        let rest = self.chunker.finish();
        if !rest.is_empty() {
            daemon.events.publish(Event::ExecOutput {
                id: id.to_string(),
                stream: self.stream,
                data: rest,
            });
        }
    }

    /// Whole output if it fits, otherwise head + marker + tail.
    fn render(&self) -> (String, bool) {
        if self.total as usize <= self.max {
            return (String::from_utf8_lossy(&self.head).into_owned(), false);
        }
        let half = self.max / 2;
        let head = &self.head[..utf8_prefix_len(&self.head[..half.min(self.head.len())])];
        let tail_start = self.tail.len().saturating_sub(half);
        let tail = &self.tail[tail_start..];
        let tail = &tail[utf8_skip_continuation(tail)..];
        let omitted = self.total - head.len() as u64 - tail.len() as u64;
        let text = format!(
            "{}\n…[{omitted} bytes omitted; read them with output.read at offset {}]…\n{}",
            String::from_utf8_lossy(head),
            head.len(),
            String::from_utf8_lossy(tail)
        );
        (text, true)
    }
}

async fn open_private(path: &Path) -> std::io::Result<tokio::fs::File> {
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).await
}

/// Params as recorded in the audit log and exec table: `stdin` is replaced by its size.
pub fn redact_params(params: &Value) -> Value {
    let mut params = params.clone();
    if let Some(stdin) = params.get("stdin").and_then(Value::as_str) {
        let size = stdin.len();
        params["stdin"] = serde_json::json!({ "bytes": size });
    }
    params
}

struct Outcome {
    status: ExecStatus,
    exit_code: Option<i32>,
    signal: Option<String>,
    error: Option<String>,
}

pub async fn run(
    daemon: &Daemon,
    client: &ClientInfo,
    params: ExecParams,
    entry: &Entry,
    loaded: &Loaded,
    approver: Option<String>,
) -> Result<ExecResult, RpcError> {
    if params.command.trim().is_empty() {
        return Err(invalid("`command` must not be empty"));
    }
    let script = build_command(&params)?;
    let timeout = Duration::from_millis(
        params
            .timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1, MAX_TIMEOUT_MS),
    );
    let max_output = params
        .max_output_bytes
        .unwrap_or(DEFAULT_MAX_OUTPUT)
        .clamp(MIN_MAX_OUTPUT, MAX_MAX_OUTPUT) as usize;

    let (lease, channel) = daemon.pool.open_channel(entry, loaded).await?;

    let id = new_id("ex");
    let started = Instant::now();
    let mut record = ExecRecord {
        id: id.clone(),
        host: entry.host.id.clone(),
        command: params.command.clone(),
        cwd: params.cwd.clone(),
        client: client.clone(),
        status: ExecStatus::Running,
        started_at: now_ms(),
        finished_at: None,
        exit_code: None,
        signal: None,
        error: None,
        stdout_bytes: 0,
        stderr_bytes: 0,
        approver,
    };
    let audit_params = redact_params(&serde_json::to_value(&params).unwrap_or(Value::Null));
    daemon
        .store
        .insert_exec(record.clone(), audit_params)
        .await?;
    daemon.events.publish(Event::ExecStarted {
        exec: record.clone(),
    });

    let mut stdout = Sink::new(OutputStream::Stdout, &daemon.outputs_dir, &id, max_output);
    let mut stderr = Sink::new(OutputStream::Stderr, &daemon.outputs_dir, &id, max_output);
    let stop = (!crate::policy::is_desktop(client)).then(|| daemon.policy.stop_signal());
    let outcome = drive(
        daemon,
        &lease,
        stop,
        channel,
        &script,
        params.stdin,
        timeout,
        &id,
        &mut stdout,
        &mut stderr,
    )
    .await;
    stdout.finish(&id, daemon).await;
    stderr.finish(&id, daemon).await;
    drop(lease);

    record.status = outcome.status;
    record.exit_code = outcome.exit_code;
    record.signal = outcome.signal.clone();
    record.error = outcome.error.clone();
    record.finished_at = Some(now_ms());
    record.stdout_bytes = stdout.total;
    record.stderr_bytes = stderr.total;
    if let Err(err) = daemon.store.finish_exec(record.clone()).await {
        tracing::warn!(%err, "cannot record exec result");
    }
    daemon.events.publish(Event::ExecFinished {
        exec: record.clone(),
    });

    if outcome.status == ExecStatus::Failed {
        daemon.pool.forget_if_closed(&record.host);
        let message = outcome.error.unwrap_or_else(|| "command failed".into());
        return Err(RpcError::new(ErrorKind::ConnectionLost, message)
            .with_hint(format!(
                "the connection is re-established on the next call; read the partial output with `hostlane output {id}` (output.read)"
            ))
            .with_details(serde_json::json!({ "output_id": id })));
    }

    let (stdout_text, stdout_cut) = stdout.render();
    let (stderr_text, stderr_cut) = stderr.render();
    Ok(ExecResult {
        host: record.host,
        status: outcome.status,
        exit_code: outcome.exit_code,
        signal: outcome.signal,
        duration_ms: started.elapsed().as_millis() as u64,
        stdout: stdout_text,
        stderr: stderr_text,
        stdout_bytes: stdout.total,
        stderr_bytes: stderr.total,
        truncated: stdout_cut || stderr_cut,
        output_id: id,
    })
}

#[allow(clippy::too_many_arguments)]
async fn drive(
    daemon: &Daemon,
    lease: &Lease,
    stop: Option<tokio::sync::watch::Receiver<bool>>,
    channel: russh::Channel<russh::client::Msg>,
    script: &str,
    stdin: Option<String>,
    timeout: Duration,
    id: &str,
    stdout: &mut Sink,
    stderr: &mut Sink,
) -> Outcome {
    let mut outcome = Outcome {
        status: ExecStatus::Exited,
        exit_code: None,
        signal: None,
        error: None,
    };
    if let Err(err) = channel.exec(true, script.as_bytes()).await {
        outcome.status = ExecStatus::Failed;
        outcome.error = Some(format!("cannot start the command: {err}"));
        return outcome;
    }
    let (mut reader, writer) = channel.split();
    let writer = Arc::new(writer);
    let stdin_task = {
        let writer = writer.clone();
        tokio::spawn(async move {
            if let Some(stdin) = stdin {
                let _ = writer.data_bytes(stdin.into_bytes()).await;
            }
            let _ = writer.eof().await;
        })
    };

    let mut deadline = tokio::time::Instant::now() + timeout;
    let mut terminating = false;
    let mut closed_cleanly = false;
    let cancellable = stop.is_some();
    let stopped = async move {
        match stop {
            Some(mut stop) => {
                let _ = stop.wait_for(|stopped| *stopped).await;
            }
            None => std::future::pending().await,
        }
    };
    tokio::pin!(stopped);
    let mut cancelled = false;
    loop {
        tokio::select! {
            _ = &mut stopped, if cancellable && !cancelled => {
                cancelled = true;
                terminating = true;
                outcome.status = ExecStatus::Cancelled;
                outcome.error = Some("terminated by the emergency stop".into());
                let _ = writer.signal(Sig::TERM).await;
                deadline = tokio::time::Instant::now() + TERM_GRACE;
            }
            message = reader.wait() => match message {
                Some(ChannelMsg::Data { data }) => stdout.push(&data, id, daemon).await,
                Some(ChannelMsg::ExtendedData { data, ext: 1 }) => stderr.push(&data, id, daemon).await,
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    outcome.exit_code = Some(exit_status as i32);
                }
                Some(ChannelMsg::ExitSignal { signal_name: signal, .. }) => {
                    outcome.signal = Some(signal_name(&signal));
                }
                Some(ChannelMsg::Failure) => {
                    outcome.status = ExecStatus::Failed;
                    outcome.error = Some("the server refused to run the command".into());
                    break;
                }
                Some(ChannelMsg::Close) => {
                    closed_cleanly = true;
                    break;
                }
                None => break,
                Some(_) => {}
            },
            _ = tokio::time::sleep_until(deadline) => {
                if terminating {
                    let _ = writer.close().await;
                    break;
                }
                terminating = true;
                outcome.status = ExecStatus::TimedOut;
                let _ = writer.signal(Sig::TERM).await;
                deadline = tokio::time::Instant::now() + TERM_GRACE;
            }
        }
    }
    stdin_task.abort();

    let finished = outcome.exit_code.is_some() || outcome.signal.is_some();
    if outcome.status == ExecStatus::Exited && !finished && !closed_cleanly {
        outcome.status = ExecStatus::Failed;
        outcome.error = Some(if lease.conn.handle.is_closed() {
            "the SSH connection was lost while the command was running".into()
        } else {
            "the channel closed before the command finished".into()
        });
    }
    outcome
}

fn valid_output_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub async fn read_output(
    daemon: &Daemon,
    params: OutputReadParams,
) -> Result<OutputChunk, RpcError> {
    let not_found = || {
        RpcError::new(
            ErrorKind::OutputNotFound,
            format!("no output with id `{}`", params.output_id),
        )
        .with_hint("use the output_id returned by exec.run")
    };
    if !valid_output_id(&params.output_id) {
        return Err(not_found());
    }
    let record = daemon
        .store
        .get_exec(params.output_id.clone())
        .await?
        .ok_or_else(not_found)?;
    let full = match params.stream {
        OutputStream::Stdout => record.stdout_bytes,
        OutputStream::Stderr => record.stderr_bytes,
    };
    let path = output_path(&daemon.outputs_dir, &params.output_id, params.stream);
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => Some(file),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(RpcError::new(ErrorKind::Internal, err.to_string())),
    };
    let total = match &file {
        Some(file) => file.metadata().await.map(|m| m.len()).unwrap_or(0),
        None if full == 0 || record.status == ExecStatus::Running => 0,
        None => {
            return Err(RpcError::new(
                ErrorKind::OutputNotFound,
                format!("the output of `{}` is no longer stored", params.output_id),
            ));
        }
    };

    let offset = params.offset.unwrap_or(0);
    let start = if offset < 0 {
        total.saturating_sub(offset.unsigned_abs())
    } else {
        (offset as u64).min(total)
    };
    let limit = params
        .limit
        .unwrap_or(DEFAULT_READ_LIMIT)
        .clamp(1, MAX_READ_LIMIT);
    let end = (start + limit).min(total);
    let mut buffer = vec![0u8; (end - start) as usize];
    if let Some(file) = &mut file {
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))?;
        file.read_exact(&mut buffer)
            .await
            .map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))?;
    }
    let skip = if start > 0 {
        utf8_skip_continuation(&buffer)
    } else {
        0
    };
    let keep = if end < total {
        utf8_prefix_len(&buffer)
    } else {
        buffer.len()
    };
    let keep = keep.max(skip);
    Ok(OutputChunk {
        data: String::from_utf8_lossy(&buffer[skip..keep]).into_owned(),
        offset: start + skip as u64,
        next_offset: start + keep as u64,
        total_bytes: total,
        capped: full > total,
    })
}

/// Deletes stored output older than `max_age`.
pub fn prune_outputs(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > max_age);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn command_prefixes_cwd_and_env() {
        let params = ExecParams {
            host: "h".into(),
            command: "make build; echo done".into(),
            cwd: Some("~/my app".into()),
            env: Some(BTreeMap::from([("MODE".into(), "it's prod".into())])),
            ..Default::default()
        };
        assert_eq!(
            build_command(&params).unwrap(),
            "cd \"$HOME\"/'my app' || exit 1\nexport MODE='it'\\''s prod'\nmake build; echo done"
        );
        let bad = ExecParams {
            env: Some(BTreeMap::from([("A-B".into(), "x".into())])),
            ..params
        };
        assert!(build_command(&bad).is_err());
    }

    #[test]
    fn redacts_stdin() {
        let params = serde_json::json!({ "command": "cat", "stdin": "secret" });
        assert_eq!(redact_params(&params)["stdin"]["bytes"], 6);
    }
}
