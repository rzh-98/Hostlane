//! Policy engine: risk levels (S1), rules (S2), the approval queue (S3/S4) and the
//! emergency stop (S5). Every call that touches a server passes [`authorize`] before
//! its handler runs.
//!
//! The policy guards against mistakes, not against a hostile local process: callers
//! name themselves, and anything running as the same OS user could reach the SSH keys
//! directly.

mod approvals;
mod classify;
mod rules;
mod shell;

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use hostlane_protocol::{
    Approval, ApprovalResolveParams, ApprovalScope, ApprovalState, ClientInfo, Decision, ErrorKind,
    Event, PolicyCheck, PolicyCheckParams, PolicyRule, PolicyRuleAddParams, PolicyStatus, Risk,
    RpcError, RuleConditions, method,
};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::Daemon;
use crate::events::EventBus;
use crate::store::Store;
use crate::util::{new_id, now_ms, sha256_hex};

pub use approvals::Approvals;
pub use rules::is_desktop;

use approvals::{PENDING_TTL_MS, Redeemed};
use classify::Assessment;
use rules::{Subject, Verdict};

/// How long a blocking call waits for an answer by default.
const DEFAULT_WAIT: Duration = Duration::from_secs(120);
const MAX_WAIT: Duration = Duration::from_secs(3600);
const SWEEP_INTERVAL: Duration = Duration::from_secs(15);
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_SETTING: &str = "policy.stop";
const SUMMARY_CHARS: usize = 240;

/// Approval options a caller may add to the params of any gated method.
#[derive(Debug, Default, Clone)]
pub struct GateOptions {
    /// Retry a call that needed approval (`ticket_id`).
    pub ticket: Option<String>,
    /// Return `APPROVAL_REQUIRED` right away instead of waiting (`async: true`).
    pub asynchronous: bool,
    /// How long to wait for an answer (`approval_timeout_secs`).
    pub wait: Option<Duration>,
}

impl GateOptions {
    /// Removes the approval options from `params`.
    pub fn take(params: &mut Value) -> Result<Self, RpcError> {
        let Some(object) = params.as_object_mut() else {
            return Ok(Self::default());
        };
        let invalid = |message: &str| RpcError::new(ErrorKind::InvalidParams, message.to_string());
        let ticket = match object.remove("ticket_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(ticket)) => Some(ticket),
            Some(_) => return Err(invalid("`ticket_id` must be a string")),
        };
        let asynchronous = match object.remove("async") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => value,
            Some(_) => return Err(invalid("`async` must be a boolean")),
        };
        let wait = match object.remove("approval_timeout_secs") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let secs = value
                    .as_u64()
                    .ok_or_else(|| invalid("`approval_timeout_secs` must be a number"))?;
                Some(Duration::from_secs(secs).min(MAX_WAIT))
            }
        };
        Ok(Self {
            ticket,
            asynchronous,
            wait,
        })
    }
}

/// Outcome of [`authorize`]: the assessed risk, and who let the call through (`None`
/// when the policy allowed it without anyone's approval) or why it may not run.
pub struct Gate {
    pub risk: Option<Risk>,
    pub result: Result<Option<String>, RpcError>,
}

/// "Allow for this session": the same call on the same host by the same agent session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Grant {
    who: String,
    host: Option<String>,
    method: String,
    key: Option<String>,
}

impl Grant {
    fn new(
        client: &ClientInfo,
        host: Option<&str>,
        method: &str,
        command: Option<&str>,
        path: Option<&str>,
    ) -> Self {
        Self {
            who: match &client.session {
                Some(session) => format!("session {session}"),
                None => format!("client {}", client.name),
            },
            host: host.map(str::to_owned),
            method: method.to_string(),
            key: command
                .map(|c| c.trim().to_string())
                .or(path.map(str::to_owned)),
        }
    }
}

pub struct Policy {
    store: Store,
    events: EventBus,
    rules: Mutex<Vec<PolicyRule>>,
    stop: Mutex<PolicyStatus>,
    stopped: watch::Sender<bool>,
    approvals: Approvals,
    grants: Mutex<HashSet<Grant>>,
    desktop_listeners: AtomicUsize,
    notify: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Counts a desktop app listening for events while alive.
pub struct DesktopListener<'a>(&'a Policy);

