//! `approvals`, `approval`, `approve`, `deny`, `stop`, `resume`, `label` and `policy …`.

use std::io::{IsTerminal, Write};

use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use hostlane_core::client::{Answer, Client};
use hostlane_protocol::{
    Approval, ApprovalResolveParams, ApprovalScope, ApprovalState, ApprovalsList,
    ApprovalsListParams, Decision, Host, HostEnv, HostLabelParams, HostsList, IdParams,
    PolicyCheck, PolicyCheckParams, PolicyRule, PolicyRuleAddParams, PolicyRules, PolicyStatus,
    PolicyStopParams, PolicyStopped, Risk, RuleConditions, method,
};
use serde_json::{Value, json};

use crate::{ago, one_line, print_json, print_table};

#[derive(Subcommand)]
pub enum PolicyCommand {
    /// List calls waiting for approval (--all adds recent decisions).
    Approvals {
        #[arg(long)]
        all: bool,
    },
    /// Show one approval: the command or diff, its risk and why.
    Approval { id: String },
    /// Approve a waiting call. Meant for people; agents must not approve their own calls.
    Approve {
        id: String,
        /// `session` also allows the same call for the rest of the agent session,
        /// `always` adds an allow rule for it.
        #[arg(long, value_enum, default_value = "once")]
        scope: ScopeArg,
        /// Do not ask for confirmation (required without a terminal).
        #[arg(long)]
        yes: bool,
    },
    /// Deny a waiting call.
    Deny {
        id: String,
        /// Reason passed back to the agent.
        #[arg(long)]
        note: Option<String>,
    },
    /// Emergency stop: refuse every agent call, deny waiting approvals, stop running
    /// agent commands and close agent tunnels.
    Stop {
        #[arg(long)]
        reason: Option<String>,
    },
    /// Lift the emergency stop.
    Resume,
    /// Set a host's environment and tags, which policy rules can match.
    Label {
        host: String,
        /// `none` removes the environment.
        #[arg(long, value_enum)]
        env: Option<EnvArg>,
        /// Replaces the host's tags. Repeatable.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Remove the labels set in Hostlane (labels from ~/.ssh/config remain).
        #[arg(long, conflicts_with_all = ["env", "tags"])]
        clear: bool,
    },
    /// Show and change policy rules, or check what the policy would do.
    #[command(subcommand)]
    Policy(PolicyAction),
}

