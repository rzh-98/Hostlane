//! `job start|status|logs|cancel` and `jobs`: background jobs that outlive the
//! connection.

use std::io::Write;
use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use hostlane_core::client::Client;
use hostlane_protocol::{
    Job, JobBackend, JobLogs, JobLogsParams, JobStartParams, JobState, JobStatusParams, JobsList,
    JobsListParams, UNTRUSTED_MEMBER, method, untrusted_fields,
};
use serde_json::{Value, json};

use crate::{ago, one_line, parse_env, print_json, print_table, print_untrusted};

#[derive(Subcommand)]
pub enum JobsCommand {
    /// Start, inspect, read or cancel a background job: a command that keeps running
    /// on the server when the connection drops (tmux when available, else nohup).
    #[command(subcommand)]
    Job(JobCommand),
    /// List background jobs, newest first, as Hostlane last saw them.
    Jobs {
        #[arg(long)]
        host: Option<String>,
        /// Only running jobs.
        #[arg(long)]
        running: bool,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

#[derive(Subcommand)]
pub enum JobCommand {
    /// Start a command in the background and print its job id.
    Start {
        host: String,
        /// The command, run by the remote login shell. Quote it or put it after `--`.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
        /// A label shown in lists.
        #[arg(long)]
        name: Option<String>,
        /// Remote working directory.
        #[arg(long)]
        cwd: Option<String>,
        /// Environment variable for the command, as NAME=VALUE. Repeatable.
        #[arg(long = "env", value_name = "NAME=VALUE")]
        env: Vec<String>,
        /// Require tmux.
        #[arg(long, conflicts_with = "nohup")]
        tmux: bool,
        /// Do not use tmux even when the server has it.
        #[arg(long)]
        nohup: bool,
    },
    /// Show a job's state, read from the server.
    Status {
        id: String,
        /// Wait up to this many seconds (at most 120) for the job to finish.
        #[arg(long, value_name = "SECONDS")]
        wait: Option<u64>,
    },
    /// Print a job's output (stdout and stderr together).
    Logs {
        id: String,
        /// Last lines to print (default 100).
        #[arg(short = 'n', long)]
        lines: Option<u32>,
        /// Print from this byte offset instead of the last lines.
        #[arg(long, conflicts_with = "lines")]
        offset: Option<u64>,
        /// Keep printing new output until the job ends; exits with its exit code.
        #[arg(short, long)]
        follow: bool,
    },
    /// Stop a job: TERM to its process group, then KILL after 5 seconds.
    Cancel { id: String },
}

fn state_label(job: &Job) -> String {
    match job.state {
        JobState::Running => "running".into(),
        JobState::Exited => format!("exited {}", job.exit_code.unwrap_or(-1)),
        JobState::Cancelled => "cancelled".into(),
        JobState::Lost => "lost".into(),
    }
}

fn backend_label(backend: JobBackend) -> &'static str {
    match backend {
        JobBackend::Tmux => "tmux",
        JobBackend::Nohup => "nohup",
    }
}

fn print_job(job: &Job) {
    let pid = job.pid.map(|p| format!(", pid {p}")).unwrap_or_default();
    let name = job
        .name
        .as_deref()
        .map(|n| format!(" ({n})"))
        .unwrap_or_default();
    println!(
        "{}{name} on {}: {} · {}{pid} · started {} by {}",
        job.id,
        job.host,
        state_label(job),
        backend_label(job.backend),
        ago(job.started_at),
        job.client.name
    );
    println!("  $ {}", job.command);
    println!("  output: {} bytes", job.output_bytes);
}

pub async fn run(client: &Client, command: JobsCommand, json: bool) -> Result<ExitCode> {
    match command {
        JobsCommand::Jobs {
            host,
            running,
            limit,
        } => {
            let params = JobsListParams {
                host,
                running,
                limit: Some(limit),
            };
            let list: JobsList = client
                .call_as(method::JOBS_LIST, serde_json::to_value(params)?)
                .await?;
            if json {
                print_json(&list);
            } else if list.jobs.is_empty() {
                println!("No jobs.");
            } else {
                let rows: Vec<[String; 5]> = list
                    .jobs
                    .iter()
                    .map(|job| {
                        [
                            job.id.clone(),
                            job.host.clone(),
                            state_label(job),
                            ago(job.started_at),
                            one_line(job.name.as_deref().unwrap_or(&job.command), 60),
                        ]
                    })
                    .collect();
                print_table(["JOB", "HOST", "STATE", "STARTED", "COMMAND"], &rows);
            }
        }
        JobsCommand::Job(JobCommand::Start {
            host,
            command,
            name,
            cwd,
            env,
            tmux,
            nohup,
        }) => {
            let params = JobStartParams {
                host,
                command: command.join(" "),
                cwd,
                env: parse_env(&env)?,
                name,
                backend: match (tmux, nohup) {
                    (true, _) => Some(JobBackend::Tmux),
                    (_, true) => Some(JobBackend::Nohup),
                    _ => None,
                },
            };
            let job: Job = client
                .call_as(method::JOBS_START, serde_json::to_value(params)?)
                .await?;
            if json {
                print_json(&job);
            } else {
                let pid = job.pid.map(|p| format!(", pid {p}")).unwrap_or_default();
                println!(
                    "Started job {} on {} ({}{pid}).",
                    job.id,
                    job.host,
                    backend_label(job.backend)
                );
                println!("Follow it with: hostlane job logs {} --follow", job.id);
            }
        }
        JobsCommand::Job(JobCommand::Status { id, wait }) => {
            let params = JobStatusParams {
                id,
                wait_ms: wait.map(|secs| secs.saturating_mul(1000)),
            };
            let job: Job = client
                .call_as(method::JOBS_STATUS, serde_json::to_value(params)?)
                .await?;
            if json {
                print_json(&job);
            } else {
                print_job(&job);
            }
        }
        JobsCommand::Job(JobCommand::Logs {
            id,
            lines,
            offset,
            follow,
        }) => return logs(client, id, lines, offset, follow, json).await,
        JobsCommand::Job(JobCommand::Cancel { id }) => {
            let job: Job = client
                .call_as(method::JOBS_CANCEL, json!({ "id": id }))
                .await?;
            if json {
                print_json(&job);
            } else if job.state == JobState::Cancelled {
                println!("Cancelled job {} on {}.", job.id, job.host);
            } else {
                println!("Job {} had already ended ({}).", job.id, state_label(&job));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn logs(
    client: &Client,
    id: String,
    lines: Option<u32>,
    offset: Option<u64>,
    follow: bool,
    json: bool,
) -> Result<ExitCode> {
    let mut params = JobLogsParams {
        id,
        offset,
        tail_lines: lines,
        limit: None,
        wait_ms: None,
    };
    loop {
        let chunk: JobLogs = client
            .call_as(method::JOBS_LOGS, serde_json::to_value(&params)?)
            .await?;
        if json && follow {
            let mut value = serde_json::to_value(&chunk)?;
            if let Value::Object(object) = &mut value {
                object.insert(
                    UNTRUSTED_MEMBER.into(),
                    json!(untrusted_fields(method::JOBS_LOGS)),
                );
            }
            println!("{}", serde_json::to_string(&value)?);
        } else if json {
            print_untrusted(method::JOBS_LOGS, &chunk);
        } else {
            print!("{}", chunk.data);
            std::io::stdout().flush()?;
        }
        if !follow || chunk.complete {
            let code = match (chunk.job.state, chunk.job.exit_code) {
                _ if !follow => 0,
                (JobState::Exited, Some(code)) => code.clamp(0, 255) as u8,
                (JobState::Cancelled, _) => 130,
                _ => 255,
            };
            if follow && !json {
                eprintln!("hostlane: job {} {}", chunk.job.id, state_label(&chunk.job));
            }
            return Ok(ExitCode::from(code));
        }
        if chunk.job.state != JobState::Running && chunk.next_offset >= chunk.job.output_bytes {
            return Ok(ExitCode::from(255));
        }
        params.offset = Some(chunk.next_offset);
        params.tail_lines = None;
        params.wait_ms = Some(30_000);
    }
}