impl Drop for DesktopListener<'_> {
    fn drop(&mut self) {
        self.0.desktop_listeners.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Policy {
    pub fn new(store: Store, events: EventBus) -> Self {
        let notify = std::env::var("HOSTLANE_NOTIFY")
            .map(|v| !matches!(v.trim(), "0" | "off" | "false" | "no"))
            .unwrap_or(true);
        Self {
            store,
            events,
            rules: Mutex::new(Vec::new()),
            stop: Mutex::new(PolicyStatus::default()),
            stopped: watch::channel(false).0,
            approvals: Approvals::default(),
            grants: Mutex::new(HashSet::new()),
            desktop_listeners: AtomicUsize::new(0),
            notify,
        }
    }

    /// Loads rules and the emergency stop state.
    pub async fn load(&self) {
        match self.store.policy_rules().await {
            Ok(rules) => *lock(&self.rules) = rules,
            Err(err) => tracing::error!(%err, "cannot load policy rules"),
        }
        match self.store.setting(STOP_SETTING).await {
            Ok(Some(text)) => {
                if let Ok(status) = serde_json::from_str::<PolicyStatus>(&text)
                    && status.stopped
                {
                    tracing::warn!("the emergency stop is on; agent calls are refused");
                    *lock(&self.stop) = status;
                    self.stopped.send_replace(true);
                }
            }
            Ok(None) => {}
            Err(err) => tracing::error!(%err, "cannot load the emergency stop state"),
        }
    }

    pub fn status(&self) -> PolicyStatus {
        let mut status = lock(&self.stop).clone();
        status.pending_approvals = self.approvals.pending_count();
        status
    }

    pub fn is_stopped(&self) -> bool {
        *self.stopped.borrow()
    }

    /// Changes to `true` when the emergency stop is switched on.
    pub fn stop_signal(&self) -> watch::Receiver<bool> {
        self.stopped.subscribe()
    }

    pub fn desktop_listener(&self) -> DesktopListener<'_> {
        self.desktop_listeners.fetch_add(1, Ordering::SeqCst);
        DesktopListener(self)
    }

    fn publish_status(&self) {
        self.events.publish(Event::Policy {
            status: self.status(),
        });
    }

    fn publish_approval(&self, approval: Option<Approval>) {
        if let Some(approval) = approval {
            self.events.publish(Event::ApprovalUpdated { approval });
        }
    }

    /// Switches the emergency stop on and denies every pending approval. The caller
    /// closes agent tunnels.
    pub async fn stop(
        &self,
        by: &ClientInfo,
        reason: Option<String>,
    ) -> Result<Vec<String>, RpcError> {
        let now = now_ms();
        let status = PolicyStatus {
            stopped: true,
            stopped_at: Some(now),
            stopped_by: Some(by.name.clone()),
            reason: reason.filter(|r| !r.trim().is_empty()),
            pending_approvals: 0,
        };
        let text = serde_json::to_string(&status).unwrap_or_default();
        self.store
            .set_setting(STOP_SETTING.into(), Some(text))
            .await?;
        *lock(&self.stop) = status;
        self.stopped.send_replace(true);
        let denied = self.approvals.deny_pending(
            "emergency stop",
            "Hostlane was paused with the emergency stop",
            now,
        );
        let ids = denied.iter().map(|a| a.id.clone()).collect();
        for approval in denied {
            self.publish_approval(Some(approval));
        }
        self.publish_status();
        Ok(ids)
    }

    pub async fn resume(&self) -> Result<PolicyStatus, RpcError> {
        self.store.set_setting(STOP_SETTING.into(), None).await?;
        *lock(&self.stop) = PolicyStatus::default();
        self.stopped.send_replace(false);
        self.publish_status();
        Ok(self.status())
    }

    pub fn rules(&self) -> Vec<PolicyRule> {
        lock(&self.rules).clone()
    }

    pub async fn add_rule(
        &self,
        params: PolicyRuleAddParams,
        created_by: String,
    ) -> Result<PolicyRule, RpcError> {
        let when = clean_conditions(params.when);
        if let (Some(min), Some(max)) = (when.min_risk, when.max_risk)
            && min > max
        {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                "`min_risk` is above `max_risk`",
            ));
        }
        let rule = PolicyRule {
            id: new_id("ru"),
            decision: params.decision,
            when,
            note: params.note.filter(|n| !n.trim().is_empty()),
            created_at: now_ms(),
            created_by,
        };
        self.store.insert_rule(rule.clone()).await?;
        lock(&self.rules).push(rule.clone());
        Ok(rule)
    }

    pub async fn delete_rule(&self, id: &str) -> Result<(), RpcError> {
        if !lock(&self.rules).iter().any(|r| r.id == id) {
            return Err(
                RpcError::new(ErrorKind::RuleNotFound, format!("no rule `{id}`"))
                    .with_hint("list rules with policy.rules.list (CLI: hostlane policy rules)"),
            );
        }
        self.store.delete_rule(id.to_string()).await?;
        lock(&self.rules).retain(|r| r.id != id);
        Ok(())
    }

    pub fn approvals(&self) -> &Approvals {
        &self.approvals
    }

    /// Decides a pending approval; "session" and "always" also cover later identical calls.
    pub async fn resolve(
        &self,
        params: ApprovalResolveParams,
        by: &ClientInfo,
    ) -> Result<Approval, RpcError> {
        let scope = params.scope.unwrap_or_default();
        let approval = self.approvals.resolve(
            &params.id,
            params.approve,
            &by.name,
            scope,
            params.note,
            now_ms(),
        )?;
        if approval.state == ApprovalState::Approved {
            match scope {
                ApprovalScope::Once => {}
                ApprovalScope::Session => {
                    lock(&self.grants).insert(Grant::new(
                        &approval.client,
                        approval.host.as_deref(),
                        &approval.method,
                        approval.command.as_deref(),
                        approval.path.as_deref(),
                    ));
                }
                ApprovalScope::Always => {
                    let when = RuleConditions {
                        host: approval.host.as_deref().map(rules::escape),
                        method: Some(rules::escape(&approval.method)),
                        command: approval.command.as_deref().map(|c| rules::escape(c.trim())),
                        path: match approval.command {
                            Some(_) => None,
                            None => approval.path.as_deref().map(rules::escape),
                        },
                        ..Default::default()
                    };
                    let params = PolicyRuleAddParams {
                        decision: Decision::Allow,
                        when,
                        note: Some(truncate(&approval.summary, SUMMARY_CHARS)),
                    };
                    if let Err(err) = self
                        .add_rule(params, format!("approval {}", approval.id))
                        .await
                    {
                        tracing::warn!(%err, "cannot save an always-allow rule");
                    }
                }
            }
        }
        self.publish_approval(Some(approval.clone()));
        self.publish_status();
        Ok(approval)
    }

    fn granted(&self, subject: &Subject) -> bool {
        let grant = Grant::new(
            &subject.client,
            subject.host.as_deref(),
            &subject.method,
            subject.command.as_deref(),
            subject.path.as_deref(),
        );
        lock(&self.grants).contains(&grant)
    }

    fn decide(&self, subject: &Subject) -> Verdict {
        rules::decide(&lock(&self.rules), self.granted(subject), subject)
    }

    /// Expires overdue approvals. Runs forever.
    pub async fn maintain(self: Arc<Self>) {
        let mut interval = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            interval.tick().await;
            let expired = self.approvals.sweep(now_ms());
            if !expired.is_empty() {
                for approval in expired {
                    self.publish_approval(Some(approval));
                }
                self.publish_status();
            }
        }
    }
}

