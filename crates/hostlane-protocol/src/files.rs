use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ClientInfo;

/// Params of `files.read`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileReadParams {
    pub host: String,
    /// Absolute, or relative to the remote home directory (`~/…` also works).
    pub path: String,
    /// First line to return, starting at 1. Defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub offset: Option<u64>,
    /// Lines to return. Defaults to 2000; output is also capped at 256 KiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub limit: Option<u64>,
}

/// Result of `files.read`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileContent {
    pub host: String,
    /// Absolute path after resolving `~` and symlinks.
    pub path: String,
    /// The requested lines, line endings included. Empty when `binary`.
    pub content: String,
    /// Of the whole file, whatever range was read. Pass it as `expected_sha256` when editing.
    pub sha256: String,
    #[ts(type = "number")]
    pub size: u64,
    /// Permission bits in octal, e.g. `0644`.
    pub mode: String,
    /// First line in `content` (1-based); `end_line` is the last one, 0 for an empty file.
    #[ts(type = "number")]
    pub start_line: u64,
    #[ts(type = "number")]
    pub end_line: u64,
    #[ts(type = "number")]
    pub total_lines: u64,
    /// More lines follow `end_line`; read them with `offset = end_line + 1`.
    pub truncated: bool,
    /// Not UTF-8 text; use `files.download` instead.
    pub binary: bool,
}

/// Params of `files.list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileListParams {
    pub host: String,
    /// Defaults to the remote home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
    /// Defaults to 1000, at most 10 000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileEntry {
    pub name: String,
    pub kind: FileKind,
    #[ts(type = "number")]
    pub size: u64,
    pub mode: String,
    /// Unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub modified_at: Option<u64>,
}

/// Result of `files.list`: directories first, then by name. Includes dotfiles.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileList {
    pub host: String,
    /// Absolute path of the directory.
    pub path: String,
    pub entries: Vec<FileEntry>,
    pub truncated: bool,
}

/// Params of `files.search`: runs `rg` on the server, or `grep -r` without it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileSearchParams {
    pub host: String,
    /// A regular expression (`rg` syntax, or `grep -E`), or a literal with `fixed`.
    pub pattern: String,
    /// File or directory to search. Defaults to the remote home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
    /// Only files matching this glob, e.g. `*.conf`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub glob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fixed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ignore_case: Option<bool>,
    /// Defaults to 200, at most 5000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max_results: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchMatch {
    pub path: String,
    #[ts(type = "number")]
    pub line: u64,
    /// The matching line, cut at 500 characters.
    pub text: String,
}

/// Result of `files.search`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileSearchResult {
    pub host: String,
    /// `rg` or `grep`. `rg` skips hidden and git-ignored files; `grep` does not.
    pub tool: String,
    pub matches: Vec<SearchMatch>,
    /// More matches exist than `max_results`.
    pub truncated: bool,
    /// Messages from the search tool, e.g. unreadable directories.
    pub warnings: Vec<String>,
}

/// Params of `files.write`. Replacing an existing file needs `expected_sha256` or `overwrite`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileWriteParams {
    pub host: String,
    pub path: String,
    pub content: String,
    /// The file must currently have this hash (from `files.read`); fails with `FILE_CHANGED` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expected_sha256: Option<String>,
    /// Replace an existing file without checking its hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub overwrite: Option<bool>,
    /// Octal permissions for a new file, e.g. `0755`. Existing files keep theirs. Defaults to `0644`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mode: Option<String>,
}

/// One search-and-replace edit for `files.patch`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileEdit {
    /// Exact text to find; must occur once unless `replace_all`.
    pub old_text: String,
    pub new_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub replace_all: Option<bool>,
}

/// Params of `files.patch`: either `edits` or a unified `diff`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FilePatchParams {
    pub host: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expected_sha256: Option<String>,
    /// Applied in order, each to the result of the previous one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub edits: Option<Vec<FileEdit>>,
    /// Unified diff of this one file (`@@` hunks; `---`/`+++` headers optional).
    /// Hunks are located by their context, so line numbers may be approximate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diff: Option<String>,
}

/// Params of `files.upload`: copies a file from this machine to the server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileUploadParams {
    pub host: String,
    /// Absolute path on this machine.
    pub local_path: String,
    pub remote_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expected_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub overwrite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FileOperation {
    Write,
    Patch,
    Upload,
    Revert,
}

/// Result of `files.write`, `files.patch`, `files.upload` and `files.revert`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileChangeResult {
    pub host: String,
    /// Absolute path that was written (the target, when `path` is a symlink).
    pub path: String,
    /// Look it up with `files.change`, undo it with `files.revert`.
    pub change_id: String,
    /// The file did not exist before; `files.revert` removes it.
    pub created: bool,
    /// The file was removed (reverting a change that created it).
    pub deleted: bool,
    /// New hash; pass it as `expected_sha256` for the next edit.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub previous_sha256: Option<String>,
    #[ts(type = "number")]
    pub size: u64,
    #[ts(type = "number")]
    pub added_lines: u64,
    #[ts(type = "number")]
    pub removed_lines: u64,
    /// E.g. the owner could not be kept because the SSH user may not chown.
    pub warnings: Vec<String>,
}

/// Params of `files.download`: copies a file from the server to this machine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileDownloadParams {
    pub host: String,
    pub remote_path: String,
    /// Absolute path on this machine; its directory must exist.
    pub local_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub overwrite: Option<bool>,
}

/// Result of `files.download`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileDownloadResult {
    pub host: String,
    pub remote_path: String,
    pub local_path: String,
    #[ts(type = "number")]
    pub size: u64,
    pub sha256: String,
}

/// Params of `files.change` and `files.revert`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileChangeParams {
    pub change_id: String,
}

/// A recorded file change, with its diff when both versions are text.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileChange {
    pub id: String,
    pub host: String,
    pub path: String,
    pub operation: FileOperation,
    pub client: ClientInfo,
    #[ts(type = "number")]
    pub created_at: u64,
    pub created: bool,
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub previous_sha256: Option<String>,
    /// Empty when the change deleted the file.
    pub sha256: String,
    #[ts(type = "number")]
    pub size: u64,
    #[ts(type = "number")]
    pub added_lines: u64,
    #[ts(type = "number")]
    pub removed_lines: u64,
    /// Unified diff, cut at 1 MiB. Missing for binary or very large files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diff: Option<String>,
    /// The previous content is kept locally, so `files.revert` can restore it.
    pub has_backup: bool,
}
