//! `hostlane mcp`: a stdio MCP server whose tools map one-to-one onto daemon methods.
//!
//! Each tool forwards its arguments (only the properties its schema declares) to the
//! daemon through [`Client`], so the daemon's policy, approvals and audit log apply to
//! MCP calls exactly as they do to the CLI. Tools only a person may use (approving,
//! rules, trusting host keys, resuming after an emergency stop) are not exposed.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use hostlane_core::client::{Client, ClientError};
use hostlane_protocol::{ClientInfo, method};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

const INSTRUCTIONS: &str = "Hostlane runs commands, edits files, inspects ports and opens \
SSH tunnels on the user's servers (the hosts in ~/.ssh/config), all from this machine. \
Start with `hosts`. Risky calls may wait for a person to approve them in the Hostlane app; \
pass `async: true` to get an APPROVAL_REQUIRED error with a `ticket_id` instead, check it \
with `get_approval`, and repeat the call with `ticket_id` once approved. Errors carry a \
`kind` and a `hint` on what to do next.";

/// How a tool reaches the daemon.
struct Spec {
    name: &'static str,
    method: &'static str,
    description: &'static str,
    properties: Value,
    required: &'static [&'static str],
    read_only: bool,
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn integer(description: &str) -> Value {
    json!({ "type": "integer", "description": description })
}

