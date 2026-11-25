//! SQLite storage: the append-only audit log (`calls`), command records for the
//! session monitor (`execs`), file changes with their diffs (`file_changes`) and the
//! user's notes on ports (`port_notes`), saved tunnels (`tunnels`), policy rules
//! (`policy_rules`), host labels (`host_labels`), hosts added in Hostlane
//! (`manual_hosts`), which secrets are in the keychain (`secrets`, never the secrets
//! themselves), background jobs (`jobs`), registered agents with the hash of their
//! token (`clients`) and small settings (`settings`).
//! The database lives at `<data dir>/hostlane.db`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use hostlane_protocol::{
    AuditEntry, AuditList, AuditListParams, AuditOutcome, ClientInfo, ErrorKind, ExecListParams,
    ExecRecord, ExecStatus, FileChange, FileOperation, HostAddParams, HostEnv, Job, JobBackend,
    JobState, JobsListParams, PolicyRule, PortNote, RegisteredClient, Risk, RpcError, SavedSecret,
    SecretKind,
};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter, types::Value as Sql};
use serde_json::{Value, json};

use crate::tunnels::TunnelConfig;

const SCHEMA_VERSION: i64 = 8;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at INTEGER NOT NULL,
    finished_at INTEGER NOT NULL,
    client TEXT NOT NULL,
    session TEXT,
    host TEXT,
    method TEXT NOT NULL,
    params TEXT NOT NULL,
    outcome TEXT NOT NULL,
    error_kind TEXT,
    error_message TEXT,
    result TEXT NOT NULL,
    approver TEXT
);
CREATE INDEX IF NOT EXISTS calls_host ON calls (host, id);
CREATE INDEX IF NOT EXISTS calls_started ON calls (started_at);
CREATE TRIGGER IF NOT EXISTS calls_no_update BEFORE UPDATE ON calls
BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;
CREATE TRIGGER IF NOT EXISTS calls_no_delete BEFORE DELETE ON calls
BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;

