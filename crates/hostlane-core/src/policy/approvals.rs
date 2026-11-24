//! The approval queue (S3). Tickets live in memory: a pending ticket expires after an
//! hour, an approved one must be used within 15 minutes, and decided tickets are kept
//! for a day as history. Who approved what is recorded in the audit log.

use std::collections::HashMap;
use std::sync::Mutex;

use hostlane_protocol::{Approval, ApprovalScope, ApprovalState, ErrorKind, RpcError};
use tokio::sync::watch;

pub const PENDING_TTL_MS: u64 = 60 * 60 * 1000;
pub const APPROVED_TTL_MS: u64 = 15 * 60 * 1000;
const HISTORY_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const HISTORY_MAX: usize = 200;

struct Ticket {
    approval: Approval,
    fingerprint: String,
    state: watch::Sender<ApprovalState>,
}

pub enum Redeemed {
    /// Approved and now marked used.
    Approved(Box<Approval>),
    Pending(watch::Receiver<ApprovalState>),
    /// The ticket is for another call (e.g. a CLI command making several calls); this
    /// call is judged on its own.
    Mismatch,
}

#[derive(Default)]
pub struct Approvals {
    tickets: Mutex<HashMap<String, Ticket>>,
}

fn not_found(id: &str) -> RpcError {
    RpcError::new(ErrorKind::ApprovalNotFound, format!("no approval `{id}`"))
        .with_hint("tickets are kept for a day and forgotten when the daemon restarts; make the call again without ticket_id")
}

impl Approvals {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Ticket>> {
        self.tickets.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn create(
        &self,
        approval: Approval,
        fingerprint: String,
    ) -> watch::Receiver<ApprovalState> {
        let (state, receiver) = watch::channel(approval.state);
        self.lock().insert(
            approval.id.clone(),
            Ticket {
                approval,
                fingerprint,
                state,
            },
        );
        receiver
    }

    pub fn get(&self, id: &str) -> Option<Approval> {
        self.lock().get(id).map(|t| t.approval.clone())
    }

    pub fn pending_count(&self) -> u32 {
        self.lock()
            .values()
            .filter(|t| t.approval.state == ApprovalState::Pending)
            .count() as u32
    }

    /// Pending first (oldest first), then the rest (newest first).
    pub fn list(&self, pending_only: bool) -> Vec<Approval> {
        let tickets = self.lock();
        let mut pending: Vec<Approval> = Vec::new();
        let mut decided: Vec<Approval> = Vec::new();
        for ticket in tickets.values() {
            if ticket.approval.state == ApprovalState::Pending {
                pending.push(ticket.approval.clone());
            } else if !pending_only {
                decided.push(ticket.approval.clone());
            }
        }
        pending.sort_by_key(|a| a.created_at);
        decided.sort_by_key(|a| std::cmp::Reverse(a.resolved_at.unwrap_or(a.created_at)));
        pending.extend(decided);
        pending
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Approval)) -> Option<Approval> {
        let mut tickets = self.lock();
        let ticket = tickets.get_mut(id)?;
        f(&mut ticket.approval);
        ticket.state.send_replace(ticket.approval.state);
        Some(ticket.approval.clone())
    }

