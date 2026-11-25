//! E2: background jobs. `jobs.start` writes the command and a small wrapper to
//! `~/.hostlane/jobs/<id>/` on the server and starts the wrapper in a detached `tmux`
//! session, or in the background with hangups ignored (like `nohup`) when the server
//! has no tmux. The wrapper writes the command's output to `out`, its pid and process
//! group to `pid` and `pgid`, and its exit status to `exit`. Jobs survive dropped
//! connections and daemon restarts; their state is read back from those files.

use std::sync::Arc;
use std::time::Duration;

use hostlane_protocol::{
    ClientInfo, ConnectionState, ErrorKind, Event, ExecParams, Job, JobBackend, JobLogs,
    JobLogsParams, JobStartParams, JobState, JobStatusParams, JobsList, JobsListParams, RpcError,
};
use serde_json::Value;
use tokio::task::JoinSet;

use crate::catalog::Catalog;
use crate::events::EventBus;
use crate::policy::is_desktop;
use crate::pool::Pool;
use crate::remote::{self, Captured};
use crate::store::Store;
use crate::util::{new_id, now_ms, sh_quote, utf8_prefix_len, utf8_skip_continuation};

const MAX_WAIT_MS: u64 = 120_000;
const DEFAULT_TAIL_LINES: u32 = 100;
const DEFAULT_LIMIT: u64 = 64 * 1024;
const MAX_LIMIT: u64 = 1024 * 1024;
const MAX_NAME: usize = 64;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const STATUS_POLL: Duration = Duration::from_secs(2);
const LOGS_POLL: Duration = Duration::from_secs(1);
/// How often running jobs on connected hosts are checked in the background.
const WATCH_INTERVAL: Duration = Duration::from_secs(10);
const HEADER_END: &[u8] = b"---\n";

/// Runs on the server as `sh run.sh`; see the module docs.
const WRAPPER: &str = r#"D=$(cd "$(dirname "$0")" && pwd)
trap '' HUP
trap 'hl_sig=1' TERM INT
pg=$(ps -o pgid= -p $$ 2>/dev/null | tr -d ' ')
[ -n "$pg" ] || pg=$(cut -d' ' -f5 /proc/$$/stat 2>/dev/null)
echo "$pg" > "$D/pgid"
"${SHELL:-/bin/sh}" -c "$(cat "$D/cmd")" > "$D/out" 2>&1 < /dev/null &
pid=$!
echo $pid > "$D/pid"
while :; do wait $pid; code=$?; kill -0 $pid 2>/dev/null || break; done
echo $code > "$D/exit.tmp" && mv "$D/exit.tmp" "$D/exit""#;

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

fn remote_failed(what: &str, out: &Captured) -> RpcError {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let message = if out.timed_out {
        format!("{what}: the server did not answer in time")
    } else {
        format!("{what}: {}", stderr.trim())
    };
    RpcError::new(ErrorKind::RemoteCommandFailed, message)
}

/// The job directory as a shell word; `id` is checked by [`check_id`].
fn dir(id: &str) -> String {
    format!("\"$HOME/.hostlane/jobs/{id}\"")
}

fn check_id(id: &str) -> Result<&str, RpcError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        Ok(id)
    } else {
        Err(invalid(format!("`{id}` is not a job id")))
    }
}

/// Our scripts are POSIX sh; the login shell may not be.
fn in_sh(script: &str) -> String {
    format!("sh -c {}", sh_quote(script))
}