CREATE TABLE IF NOT EXISTS execs (
    id TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    command TEXT NOT NULL,
    cwd TEXT,
    client TEXT NOT NULL,
    session TEXT,
    status TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    exit_code INTEGER,
    signal TEXT,
    error TEXT,
    stdout_bytes INTEGER NOT NULL DEFAULT 0,
    stderr_bytes INTEGER NOT NULL DEFAULT 0,
    params TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS execs_started ON execs (started_at);
CREATE INDEX IF NOT EXISTS execs_host ON execs (host, started_at);

CREATE TABLE IF NOT EXISTS file_changes (
    id TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    path TEXT NOT NULL,
    operation TEXT NOT NULL,
    client TEXT NOT NULL,
    session TEXT,
    created_at INTEGER NOT NULL,
    created INTEGER NOT NULL,
    deleted INTEGER NOT NULL,
    previous_sha256 TEXT,
    sha256 TEXT NOT NULL,
    size INTEGER NOT NULL,
    added_lines INTEGER NOT NULL,
    removed_lines INTEGER NOT NULL,
    diff TEXT,
    has_backup INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS file_changes_host ON file_changes (host, path, created_at);

CREATE TABLE IF NOT EXISTS port_notes (
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    name TEXT,
    project TEXT,
    note TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (host, port)
);

CREATE TABLE IF NOT EXISTS tunnels (
    id TEXT PRIMARY KEY,
    name TEXT,
    host TEXT NOT NULL,
    bind TEXT NOT NULL,
    local_port INTEGER NOT NULL,
    remote_host TEXT NOT NULL,
    remote_port INTEGER NOT NULL,
    auto_start INTEGER NOT NULL,
    client TEXT NOT NULL,
    session TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS policy_rules (
    id TEXT PRIMARY KEY,
    decision TEXT NOT NULL,
    conditions TEXT NOT NULL,
    note TEXT,
    created_at INTEGER NOT NULL,
    created_by TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS host_labels (
    host TEXT PRIMARY KEY,
    env TEXT,
    tags TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS manual_hosts (
    id TEXT PRIMARY KEY,
    hostname TEXT NOT NULL,
    user TEXT,
    port INTEGER NOT NULL,
    proxy_jump TEXT,
    identity_file TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS secrets (
    account TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    host TEXT,
    key_path TEXT,
    saved_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    name TEXT,
    command TEXT NOT NULL,
    cwd TEXT,
    client TEXT NOT NULL,
    session TEXT,
    verified INTEGER NOT NULL DEFAULT 0,
    backend TEXT NOT NULL,
    state TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    exit_code INTEGER,
    pid INTEGER,
    output_bytes INTEGER NOT NULL DEFAULT 0,
    checked_at INTEGER NOT NULL,
    approver TEXT,
    params TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS jobs_started ON jobs (started_at);

CREATE TABLE IF NOT EXISTS clients (
    name TEXT PRIMARY KEY,
    token_sha256 TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Columns added after a table was first created: (table, column, definition).
const ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    ("calls", "task", "TEXT"),
    ("execs", "task", "TEXT"),
    ("file_changes", "task", "TEXT"),
    ("calls", "risk", "INTEGER"),
    ("execs", "approver", "TEXT"),
    ("calls", "verified", "INTEGER NOT NULL DEFAULT 0"),
    ("execs", "verified", "INTEGER NOT NULL DEFAULT 0"),
];

/// Host labels set in Hostlane: environment and tags.
pub type HostLabels = HashMap<String, (Option<HostEnv>, Vec<String>)>;

/// A call to append to the audit log.
pub struct NewCall {
    pub started_at: u64,
    pub finished_at: u64,
    pub client: ClientInfo,
    pub host: Option<String>,
    pub method: String,
    pub params: Value,
    pub outcome: AuditOutcome,
    pub error: Option<RpcError>,
    pub result: Value,
    pub approver: Option<String>,
    pub risk: Option<Risk>,
}

impl NewCall {
    /// The entry this call becomes once stored with `id`.
    pub fn entry(&self, id: i64) -> AuditEntry {
        AuditEntry {
            id,
            started_at: self.started_at,
            finished_at: self.finished_at,
            client: self.client.clone(),
            host: self.host.clone(),
            method: self.method.clone(),
            params: self.params.clone(),
            outcome: self.outcome,
            error_kind: self.error.as_ref().map(RpcError::kind),
            error_message: self.error.as_ref().map(|e| e.message.clone()),
            result: self.result.clone(),
            approver: self.approver.clone(),
            risk: self.risk,
        }
    }
}

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

fn internal(err: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorKind::Internal, format!("database error: {err}"))
}

fn enum_text(value: impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn enum_parse<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_value(Value::String(text.to_owned())).ok()
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, rusqlite::Error> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        for (table, column, definition) in ADDED_COLUMNS {
            let exists: bool = conn.query_row(
                &format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                [column],
                |row| row.get::<_, i64>(0).map(|n| n > 0),
            )?;
            if !exists {
                conn.execute_batch(&format!(
                    "ALTER TABLE {table} ADD COLUMN {column} {definition}"
                ))?;
            }
        }
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        conn.execute_batch("CREATE INDEX IF NOT EXISTS calls_task ON calls(task,id); CREATE INDEX IF NOT EXISTS calls_session ON calls(session,id); CREATE INDEX IF NOT EXISTS changes_task ON file_changes(task,created_at)")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `f` on a blocking thread with the connection locked.
    async fn with<T, F>(&self, f: F) -> Result<T, RpcError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|e| e.into_inner());
            f(&conn)
        })
        .await
        .map_err(internal)?
        .map_err(internal)
    }

    pub async fn append_call(&self, call: NewCall) -> Result<i64, RpcError> {
        self.with(move |conn| insert_call(conn, &call)).await
    }

    pub async fn list_calls(&self, params: AuditListParams) -> Result<AuditList, RpcError> {
        self.with(move |conn| {
            let mut sql = String::from("SELECT * FROM calls WHERE 1=1");
            let mut args: Vec<Sql> = Vec::new();
            let mut filter = |clause: &str, value: Sql| {
                sql.push_str(clause);
                args.push(value);
            };
            if let Some(host) = params.host {
                filter(" AND host = ?", Sql::Text(host));
            }
            if let Some(task) = params.task {
                filter(" AND task = ?", Sql::Text(task));
            }
            if let Some(session) = params.session {
                filter(" AND session = ?", Sql::Text(session));
            }
            if let Some(client) = params.client {
                filter(" AND client = ?", Sql::Text(client));
            }
            if let Some(method) = params.method {
                filter(" AND method = ?", Sql::Text(method));
            }
            if let Some(risk) = params.min_risk {
                filter(" AND risk >= ?", Sql::Integer(risk.level() as i64));
            }
            if let Some(since) = params.since {
                filter(" AND started_at >= ?", Sql::Integer(since as i64));
            }
            if let Some(until) = params.until {
                filter(" AND started_at < ?", Sql::Integer(until as i64));
            }
            if let Some(before) = params.before_id {
                filter(" AND id < ?", Sql::Integer(before));
            }
            let limit = params.limit.unwrap_or(100).clamp(1, 1000) as usize;
            sql.push_str(&format!(" ORDER BY id DESC LIMIT {}", limit + 1));
            let mut stmt = conn.prepare(&sql)?;
            let mut entries = stmt
                .query_map(params_from_iter(args), call_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            let has_more = entries.len() > limit;
            entries.truncate(limit);
            Ok(AuditList { entries, has_more })
        })
        .await
    }

    pub async fn insert_exec(&self, record: ExecRecord, params: Value) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute(
                "INSERT INTO execs (id, host, command, cwd, client, session, status, started_at, params,
                 approver, verified, task)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    record.id,
                    record.host,
                    record.command,
                    record.cwd,
                    record.client.name,
                    record.client.session,
                    enum_text(record.status),
                    record.started_at as i64,
                    params.to_string(),
                    record.approver,
                    record.client.verified,
                    record.client.task,
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn finish_exec(&self, record: ExecRecord) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute(
                "UPDATE execs SET status = ?2, finished_at = ?3, exit_code = ?4, signal = ?5,
                 error = ?6, stdout_bytes = ?7, stderr_bytes = ?8 WHERE id = ?1",
                params![
                    record.id,
                    enum_text(record.status),
                    record.finished_at.map(|t| t as i64),
                    record.exit_code,
                    record.signal,
                    record.error,
                    record.stdout_bytes as i64,
                    record.stderr_bytes as i64,
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn get_exec(&self, id: String) -> Result<Option<ExecRecord>, RpcError> {
        self.with(move |conn| {
            conn.query_row("SELECT * FROM execs WHERE id = ?1", [id], exec_from_row)
                .optional()
        })
        .await
    }

    pub async fn list_execs(&self, params: ExecListParams) -> Result<Vec<ExecRecord>, RpcError> {
        self.with(move |conn| {
            let mut sql = String::from("SELECT * FROM execs WHERE 1=1");
            let mut args: Vec<Sql> = Vec::new();
            if let Some(host) = params.host {
                sql.push_str(" AND host = ?");
                args.push(Sql::Text(host));
            }
            if let Some(before) = params.before {
                sql.push_str(" AND started_at < ?");
                args.push(Sql::Integer(before as i64));
            }
            let limit = params.limit.unwrap_or(50).clamp(1, 500);
            sql.push_str(&format!(" ORDER BY started_at DESC, id DESC LIMIT {limit}"));
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(params_from_iter(args), exec_from_row)?
                .collect()
        })
        .await
    }

    pub async fn insert_file_change(&self, change: FileChange) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute(
                "INSERT INTO file_changes (id, host, path, operation, client, session, created_at,
                 created, deleted, previous_sha256, sha256, size, added_lines, removed_lines, diff,
                 has_backup, task)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    change.id,
                    change.host,
                    change.path,
                    enum_text(change.operation),
                    change.client.name,
                    change.client.session,
                    change.created_at as i64,
                    change.created,
                    change.deleted,
                    change.previous_sha256,
                    change.sha256,
                    change.size as i64,
                    change.added_lines as i64,
                    change.removed_lines as i64,
                    change.diff,
                    change.has_backup,
                    change.client.task,
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn get_file_change(&self, id: String) -> Result<Option<FileChange>, RpcError> {
        self.with(move |conn| {
            conn.query_row(
                "SELECT * FROM file_changes WHERE id = ?1",
                [id],
                file_change_from_row,
            )
            .optional()
        })
        .await
    }

    pub async fn rollback_plan(
        &self,
        scope: hostlane_protocol::RollbackScope,
    ) -> Result<Vec<hostlane_protocol::RollbackItem>, RpcError> {
        if scope.task.trim().is_empty() {
            return Err(RpcError::new(
                hostlane_protocol::ErrorKind::InvalidParams,
                "task must not be empty",
            ));
        }
        self.with(move |conn| {
            let mut stmt = conn.prepare("SELECT id,host,path FROM file_changes WHERE task=?1 AND (?2 IS NULL OR host=?2) AND (?3 IS NULL OR client=?3) AND operation != 'revert' AND id NOT IN (SELECT json_extract(params,'$.change_id') FROM calls WHERE method='files.revert' AND outcome='ok' AND json_type(params,'$.change_id')='text') ORDER BY rowid DESC")?;
            stmt.query_map(params![scope.task,scope.host,scope.client], |row| Ok(hostlane_protocol::RollbackItem {change_id:row.get(0)?, host:row.get(1)?, path:row.get(2)?}))?.collect()
        }).await
    }

    /// Notes on `host`'s ports, by port.
    pub async fn port_notes(&self, host: String) -> Result<BTreeMap<u16, PortNote>, RpcError> {
        self.with(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT port, name, project, note, updated_at FROM port_notes WHERE host = ?1",
            )?;
            stmt.query_map([host], |row| {
                Ok((
                    row.get::<_, u16>("port")?,
                    PortNote {
                        name: row.get("name")?,
                        project: row.get("project")?,
                        note: row.get("note")?,
                        updated_at: row.get::<_, i64>("updated_at")? as u64,
                    },
                ))
            })?
            .collect()
        })
        .await
    }

    /// Stores a note, or removes it when `note` is `None`.
    pub async fn set_port_note(
        &self,
        host: String,
        port: u16,
        note: Option<PortNote>,
    ) -> Result<(), RpcError> {
        self.with(move |conn| {
            match note {
                Some(note) => conn.execute(
                    "INSERT INTO port_notes (host, port, name, project, note, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (host, port) DO UPDATE SET name = ?3, project = ?4, note = ?5,
                     updated_at = ?6",
                    params![
                        host,
                        port,
                        note.name,
                        note.project,
                        note.note,
                        note.updated_at as i64
                    ],
                )?,
                None => conn.execute(
                    "DELETE FROM port_notes WHERE host = ?1 AND port = ?2",
                    params![host, port],
                )?,
            };
            Ok(())
        })
        .await
    }

    /// Saved tunnels, oldest first.
    pub async fn tunnels(&self) -> Result<Vec<TunnelConfig>, RpcError> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT * FROM tunnels ORDER BY created_at, id")?;
            stmt.query_map([], |row| {
                Ok(TunnelConfig {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    host: row.get("host")?,
                    bind: row.get("bind")?,
                    local_port: row.get("local_port")?,
                    remote_host: row.get("remote_host")?,
                    remote_port: row.get("remote_port")?,
                    auto_start: row.get("auto_start")?,
                    created_by: client_from_row(row, false)?,
                    created_at: row.get::<_, i64>("created_at")? as u64,
                })
            })?
            .collect()
        })
        .await
    }

    pub async fn save_tunnel(&self, tunnel: TunnelConfig) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO tunnels (id, name, host, bind, local_port, remote_host,
                 remote_port, auto_start, client, session, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    tunnel.id,
                    tunnel.name,
                    tunnel.host,
                    tunnel.bind,
                    tunnel.local_port,
                    tunnel.remote_host,
                    tunnel.remote_port,
                    tunnel.auto_start,
                    tunnel.created_by.name,
                    tunnel.created_by.session,
                    tunnel.created_at as i64
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn delete_tunnel(&self, id: String) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute("DELETE FROM tunnels WHERE id = ?1", [id])?;
            Ok(())
        })
        .await
    }

    pub async fn policy_rules(&self) -> Result<Vec<PolicyRule>, RpcError> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT * FROM policy_rules ORDER BY created_at, id")?;
            stmt.query_map([], |row| {
                let decision: String = row.get("decision")?;
                let conditions: String = row.get("conditions")?;
                Ok(PolicyRule {
                    id: row.get("id")?,
                    decision: enum_parse(&decision).unwrap_or(hostlane_protocol::Decision::Approve),
                    when: serde_json::from_str(&conditions).unwrap_or_default(),
                    note: row.get("note")?,
                    created_at: row.get::<_, i64>("created_at")? as u64,
                    created_by: row.get("created_by")?,
                })
            })?
            .collect()
        })
        .await
    }

    pub async fn insert_rule(&self, rule: PolicyRule) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute(
                "INSERT INTO policy_rules (id, decision, conditions, note, created_at, created_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    rule.id,
                    enum_text(rule.decision),
                    serde_json::to_string(&rule.when).unwrap_or_default(),
                    rule.note,
                    rule.created_at as i64,
                    rule.created_by,
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn delete_rule(&self, id: String) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute("DELETE FROM policy_rules WHERE id = ?1", [id])?;
            Ok(())
        })
        .await
    }

    pub async fn setting(&self, key: &str) -> Result<Option<String>, RpcError> {
        let key = key.to_string();
        self.with(move |conn| {
            conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
        })
        .await
    }

    /// Stores a setting, or removes it when `value` is `None`.
    pub async fn set_setting(&self, key: String, value: Option<String>) -> Result<(), RpcError> {
        self.with(move |conn| {
            match value {
                Some(value) => conn.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT (key) DO UPDATE SET value = ?2",
                    params![key, value],
                )?,
                None => conn.execute("DELETE FROM settings WHERE key = ?1", [key])?,
            };
            Ok(())
        })
        .await
    }

    pub async fn host_labels(&self) -> Result<HostLabels, RpcError> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT host, env, tags FROM host_labels")?;
            stmt.query_map([], |row| {
                let env: Option<String> = row.get("env")?;
                let tags: String = row.get("tags")?;
                Ok((
                    row.get::<_, String>("host")?,
                    (
                        env.as_deref().and_then(enum_parse),
                        serde_json::from_str(&tags).unwrap_or_default(),
                    ),
                ))
            })?
            .collect()
        })
        .await
    }

    /// Stores a host's labels; no env and no tags removes them.
    pub async fn set_host_labels(
        &self,
        host: String,
        env: Option<HostEnv>,
        tags: Vec<String>,
    ) -> Result<(), RpcError> {
        self.with(move |conn| {
            if env.is_none() && tags.is_empty() {
                conn.execute("DELETE FROM host_labels WHERE host = ?1", [host])?;
            } else {
                conn.execute(
                    "INSERT INTO host_labels (host, env, tags, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (host) DO UPDATE SET env = ?2, tags = ?3, updated_at = ?4",
                    params![
                        host,
                        env.map(enum_text),
                        serde_json::to_string(&tags).unwrap_or_default(),
                        crate::util::now_ms() as i64,
                    ],
                )?;
            }
            Ok(())
        })
        .await
    }

    /// Hosts added in Hostlane, by id (labels live in `host_labels`).
    pub async fn manual_hosts(&self) -> Result<Vec<HostAddParams>, RpcError> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT * FROM manual_hosts ORDER BY id")?;
            stmt.query_map([], |row| {
                Ok(HostAddParams {
                    id: row.get("id")?,
                    hostname: row.get("hostname")?,
                    user: row.get("user")?,
                    port: Some(row.get::<_, i64>("port")? as u16),
                    proxy_jump: row.get("proxy_jump")?,
                    identity_file: row.get("identity_file")?,
                    env: None,
                    tags: Vec::new(),
                })
            })?
            .collect()
        })
        .await
    }

    /// Inserts or replaces a manual host.
    pub async fn save_manual_host(&self, host: HostAddParams) -> Result<(), RpcError> {
        self.with(move |conn| {
            let now = crate::util::now_ms() as i64;
            conn.execute(
                "INSERT INTO manual_hosts (id, hostname, user, port, proxy_jump, identity_file,
                 created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT (id) DO UPDATE SET hostname = ?2, user = ?3, port = ?4,
                 proxy_jump = ?5, identity_file = ?6, updated_at = ?7",
                params![
                    host.id,
                    host.hostname,
                    host.user,
                    host.port.unwrap_or(22),
                    host.proxy_jump,
                    host.identity_file,
                    now,
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Removes a manual host and its labels; returns whether it existed.
    pub async fn delete_manual_host(&self, id: String) -> Result<bool, RpcError> {
        self.with(move |conn| {
            let removed = conn.execute("DELETE FROM manual_hosts WHERE id = ?1", [&id])?;
            conn.execute("DELETE FROM host_labels WHERE host = ?1", [&id])?;
            Ok(removed > 0)
        })
        .await
    }

    /// Which secrets are saved in the keychain.
    pub async fn secrets(&self) -> Result<Vec<SavedSecret>, RpcError> {
        self.with(|conn| {
            let mut stmt = conn
                .prepare("SELECT kind, host, key_path, saved_at FROM secrets ORDER BY account")?;
            stmt.query_map([], |row| {
                let kind: String = row.get("kind")?;
                Ok(SavedSecret {
                    kind: enum_parse(&kind).unwrap_or(SecretKind::Password),
                    host: row.get("host")?,
                    key_path: row.get("key_path")?,
                    saved_at: row.get::<_, i64>("saved_at")? as u64,
                })
            })?
            .collect()
        })
        .await
    }

    pub async fn record_secret(
        &self,
        account: String,
        secret: SavedSecret,
    ) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute(
                "INSERT INTO secrets (account, kind, host, key_path, saved_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (account) DO UPDATE SET saved_at = ?5",
                params![
                    account,
                    enum_text(secret.kind),
                    secret.host,
                    secret.key_path,
                    secret.saved_at as i64,
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn forget_secret(&self, account: String) -> Result<bool, RpcError> {
        self.with(move |conn| {
            Ok(conn.execute("DELETE FROM secrets WHERE account = ?1", [account])? > 0)
        })
        .await
    }

    pub async fn insert_job(&self, job: Job, params: Value) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute(
                "INSERT INTO jobs (id, host, name, command, cwd, client, session, verified, backend,
                 state, started_at, checked_at, approver, params)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    job.id,
                    job.host,
                    job.name,
                    job.command,
                    job.cwd,
                    job.client.name,
                    job.client.session,
                    job.client.verified,
                    enum_text(job.backend),
                    enum_text(job.state),
                    job.started_at as i64,
                    job.checked_at as i64,
                    job.approver,
                    params.to_string(),
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Stores the state last read from the server.
    pub async fn update_job(&self, job: Job) -> Result<(), RpcError> {
        self.with(move |conn| {
            conn.execute(
                "UPDATE jobs SET state = ?2, finished_at = ?3, exit_code = ?4, pid = ?5,
                 output_bytes = ?6, checked_at = ?7 WHERE id = ?1",
                params![
                    job.id,
                    enum_text(job.state),
                    job.finished_at.map(|t| t as i64),
                    job.exit_code,
                    job.pid,
                    job.output_bytes as i64,
                    job.checked_at as i64,
                ],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn get_job(&self, id: String) -> Result<Option<Job>, RpcError> {
        self.with(move |conn| {
            conn.query_row("SELECT * FROM jobs WHERE id = ?1", [id], job_from_row)
                .optional()
        })
        .await
    }

    pub async fn list_jobs(&self, params: JobsListParams) -> Result<Vec<Job>, RpcError> {
        self.with(move |conn| {
            let mut sql = String::from("SELECT * FROM jobs WHERE 1=1");
            let mut args: Vec<Sql> = Vec::new();
            if let Some(host) = params.host {
                sql.push_str(" AND host = ?");
                args.push(Sql::Text(host));
            }
            if params.running {
                sql.push_str(" AND state = 'running'");
            }
            let limit = params.limit.unwrap_or(50).clamp(1, 500);
            sql.push_str(&format!(" ORDER BY started_at DESC, id DESC LIMIT {limit}"));
            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(params_from_iter(args), job_from_row)?
                .collect()
        })
        .await
    }

    pub async fn clients(&self) -> Result<Vec<RegisteredClient>, RpcError> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT * FROM clients ORDER BY name")?;
            stmt.query_map([], registered_client_from_row)?.collect()
        })
        .await
    }

    /// Registers a client; returns false if the name is taken.
    pub async fn insert_client(
        &self,
        client: RegisteredClient,
        token_sha256: String,
    ) -> Result<bool, RpcError> {
        self.with(move |conn| {
            let added = conn.execute(
                "INSERT OR IGNORE INTO clients (name, token_sha256, created_at) VALUES (?1, ?2, ?3)",
                params![client.name, token_sha256, client.created_at as i64],
            )?;
            Ok(added > 0)
        })
        .await
    }

    pub async fn delete_client(&self, name: String) -> Result<bool, RpcError> {
        self.with(move |conn| Ok(conn.execute("DELETE FROM clients WHERE name = ?1", [name])? > 0))
            .await
    }

    /// The registered name for a token hash, marking it used at `now`.
    pub async fn use_client_token(
        &self,
        token_sha256: String,
        now: u64,
    ) -> Result<Option<String>, RpcError> {
        self.with(move |conn| {
            let name: Option<String> = conn
                .query_row(
                    "SELECT name FROM clients WHERE token_sha256 = ?1",
                    [&token_sha256],
                    |row| row.get(0),
                )
                .optional()?;
            if name.is_some() {
                conn.execute(
                    "UPDATE clients SET last_used_at = ?2 WHERE token_sha256 = ?1
                     AND (last_used_at IS NULL OR last_used_at < ?2 - 60000)",
                    params![token_sha256, now as i64],
                )?;
            }
            Ok(name)
        })
        .await
    }

    /// Marks commands left running by a previous daemon as interrupted and audits them.
    pub async fn recover_interrupted(&self, now: u64) -> Result<usize, RpcError> {
        self.with(move |conn| {
            let mut stmt = conn.prepare("SELECT * FROM execs WHERE status = 'running'")?;
            let stale = stmt
                .query_map([], |row| {
                    Ok((exec_from_row(row)?, row.get::<_, String>("params")?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (record, params) in &stale {
                conn.execute(
                    "UPDATE execs SET status = 'interrupted', finished_at = ?2 WHERE id = ?1",
                    params![record.id, now as i64],
                )?;
                insert_call(
                    conn,
                    &NewCall {
                        started_at: record.started_at,
                        finished_at: now,
                        client: record.client.clone(),
                        host: Some(record.host.clone()),
                        method: hostlane_protocol::method::EXEC_RUN.into(),
                        params: serde_json::from_str(params).unwrap_or(Value::Null),
                        outcome: AuditOutcome::Interrupted,
                        error: None,
                        result: json!({ "output_id": record.id }),
                        approver: record.approver.clone(),
                        risk: None,
                    },
                )?;
            }
            Ok(stale.len())
        })
        .await
    }
}

fn insert_call(conn: &Connection, call: &NewCall) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO calls (started_at, finished_at, client, session, host, method, params,
         outcome, error_kind, error_message, result, approver, risk, verified, task)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            call.started_at as i64,
            call.finished_at as i64,
            call.client.name,
            call.client.session,
            call.host,
            call.method,
            call.params.to_string(),
            enum_text(call.outcome),
            call.error.as_ref().map(|e| enum_text(e.kind())),
            call.error.as_ref().map(|e| e.message.clone()),
            call.result.to_string(),
            call.approver,
            call.risk.map(|r| r.level() as i64),
            call.client.verified,
            call.client.task,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn json_column(row: &Row, name: &str) -> rusqlite::Result<Value> {
    let text: String = row.get(name)?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn call_from_row(row: &Row) -> rusqlite::Result<AuditEntry> {
    let outcome: String = row.get("outcome")?;
    let error_kind: Option<String> = row.get("error_kind")?;
    Ok(AuditEntry {
        id: row.get("id")?,
        started_at: row.get::<_, i64>("started_at")? as u64,
        finished_at: row.get::<_, i64>("finished_at")? as u64,
        client: client_from_row(row, true)?,
        host: row.get("host")?,
        method: row.get("method")?,
        params: json_column(row, "params")?,
        outcome: enum_parse(&outcome).unwrap_or(AuditOutcome::Error),
        error_kind: error_kind.as_deref().and_then(enum_parse),
        error_message: row.get("error_message")?,
        result: json_column(row, "result")?,
        approver: row.get("approver")?,
        risk: row
            .get::<_, Option<i64>>("risk")?
            .and_then(|level| Risk::from_level(level as u8)),
    })
}

fn client_from_row(row: &Row, with_verified: bool) -> rusqlite::Result<ClientInfo> {
    Ok(ClientInfo {
        name: row.get("client")?,
        session: row.get("session")?,
        task: match row.as_ref().column_index("task") {
            Ok(index) => row.get(index)?,
            Err(_) => None,
        },
        verified: with_verified && row.get::<_, Option<bool>>("verified")?.unwrap_or(false),
    })
}

fn job_from_row(row: &Row) -> rusqlite::Result<Job> {
    let backend: String = row.get("backend")?;
    let state: String = row.get("state")?;
    Ok(Job {
        id: row.get("id")?,
        host: row.get("host")?,
        name: row.get("name")?,
        command: row.get("command")?,
        cwd: row.get("cwd")?,
        client: client_from_row(row, true)?,
        backend: enum_parse(&backend).unwrap_or(JobBackend::Nohup),
        state: enum_parse(&state).unwrap_or(JobState::Lost),
        started_at: row.get::<_, i64>("started_at")? as u64,
        finished_at: row.get::<_, Option<i64>>("finished_at")?.map(|t| t as u64),
        exit_code: row.get("exit_code")?,
        pid: row.get("pid")?,
        output_bytes: row.get::<_, i64>("output_bytes")? as u64,
        checked_at: row.get::<_, i64>("checked_at")? as u64,
        approver: row.get("approver")?,
    })
}

fn registered_client_from_row(row: &Row) -> rusqlite::Result<RegisteredClient> {
    Ok(RegisteredClient {
        name: row.get("name")?,
        created_at: row.get::<_, i64>("created_at")? as u64,
        last_used_at: row.get::<_, Option<i64>>("last_used_at")?.map(|t| t as u64),
    })
}

fn exec_from_row(row: &Row) -> rusqlite::Result<ExecRecord> {
    let status: String = row.get("status")?;
    Ok(ExecRecord {
        id: row.get("id")?,
        host: row.get("host")?,
        command: row.get("command")?,
        cwd: row.get("cwd")?,
        client: client_from_row(row, true)?,
        status: enum_parse(&status).unwrap_or(ExecStatus::Failed),
        started_at: row.get::<_, i64>("started_at")? as u64,
        finished_at: row.get::<_, Option<i64>>("finished_at")?.map(|t| t as u64),
        exit_code: row.get("exit_code")?,
        signal: row.get("signal")?,
        error: row.get("error")?,
        stdout_bytes: row.get::<_, i64>("stdout_bytes")? as u64,
        stderr_bytes: row.get::<_, i64>("stderr_bytes")? as u64,
        approver: row.get("approver")?,
    })
}

fn file_change_from_row(row: &Row) -> rusqlite::Result<FileChange> {
    let operation: String = row.get("operation")?;
    Ok(FileChange {
        id: row.get("id")?,
        host: row.get("host")?,
        path: row.get("path")?,
        operation: enum_parse(&operation).unwrap_or(FileOperation::Write),
        client: client_from_row(row, false)?,
        created_at: row.get::<_, i64>("created_at")? as u64,
        created: row.get("created")?,
        deleted: row.get("deleted")?,
        previous_sha256: row.get("previous_sha256")?,
        sha256: row.get("sha256")?,
        size: row.get::<_, i64>("size")? as u64,
        added_lines: row.get::<_, i64>("added_lines")? as u64,
        removed_lines: row.get::<_, i64>("removed_lines")? as u64,
        diff: row.get("diff")?,
        has_backup: row.get("has_backup")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(host: &str, started_at: u64) -> NewCall {
        NewCall {
            started_at,
            finished_at: started_at + 5,
            client: ClientInfo::new("cli"),
            host: Some(host.into()),
            method: "exec.run".into(),
            params: json!({ "command": "uptime" }),
            outcome: AuditOutcome::Ok,
            error: None,
            result: json!({ "exit_code": 0 }),
            approver: None,
            risk: Some(Risk::from_level((started_at % 4) as u8).unwrap()),
        }
    }

    #[tokio::test]
    async fn audit_log_is_append_only_and_filterable() {
        let store = Store::open_in_memory().unwrap();
        for (i, host) in ["a", "b", "a"].iter().enumerate() {
            store.append_call(call(host, 100 + i as u64)).await.unwrap();
        }
        let list = store
            .list_calls(AuditListParams {
                host: Some("a".into()),
                limit: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(list.entries.len(), 1);
        assert!(list.has_more);
        assert_eq!(list.entries[0].started_at, 102);
        assert_eq!(list.entries[0].params["command"], "uptime");
        let risky = store
            .list_calls(AuditListParams {
                min_risk: Some(Risk::L1),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(risky.entries.len(), 2);
        assert_eq!(risky.entries[0].risk, Some(Risk::L2));

        let conn = store.conn.lock().unwrap();
        assert!(conn.execute("UPDATE calls SET host = 'x'", []).is_err());
        assert!(conn.execute("DELETE FROM calls", []).is_err());
    }

    #[tokio::test]
    async fn interrupted_execs_are_recovered_and_audited() {
        let store = Store::open_in_memory().unwrap();
        let record = ExecRecord {
            id: "ex-1".into(),
            host: "web".into(),
            command: "sleep 100".into(),
            cwd: None,
            client: ClientInfo::new("cli"),
            status: ExecStatus::Running,
            started_at: 10,
            finished_at: None,
            exit_code: None,
            signal: None,
            error: None,
            stdout_bytes: 0,
            stderr_bytes: 0,
            approver: Some("desktop".into()),
        };
        store.insert_exec(record, json!({})).await.unwrap();
        assert_eq!(store.recover_interrupted(20).await.unwrap(), 1);
        let exec = store.get_exec("ex-1".into()).await.unwrap().unwrap();
        assert_eq!(exec.status, ExecStatus::Interrupted);
        assert_eq!(exec.approver.as_deref(), Some("desktop"));
        let audit = store.list_calls(AuditListParams::default()).await.unwrap();
        assert_eq!(audit.entries[0].outcome, AuditOutcome::Interrupted);
    }

    #[tokio::test]
    async fn file_changes_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let change = FileChange {
            id: "fc-1".into(),
            host: "web".into(),
            path: "/etc/app.conf".into(),
            operation: FileOperation::Patch,
            client: ClientInfo::new("cli"),
            created_at: 10,
            created: false,
            deleted: false,
            previous_sha256: Some("aa".into()),
            sha256: "bb".into(),
            size: 3,
            added_lines: 1,
            removed_lines: 1,
            diff: Some("-a\n+b\n".into()),
            has_backup: true,
        };
        store.insert_file_change(change).await.unwrap();
        let loaded = store.get_file_change("fc-1".into()).await.unwrap().unwrap();
        assert_eq!(loaded.operation, FileOperation::Patch);
        assert_eq!(loaded.previous_sha256.as_deref(), Some("aa"));
        assert!(loaded.has_backup && !loaded.deleted);
        assert!(
            store
                .get_file_change("fc-2".into())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn tunnels_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let tunnel = TunnelConfig {
            id: "tn-1".into(),
            name: Some("web".into()),
            host: "prod".into(),
            bind: "127.0.0.1".into(),
            local_port: 8080,
            remote_host: "127.0.0.1".into(),
            remote_port: 80,
            auto_start: true,
            created_by: ClientInfo::new("desktop"),
            created_at: 5,
        };
        store.save_tunnel(tunnel.clone()).await.unwrap();
        let updated = TunnelConfig {
            local_port: 8081,
            ..tunnel.clone()
        };
        store.save_tunnel(updated.clone()).await.unwrap();
        assert_eq!(store.tunnels().await.unwrap(), vec![updated]);
        store.delete_tunnel("tn-1".into()).await.unwrap();
        assert!(store.tunnels().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn policy_rules_settings_and_labels_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let rule = PolicyRule {
            id: "ru-1".into(),
            decision: hostlane_protocol::Decision::Deny,
            when: hostlane_protocol::RuleConditions {
                env: Some(HostEnv::Prod),
                min_risk: Some(Risk::L3),
                ..Default::default()
            },
            note: Some("no".into()),
            created_at: 1,
            created_by: "desktop".into(),
        };
        store.insert_rule(rule.clone()).await.unwrap();
        assert_eq!(store.policy_rules().await.unwrap(), vec![rule]);
        store.delete_rule("ru-1".into()).await.unwrap();
        assert!(store.policy_rules().await.unwrap().is_empty());

        store
            .set_setting("k".into(), Some("v".into()))
            .await
            .unwrap();
        assert_eq!(store.setting("k").await.unwrap().as_deref(), Some("v"));
        store.set_setting("k".into(), None).await.unwrap();
        assert_eq!(store.setting("k").await.unwrap(), None);

        store
            .set_host_labels("web".into(), Some(HostEnv::Prod), vec!["eu".into()])
            .await
            .unwrap();
        let labels = store.host_labels().await.unwrap();
        assert_eq!(labels["web"], (Some(HostEnv::Prod), vec!["eu".to_string()]));
        store
            .set_host_labels("web".into(), None, Vec::new())
            .await
            .unwrap();
        assert!(store.host_labels().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn port_notes_upsert_and_delete() {
        let store = Store::open_in_memory().unwrap();
        let note = |name: &str, at: u64| PortNote {
            name: Some(name.into()),
            project: Some("shop".into()),
            note: None,
            updated_at: at,
        };
        store
            .set_port_note("web".into(), 8080, Some(note("api", 1)))
            .await
            .unwrap();
        store
            .set_port_note("web".into(), 8080, Some(note("api-v2", 2)))
            .await
            .unwrap();
        store
            .set_port_note("db".into(), 5432, Some(note("pg", 3)))
            .await
            .unwrap();
        let notes = store.port_notes("web".into()).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[&8080].name.as_deref(), Some("api-v2"));
        assert_eq!(notes[&8080].updated_at, 2);
        store.set_port_note("web".into(), 8080, None).await.unwrap();
        assert!(store.port_notes("web".into()).await.unwrap().is_empty());
    }
}