#[derive(Subcommand)]
pub enum PolicyAction {
    /// Emergency stop state, pending approvals and rules.
    Status,
    /// Add a rule. Conditions are globs (`*`, `?`); unset conditions match anything.
    ///
    /// When several rules match a call, deny wins over allow, and allow over approve.
    /// Calls no rule matches: L0 allowed; L1 allowed (approval on prod); L2 approval;
    /// L3 approval (refused on prod). The desktop app is always allowed.
    Add {
        #[arg(value_enum)]
        decision: DecisionArg,
        /// Host alias, e.g. `web-*`.
        #[arg(long)]
        host: Option<String>,
        /// Environment label of the host (`none`: hosts without one).
        #[arg(long, value_enum)]
        env: Option<EnvArg>,
        /// A tag the host has.
        #[arg(long)]
        tag: Option<String>,
        /// Client name, e.g. `claude-code` (HOSTLANE_CLIENT of the agent).
        #[arg(long)]
        client: Option<String>,
        /// e.g. `exec.run` or `files.*`.
        #[arg(long)]
        method: Option<String>,
        /// Whole command line of exec, e.g. `systemctl restart *`.
        #[arg(long)]
        command: Option<String>,
        /// Remote path, e.g. `/etc/*`.
        #[arg(long)]
        path: Option<String>,
        /// Only calls of at least this risk.
        #[arg(long, value_enum)]
        min_risk: Option<RiskArg>,
        /// Only calls of at most this risk.
        #[arg(long, value_enum)]
        max_risk: Option<RiskArg>,
        /// Why the rule exists.
        #[arg(long)]
        note: Option<String>,
    },
    /// Delete a rule.
    Delete { id: String },
    /// What the policy would do with a command, without running it.
    Check {
        host: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
        #[arg(long)]
        cwd: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ScopeArg {
    Once,
    Session,
    Always,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EnvArg {
    Dev,
    Staging,
    Prod,
    None,
}

impl EnvArg {
    fn env(self) -> Option<HostEnv> {
        match self {
            Self::Dev => Some(HostEnv::Dev),
            Self::Staging => Some(HostEnv::Staging),
            Self::Prod => Some(HostEnv::Prod),
            Self::None => None,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum DecisionArg {
    Allow,
    Approve,
    Deny,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum RiskArg {
    #[value(name = "L0", alias = "l0")]
    L0,
    #[value(name = "L1", alias = "l1")]
    L1,
    #[value(name = "L2", alias = "l2")]
    L2,
    #[value(name = "L3", alias = "l3")]
    L3,
}

impl From<RiskArg> for Risk {
    fn from(value: RiskArg) -> Self {
        match value {
            RiskArg::L0 => Risk::L0,
            RiskArg::L1 => Risk::L1,
            RiskArg::L2 => Risk::L2,
            RiskArg::L3 => Risk::L3,
        }
    }
}

pub fn label<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub async fn run(client: &Client, command: PolicyCommand, json: bool) -> Result<()> {
    match command {
        PolicyCommand::Approvals { all } => {
            let params = ApprovalsListParams { pending_only: !all };
            let list: ApprovalsList = client
                .call_as(method::APPROVALS_LIST, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&list);
            } else if list.approvals.is_empty() {
                println!("Nothing is waiting for approval.");
            } else {
                approvals_table(&list.approvals);
            }
        }
        PolicyCommand::Approval { id } => {
            let approval: Approval = client
                .call_as(
                    method::APPROVALS_GET,
                    serde_json::to_value(IdParams { id })?,
                )
                .await?;
            if json {
                print_json(&approval);
            } else {
                show_approval(&approval);
            }
        }
        PolicyCommand::Approve { id, scope, yes } => {
            if !yes {
                if !std::io::stdin().is_terminal() {
                    bail!("approving needs a person: run it in a terminal, or pass --yes");
                }
                let approval: Approval = client
                    .call_as(method::APPROVALS_GET, json!({ "id": id }))
                    .await?;
                show_approval(&approval);
                print!("Approve this call? [y/N] ");
                std::io::stdout().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
                    bail!("not approved");
                }
            }
            let scope = match scope {
                ScopeArg::Once => ApprovalScope::Once,
                ScopeArg::Session => ApprovalScope::Session,
                ScopeArg::Always => ApprovalScope::Always,
            };
            let params = ApprovalResolveParams {
                id,
                approve: true,
                scope: Some(scope),
                note: None,
            };
            let approval: Approval = client
                .call_as(method::APPROVALS_RESOLVE, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&approval);
            } else {
                println!("Approved {} ({}).", approval.id, label(scope));
            }
        }
        PolicyCommand::Deny { id, note } => {
            let params = ApprovalResolveParams {
                id,
                approve: false,
                scope: None,
                note,
            };
            let approval: Approval = client
                .call_as(method::APPROVALS_RESOLVE, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&approval);
            } else {
                println!("Denied {}.", approval.id);
            }
        }
        PolicyCommand::Stop { reason } => {
            let stopped: PolicyStopped = client
                .call_as(
                    method::POLICY_STOP,
                    serde_json::to_value(PolicyStopParams { reason })?,
                )
                .await?;
            if json {
                print_json(&stopped);
            } else {
                println!("Emergency stop is on: agent calls are refused.");
                println!(
                    "  denied {} waiting approvals, closed {} agent tunnels",
                    stopped.denied_approvals.len(),
                    stopped.closed_tunnels.len()
                );
                println!("Lift it with `hostlane resume`.");
            }
        }
        PolicyCommand::Resume => {
            let status: PolicyStatus = client.call_as(method::POLICY_RESUME, Value::Null).await?;
            if json {
                print_json(&status);
            } else {
                println!("Resumed: agent calls follow the policy again.");
            }
        }
        PolicyCommand::Label {
            host,
            env,
            tags,
            clear,
        } => {
            let params = if clear {
                HostLabelParams {
                    host,
                    env: None,
                    tags: Vec::new(),
                }
            } else {
                if env.is_none() && tags.is_empty() {
                    bail!("pass --env, --tag or --clear");
                }
                let hosts: HostsList = client.call_as(method::HOSTS_LIST, Value::Null).await?;
                let current = hosts.hosts.iter().find(|h| h.id == host);
                HostLabelParams {
                    env: match env {
                        Some(env) => env.env(),
                        None => current.and_then(|h| h.env),
                    },
                    tags: if tags.is_empty() {
                        current.map(|h| h.tags.clone()).unwrap_or_default()
                    } else {
                        tags
                    },
                    host,
                }
            };
            let host: Host = client
                .call_as(method::HOSTS_LABEL, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&host);
            } else {
                println!(
                    "{}: env {}, tags {}",
                    host.id,
                    host.env.map(label).unwrap_or_else(|| "-".into()),
                    if host.tags.is_empty() {
                        "-".into()
                    } else {
                        host.tags.join(", ")
                    }
                );
            }
        }
        PolicyCommand::Policy(action) => policy(client, action, json).await?,
    }
    Ok(())
}

async fn policy(client: &Client, action: PolicyAction, json: bool) -> Result<()> {
    match action {
        PolicyAction::Status => {
            let status: PolicyStatus = client.call_as(method::POLICY_STATUS, Value::Null).await?;
            let rules: PolicyRules = client
                .call_as(method::POLICY_RULES_LIST, Value::Null)
                .await?;
            if json {
                print_json(&json!({ "status": status, "rules": rules.rules }));
                return Ok(());
            }
            if status.stopped {
                let by = status.stopped_by.as_deref().unwrap_or("?");
                let when = status.stopped_at.map(ago).unwrap_or_default();
                println!("Emergency stop is ON (by {by}, {when}); agent calls are refused.");
                if let Some(reason) = &status.reason {
                    println!("  reason: {reason}");
                }
            } else {
                println!("Emergency stop is off.");
            }
            println!("{} call(s) waiting for approval.", status.pending_approvals);
            println!();
            if rules.rules.is_empty() {
                println!("No rules; the defaults apply (see `hostlane policy add --help`).");
            } else {
                rules_table(&rules.rules);
            }
        }
        PolicyAction::Add {
            decision,
            host,
            env,
            tag,
            client: client_name,
            method: method_name,
            command,
            path,
            min_risk,
            max_risk,
            note,
        } => {
            let params = PolicyRuleAddParams {
                decision: match decision {
                    DecisionArg::Allow => Decision::Allow,
                    DecisionArg::Approve => Decision::Approve,
                    DecisionArg::Deny => Decision::Deny,
                },
                when: RuleConditions {
                    host,
                    env: env.and_then(EnvArg::env),
                    tag,
                    client: client_name,
                    method: method_name,
                    command,
                    path,
                    min_risk: min_risk.map(Into::into),
                    max_risk: max_risk.map(Into::into),
                },
                note,
            };
            let rule: PolicyRule = client
                .call_as(method::POLICY_RULES_ADD, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&rule);
            } else {
                println!("Added rule {}: {}", rule.id, describe_rule(&rule));
            }
        }
        PolicyAction::Delete { id } => {
            let reply = client
                .call(method::POLICY_RULES_DELETE, json!({ "id": id }))
                .await?;
            crate::output_value(json, reply, &format!("Deleted rule {id}."));
        }
        PolicyAction::Check { host, command, cwd } => {
            let params = PolicyCheckParams {
                method: method::EXEC_RUN.into(),
                params: json!({ "host": host, "command": command.join(" "), "cwd": cwd }),
            };
            let check: PolicyCheck = client
                .call_as(method::POLICY_CHECK, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&check);
                return Ok(());
            }
            println!(
                "{} (risk {}, {})",
                label(check.decision),
                label(check.risk),
                check.matched
            );
            for reason in &check.reasons {
                println!("  - {reason}");
            }
            if check.stopped {
                println!("The emergency stop is on, so it would be refused anyway.");
            }
        }
    }
    Ok(())
}