    pub fn resolve(
        &self,
        id: &str,
        approve: bool,
        by: &str,
        scope: ApprovalScope,
        note: Option<String>,
        now: u64,
    ) -> Result<Approval, RpcError> {
        let mut tickets = self.lock();
        let ticket = tickets.get_mut(id).ok_or_else(|| not_found(id))?;
        if ticket.approval.state != ApprovalState::Pending {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                format!("approval `{id}` is already {:?}", ticket.approval.state).to_lowercase(),
            ));
        }
        let approval = &mut ticket.approval;
        approval.state = if approve {
            ApprovalState::Approved
        } else {
            ApprovalState::Denied
        };
        approval.resolved_at = Some(now);
        approval.resolved_by = Some(by.to_string());
        approval.scope = approve.then_some(scope);
        approval.note = note.filter(|n| !n.trim().is_empty());
        if approve {
            approval.expires_at = now + APPROVED_TTL_MS;
        }
        ticket.state.send_replace(approval.state);
        Ok(approval.clone())
    }

    /// Uses an approved ticket for the call with `fingerprint`, or returns a receiver to
    /// wait on while it is pending.
    pub fn redeem(&self, id: &str, fingerprint: &str, now: u64) -> Result<Redeemed, RpcError> {
        let mut tickets = self.lock();
        let ticket = tickets.get_mut(id).ok_or_else(|| not_found(id))?;
        if ticket.fingerprint != fingerprint {
            return Ok(Redeemed::Mismatch);
        }
        let approval = &mut ticket.approval;
        match approval.state {
            ApprovalState::Pending => Ok(Redeemed::Pending(ticket.state.subscribe())),
            ApprovalState::Approved if approval.used => Err(RpcError::new(
                ErrorKind::InvalidParams,
                format!("approval `{id}` was already used"),
            )
            .with_hint("each approval allows one call; make the call again without ticket_id")),
            ApprovalState::Approved if approval.expires_at <= now => {
                approval.state = ApprovalState::Expired;
                ticket.state.send_replace(ApprovalState::Expired);
                Err(expired(id))
            }
            ApprovalState::Approved => {
                approval.used = true;
                Ok(Redeemed::Approved(Box::new(approval.clone())))
            }
            ApprovalState::Denied => Err(denied(approval)),
            ApprovalState::Expired => Err(expired(id)),
        }
    }

    /// Marks an approved ticket used after a blocked caller woke up.
    /// Returns `None` when the ticket is not approved or another call used it first.
    pub fn take(&self, id: &str) -> Option<Approval> {
        let mut tickets = self.lock();
        let approval = &mut tickets.get_mut(id)?.approval;
        if approval.state != ApprovalState::Approved || approval.used {
            return None;
        }
        approval.used = true;
        Some(approval.clone())
    }

    pub fn set_waiting(&self, id: &str, waiting: bool) -> Option<Approval> {
        self.update(id, |a| a.waiting = waiting)
    }

    /// Denies every pending ticket.
    pub fn deny_pending(&self, by: &str, note: &str, now: u64) -> Vec<Approval> {
        let mut tickets = self.lock();
        let mut denied = Vec::new();
        for ticket in tickets.values_mut() {
            if ticket.approval.state == ApprovalState::Pending {
                let approval = &mut ticket.approval;
                approval.state = ApprovalState::Denied;
                approval.resolved_at = Some(now);
                approval.resolved_by = Some(by.to_string());
                approval.note = Some(note.to_string());
                ticket.state.send_replace(ApprovalState::Denied);
                denied.push(approval.clone());
            }
        }
        denied
    }

    /// Expires overdue tickets (returned) and forgets old history.
    pub fn sweep(&self, now: u64) -> Vec<Approval> {
        let mut tickets = self.lock();
        let mut expired = Vec::new();
        for ticket in tickets.values_mut() {
            let approval = &mut ticket.approval;
            let overdue = approval.expires_at <= now
                && (approval.state == ApprovalState::Pending
                    || approval.state == ApprovalState::Approved && !approval.used);
            if overdue {
                approval.state = ApprovalState::Expired;
                approval.resolved_at.get_or_insert(now);
                ticket.state.send_replace(ApprovalState::Expired);
                expired.push(approval.clone());
            }
        }
        let mut decided: Vec<(u64, String)> = tickets
            .values()
            .filter(|t| t.approval.state != ApprovalState::Pending)
            .map(|t| {
                (
                    t.approval.resolved_at.unwrap_or(t.approval.created_at),
                    t.approval.id.clone(),
                )
            })
            .collect();
        decided.sort();
        let excess = decided.len().saturating_sub(HISTORY_MAX);
        for (index, (at, id)) in decided.iter().enumerate() {
            if index < excess || at + HISTORY_TTL_MS <= now {
                tickets.remove(id);
            }
        }
        expired
    }
}