fn boolean(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

fn host() -> Value {
    string("Host alias from `hosts`.")
}

fn port(description: &str) -> Value {
    json!({ "type": "integer", "minimum": 1, "maximum": 65535, "description": description })
}

fn specs() -> Vec<Spec> {
    vec![
        Spec {
            name: "hosts",
            method: method::HOSTS_LIST,
            description: "List the servers Hostlane knows (from ~/.ssh/config) with their \
                labels (env, tags) and current connection state.",
            properties: json!({}),
            required: &[],
            read_only: true,
        },
        Spec {
            name: "exec",
            method: method::EXEC_RUN,
            description: "Run a shell command on a host, like `ssh host '<command>'`, and \
                return exit_code, stdout and stderr. Long output keeps its head and tail; \
                read the rest with `read_output` and the returned output_id. Times out after \
                timeout_ms (default 120000) with timed_out set.",
            properties: json!({
                "host": host(),
                "command": string("Run by the remote user's login shell."),
                "cwd": string("Remote working directory (`~/…` is relative to home)."),
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Environment variables for the command."
                },
                "stdin": string("Text sent to the command's standard input, then closed."),
                "timeout_ms": integer("Milliseconds before the command is stopped (default 120000)."),
                "max_output_bytes": integer("Bytes kept per stream (default 16384).")
            }),
            required: &["host", "command"],
            read_only: false,
        },
        Spec {
            name: "read_output",
            method: method::OUTPUT_READ,
            description: "Read stored output of an earlier `exec` by its output_id, e.g. the \
                part truncated from the middle.",
            properties: json!({
                "output_id": string("From the `exec` result."),
                "stream": { "type": "string", "enum": ["stdout", "stderr"] },
                "offset": integer("Byte offset; negative counts from the end."),
                "limit": integer("Bytes to read (default 65536, at most 1 MiB).")
            }),
            required: &["output_id", "stream"],
            read_only: true,
        },
        Spec {
            name: "exec_history",
            method: method::EXEC_LIST,
            description: "List recent commands run through Hostlane, newest first.",
            properties: json!({
                "host": host(),
                "before": integer("Only commands started before this time (Unix ms), for paging."),
                "limit": integer("Default 50, at most 500.")
            }),
            required: &[],
            read_only: true,
        },
        Spec {
            name: "read_file",
            method: method::FILES_READ,
            description: "Read a remote text file with line numbers. Returns sha256; pass it \
                as expected_sha256 when writing or editing the file.",
            properties: json!({
                "host": host(),
                "path": string("Absolute, or relative to the remote home directory."),
                "offset": integer("First line, starting at 1."),
                "limit": integer("Lines to return (default 2000).")
            }),
            required: &["host", "path"],
            read_only: true,
        },
        Spec {
            name: "list_dir",
            method: method::FILES_LIST,
            description: "List a remote directory.",
            properties: json!({
                "host": host(),
                "path": string("Defaults to the remote home directory."),
                "limit": integer("Default 1000, at most 10000.")
            }),
            required: &["host"],
            read_only: true,
        },
        Spec {
            name: "search_files",
            method: method::FILES_SEARCH,
            description: "Search remote files for a pattern (ripgrep, or grep -E).",
            properties: json!({
                "host": host(),
                "pattern": string("Regular expression, or a literal with fixed."),
                "path": string("File or directory; defaults to the remote home directory."),
                "glob": string("Only files matching this glob, e.g. `*.conf`."),
                "fixed": boolean("Treat pattern as literal text."),
                "ignore_case": boolean("Case-insensitive match."),
                "max_results": integer("Default 200, at most 5000.")
            }),
            required: &["host", "pattern"],
            read_only: true,
        },
        Spec {
            name: "write_file",
            method: method::FILES_WRITE,
            description: "Create or replace a remote file. Replacing needs expected_sha256 \
                (from read_file) or overwrite. Returns a change_id for revert_change.",
            properties: json!({
                "host": host(),
                "path": string("Remote path."),
                "content": string("Full new content."),
                "expected_sha256": string("Current hash; fails with FILE_CHANGED otherwise."),
                "overwrite": boolean("Replace without checking the hash."),
                "mode": string("Octal permissions for a new file, e.g. `0755`.")
            }),
            required: &["host", "path", "content"],
            read_only: false,
        },
        Spec {
            name: "edit_file",
            method: method::FILES_PATCH,
            description: "Edit a remote file by exact text replacement. Each old_text must \
                occur once unless replace_all. Returns a diff and a change_id.",
            properties: json!({
                "host": host(),
                "path": string("Remote path."),
                "edits": {
                    "type": "array",
                    "description": "Applied in order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_text": { "type": "string" },
                            "new_text": { "type": "string" },
                            "replace_all": { "type": "boolean" }
                        },
                        "required": ["old_text", "new_text"]
                    }
                },
                "expected_sha256": string("Current hash; fails with FILE_CHANGED otherwise.")
            }),
            required: &["host", "path", "edits"],
            read_only: false,
        },
        Spec {
            name: "apply_diff",
            method: method::FILES_PATCH,
            description: "Apply a unified diff of one remote file. Hunks are located by \
                context, so line numbers may be approximate. Returns a change_id.",
            properties: json!({
                "host": host(),
                "path": string("Remote path."),
                "diff": string("`@@` hunks; `---`/`+++` headers optional."),
                "expected_sha256": string("Current hash; fails with FILE_CHANGED otherwise.")
            }),
            required: &["host", "path", "diff"],
            read_only: false,
        },
        Spec {
            name: "upload_file",
            method: method::FILES_UPLOAD,
            description: "Copy a local file to a host.",
            properties: json!({
                "host": host(),
                "local_path": string("Absolute path on this machine."),
                "remote_path": string("Remote path."),
                "expected_sha256": string("Current hash of the remote file, when replacing it."),
                "overwrite": boolean("Replace without checking the hash."),
                "mode": string("Octal permissions for a new file.")
            }),
            required: &["host", "local_path", "remote_path"],
            read_only: false,
        },
        Spec {
            name: "download_file",
            method: method::FILES_DOWNLOAD,
            description: "Copy a remote file to this machine.",
            properties: json!({
                "host": host(),
                "remote_path": string("Remote path."),
                "local_path": string("Absolute path on this machine; its directory must exist."),
                "overwrite": boolean("Replace an existing local file.")
            }),
            required: &["host", "remote_path", "local_path"],
            read_only: false,
        },
        Spec {
            name: "revert_change",
            method: method::FILES_REVERT,
            description: "Undo a file change made through Hostlane, by its change_id.",
            properties: json!({ "change_id": string("From write_file, edit_file or apply_diff.") }),
            required: &["change_id"],
            read_only: false,
        },
        Spec {
            name: "list_ports",
            method: method::PORTS_LIST,
            description: "List listening ports on a host with their process or Docker \
                container, whether they answer HTTP(S), and saved notes.",
            properties: json!({
                "host": host(),
                "probe": boolean("Probe each port for HTTP/HTTPS (default true).")
            }),
            required: &["host"],
            read_only: true,
        },
        Spec {
            name: "inspect_port",
            method: method::PORTS_INSPECT,
            description: "Details of one port: process, container, HTTP response.",
            properties: json!({
                "host": host(),
                "port": port("Remote port."),
                "server_name": string("Host header and TLS SNI, e.g. the site's domain.")
            }),
            required: &["host", "port"],
            read_only: true,
        },
        Spec {
            name: "check_port",
            method: method::PORTS_CHECK,
            description: "Whether a port is free on a host, and who uses it if not.",
            properties: json!({ "host": host(), "port": port("Remote port.") }),
            required: &["host", "port"],
            read_only: true,
        },
        Spec {
            name: "find_free_port",
            method: method::PORTS_FIND_FREE,
            description: "Find free ports on a host for a new service.",
            properties: json!({
                "host": host(),
                "from": port("Start of the range (default 8000)."),
                "to": port("End of the range, inclusive (default 9999)."),
                "count": integer("Ports to return (default 1, at most 100).")
            }),
            required: &["host"],
            read_only: true,
        },
        Spec {
            name: "annotate_port",
            method: method::PORTS_ANNOTATE,
            description: "Save a name, project or note for a port on a host.",
            properties: json!({
                "host": host(),
                "port": port("Remote port."),
                "name": string("Service name."),
                "project": string("Project it belongs to."),
                "note": string("Free text.")
            }),
            required: &["host", "port"],
            read_only: false,
        },
        Spec {
            name: "list_tunnels",
            method: method::TUNNELS_LIST,
            description: "List SSH tunnels (local port → host:port) and their state.",
            properties: json!({}),
            required: &[],
            read_only: true,
        },
        Spec {
            name: "open_tunnel",
            method: method::TUNNELS_OPEN,
            description: "Forward a local port to a port on (or reachable from) a host, e.g. \
                to open a web service in a local browser. Returns the local URL. Tunnels \
                opened by agents expire after ttl_secs (default 3600) unless saved.",
            properties: json!({
                "host": host(),
                "remote_port": port("Port on the server."),
                "remote_host": string("Address to connect to from the server (default 127.0.0.1)."),
                "local_port": port("Defaults to remote_port when free, otherwise a nearby free port."),
                "name": string("Label shown in the app."),
                "ttl_secs": integer("Lifetime in seconds (max 86400).")
            }),
            required: &["host", "remote_port"],
            read_only: false,
        },
        Spec {
            name: "close_tunnel",
            method: method::TUNNELS_CLOSE,
            description: "Close a tunnel by id.",
            properties: json!({ "id": string("From open_tunnel or list_tunnels.") }),
            required: &["id"],
            read_only: false,
        },
        Spec {
            name: "check_policy",
            method: method::POLICY_CHECK,
            description: "Dry run: how Hostlane's policy would treat a tool call (allow, \
                ask or deny, with risk and reasons), without running it.",
            properties: json!({
                "tool": string("Tool name, e.g. `exec`."),
                "arguments": { "type": "object", "description": "Arguments for that tool." }
            }),
            required: &["tool", "arguments"],
            read_only: true,
        },
        Spec {
            name: "get_approval",
            method: method::APPROVALS_GET,
            description: "State of an approval request by the ticket_id from an \
                APPROVAL_REQUIRED error: pending, approved, denied (with a note) or expired.",
            properties: json!({ "id": string("The ticket_id.") }),
            required: &["id"],
            read_only: true,
        },
    ]
}