fn start_script(id: &str, command: &str, backend: Option<JobBackend>) -> String {
    let d = dir(id);
    let tmux = format!("tmux new-session -d -s hl-{id} \"sh '$D/run.sh'\"");
    let nohup = "sh \"$D/run.sh\" >/dev/null 2>&1 </dev/null &\necho backend=nohup";
    let launch = match backend {
        Some(JobBackend::Tmux) => format!(
            "command -v tmux >/dev/null 2>&1 || {{ echo 'tmux is not installed' >&2; exit 1; }}\n{tmux} || exit 1\necho backend=tmux"
        ),
        Some(JobBackend::Nohup) => nohup.to_string(),
        None => format!(
            "if command -v tmux >/dev/null 2>&1 && {tmux} 2>/dev/null; then\necho backend=tmux\nelse\n{nohup}\nfi"
        ),
    };
    format!(
        "umask 077\nD={d}\nmkdir -p \"$D\" || exit 1\nprintf '%s' {cmd} > \"$D/cmd\" || exit 1\ncat > \"$D/run.sh\" <<'HOSTLANE_RUN'\n{WRAPPER}\nHOSTLANE_RUN\n{launch}\ni=0\nwhile [ ! -s \"$D/pid\" ] && [ $i -lt 30 ]; do sleep 0.1 2>/dev/null || sleep 1; i=$((i+1)); done\necho \"pid=$(cat \"$D/pid\" 2>/dev/null)\"\n",
        cmd = sh_quote(command),
    )
}

/// Prints `state=`, `pid=` and `bytes=` lines. The state is read before the size, so
/// a finished job's size is final.
fn status_lines(id: &str) -> String {
    format!(
        r#"D={d}
if [ ! -d "$D" ]; then echo state=missing; echo bytes=0; exit 0; fi
st=running
if [ -f "$D/exit" ]; then st="exit=$(cat "$D/exit")"
elif [ -s "$D/pid" ] && ! kill -0 "$(cat "$D/pid")" 2>/dev/null; then
  sleep 1
  if [ -f "$D/exit" ]; then st="exit=$(cat "$D/exit")"; else st=gone; fi
fi
echo "state=$st"
echo "pid=$(cat "$D/pid" 2>/dev/null)"
S=$(wc -c < "$D/out" 2>/dev/null | tr -d ' ')
echo "bytes=${{S:-0}}"
"#,
        d = dir(id)
    )
}

fn logs_script(id: &str, offset: Option<u64>, tail_lines: u32, limit: u64) -> String {
    let read = match offset {
        Some(offset) => format!(
            "tail -c +{} \"$D/out\" 2>/dev/null | head -c {limit}",
            offset + 1
        ),
        None => format!(
            "head -c \"${{S:-0}}\" \"$D/out\" 2>/dev/null | tail -n {tail_lines} | tail -c {limit}"
        ),
    };
    format!("{}echo ---\n{read}\n", status_lines(id))
}