fn approvals_table(approvals: &[Approval]) {
    let rows: Vec<[String; 7]> = approvals
        .iter()
        .map(|a| {
            let state = match (a.state, a.waiting) {
                (ApprovalState::Pending, true) => "waiting".to_string(),
                (state, _) => label(state),
            };
            [
                a.id.clone(),
                ago(a.created_at),
                a.client.name.clone(),
                a.host.clone().unwrap_or_else(|| "-".into()),
                label(a.risk),
                state,
                one_line(&a.summary, 60),
            ]
        })
        .collect();
    print_table(
        ["ID", "WHEN", "CLIENT", "HOST", "RISK", "STATE", "CALL"],
        &rows,
    );
}

fn show_approval(approval: &Approval) {
    let host = approval.host.as_deref().unwrap_or("-");
    let env = approval
        .env
        .map(|e| format!(" ({})", label(e)))
        .unwrap_or_default();
    println!("{}  {}", approval.id, label(approval.state));
    println!("  host     {host}{env}");
    let session = approval
        .client
        .session
        .as_ref()
        .map(|s| format!(" · session {s}"))
        .unwrap_or_default();
    println!("  client   {}{session}", approval.client.name);
    println!("  method   {}", approval.method);
    println!("  risk     {} ({})", label(approval.risk), approval.matched);
    for reason in &approval.reasons {
        println!("           - {reason}");
    }
    if let Some(cwd) = &approval.cwd {
        println!("  cwd      {cwd}");
    }
    match (&approval.command, &approval.diff) {
        (Some(command), _) => println!("  command\n{}", indent(command)),
        (None, Some(diff)) => println!("  diff\n{}", indent(diff)),
        (None, None) => println!("  call     {}", approval.summary),
    }
    if let Some(by) = &approval.resolved_by {
        let scope = approval
            .scope
            .map(|s| format!(" ({})", label(s)))
            .unwrap_or_default();
        println!("  decided  by {by}{scope}");
    }
    if let Some(note) = &approval.note {
        println!("  note     {note}");
    }
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn describe_rule(rule: &PolicyRule) -> String {
    let when = &rule.when;
    let mut parts = Vec::new();
    let mut add = |name: &str, value: Option<String>| {
        if let Some(value) = value {
            parts.push(format!("{name}={value}"));
        }
    };
    add("host", when.host.clone());
    add("env", when.env.map(label));
    add("tag", when.tag.clone());
    add("client", when.client.clone());
    add("method", when.method.clone());
    add("command", when.command.clone());
    add("path", when.path.clone());
    add("risk>=", when.min_risk.map(label));
    add("risk<=", when.max_risk.map(label));
    let conditions = if parts.is_empty() {
        "every call".to_string()
    } else {
        parts.join(" ")
    };
    format!("{} {conditions}", label(rule.decision))
}

fn rules_table(rules: &[PolicyRule]) {
    let rows: Vec<[String; 4]> = rules
        .iter()
        .map(|r| {
            [
                r.id.clone(),
                describe_rule(r),
                r.created_by.clone(),
                r.note.clone().unwrap_or_default(),
            ]
        })
        .collect();
    print_table(["ID", "RULE", "BY", "NOTE"], &rows);
}

/// Asks the person at the terminal about a call of this process that needs approval.
pub fn ask(approval: &Approval) -> Answer {
    let host = approval
        .host
        .as_deref()
        .map(|host| format!(" on {host}"))
        .unwrap_or_default();
    eprintln!(
        "hostlane: `{}`{host} needs approval ({})",
        approval.summary,
        label(approval.risk)
    );
    for reason in &approval.reasons {
        eprintln!("  - {reason}");
    }
    eprint!("Approve it? [y]es / [n]o / Enter to wait for the Hostlane app: ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        eprintln!();
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Answer::Approve,
        "n" | "no" => Answer::Deny,
        _ => {
            eprintln!("hostlane: waiting for an answer in the Hostlane app…");
            Answer::Wait
        }
    }
}
