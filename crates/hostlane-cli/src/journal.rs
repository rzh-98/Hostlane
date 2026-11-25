use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand, ValueEnum};
use hostlane_core::client::Client;
use hostlane_protocol::{
    AuditEntry, AuditList, AuditListParams, OutputChunk, RollbackItem, method,
};
use serde_json::json;
use std::io::{self, Write};

#[derive(Args, Default)]
pub struct Scope {
    #[arg(long)]
    task: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    client: Option<String>,
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    since: Option<u64>,
    #[arg(long, value_enum)]
    min_risk: Option<crate::policy::RiskArg>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Jsonl,
    Csv,
}

#[derive(Subcommand)]
pub enum JournalCommand {
    /// Show audited calls in start-time order for a task or session.
    Timeline {
        #[command(flatten)]
        scope: Scope,
    },
    /// Export the redacted audit log to stdout, newest record first.
    AuditExport {
        #[command(flatten)]
        scope: Scope,
        #[arg(long, value_enum, default_value = "jsonl")]
        format: Format,
    },
    /// List recorded commands, or jump to the output of a numbered command. Never executes commands.
    Replay {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        command: Option<usize>,
    },
    /// Preview a task's file changes newest first. --apply reverts each through the normal policy gate.
    Rollback {
        task: String,
        #[arg(long)]
        client: Option<String>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        apply: bool,
    },
}

impl Scope {
    async fn page(&self, client: &Client, before_id: Option<i64>) -> Result<AuditList> {
        let params = AuditListParams {
            task: self.task.clone(),
            session: self.session.clone(),
            host: self.host.clone(),
            client: self.client.clone(),
            since: self.since,
            min_risk: self.min_risk.map(Into::into),
            before_id,
            limit: Some(250),
            ..Default::default()
        };
        Ok(client
            .call_as(method::AUDIT_LIST, serde_json::to_value(params)?)
            .await?)
    }
    async fn ordered(&self, client: &Client) -> Result<Vec<AuditEntry>> {
        ensure!(
            self.task
                .as_ref()
                .or(self.session.as_ref())
                .is_some_and(|id| !id.trim().is_empty()),
            "specify --task or --session"
        );
        let mut entries = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.page(client, cursor).await?;
            cursor = page.entries.last().map(|entry| entry.id);
            entries.extend(page.entries);
            if !page.has_more {
                break;
            }
        }
        entries.sort_by_key(|entry| (entry.started_at, entry.id));
        Ok(entries)
    }
}

fn csv_cell(value: &str) -> String {
    let unsafe_prefix = value.trim_start().starts_with(['=', '+', '-', '@'])
        || value.starts_with(['\t', '\r', '\n']);
    format!(
        "\"{}{}\"",
        if unsafe_prefix { "'" } else { "" },
        value.replace('"', "\"\"")
    )
}

async fn export(client: &Client, scope: Scope, format: Format, json_mode: bool) -> Result<()> {
    let mut cursor = None;
    let mut output = io::BufWriter::new(io::stdout());
    let mut data = String::new();
    let mut line = |text: String| -> io::Result<()> {
        if json_mode {
            data.push_str(&text);
            data.push('\n');
            Ok(())
        } else {
            writeln!(output, "{text}")
        }
    };
    if matches!(format, Format::Csv) {
        line("id,started_at,finished_at,client,verified,session,task,host,method,outcome,risk,approver,params,result,error_kind,error_message".into())?;
    }
    loop {
        let page = scope.page(client, cursor).await?;
        cursor = page.entries.last().map(|entry| entry.id);
        for entry in page.entries {
            match format {
                Format::Jsonl => line(serde_json::to_string(&entry)?)?,
                Format::Csv => {
                    let value = serde_json::to_value(&entry)?;
                    let fields: Vec<String> = [
                        "/id",
                        "/started_at",
                        "/finished_at",
                        "/client/name",
                        "/client/verified",
                        "/client/session",
                        "/client/task",
                        "/host",
                        "/method",
                        "/outcome",
                        "/risk",
                        "/approver",
                        "/params",
                        "/result",
                        "/error_kind",
                        "/error_message",
                    ]
                    .iter()
                    .map(|key| {
                        value
                            .pointer(key)
                            .map(|v| {
                                csv_cell(
                                    &v.as_str()
                                        .map(str::to_string)
                                        .unwrap_or_else(|| v.to_string()),
                                )
                            })
                            .unwrap_or_default()
                    })
                    .collect();
                    line(fields.join(","))?;
                }
            }
        }
        if !page.has_more {
            break;
        }
    }
    if json_mode {
        crate::print_json(&json!({"data":data}));
    } else {
        output.flush()?;
    }
    Ok(())
}

pub async fn run(client: &Client, command: JournalCommand, json_mode: bool) -> Result<()> {
    match command {
        JournalCommand::AuditExport { scope, format } => {
            export(client, scope, format, json_mode).await?
        }
        JournalCommand::Timeline { scope } => {
            let entries = scope.ordered(client).await?;
            if json_mode {
                crate::print_json(&entries);
            } else {
                for entry in entries {
                    println!(
                        "{}  {}  {}  {}  {:?}",
                        entry.started_at,
                        entry.client.name,
                        entry.host.as_deref().unwrap_or("—"),
                        entry.method,
                        entry.outcome
                    );
                }
            }
        }
        JournalCommand::Replay { scope, command } => {
            let entries: Vec<_> = scope
                .ordered(client)
                .await?
                .into_iter()
                .filter(|entry| entry.result["output_id"].is_string())
                .collect();
            if let Some(index) = command {
                let entry = index
                    .checked_sub(1)
                    .and_then(|index| entries.get(index))
                    .context("command number out of range (starts at 1)")?;
                let mut data = json!({"entry":entry,"untrusted":["stdout","stderr"]});
                for stream in ["stdout", "stderr"] {
                    let chunk:OutputChunk=client.call_as(method::OUTPUT_READ,json!({"output_id":entry.result["output_id"],"stream":stream,"limit":1048576})).await?;
                    data[stream] = serde_json::to_value(chunk)?;
                }
                crate::print_json(&data);
            } else if json_mode {
                crate::print_json(&entries);
            } else {
                for (index, entry) in entries.iter().enumerate() {
                    println!(
                        "{:>4}  {}  {}",
                        index + 1,
                        entry.host.as_deref().unwrap_or("—"),
                        entry.params["command"]
                    );
                }
            }
        }
        JournalCommand::Rollback {
            task,
            host,
            client: caller,
            apply,
        } => {
            let plan: Vec<RollbackItem> = client
                .call_as(
                    method::FILES_ROLLBACK_PLAN,
                    json!({"task":task,"host":host,"client":caller}),
                )
                .await?;
            if !apply {
                crate::print_json(&plan);
                return Ok(());
            }
            let mut reverted = Vec::new();
            for item in plan {
                match client
                    .call(method::FILES_REVERT, json!({"change_id":item.change_id}))
                    .await
                {
                    Ok(_) => reverted.push(item.change_id),
                    Err(err) => {
                        eprintln!(
                            "Stopped at {} ({}:{}); {} already reverted",
                            item.change_id,
                            item.host,
                            item.path,
                            reverted.len()
                        );
                        return Err(err.into());
                    }
                }
            }
            crate::print_json(&json!({"reverted":reverted}));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_escapes_quotes_and_formulas() {
        assert_eq!(csv_cell("a,\"b\"\nc"), "\"a,\"\"b\"\"\nc\"");
        assert_eq!(csv_cell(" =cmd"), "\"' =cmd\"");
    }
}
