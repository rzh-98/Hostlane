use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::{ClientInfo, HostEnv};

/// Risk level of a call, from its method, parsed command line and target paths.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize, TS,
)]
#[ts(export)]
pub enum Risk {
    /// Read-only.
    #[default]
    L0,
    /// Changes in user space: files outside system paths, user processes, unknown programs.
    L1,
    /// System changes: `sudo`, services, packages, containers, system paths.
    L2,
    /// Destructive: recursive deletes of broad paths, disks, reboots, dropped databases.
    L3,
}

impl Risk {
    pub fn level(self) -> u8 {
        self as u8
    }

    pub fn from_level(level: u8) -> Option<Self> {
        match level {
            0 => Some(Self::L0),
            1 => Some(Self::L1),
            2 => Some(Self::L2),
            3 => Some(Self::L3),
            _ => None,
        }
    }
}

/// What the policy does with a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Decision {
    Allow,
    /// A person must approve the call first.
    Approve,
    Deny,
}

/// Conditions of a rule. Every condition that is set must hold; glob patterns use `*`
/// (any text, including `/`) and `?`, and `\` escapes them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RuleConditions {
    /// Glob over host ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub env: Option<HostEnv>,
    /// The host has this tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tag: Option<String>,
    /// Glob over client names, e.g. `claude-code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub client: Option<String>,
    /// Glob over method names, e.g. `files.*`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub method: Option<String>,
    /// Glob over the whole command line of `exec.run`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub command: Option<String>,
    /// Glob over remote paths: file targets, and paths named or written by a command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub min_risk: Option<Risk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max_risk: Option<Risk>,
}

/// A user rule. When several match, `deny` wins over `allow`, and `allow` over `approve`;
/// calls no rule matches get the built-in defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PolicyRule {
    pub id: String,
    pub decision: Decision,
    pub when: RuleConditions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
    #[ts(type = "number")]
    pub created_at: u64,
    /// Client name, or `approval <id>` for rules made by "always allow".
    pub created_by: String,
}

/// Params of `policy.rules.add`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PolicyRuleAddParams {
    pub decision: Decision,
    #[serde(default)]
    pub when: RuleConditions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

/// Params of `policy.rules.delete`, `approvals.get`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IdParams {
    pub id: String,
}

/// Result of `policy.rules.list`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PolicyRules {
    pub rules: Vec<PolicyRule>,
}

/// Result of `policy.status`, `policy.resume`, and pushed in `policy` events.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PolicyStatus {
    /// The emergency stop is on: every call from an agent is refused.
    pub stopped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub stopped_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stopped_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    pub pending_approvals: u32,
}

/// Params of `policy.stop`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PolicyStopParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}

/// Result of `policy.stop`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PolicyStopped {
    pub status: PolicyStatus,
    /// Agent tunnels that were closed.
    pub closed_tunnels: Vec<String>,
    /// Pending approvals that were denied.
    pub denied_approvals: Vec<String>,
}

/// Params of `policy.check`: what the policy would do with a call, without running it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PolicyCheckParams {
    pub method: String,
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub params: Value,
}

/// Result of `policy.check`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PolicyCheck {
    pub decision: Decision,
    pub risk: Risk,
    pub reasons: Vec<String>,
    /// What decided: `rule <id>`, `session grant`, or `default`.
    pub matched: String,
    pub stopped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ApprovalState {
    Pending,
    Approved,
    Denied,
    /// Nobody answered in time, or an approved ticket was not used in time.
    Expired,
}

/// How far an approval reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ApprovalScope {
    /// This call only.
    #[default]
    Once,
    /// The same call (command, path) on this host for the rest of the agent session.
    Session,
    /// Adds an allow rule for the same call on this host.
    Always,
}

/// A call waiting for, or decided by, a person.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Approval {
    /// The `ticket_id`.
    pub id: String,
    pub state: ApprovalState,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub env: Option<HostEnv>,
    pub tags: Vec<String>,
    pub client: ClientInfo,
    pub risk: Risk,
    pub reasons: Vec<String>,
    /// One line for lists and notifications.
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
    /// The change a file call would make, when it can be computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diff: Option<String>,
    /// Call params; file content and stdin are replaced by their size.
    #[ts(type = "unknown")]
    pub params: Value,
    /// What asked for approval: `rule <id>` or `default`.
    pub matched: String,
    #[ts(type = "number")]
    pub created_at: u64,
    /// Pending: when the ticket expires. Approved: when it must be used by.
    #[ts(type = "number")]
    pub expires_at: u64,
    /// A caller is blocked waiting for the answer.
    pub waiting: bool,
    /// The approved call has run.
    pub used: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub resolved_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub resolved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scope: Option<ApprovalScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

/// Params of `approvals.list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApprovalsListParams {
    #[serde(default)]
    pub pending_only: bool,
}

/// Result of `approvals.list`: pending first (oldest first), then decided ones (newest first).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApprovalsList {
    pub approvals: Vec<Approval>,
}

/// Params of `approvals.resolve`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApprovalResolveParams {
    pub id: String,
    pub approve: bool,
    /// Defaults to `once`; ignored when denying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scope: Option<ApprovalScope>,
    /// Passed back to the agent with a denial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

/// Params of `hosts.label`. Replaces the host's labels; an unset `env` with no `tags`
/// goes back to the labels from `~/.ssh/config` (`# hostlane: env=prod tags=web,eu`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HostLabelParams {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub env: Option<HostEnv>,
    #[serde(default)]
    pub tags: Vec<String>,
}