fn cancel_script(id: &str) -> String {
    format!(
        r#"D={d}
[ -d "$D" ] || {{ echo result=missing; exit 0; }}
[ -f "$D/exit" ] && {{ echo result=finished; exit 0; }}
pid=$(cat "$D/pid" 2>/dev/null)
pg=$(cat "$D/pgid" 2>/dev/null)
sig() {{
  if [ -n "$pg" ] && [ "$pg" -gt 1 ] 2>/dev/null && kill -$1 "-$pg" 2>/dev/null; then return 0; fi
  [ -n "$pid" ] && kill -$1 "$pid" 2>/dev/null
}}
sig TERM
i=0
while [ $i -lt 50 ] && [ ! -f "$D/exit" ]; do sleep 0.1 2>/dev/null || sleep 1; i=$((i+1)); done
if [ ! -f "$D/exit" ] && [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then sig KILL; echo result=killed; else echo result=terminated; fi
command -v tmux >/dev/null 2>&1 && tmux kill-session -t hl-{id} 2>/dev/null
exit 0
"#,
        d = dir(id)
    )
}

#[derive(Debug, PartialEq)]
enum Seen {
    Running,
    Exited(i32),
    /// The process is gone and left no exit status.
    Gone,
    /// The job directory is gone.
    Missing,
}

#[derive(Debug)]
struct Probe {
    seen: Seen,
    pid: Option<u32>,
    bytes: u64,
}

fn parse_probe(header: &str) -> Probe {
    let mut probe = Probe {
        seen: Seen::Running,
        pid: None,
        bytes: 0,
    };
    for line in header.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "state" => {
                probe.seen = match value {
                    "running" => Seen::Running,
                    "gone" => Seen::Gone,
                    "missing" => Seen::Missing,
                    other => match other.strip_prefix("exit=").map(str::trim) {
                        Some(code) => code.parse().map_or(Seen::Gone, Seen::Exited),
                        None => Seen::Gone,
                    },
                }
            }
            "pid" => probe.pid = value.trim().parse().ok(),
            "bytes" => probe.bytes = value.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    probe
}

/// Splits command output at the `---` line into the header and the data after it.
fn split_header(bytes: &[u8]) -> (String, &[u8]) {
    let mut start = 0;
    while start < bytes.len() {
        let end = bytes[start..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| start + i + 1);
        if &bytes[start..end] == HEADER_END {
            return (
                String::from_utf8_lossy(&bytes[..start]).into_owned(),
                &bytes[end..],
            );
        }
        start = end;
    }
    (String::from_utf8_lossy(bytes).into_owned(), &[])
}

/// Applies what the server reported; returns whether anything visible changed.
fn apply(job: &mut Job, probe: &Probe, now: u64) -> bool {
    let before = (job.state, job.exit_code, job.pid, job.output_bytes);
    job.checked_at = now;
    if probe.pid.is_some() {
        job.pid = probe.pid;
    }
    job.output_bytes = job.output_bytes.max(probe.bytes);
    let cancelled = job.state == JobState::Cancelled;
    match probe.seen {
        Seen::Running if !cancelled => job.state = JobState::Running,
        Seen::Running => {}
        Seen::Exited(code) => {
            job.exit_code = Some(code);
            if !cancelled {
                job.state = JobState::Exited;
            }
        }
        Seen::Gone | Seen::Missing if !cancelled => job.state = JobState::Lost,
        Seen::Gone | Seen::Missing => {}
    }
    if job.state != JobState::Running && job.finished_at.is_none() {
        job.finished_at = Some(now);
    }
    before != (job.state, job.exit_code, job.pid, job.output_bytes)
}

pub struct Jobs {
    pool: Arc<Pool>,
    catalog: Catalog,
    store: Store,
    events: EventBus,
}

impl Jobs {
    pub fn new(pool: Arc<Pool>, catalog: Catalog, store: Store, events: EventBus) -> Self {
        Self {
            pool,
            catalog,
            store,
            events,
        }
    }

    async fn run(&self, host: &str, script: &str, max_bytes: usize) -> Result<Captured, RpcError> {
        let (entry, loaded) = self.catalog.resolve(host).await?;
        let (_lease, channel) = self.pool.open_channel(&entry, &loaded).await?;
        remote::capture(channel, &in_sh(script), max_bytes, None, COMMAND_TIMEOUT).await
    }

    pub async fn get(&self, id: &str) -> Result<Job, RpcError> {
        let id = check_id(id)?;
        self.store.get_job(id.to_string()).await?.ok_or_else(|| {
            RpcError::new(ErrorKind::JobNotFound, format!("no job `{id}`"))
                .with_hint("list jobs with jobs.list (CLI: hostlane jobs)")
        })
    }

    /// The host of a job, for the policy and the audit log.
    pub async fn host_of(&self, id: &str) -> Option<(String, String)> {
        let job = self.get(id).await.ok()?;
        Some((job.host, job.command))
    }

    async fn save(&self, job: &Job, changed: bool) {
        if let Err(err) = self.store.update_job(job.clone()).await {
            tracing::warn!(%err, job = job.id, "cannot record job state");
        }
        if changed {
            self.events.publish(Event::Job { job: job.clone() });
        }
    }

    pub async fn start(
        &self,
        client: &ClientInfo,
        params: JobStartParams,
        approver: Option<String>,
    ) -> Result<Job, RpcError> {
        if params.command.trim().is_empty() {
            return Err(invalid("`command` must not be empty"));
        }
        let name = params
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| n.chars().take(MAX_NAME).collect::<String>());
        let script = crate::exec::build_command(&ExecParams {
            host: params.host.clone(),
            command: params.command.clone(),
            cwd: params.cwd.clone(),
            env: params.env.clone(),
            stdin: None,
            timeout_ms: None,
            max_output_bytes: None,
        })?;
        let (entry, _) = self.catalog.resolve(&params.host).await?;
        let id = new_id("jb");
        let out = self
            .run(
                &params.host,
                &start_script(&id, &script, params.backend),
                4096,
            )
            .await?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let backend = if text.contains("backend=tmux") {
            JobBackend::Tmux
        } else if text.contains("backend=nohup") {
            JobBackend::Nohup
        } else {
            return Err(remote_failed("cannot start the job", &out));
        };
        let pid = text
            .lines()
            .find_map(|line| line.strip_prefix("pid="))
            .and_then(|pid| pid.trim().parse().ok());
        let now = now_ms();
        let job = Job {
            id,
            host: entry.host.id.clone(),
            name,
            command: params.command.clone(),
            cwd: params.cwd.clone(),
            client: client.clone(),
            backend,
            state: JobState::Running,
            started_at: now,
            finished_at: None,
            exit_code: None,
            pid,
            output_bytes: 0,
            checked_at: now,
            approver,
        };
        let recorded = serde_json::to_value(&params).unwrap_or(Value::Null);
        self.store
            .insert_job(job.clone(), crate::exec::redact_params(&recorded))
            .await?;
        self.events.publish(Event::Job { job: job.clone() });
        Ok(job)
    }

    /// Reads the job's state from the server.
    async fn refresh(&self, mut job: Job) -> Result<Job, RpcError> {
        if job.state != JobState::Running {
            return Ok(job);
        }
        let out = self.run(&job.host, &status_lines(&job.id), 4096).await?;
        if out.exit_code != Some(0) {
            return Err(remote_failed("cannot read the job state", &out));
        }
        let probe = parse_probe(&String::from_utf8_lossy(&out.stdout));
        let changed = apply(&mut job, &probe, now_ms());
        self.save(&job, changed).await;
        Ok(job)
    }

    pub async fn status(&self, params: JobStatusParams) -> Result<Job, RpcError> {
        let wait = Duration::from_millis(params.wait_ms.unwrap_or(0).min(MAX_WAIT_MS));
        let deadline = tokio::time::Instant::now() + wait;
        let mut job = self.refresh(self.get(&params.id).await?).await?;
        while job.state == JobState::Running
            && tokio::time::Instant::now() + STATUS_POLL <= deadline
        {
            tokio::time::sleep(STATUS_POLL).await;
            job = self.refresh(job).await?;
        }
        Ok(job)
    }

    pub async fn list(&self, params: JobsListParams) -> Result<JobsList, RpcError> {
        Ok(JobsList {
            jobs: self.store.list_jobs(params).await?,
        })
    }

    pub async fn logs(&self, params: JobLogsParams) -> Result<JobLogs, RpcError> {
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let tail_lines = params.tail_lines.unwrap_or(DEFAULT_TAIL_LINES).max(1);
        let wait = match params.offset {
            Some(_) => Duration::from_millis(params.wait_ms.unwrap_or(0).min(MAX_WAIT_MS)),
            None => Duration::ZERO,
        };
        let deadline = tokio::time::Instant::now() + wait;
        let mut job = self.get(&params.id).await?;
        loop {
            let script = logs_script(&job.id, params.offset, tail_lines, limit);
            let out = self.run(&job.host, &script, limit as usize + 4096).await?;
            if out.exit_code != Some(0) && out.stdout.is_empty() {
                return Err(remote_failed("cannot read the job output", &out));
            }
            let (header, data) = split_header(&out.stdout);
            let probe = parse_probe(&header);
            if job.state == JobState::Running {
                let changed = apply(&mut job, &probe, now_ms());
                self.save(&job, changed).await;
            }
            let more_coming = job.state == JobState::Running
                && data.is_empty()
                && tokio::time::Instant::now() + LOGS_POLL <= deadline;
            if more_coming {
                tokio::time::sleep(LOGS_POLL).await;
                continue;
            }
            let (offset, data) = match params.offset {
                Some(offset) => {
                    let end = if (data.len() as u64) < limit && job.state != JobState::Running {
                        data.len()
                    } else {
                        utf8_prefix_len(data)
                    };
                    (offset, &data[..end])
                }
                None => {
                    let skip = utf8_skip_continuation(data);
                    let start = probe.bytes.saturating_sub(data.len() as u64);
                    (start + skip as u64, &data[skip..])
                }
            };
            let next_offset = offset + data.len() as u64;
            return Ok(JobLogs {
                complete: job.state != JobState::Running && next_offset >= probe.bytes,
                data: String::from_utf8_lossy(data).into_owned(),
                offset,
                next_offset,
                job,
            });
        }
    }

    pub async fn cancel(&self, id: &str) -> Result<Job, RpcError> {
        let job = self.refresh(self.get(id).await?).await?;
        if job.state != JobState::Running {
            return Ok(job);
        }
        let out = self.run(&job.host, &cancel_script(&job.id), 4096).await?;
        let text = String::from_utf8_lossy(&out.stdout);
        if !text.contains("result=") {
            return Err(remote_failed("cannot cancel the job", &out));
        }
        let mut job = job;
        if !text.contains("result=finished") {
            job.state = JobState::Cancelled;
        }
        let out = self.run(&job.host, &status_lines(&job.id), 4096).await?;
        let probe = parse_probe(&String::from_utf8_lossy(&out.stdout));
        apply(&mut job, &probe, now_ms());
        self.save(&job, true).await;
        Ok(job)
    }

    /// Cancels running jobs started by agents (emergency stop). Returns their ids.
    pub async fn cancel_agent_jobs(self: &Arc<Self>) -> Vec<String> {
        let params = JobsListParams {
            running: true,
            limit: Some(500),
            ..Default::default()
        };
        let running = self.store.list_jobs(params).await.unwrap_or_default();
        let mut tasks = JoinSet::new();
        for job in running.into_iter().filter(|j| !is_desktop(&j.client)) {
            let jobs = self.clone();
            tasks.spawn(async move {
                let cancelled = tokio::time::timeout(COMMAND_TIMEOUT, jobs.cancel(&job.id)).await;
                match cancelled {
                    Ok(Ok(job)) if job.state == JobState::Cancelled => Some(job.id),
                    Ok(Err(err)) => {
                        tracing::warn!(%err, job = job.id, "cannot cancel job");
                        None
                    }
                    Err(_) => {
                        tracing::warn!(job = job.id, "cancelling the job timed out");
                        None
                    }
                    _ => None,
                }
            });
        }
        let mut ids: Vec<String> = tasks.join_all().await.into_iter().flatten().collect();
        ids.sort();
        ids
    }

    /// Checks running jobs on hosts that are connected anyway, so finished jobs show up
    /// without anyone asking. Runs forever.
    pub async fn maintain(self: Arc<Self>) {
        let mut interval = tokio::time::interval(WATCH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let params = JobsListParams {
                running: true,
                limit: Some(500),
                ..Default::default()
            };
            let Ok(running) = self.store.list_jobs(params).await else {
                continue;
            };
            if running.is_empty() {
                continue;
            }
            let connected: Vec<String> = self
                .pool
                .list()
                .into_iter()
                .filter(|c| c.state == ConnectionState::Connected)
                .map(|c| c.host)
                .collect();
            for job in running {
                if !connected.contains(&job.host) {
                    continue;
                }
                let id = job.id.clone();
                match tokio::time::timeout(COMMAND_TIMEOUT, self.refresh(job)).await {
                    Ok(Err(err)) => tracing::debug!(%err, job = id, "cannot check job"),
                    Err(_) => tracing::debug!(job = id, "checking the job timed out"),
                    Ok(Ok(_)) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> Job {
        Job {
            id: "jb-1".into(),
            host: "h".into(),
            name: None,
            command: "make".into(),
            cwd: None,
            client: ClientInfo::new("codex"),
            backend: JobBackend::Nohup,
            state: JobState::Running,
            started_at: 1,
            finished_at: None,
            exit_code: None,
            pid: None,
            output_bytes: 0,
            checked_at: 1,
            approver: None,
        }
    }

    #[test]
    fn parses_probes_and_applies_them() {
        let probe = parse_probe("state=running\npid=4242\nbytes=17\n");
        assert_eq!(probe.seen, Seen::Running);
        let mut running = job();
        assert!(apply(&mut running, &probe, 5));
        assert_eq!(running.pid, Some(4242));
        assert_eq!(running.output_bytes, 17);
        assert!(!apply(&mut running, &probe, 6));

        let exited = parse_probe("state=exit=3\npid=4242\nbytes=20\n");
        assert!(apply(&mut running, &exited, 7));
        assert_eq!(running.state, JobState::Exited);
        assert_eq!(running.exit_code, Some(3));
        assert_eq!(running.finished_at, Some(7));

        let mut cancelled = job();
        cancelled.state = JobState::Cancelled;
        apply(&mut cancelled, &parse_probe("state=exit=143\nbytes=0\n"), 9);
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert_eq!(cancelled.exit_code, Some(143));

        let mut lost = job();
        apply(&mut lost, &parse_probe("state=gone\npid=\nbytes=0\n"), 9);
        assert_eq!(lost.state, JobState::Lost);
        assert_eq!(parse_probe("state=missing\n").seen, Seen::Missing);
    }

    #[test]
    fn splits_output_at_the_header() {
        let (header, data) = split_header(b"state=running\nbytes=4\n---\nab\n---\ncd");
        assert_eq!(header, "state=running\nbytes=4\n");
        assert_eq!(data, b"ab\n---\ncd");
        let (_, empty) = split_header(b"state=missing\n");
        assert!(empty.is_empty());
    }

    #[test]
    fn scripts_quote_the_command() {
        let script = start_script("jb-1", "echo 'hi' && sleep 1", None);
        assert!(script.contains(r#"printf '%s' 'echo '\''hi'\'' && sleep 1' > "$D/cmd""#));
        assert!(script.contains("tmux new-session -d -s hl-jb-1"));
        assert!(script.contains("backend=nohup"));
        let nohup = start_script("jb-1", "true", Some(JobBackend::Nohup));
        assert!(!nohup.contains("tmux"));
        assert!(check_id("jb-1; rm").is_err());
        assert!(logs_script("jb-1", Some(10), 100, 50).contains("tail -c +11"));
    }

    /// Starts a nohup job and cancels it with each local POSIX shell (dash's `kill`
    /// rejects `--`), then checks that no process of the job is left.
    #[cfg(unix)]
    #[test]
    fn cancel_stops_the_whole_process_group() {
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        for (n, shell) in ["/bin/sh", "/bin/dash"].into_iter().enumerate() {
            if !std::path::Path::new(shell).exists() {
                continue;
            }
            let home = std::env::temp_dir().join(format!("hl-job-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&home).unwrap();
            let marker = format!("sleep 30{}{n}", std::process::id() % 1000);
            let run = |script: String| {
                let out = Command::new(shell)
                    .arg("-c")
                    .arg(script)
                    .env("HOME", &home)
                    .env("SHELL", shell)
                    .process_group(0)
                    .output()
                    .unwrap();
                String::from_utf8_lossy(&out.stdout).into_owned()
            };
            let left = || {
                let ps = Command::new("ps").args(["-eo", "args="]).output().unwrap();
                String::from_utf8_lossy(&ps.stdout)
                    .lines()
                    .filter(|line| line.trim() == marker)
                    .count()
            };
            let id = format!("jb-test-{n}");
            let started = run(start_script(
                &id,
                &format!("{marker} & {marker}"),
                Some(JobBackend::Nohup),
            ));
            assert!(started.contains("backend=nohup"), "{shell}: {started}");
            for _ in 0..30 {
                if left() == 2 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(left(), 2, "{shell}: job did not start");
            let cancelled = run(cancel_script(&id));
            assert!(cancelled.contains("result="), "{shell}: {cancelled}");
            std::thread::sleep(Duration::from_millis(300));
            let remaining = left();
            std::fs::remove_dir_all(&home).unwrap();
            assert_eq!(remaining, 0, "{shell}: processes left after cancel");
        }
    }
}