fn approval_properties() -> Value {
    json!({
        "async": boolean("If the call needs approval, fail at once with APPROVAL_REQUIRED and a ticket_id instead of waiting."),
        "ticket_id": string("Repeat a call that needed approval, once it is approved."),
        "approval_timeout_secs": integer("Seconds to wait for approval (default 120).")
    })
}

fn tool(spec: &Spec) -> Tool {
    let mut properties = spec.properties.as_object().cloned().unwrap_or_default();
    if method::GATED.contains(&spec.method) {
        properties.extend(
            approval_properties()
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
    }
    let mut schema = JsonObject::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    schema.insert("required".into(), json!(spec.required));
    let annotations = if spec.read_only {
        ToolAnnotations::new().read_only(true)
    } else {
        ToolAnnotations::new().read_only(false).open_world(true)
    };
    Tool::new(spec.name, spec.description, Arc::new(schema)).with_annotations(annotations)
}

/// The daemon method and params for a tool call; unknown properties are dropped.
fn translate(
    specs: &[Spec],
    name: &str,
    arguments: &JsonObject,
) -> Result<(&'static str, Value), String> {
    let spec = specs
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("unknown tool `{name}`"))?;
    if let Some(missing) = spec.required.iter().find(|k| !arguments.contains_key(**k)) {
        return Err(format!("`{name}` needs `{missing}`"));
    }
    let gated = method::GATED.contains(&spec.method);
    let declared = spec.properties.as_object();
    let params: JsonObject = arguments
        .iter()
        .filter(|(key, _)| {
            declared.is_some_and(|d| d.contains_key(*key))
                || (gated && approval_properties().get(key.as_str()).is_some())
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if spec.method == method::POLICY_CHECK {
        let tool = params
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let arguments = params
            .get("arguments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let (target, target_params) = translate(specs, tool, &arguments)?;
        return Ok((
            spec.method,
            json!({ "method": target, "params": target_params }),
        ));
    }
    Ok((spec.method, Value::Object(params)))
}

fn error_result(error: &ClientError) -> CallToolResult {
    let (kind, hint, details) = match error {
        ClientError::Rpc(rpc) => (
            serde_json::to_value(rpc.kind()).unwrap_or(Value::Null),
            rpc.data.hint.clone(),
            rpc.data.details.clone(),
        ),
        ClientError::NotRunning(_) => (
            json!("DAEMON_NOT_RUNNING"),
            Some("start it with `hostlane daemon start` or open the Hostlane app".into()),
            None,
        ),
        _ => (json!("ERROR"), None, None),
    };
    let mut body = json!({ "kind": kind, "message": error.to_string() });
    if let Some(hint) = hint {
        body["hint"] = json!(hint);
    }
    if let Some(details) = details {
        body["details"] = details;
    }
    CallToolResult::structured_error(json!({ "error": body }))
}

fn invalid(message: String) -> CallToolResult {
    CallToolResult::structured_error(
        json!({ "error": { "kind": "INVALID_PARAMS", "message": message } }),
    )
}

/// Names that the daemon treats as a person; an MCP client never gets them.
fn client_name() -> String {
    std::env::var("HOSTLANE_CLIENT")
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty() && name != "cli" && name != "desktop")
        .unwrap_or_else(|| "mcp".into())
}

/// One session per MCP server process, named after the MCP client, e.g. `claude-code-4121`.
fn session_name(peer: Option<&str>) -> String {
    if let Some(session) = std::env::var("HOSTLANE_SESSION")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        return session;
    }
    let peer: String = peer
        .unwrap_or("mcp")
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(32)
        .collect();
    let peer = peer.trim_matches('-');
    let peer = if peer.is_empty() { "mcp" } else { peer };
    format!("{peer}-{}", std::process::id())
}

struct Server {
    socket: PathBuf,
    explicit_socket: Option<PathBuf>,
    specs: Vec<Spec>,
}

impl Server {
    fn client(&self, context: &RequestContext<RoleServer>) -> Client {
        let peer = context.peer.peer_info();
        let peer = peer.as_ref().map(|info| info.client_info.name.as_str());
        Client::new(&self.socket).with_info(ClientInfo {
            name: client_name(),
            session: Some(session_name(peer)),
        })
    }

    async fn call(
        &self,
        client: &Client,
        method: &str,
        params: Value,
    ) -> Result<Value, ClientError> {
        match client.call(method, params.clone()).await {
            Err(ClientError::NotRunning(_)) => {
                if crate::ensure_daemon(client, self.explicit_socket.clone())
                    .await
                    .is_err()
                {
                    return Err(ClientError::NotRunning(self.socket.display().to_string()));
                }
                client.call(method, params).await
            }
            other => other,
        }
    }

    async fn run_tool(
        &self,
        client: &Client,
        name: &str,
        arguments: &JsonObject,
    ) -> CallToolResult {
        let (method, params) = match translate(&self.specs, name, arguments) {
            Ok(call) => call,
            Err(message) => return invalid(message),
        };
        let mut result = match self.call(client, method, params).await {
            Ok(result) => result,
            Err(error) => return error_result(&error),
        };
        if method == method::HOSTS_LIST
            && let Ok(connections) = client.call(method::CONNECTIONS_LIST, Value::Null).await
        {
            result["connections"] = connections["connections"].clone();
        }
        CallToolResult::structured(result)
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("hostlane", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(
            self.specs.iter().map(tool).collect(),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.specs.iter().find(|s| s.name == name).map(tool)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if !self.specs.iter().any(|s| s.name == request.name) {
            return Err(ErrorData::invalid_params(
                format!("unknown tool `{}`", request.name),
                None,
            ));
        }
        let client = self.client(&context);
        let arguments = request.arguments.unwrap_or_default();
        Ok(self
            .run_tool(&client, &request.name, &arguments)
            .await
            .into())
    }
}

pub async fn serve(socket: PathBuf, explicit_socket: Option<PathBuf>) -> Result<()> {
    let server = Server {
        socket,
        explicit_socket,
        specs: specs(),
    };
    let probe = Client::new(&server.socket);
    let _ = crate::ensure_daemon(&probe, server.explicit_socket.clone()).await;
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> JsonObject {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn tool_names_are_unique_and_methods_exist() {
        let specs = specs();
        let mut names: Vec<_> = specs.iter().map(|s| s.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), specs.len());
        for spec in &specs {
            assert!(method::ALL.contains(&spec.method), "{}", spec.method);
            for key in spec.required {
                assert!(spec.properties.get(key).is_some(), "{}: {key}", spec.name);
            }
        }
    }

    #[test]
    fn person_only_methods_are_not_exposed() {
        for spec in specs() {
            assert!(
                ![
                    method::APPROVALS_RESOLVE,
                    method::POLICY_RESUME,
                    method::POLICY_RULES_ADD,
                    method::POLICY_RULES_DELETE,
                    method::HOSTS_TRUST,
                    method::HOSTS_LABEL,
                    method::DAEMON_SHUTDOWN,
                ]
                .contains(&spec.method)
            );
        }
    }

    #[test]
    fn gated_tools_accept_approval_options() {
        let specs = specs();
        let exec = tool(specs.iter().find(|s| s.name == "exec").unwrap());
        assert!(exec.input_schema["properties"].get("ticket_id").is_some());
        let output = tool(specs.iter().find(|s| s.name == "read_output").unwrap());
        assert!(output.input_schema["properties"].get("ticket_id").is_none());
    }

    #[test]
    fn translate_filters_unknown_properties() {
        let specs = specs();
        let (method, params) = translate(
            &specs,
            "exec",
            &args(json!({ "host": "web", "command": "ls", "async": true, "approver": "me" })),
        )
        .unwrap();
        assert_eq!(method, method::EXEC_RUN);
        assert_eq!(
            params,
            json!({ "host": "web", "command": "ls", "async": true })
        );

        let (_, params) = translate(
            &specs,
            "read_output",
            &args(json!({ "output_id": "o", "stream": "stdout", "ticket_id": "t" })),
        )
        .unwrap();
        assert!(params.get("ticket_id").is_none());
    }

    #[test]
    fn translate_reports_missing_arguments() {
        let error = translate(&specs(), "exec", &args(json!({ "host": "web" }))).unwrap_err();
        assert!(error.contains("command"), "{error}");
    }

    #[test]
    fn check_policy_translates_the_target_tool() {
        let (method, params) = translate(
            &specs(),
            "check_policy",
            &args(json!({ "tool": "edit_file", "arguments": { "host": "web", "path": "/a", "edits": [] } })),
        )
        .unwrap();
        assert_eq!(method, method::POLICY_CHECK);
        assert_eq!(params["method"], method::FILES_PATCH);
        assert_eq!(params["params"]["path"], "/a");
    }

    #[test]
    fn session_names_are_sanitised() {
        let name = session_name(Some("Claude Code"));
        assert!(name.starts_with("claude-code-"), "{name}");
        assert!(session_name(Some("!!")).starts_with("mcp-"));
    }
}