pub fn denied(approval: &Approval) -> RpcError {
    let by = approval.resolved_by.as_deref().unwrap_or("a person");
    let mut error = RpcError::new(
        ErrorKind::PolicyDenied,
        format!("{by} denied `{}`", approval.summary),
    )
    .with_details(serde_json::json!({
        "ticket_id": approval.id,
        "risk": approval.risk,
        "note": approval.note,
    }));
    error = match &approval.note {
        Some(note) => error.with_hint(format!("reason given: {note}")),
        None => error.with_hint("do not retry this call; ask the user how to proceed"),
    };
    error
}

fn expired(id: &str) -> RpcError {
    RpcError::new(
        ErrorKind::ApprovalRequired,
        format!("approval `{id}` expired before it was used"),
    )
    .with_hint("make the call again without ticket_id to ask for a new approval")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hostlane_protocol::{ClientInfo, Risk};

    fn approval(id: &str, created_at: u64) -> Approval {
        Approval {
            id: id.into(),
            state: ApprovalState::Pending,
            method: "exec.run".into(),
            host: Some("web".into()),
            env: None,
            tags: Vec::new(),
            client: ClientInfo::new("agent"),
            risk: Risk::L2,
            reasons: Vec::new(),
            summary: "systemctl restart nginx".into(),
            command: Some("systemctl restart nginx".into()),
            cwd: None,
            path: None,
            diff: None,
            params: serde_json::Value::Null,
            matched: "default".into(),
            created_at,
            expires_at: created_at + PENDING_TTL_MS,
            waiting: false,
            used: false,
            resolved_at: None,
            resolved_by: None,
            scope: None,
            note: None,
        }
    }

    #[test]
    fn approve_then_redeem_once() {
        let approvals = Approvals::default();
        let mut receiver = approvals.create(approval("ap-1", 10), "fp".into());
        assert_eq!(approvals.pending_count(), 1);
        assert!(matches!(
            approvals.redeem("ap-1", "fp", 11).unwrap(),
            Redeemed::Pending(_)
        ));
        assert!(matches!(
            approvals.redeem("ap-1", "other", 11).unwrap(),
            Redeemed::Mismatch
        ));
        approvals
            .resolve("ap-1", true, "desktop", ApprovalScope::Once, None, 20)
            .unwrap();
        assert!(receiver.has_changed().unwrap());
        assert_eq!(*receiver.borrow_and_update(), ApprovalState::Approved);
        assert!(
            approvals
                .resolve("ap-1", false, "desktop", ApprovalScope::Once, None, 21)
                .is_err()
        );
        let Redeemed::Approved(used) = approvals.redeem("ap-1", "fp", 30).unwrap() else {
            panic!("expected an approved ticket");
        };
        assert!(used.used);
        assert!(approvals.redeem("ap-1", "fp", 31).is_err());
    }

    #[test]
    fn denial_expiry_and_history() {
        let approvals = Approvals::default();
        approvals.create(approval("ap-1", 10), "a".into());
        approvals.create(approval("ap-2", 20), "b".into());
        let denied = approvals.deny_pending("emergency stop", "stopped", 30);
        assert_eq!(denied.len(), 2);
        let error = approvals.redeem("ap-1", "a", 31).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::PolicyDenied);

        approvals.create(approval("ap-3", 40), "c".into());
        assert_eq!(approvals.list(true).len(), 1);
        let expired = approvals.sweep(40 + PENDING_TTL_MS);
        assert_eq!(expired.len(), 1);
        assert_eq!(approvals.get("ap-3").unwrap().state, ApprovalState::Expired);
        assert!(
            approvals
                .sweep(40 + PENDING_TTL_MS + HISTORY_TTL_MS)
                .is_empty()
        );
        assert!(approvals.list(false).is_empty());
    }
}
