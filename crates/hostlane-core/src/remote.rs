//! Commands Hostlane runs for itself (search, rename fallback). They are audited as
//! part of the calling method and do not appear in the session monitor.

use std::time::Duration;

use hostlane_protocol::{ErrorKind, RpcError};
use russh::client::Msg;
use russh::{Channel, ChannelMsg};

const STDERR_CAP: usize = 64 * 1024;

#[derive(Debug, Default)]
pub struct Captured {
    pub exit_code: Option<u32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Reading stopped at `max_bytes` or `max_lines`; the channel was closed.
    pub truncated: bool,
    pub timed_out: bool,
}

/// Runs `command` and collects its output, stopping early once stdout reaches
/// `max_bytes` or `max_lines` newlines.
pub async fn capture(
    mut channel: Channel<Msg>,
    command: &str,
    max_bytes: usize,
    max_lines: Option<usize>,
    timeout: Duration,
) -> Result<Captured, RpcError> {
    channel
        .exec(true, command.as_bytes())
        .await
        .map_err(|err| {
            RpcError::new(
                ErrorKind::ConnectionLost,
                format!("cannot start a command: {err}"),
            )
        })?;
    let _ = channel.eof().await;
    let mut out = Captured::default();
    let mut lines = 0;
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data }) => {
                    let room = max_bytes.saturating_sub(out.stdout.len());
                    let mut take = room.min(data.len());
                    if let Some(max_lines) = max_lines {
                        for (index, byte) in data[..take].iter().enumerate() {
                            if *byte == b'\n' {
                                lines += 1;
                                if lines >= max_lines {
                                    take = index + 1;
                                    break;
                                }
                            }
                        }
                    }
                    out.stdout.extend_from_slice(&data[..take]);
                    if take < data.len() || max_lines.is_some_and(|max| lines >= max) {
                        out.truncated = true;
                        let _ = channel.close().await;
                        break;
                    }
                }
                Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                    let room = STDERR_CAP.saturating_sub(out.stderr.len());
                    out.stderr.extend_from_slice(&data[..room.min(data.len())]);
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => out.exit_code = Some(exit_status),
                Some(ChannelMsg::Failure) => {
                    return Err(RpcError::new(
                        ErrorKind::ConnectionLost,
                        "the server refused to run a command",
                    ));
                }
                Some(ChannelMsg::Close) | None => break,
                Some(_) => {}
            },
            _ = &mut deadline => {
                out.timed_out = true;
                let _ = channel.close().await;
                break;
            }
        }
    }
    Ok(out)
}