fn clean_conditions(when: RuleConditions) -> RuleConditions {
    let text = |value: Option<String>| {
        value
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    RuleConditions {
        host: text(when.host),
        tag: text(when.tag),
        client: text(when.client),
        method: text(when.method),
        command: text(when.command),
        path: text(when.path),
        ..when
    }
}

/// Approving, changing rules and labels, and resuming after an emergency stop are for
/// people: the desktop app, or the CLI typed by hand.
pub fn require_person(client: &ClientInfo, action: &str) -> Result<(), RpcError> {
    if is_desktop(client) || client.name == "cli" {
        return Ok(());
    }
    Err(RpcError::new(
        ErrorKind::PolicyDenied,
        format!("`{}` may not {action}", client.name),
    )
    .with_hint("only a person can do this, in the Hostlane app or with the hostlane CLI"))
}

fn truncate(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut result: String = line.chars().take(max).collect();
    if result.len() < line.len() || text.lines().nth(1).is_some() {
        result.push('…');
    }
    result
}

/// What a call does, before host labels are looked up.
struct Described {
    host: Option<String>,
    assessment: Assessment,
    command: Option<String>,
    cwd: Option<String>,
    path: Option<String>,
    summary: String,
}

fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

async fn describe(daemon: &Daemon, name: &str, params: &Value) -> Described {
    let mut host = str_param(params, "host").map(str::to_owned);
    let mut command = None;
    let mut cwd = None;
    let mut path = None;
    let mut assessment = Assessment::new();
    let summary;
    match name {
        method::EXEC_RUN => {
            let line = str_param(params, "command").unwrap_or_default();
            cwd = str_param(params, "cwd").map(str::to_owned);
            assessment = classify::command(line, cwd.as_deref(), str_param(params, "stdin"));
            command = Some(line.to_string());
            summary = truncate(line.trim(), SUMMARY_CHARS);
        }
        method::FILES_WRITE | method::FILES_PATCH | method::FILES_UPLOAD => {
            let target = str_param(params, "path")
                .or(str_param(params, "remote_path"))
                .unwrap_or_default();
            assessment = classify::file_write(target);
            path = Some(classify::normalize(target));
            let verb = name.trim_start_matches("files.");
            summary = format!("{verb} {target}");
        }
        method::FILES_REVERT => {
            let id = str_param(params, "change_id").unwrap_or_default();
            let change = daemon
                .store
                .get_file_change(id.to_string())
                .await
                .ok()
                .flatten();
            match change {
                Some(change) => {
                    assessment = classify::file_write(&change.path);
                    summary = format!("revert {} ({id})", change.path);
                    path = Some(change.path);
                    host = Some(change.host);
                }
                None => {
                    assessment.raise(Risk::L1, "reverts a file change");
                    summary = format!("revert {id}");
                }
            }
        }
        method::FILES_READ | method::FILES_LIST | method::FILES_SEARCH | method::FILES_DOWNLOAD => {
            let target = str_param(params, "path")
                .or(str_param(params, "remote_path"))
                .unwrap_or("~");
            assessment = classify::file_read(target);
            path = Some(classify::normalize(target));
            summary = format!("{} {target}", name.trim_start_matches("files."));
        }
        method::SERVICES_LOGS
        | method::SERVICES_START
        | method::SERVICES_STOP
        | method::SERVICES_RESTART
        | method::PROCESSES_KILL => {
            let user = match host.as_deref() {
                Some(host) => crate::services::login_user(daemon, host)
                    .await
                    .unwrap_or_else(|_| crate::ssh::local_user()),
                None => crate::ssh::local_user(),
            };
            match crate::services::command(name, params, &user) {
                Ok(line) => {
                    assessment = classify::command(&line, None, None);
                    summary = truncate(&line, SUMMARY_CHARS);
                    command = Some(line);
                }
                Err(err) => {
                    assessment.raise(Risk::L2, format!("invalid params: {}", err.message));
                    summary = name.to_string();
                }
            }
        }
        method::HOSTS_TRUST => {
            let fingerprint = str_param(params, "fingerprint").unwrap_or_default();
            assessment.raise(Risk::L2, format!("trusts a new host key ({fingerprint})"));
            summary = format!("trust host key {fingerprint}");
        }
        method::TUNNELS_OPEN | method::TUNNELS_UPDATE => {
            let remote = format!(
                "{}:{}",
                str_param(params, "remote_host").unwrap_or("127.0.0.1"),
                params
                    .get("remote_port")
                    .and_then(Value::as_u64)
                    .map_or("…".into(), |p| p.to_string())
            );
            if name == method::TUNNELS_UPDATE {
                let id = str_param(params, "id").unwrap_or_default();
                host = daemon.tunnels.host_of(id);
                summary = format!("update tunnel {id}");
            } else {
                summary = format!("forward {remote} to this machine");
            }
            assessment.raise(Risk::L1, "makes a server port reachable on this machine");
            let bind = str_param(params, "bind").unwrap_or("127.0.0.1");
            let public = params.get("allow_public").and_then(Value::as_bool) == Some(true);
            let loopback = bind
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
            if public || !loopback {
                assessment.raise(
                    Risk::L2,
                    format!("listens on {bind}, reachable from the network"),
                );
            }
        }
        method::TUNNELS_START | method::TUNNELS_DELETE | method::TUNNELS_CLOSE => {
            let id = str_param(params, "id").unwrap_or_default();
            host = daemon.tunnels.host_of(id);
            if name != method::TUNNELS_CLOSE {
                assessment.raise(Risk::L1, "starts or deletes a saved tunnel");
            }
            summary = format!("{} tunnel {id}", name.trim_start_matches("tunnels."));
        }
        _ => {
            summary = match &host {
                Some(host) => format!("{name} on {host}"),
                None => name.to_string(),
            };
        }
    }
    Described {
        host,
        assessment,
        command,
        cwd,
        path,
        summary,
    }
}

async fn subject(
    daemon: &Daemon,
    client: &ClientInfo,
    name: &str,
    params: &Value,
) -> (Subject, Described) {
    let described = describe(daemon, name, params).await;
    let (mut env, mut tags) = (None, Vec::new());
    if let Some(host) = &described.host
        && let Ok(loaded) = daemon.load_hosts().await
        && let Some(entry) = loaded.find(host)
    {
        env = entry.host.env;
        tags = entry.host.tags.clone();
    }
    let subject = Subject {
        method: name.to_string(),
        host: described.host.clone(),
        env,
        tags,
        client: client.clone(),
        command: described.command.clone(),
        path: described.path.clone(),
        paths: described.assessment.paths.clone(),
        risk: described.assessment.risk,
    };
    (subject, described)
}

/// Identifies "the same call" for tickets: caller, method and params.
fn fingerprint(client: &ClientInfo, name: &str, params: &Value) -> String {
    let text = json!([client.name, client.session, name, params]).to_string();
    sha256_hex(text.as_bytes())
}

fn stopped_error(status: &PolicyStatus) -> RpcError {
    let mut message = "Hostlane is paused by the emergency stop".to_string();
    if let Some(reason) = &status.reason {
        message.push_str(&format!(": {reason}"));
    }
    RpcError::new(ErrorKind::PolicyDenied, message)
        .with_hint("stop and tell the user; only they can resume Hostlane")
        .with_details(json!({ "stopped": true }))
}

fn denied_error(subject: &Subject, described: &Described, verdict: &Verdict) -> RpcError {
    let reasons = described.assessment.reasons();
    let hint = match verdict.matched.as_str() {
        "default" => format!(
            "{:?} calls are refused on {} hosts by default; ask the user to do it, or to add an allow rule",
            subject.risk,
            subject
                .env
                .map_or("these".into(), |e| format!("{e:?}").to_lowercase()),
        ),
        rule => format!("refused by policy {rule}; ask the user"),
    };
    RpcError::new(
        ErrorKind::PolicyDenied,
        format!("policy refuses `{}`", described.summary),
    )
    .with_hint(hint)
    .with_details(json!({
        "risk": subject.risk,
        "reasons": reasons,
        "matched": verdict.matched,
    }))
}

fn approval_required(approval: &Approval, waited: Option<Duration>) -> RpcError {
    let message = match waited {
        Some(waited) => format!(
            "no answer to approval `{}` within {}s; it is still pending",
            approval.id,
            waited.as_secs()
        ),
        None => format!("`{}` needs approval", approval.summary),
    };
    RpcError::new(ErrorKind::ApprovalRequired, message)
        .with_hint(format!(
            "the user decides in the Hostlane app; repeat the same call with \"ticket_id\": \"{}\" to wait for the answer (CLI: --ticket {}), or poll approvals.get",
            approval.id, approval.id
        ))
        .with_details(json!({
            "ticket_id": approval.id,
            "risk": approval.risk,
            "reasons": approval.reasons,
            "expires_at": approval.expires_at,
        }))
}

/// Applies the emergency stop, approvals and rules to a call of a gated method.
pub async fn authorize(
    daemon: &Daemon,
    client: &ClientInfo,
    name: &str,
    params: &Value,
    options: GateOptions,
) -> Gate {
    let policy = &daemon.policy;
    if policy.is_stopped() && !is_desktop(client) {
        return Gate {
            risk: None,
            result: Err(stopped_error(&policy.status())),
        };
    }
    let (subject, described) = subject(daemon, client, name, params).await;
    let risk = Some(subject.risk);
    let fingerprint = fingerprint(client, name, params);
    let wait = options.wait.unwrap_or(DEFAULT_WAIT);

    if let Some(ticket) = &options.ticket {
        let result = match policy.approvals.redeem(ticket, &fingerprint, now_ms()) {
            Ok(Redeemed::Approved(approval)) => {
                let approver = approval.resolved_by.clone();
                policy.publish_approval(Some(*approval));
                Some(Ok(approver))
            }
            Ok(Redeemed::Pending(receiver)) if !options.asynchronous => {
                Some(wait_for(policy, ticket, receiver, wait).await)
            }
            Ok(Redeemed::Pending(_)) => Some(match policy.approvals.get(ticket) {
                Some(approval) => Err(approval_required(&approval, None)),
                None => Err(RpcError::new(
                    ErrorKind::ApprovalNotFound,
                    "approval vanished",
                )),
            }),
            Ok(Redeemed::Mismatch) => None,
            Err(err) => Some(Err(err)),
        };
        if let Some(result) = result {
            return Gate { risk, result };
        }
    }

    let verdict = policy.decide(&subject);
    let result = match verdict.decision {
        Decision::Allow => Ok(verdict.approver),
        Decision::Deny => Err(denied_error(&subject, &described, &verdict)),
        Decision::Approve => {
            let approval = new_approval(daemon, name, params, &subject, described, verdict).await;
            let id = approval.id.clone();
            let receiver = policy.approvals.create(approval.clone(), fingerprint);
            if policy.notify && policy.desktop_listeners.load(Ordering::SeqCst) == 0 {
                notify(&approval);
            }
            daemon.events.publish(Event::ApprovalRequested {
                approval: approval.clone(),
            });
            policy.publish_status();
            if options.asynchronous {
                Err(approval_required(&approval, None))
            } else {
                wait_for(policy, &id, receiver, wait).await
            }
        }
    };
    Gate { risk, result }
}

async fn new_approval(
    daemon: &Daemon,
    name: &str,
    params: &Value,
    subject: &Subject,
    described: Described,
    verdict: Verdict,
) -> Approval {
    let diff = if matches!(name, method::FILES_WRITE | method::FILES_PATCH) {
        let preview = tokio::time::timeout(PREVIEW_TIMEOUT, crate::files::preview(daemon, params))
            .await
            .ok()
            .flatten();
        preview.or_else(|| str_param(params, "diff").map(str::to_owned))
    } else {
        None
    };
    let now = now_ms();
    Approval {
        id: new_id("ap"),
        state: ApprovalState::Pending,
        method: name.to_string(),
        host: subject.host.clone(),
        env: subject.env,
        tags: subject.tags.clone(),
        client: subject.client.clone(),
        risk: subject.risk,
        reasons: described.assessment.reasons(),
        summary: described.summary,
        command: described.command,
        cwd: described.cwd,
        path: described.path,
        diff,
        params: crate::handlers::redact_params(params),
        matched: verdict.matched,
        created_at: now,
        expires_at: now + PENDING_TTL_MS,
        waiting: false,
        used: false,
        resolved_at: None,
        resolved_by: None,
        scope: None,
        note: None,
    }
}

async fn wait_for(
    policy: &Policy,
    id: &str,
    mut receiver: watch::Receiver<ApprovalState>,
    wait: Duration,
) -> Result<Option<String>, RpcError> {
    policy.publish_approval(policy.approvals.set_waiting(id, true));
    let answered = tokio::time::timeout(wait, async {
        receiver
            .wait_for(|state| *state != ApprovalState::Pending)
            .await
            .is_ok()
    })
    .await;
    let approval = policy.approvals.set_waiting(id, false);
    let Some(approval) = approval else {
        return Err(RpcError::new(
            ErrorKind::ApprovalNotFound,
            format!("approval `{id}` was forgotten"),
        ));
    };
    match (answered, approval.state) {
        (_, ApprovalState::Approved) => match policy.approvals.take(id) {
            Some(approval) => {
                let approver = approval.resolved_by.clone();
                policy.publish_approval(Some(approval));
                Ok(approver)
            }
            None => Err(RpcError::new(
                ErrorKind::ApprovalRequired,
                format!("approval `{id}` was already used"),
            )
            .with_hint("make the call again without ticket_id to ask for a new approval")),
        },
        (_, ApprovalState::Denied) => {
            policy.publish_approval(Some(approval.clone()));
            Err(approvals::denied(&approval))
        }
        (Err(_), ApprovalState::Pending) => {
            policy.publish_approval(Some(approval.clone()));
            Err(approval_required(&approval, Some(wait)))
        }
        _ => {
            policy.publish_approval(Some(approval.clone()));
            Err(RpcError::new(
                ErrorKind::ApprovalRequired,
                format!("approval `{id}` expired or was already used"),
            )
            .with_hint("make the call again without ticket_id to ask for a new approval"))
        }
    }
}

/// `policy.check`: what would happen to a call, without running it or asking anyone.
pub async fn check(daemon: &Daemon, client: &ClientInfo, params: PolicyCheckParams) -> PolicyCheck {
    let (subject, described) = subject(daemon, client, &params.method, &params.params).await;
    let verdict = daemon.policy.decide(&subject);
    PolicyCheck {
        decision: verdict.decision,
        risk: subject.risk,
        reasons: described.assessment.reasons(),
        matched: verdict.matched,
        stopped: daemon.policy.is_stopped() && !is_desktop(client),
    }
}

/// System notification for an approval when no desktop app is listening.
fn notify(approval: &Approval) {
    let title = "Hostlane: approval needed";
    let host = approval.host.as_deref().unwrap_or("Hostlane");
    let body = truncate(
        &format!(
            "{} on {host} ({:?}): {}",
            approval.client.name, approval.risk, approval.summary
        ),
        180,
    );
    #[cfg(target_os = "macos")]
    let command = {
        let quote = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
        let script = format!(
            "display notification {} with title {}",
            quote(&body),
            quote(title)
        );
        let mut command = std::process::Command::new("osascript");
        command.arg("-e").arg(script);
        Some(command)
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let command = {
        let mut command = std::process::Command::new("notify-send");
        command.arg("--app-name=Hostlane").arg(title).arg(&body);
        Some(command)
    };
    #[cfg(not(unix))]
    let command: Option<std::process::Command> = {
        let _ = (title, body);
        None
    };
    if let Some(mut command) = command {
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        std::thread::spawn(move || {
            if let Err(err) = command.status() {
                tracing::debug!(%err, "cannot show a system notification");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_options_are_removed_from_params() {
        let mut params = json!({
            "host": "web",
            "command": "ls",
            "ticket_id": "ap-1",
            "async": true,
            "approval_timeout_secs": 99999,
        });
        let options = GateOptions::take(&mut params).unwrap();
        assert_eq!(options.ticket.as_deref(), Some("ap-1"));
        assert!(options.asynchronous);
        assert_eq!(options.wait, Some(MAX_WAIT));
        assert_eq!(params, json!({ "host": "web", "command": "ls" }));
        assert!(GateOptions::take(&mut json!({ "async": "yes" })).is_err());
    }

    #[test]
    fn grants_key_on_session_and_call() {
        let mut client = ClientInfo::new("agent");
        client.session = Some("s1".into());
        let a = Grant::new(&client, Some("web"), "exec.run", Some(" ls "), None);
        let b = Grant::new(&client, Some("web"), "exec.run", Some("ls"), None);
        assert_eq!(a, b);
        client.session = Some("s2".into());
        assert_ne!(
            a,
            Grant::new(&client, Some("web"), "exec.run", Some("ls"), None)
        );
    }

    #[test]
    fn summaries_are_one_line() {
        assert_eq!(truncate("a\nb", 10), "a…");
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("abc", 3), "abc");
    }
}
