//! `files.*`: remote file operations over SFTP, plus `rg`/`grep` for search.
//!
//! Every change goes through [`change`]: the current content is copied to
//! `<data dir>/backups/<change id>` while its hash is checked, the new content is
//! written to a temporary file in the same directory, given the old owner and
//! permissions, and renamed over the target in one step. The change and its diff are
//! recorded for `files.change` and `files.revert`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hostlane_protocol::{
    ClientInfo, ErrorKind, Event, FileChange, FileChangeResult, FileContent, FileDownloadParams,
    FileDownloadResult, FileEntry, FileKind, FileList, FileListParams, FileOperation,
    FilePatchParams, FileReadParams, FileSearchParams, FileSearchResult, FileUploadParams,
    FileWriteParams, RpcError, SearchMatch,
};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::{Config, SftpSession};
use russh_sftp::protocol::{FileAttributes, FileType, OpenFlags, StatusCode};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::Daemon;
use crate::patch;
use crate::pool::Lease;
use crate::remote;
use crate::util::{new_id, now_ms, sh_path, sh_quote, utf8_prefix_len};

/// Largest file `files.read` returns and `files.patch` edits.
const MAX_TEXT_FILE: u64 = 16 * 1024 * 1024;
const DEFAULT_READ_LINES: u64 = 2000;
const MAX_READ_BYTES: usize = 256 * 1024;
const DEFAULT_LIST_LIMIT: u32 = 1000;
const MAX_LIST_LIMIT: u32 = 10_000;
const DEFAULT_MAX_RESULTS: u32 = 200;
const MAX_MAX_RESULTS: u32 = 5000;
const SEARCH_OUTPUT_CAP: usize = 8 * 1024 * 1024;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
const MATCH_TEXT_CAP: usize = 500;
const SFTP_TIMEOUT_SECS: u64 = 30;
const CHUNK: usize = 256 * 1024;
const DEFAULT_NEW_MODE: u32 = 0o644;

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorKind::InvalidParams, message)
}

fn hex(digest: impl AsRef<[u8]>) -> String {
    data_encoding::HEXLOWER.encode(digest.as_ref())
}

fn mode_text(attrs: &FileAttributes) -> String {
    format!("{:04o}", attrs.permissions.unwrap_or(0) & 0o7777)
}

/// `0644`, `644` or `0o644`.
pub fn parse_mode(text: &str) -> Result<u32, RpcError> {
    let digits = text.trim().trim_start_matches("0o");
    u32::from_str_radix(digits, 8)
        .ok()
        .filter(|mode| *mode <= 0o7777 && !digits.is_empty())
        .ok_or_else(|| invalid(format!("invalid mode `{text}`; use octal such as 0644")))
}

/// Accepts `abc…` or `sha256:abc…`, any case.
fn normalize_sha(text: &str) -> String {
    let text = text.trim().to_ascii_lowercase();
    text.strip_prefix("sha256:").unwrap_or(&text).to_string()
}

/// The path as sent over SFTP, where relative paths start at the home directory.
fn sftp_path(path: &str) -> Result<String, RpcError> {
    if path.is_empty() {
        return Err(invalid("`path` must not be empty"));
    }
    if path.contains('\0') {
        return Err(invalid("`path` must not contain NUL"));
    }
    Ok(match path {
        "~" | "~/" => ".".into(),
        _ => path.strip_prefix("~/").unwrap_or(path).to_string(),
    })
}

/// Directory and file name of a path.
fn split(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(0) => ("/", &path[1..]),
        Some(index) => (&path[..index], &path[index + 1..]),
        None => (".", path),
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn sftp_error(err: SftpError, path: &str) -> RpcError {
    match err {
        SftpError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => RpcError::new(
                ErrorKind::FileNotFound,
                format!("`{path}`: no such file or directory"),
            ),
            StatusCode::PermissionDenied => RpcError::new(
                ErrorKind::PermissionDenied,
                format!("`{path}`: permission denied"),
            )
            .with_hint(
                "file operations run as the SSH user without sudo; pick a path that user may access, or use exec",
            ),
            code => {
                let reason = if status.error_message.is_empty() {
                    code.to_string()
                } else {
                    status.error_message
                };
                RpcError::new(ErrorKind::FileError, format!("`{path}`: {reason}"))
            }
        },
        SftpError::Timeout => RpcError::new(
            ErrorKind::ConnectionLost,
            format!("`{path}`: the server did not answer within {SFTP_TIMEOUT_SECS} s"),
        ),
        SftpError::IO(message) => RpcError::new(
            ErrorKind::ConnectionLost,
            format!("`{path}`: SFTP channel failed: {message}"),
        ),
        other => RpcError::new(ErrorKind::FileError, format!("`{path}`: {other}")),
    }
}

fn io_error(err: std::io::Error, path: &str) -> RpcError {
    if let Some(inner) = err.get_ref().and_then(|e| e.downcast_ref::<SftpError>()) {
        return sftp_error(inner.clone(), path);
    }
    let kind = match err.kind() {
        std::io::ErrorKind::NotFound => ErrorKind::FileNotFound,
        std::io::ErrorKind::PermissionDenied => ErrorKind::PermissionDenied,
        std::io::ErrorKind::TimedOut => ErrorKind::ConnectionLost,
        _ => ErrorKind::FileError,
    };
    RpcError::new(kind, format!("`{path}`: {err}"))
}

fn local_error(err: std::io::Error, path: &Path) -> RpcError {
    let kind = match err.kind() {
        std::io::ErrorKind::NotFound => ErrorKind::FileNotFound,
        std::io::ErrorKind::PermissionDenied => ErrorKind::PermissionDenied,
        _ => ErrorKind::FileError,
    };
    RpcError::new(kind, format!("local `{}`: {err}", path.display()))
}

fn absolute_local(path: &str) -> Result<PathBuf, RpcError> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(invalid(format!(
            "`{}` must be an absolute path on this machine",
            path.display()
        )));
    }
    Ok(path)
}

async fn open_private(path: &Path) -> Result<tokio::fs::File, RpcError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(path)
        .await
        .map_err(|err| local_error(err, path))
}

/// An SFTP session on a pooled connection; the channel slot is held until drop.
struct Remote {
    sftp: SftpSession,
    host: String,
    lease: Lease,
}

/// Content of a file that is about to be replaced.
struct Snapshot {
    sha256: String,
    size: u64,
    /// Kept when the file is at most [`MAX_TEXT_FILE`].
    bytes: Option<Vec<u8>>,
    attrs: FileAttributes,
}

/// Where a change writes: the resolved absolute path and what is there now.
struct Target {
    path: String,
    existing: Option<FileAttributes>,
}

enum Source {
    Bytes(Vec<u8>),
    Local(PathBuf),
}

impl Remote {
    async fn open(daemon: &Daemon, host: &str) -> Result<Self, RpcError> {
        let (entry, loaded) = daemon.resolve_host(host).await?;
        let (lease, mut channel) = daemon.pool.open_channel(&entry, &loaded).await?;
        let unavailable = |reason: String| {
            RpcError::new(
                ErrorKind::FileError,
                format!("cannot start SFTP on `{host}`: {reason}"),
            )
            .with_hint(
                "Hostlane needs the server's SFTP subsystem (`Subsystem sftp …` in sshd_config)",
            )
        };
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|err| unavailable(err.to_string()))?;
        loop {
            match channel.wait().await {
                Some(russh::ChannelMsg::Success) => break,
                Some(russh::ChannelMsg::Failure) => {
                    return Err(unavailable("the server refused the request".into()));
                }
                Some(russh::ChannelMsg::Close) | None => {
                    return Err(unavailable("the channel closed".into()));
                }
                Some(_) => {}
            }
        }
        let config = Config {
            request_timeout_secs: SFTP_TIMEOUT_SECS,
            ..Config::default()
        };
        let sftp = SftpSession::new_with_config(channel.into_stream(), config)
            .await
            .map_err(|err| unavailable(err.to_string()))?;
        Ok(Self {
            sftp,
            host: host.to_string(),
            lease,
        })
    }

    async fn canonicalize(&self, path: &str) -> Result<String, RpcError> {
        let real = self
            .sftp
            .canonicalize(path)
            .await
            .map_err(|err| sftp_error(err, path))?;
        Ok(real)
    }

    async fn stat(&self, path: &str) -> Result<FileAttributes, RpcError> {
        self.sftp
            .metadata(path)
            .await
            .map_err(|err| sftp_error(err, path))
    }

    /// An existing regular file, after following symlinks.
    async fn regular_file(&self, path: &str) -> Result<(String, FileAttributes), RpcError> {
        let real = self.canonicalize(path).await?;
        let attrs = self.stat(&real).await?;
        if attrs.is_dir() {
            return Err(invalid(format!("`{real}` is a directory; use files.list")));
        }
        if !attrs.is_regular() {
            return Err(invalid(format!("`{real}` is not a regular file")));
        }
        Ok((real, attrs))
    }

    /// Resolves where a write lands. Symlinks are followed, so the link stays a link.
    async fn target(&self, path: &str) -> Result<Target, RpcError> {
        match self.sftp.symlink_metadata(path).await {
            Ok(_) => {
                let (real, attrs) = self.regular_file(path).await?;
                Ok(Target {
                    path: real,
                    existing: Some(attrs),
                })
            }
            Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
                let (dir, name) = split(path.trim_end_matches('/'));
                if name.is_empty() || name == "." || name == ".." || path.ends_with('/') {
                    return Err(invalid(format!("`{path}` is not a file path")));
                }
                let missing = |err: RpcError| {
                    if err.kind() == ErrorKind::FileNotFound {
                        err.with_hint(format!(
                            "the directory `{dir}` does not exist; create it first, e.g. with exec `mkdir -p`"
                        ))
                    } else {
                        err
                    }
                };
                let dir = self.canonicalize(dir).await.map_err(missing)?;
                if !self.stat(&dir).await.map_err(missing)?.is_dir() {
                    return Err(invalid(format!("`{dir}` is not a directory")));
                }
                Ok(Target {
                    path: join(&dir, name),
                    existing: None,
                })
            }
            Err(err) => Err(sftp_error(err, path)),
        }
    }

    /// Reads a file while hashing it, optionally copying it to `backup`.
    async fn snapshot(
        &self,
        path: &str,
        attrs: FileAttributes,
        backup: Option<&Path>,
    ) -> Result<Snapshot, RpcError> {
        let mut file = self
            .sftp
            .open(path)
            .await
            .map_err(|err| sftp_error(err, path))?;
        let mut local = match backup {
            Some(backup) => Some(open_private(backup).await?),
            None => None,
        };
        let keep = attrs.len() <= MAX_TEXT_FILE;
        let mut hasher = Sha256::new();
        let mut bytes = Vec::new();
        let mut size = 0u64;
        let mut buffer = vec![0u8; CHUNK];
        loop {
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|err| io_error(err, path))?;
            if read == 0 {
                break;
            }
            let chunk = &buffer[..read];
            hasher.update(chunk);
            size += read as u64;
            if keep && size <= MAX_TEXT_FILE {
                bytes.extend_from_slice(chunk);
            }
            if let (Some(local), Some(backup)) = (&mut local, backup) {
                local
                    .write_all(chunk)
                    .await
                    .map_err(|err| local_error(err, backup))?;
            }
        }
        if let (Some(mut local), Some(backup)) = (local, backup) {
            local
                .flush()
                .await
                .map_err(|err| local_error(err, backup))?;
        }
        Ok(Snapshot {
            sha256: hex(hasher.finalize()),
            size,
            bytes: (keep && size <= MAX_TEXT_FILE).then_some(bytes),
            attrs,
        })
    }

    /// Writes `source` to a temporary file next to `target`, copies the owner and
    /// permissions of `original` (or applies `new_mode`), and renames it into place.
    async fn replace(
        &self,
        target: &Target,
        source: &Source,
        new_mode: u32,
        warnings: &mut Vec<String>,
    ) -> Result<(String, u64), RpcError> {
        let (dir, name) = split(&target.path);
        let temp = join(dir, &format!(".{name}.hostlane-{}", &new_id("t")[2..]));
        let attrs = FileAttributes {
            permissions: Some(0o600),
            ..FileAttributes::empty()
        };
        let mut file = self
            .sftp
            .open_with_flags_and_attributes(
                &temp,
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                attrs,
            )
            .await
            .map_err(|err| sftp_error(err, &target.path))?;
        let result = async {
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            match source {
                Source::Bytes(bytes) => {
                    hasher.update(bytes);
                    size = bytes.len() as u64;
                    file.write_all(bytes)
                        .await
                        .map_err(|err| io_error(err, &temp))?;
                }
                Source::Local(path) => {
                    let mut local = tokio::fs::File::open(path)
                        .await
                        .map_err(|err| local_error(err, path))?;
                    let mut buffer = vec![0u8; CHUNK];
                    loop {
                        let read = local
                            .read(&mut buffer)
                            .await
                            .map_err(|err| local_error(err, path))?;
                        if read == 0 {
                            break;
                        }
                        hasher.update(&buffer[..read]);
                        size += read as u64;
                        file.write_all(&buffer[..read])
                            .await
                            .map_err(|err| io_error(err, &temp))?;
                    }
                }
            }
            file.close().await.map_err(|err| io_error(err, &temp))?;
            self.copy_attributes(&temp, target, new_mode, warnings)
                .await?;
            if let Some(before) = &target.existing {
                let now = self.stat(&target.path).await?;
                if now.size != before.size || now.mtime != before.mtime {
                    return Err(RpcError::new(
                        ErrorKind::FileChanged,
                        format!("`{}` changed while Hostlane was writing it", target.path),
                    )
                    .with_hint("read the file again with files.read and redo the edit"));
                }
            }
            self.rename_over(&temp, target).await?;
            Ok((hex(hasher.finalize()), size))
        }
        .await;
        if result.is_err() {
            let _ = self.sftp.remove_file(&temp).await;
        }
        result
    }

    async fn copy_attributes(
        &self,
        temp: &str,
        target: &Target,
        new_mode: u32,
        warnings: &mut Vec<String>,
    ) -> Result<(), RpcError> {
        let mode = match &target.existing {
            Some(original) => {
                let current = self.stat(temp).await?;
                if let (Some(uid), Some(gid)) = (original.uid, original.gid)
                    && (current.uid != Some(uid) || current.gid != Some(gid))
                {
                    let owner = FileAttributes {
                        uid: Some(uid),
                        gid: Some(gid),
                        ..FileAttributes::empty()
                    };
                    if let Err(err) = self.sftp.set_metadata(temp, owner).await {
                        warnings.push(format!(
                            "could not keep owner {uid}:{gid} ({err}); the file is now owned by the SSH user"
                        ));
                    }
                }
                original.permissions.unwrap_or(DEFAULT_NEW_MODE) & 0o7777
            }
            None => new_mode,
        };
        let permissions = FileAttributes {
            permissions: Some(mode),
            ..FileAttributes::empty()
        };
        self.sftp
            .set_metadata(temp, permissions)
            .await
            .map_err(|err| sftp_error(err, temp))
    }

    async fn rename_over(&self, temp: &str, target: &Target) -> Result<(), RpcError> {
        let renamed = self
            .sftp
            .posix_rename(temp, target.path.as_str())
            .await
            .map_err(|err| sftp_error(err, &target.path))?;
        if renamed {
            return Ok(());
        }
        if target.existing.is_none() {
            return self
                .sftp
                .rename(temp, target.path.as_str())
                .await
                .map_err(|err| sftp_error(err, &target.path));
        }
        // Plain SFTP rename refuses to replace a file; mv uses rename(2), which is atomic.
        let channel = self.lease.open_session().await?;
        let command = format!("mv -f -- {} {}", sh_quote(temp), sh_quote(&target.path));
        let out = remote::capture(channel, &command, 4096, None, Duration::from_secs(30)).await?;
        if out.exit_code == Some(0) {
            Ok(())
        } else {
            Err(RpcError::new(
                ErrorKind::FileError,
                format!(
                    "cannot replace `{}`: {}",
                    target.path,
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            ))
        }
    }
}

/// Lines `offset..offset+limit` (1-based), cut at `max_bytes`.
struct Lines {
    content: String,
    start: u64,
    end: u64,
    total: u64,
}

fn select_lines(text: &str, offset: u64, limit: u64, max_bytes: usize) -> Lines {
    let total = text.split_inclusive('\n').count() as u64;
    let start = offset.max(1);
    let mut content = String::new();
    let mut end = start - 1;
    for line in text
        .split_inclusive('\n')
        .skip((start - 1) as usize)
        .take(limit as usize)
    {
        if content.len() + line.len() > max_bytes {
            if content.is_empty() {
                let cut = utf8_prefix_len(&line.as_bytes()[..max_bytes]);
                content.push_str(&line[..cut]);
                end += 1;
            }
            break;
        }
        content.push_str(line);
        end += 1;
    }
    Lines {
        content,
        start,
        end,
        total,
    }
}

pub async fn read(daemon: &Daemon, params: FileReadParams) -> Result<FileContent, RpcError> {
    let path = sftp_path(&params.path)?;
    let remote = Remote::open(daemon, &params.host).await?;
    let (real, attrs) = remote.regular_file(&path).await?;
    if attrs.len() > MAX_TEXT_FILE {
        return Err(invalid(format!(
            "`{real}` is {} bytes; files.read returns files up to 16 MiB",
            attrs.len()
        ))
        .with_hint("use files.download, or exec with head, tail or grep"));
    }
    let snapshot = remote.snapshot(&real, attrs, None).await?;
    let bytes = snapshot.bytes.unwrap_or_default();
    let text = std::str::from_utf8(&bytes)
        .ok()
        .filter(|text| !text.contains('\0'));
    let base = FileContent {
        host: remote.host.clone(),
        path: real,
        content: String::new(),
        sha256: snapshot.sha256,
        size: snapshot.size,
        mode: mode_text(&snapshot.attrs),
        start_line: 0,
        end_line: 0,
        total_lines: 0,
        truncated: false,
        binary: true,
    };
    let Some(text) = text else {
        return Ok(base);
    };
    let lines = select_lines(
        text,
        params.offset.unwrap_or(1),
        params.limit.unwrap_or(DEFAULT_READ_LINES).max(1),
        MAX_READ_BYTES,
    );
    Ok(FileContent {
        content: lines.content,
        start_line: lines.start,
        end_line: lines.end,
        total_lines: lines.total,
        truncated: lines.end < lines.total,
        binary: false,
        ..base
    })
}

pub async fn list(daemon: &Daemon, params: FileListParams) -> Result<FileList, RpcError> {
    let path = sftp_path(params.path.as_deref().unwrap_or("."))?;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT) as usize;
    let remote = Remote::open(daemon, &params.host).await?;
    let real = remote.canonicalize(&path).await?;
    let dir = remote
        .sftp
        .read_dir(real.as_str())
        .await
        .map_err(|err| sftp_error(err, &real))?;
    let mut entries: Vec<FileEntry> = dir
        .map(|entry| {
            let attrs = entry.metadata();
            FileEntry {
                name: entry.file_name(),
                kind: match entry.file_type() {
                    FileType::Dir => FileKind::Dir,
                    FileType::File => FileKind::File,
                    FileType::Symlink => FileKind::Symlink,
                    _ => FileKind::Other,
                },
                size: attrs.len(),
                mode: mode_text(&attrs),
                modified_at: attrs.mtime.map(|t| t as u64 * 1000),
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        (a.kind != FileKind::Dir)
            .cmp(&(b.kind != FileKind::Dir))
            .then_with(|| a.name.cmp(&b.name))
    });
    let truncated = entries.len() > limit;
    entries.truncate(limit);
    Ok(FileList {
        host: remote.host,
        path: real,
        entries,
        truncated,
    })
}

/// The shell script for `files.search`: prints the tool name, then `path\0line:text` lines.
pub fn search_command(params: &FileSearchParams) -> String {
    let path = sh_path(params.path.as_deref().unwrap_or("."));
    let pattern = sh_quote(&params.pattern);
    let fixed = params.fixed.unwrap_or(false);
    let ignore_case = params.ignore_case.unwrap_or(false);

    let mut rg = String::from(
        "rg --no-heading --with-filename --line-number --color=never --null --no-config --max-columns=500 --max-columns-preview",
    );
    let mut grep = String::from("grep -rnI --null --color=never");
    grep.push_str(if fixed { " -F" } else { " -E" });
    if fixed {
        rg.push_str(" -F");
    }
    if ignore_case {
        rg.push_str(" -i");
        grep.push_str(" -i");
    }
    if let Some(glob) = params.glob.as_deref().filter(|g| !g.is_empty()) {
        rg.push_str(&format!(" --glob {}", sh_quote(glob)));
        grep.push_str(&format!(" --include={}", sh_quote(glob)));
    }
    for command in [&mut rg, &mut grep] {
        command.push_str(&format!(" -e {pattern} -- {path}"));
    }
    format!(
        "if command -v rg >/dev/null 2>&1; then echo rg; exec {rg}; else echo grep; exec {grep}; fi"
    )
}

/// Parses `path\0line:text` lines after the first (tool name) line.
fn parse_matches(stdout: &[u8]) -> (String, Vec<SearchMatch>) {
    let text = String::from_utf8_lossy(stdout);
    let mut lines = text.split('\n');
    let tool = lines.next().unwrap_or_default().trim().to_string();
    let matches = lines
        .filter_map(|line| {
            let (path, rest) = line.split_once('\0')?;
            let (number, text) = rest.split_once(':')?;
            let text = text.strip_suffix('\r').unwrap_or(text);
            let text = match text.char_indices().nth(MATCH_TEXT_CAP) {
                Some((cut, _)) => format!("{}…", &text[..cut]),
                None => text.to_string(),
            };
            Some(SearchMatch {
                path: path.to_string(),
                line: number.parse().ok()?,
                text,
            })
        })
        .collect();
    (tool, matches)
}

pub async fn search(
    daemon: &Daemon,
    params: FileSearchParams,
) -> Result<FileSearchResult, RpcError> {
    if params.pattern.is_empty() {
        return Err(invalid("`pattern` must not be empty"));
    }
    if let Some(path) = &params.path {
        sftp_path(path)?;
    }
    let max = params
        .max_results
        .unwrap_or(DEFAULT_MAX_RESULTS)
        .clamp(1, MAX_MAX_RESULTS) as usize;
    let (entry, loaded) = daemon.resolve_host(&params.host).await?;
    let (lease, channel) = daemon.pool.open_channel(&entry, &loaded).await?;
    let out = remote::capture(
        channel,
        &search_command(&params),
        SEARCH_OUTPUT_CAP,
        Some(max + 2),
        SEARCH_TIMEOUT,
    )
    .await?;
    drop(lease);

    let (tool, mut matches) = parse_matches(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut warnings: Vec<String> = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(20)
        .map(str::to_owned)
        .collect();
    let failed = !out.truncated && !out.timed_out && out.exit_code.is_some_and(|code| code > 1);
    if failed && matches.is_empty() {
        let reason = if warnings.is_empty() {
            format!("{tool} exited with {}", out.exit_code.unwrap_or(2))
        } else {
            warnings
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        };
        return Err(invalid(format!("search failed: {reason}")));
    }
    if out.timed_out {
        warnings.push(format!(
            "the search stopped after {} s; narrow `path` or `glob`",
            SEARCH_TIMEOUT.as_secs()
        ));
    }
    let truncated = matches.len() > max || out.truncated || out.timed_out;
    matches.truncate(max);
    Ok(FileSearchResult {
        host: params.host,
        tool,
        matches,
        truncated,
        warnings,
    })
}

/// What a change writes.
enum NewContent {
    Bytes(Vec<u8>),
    Local(PathBuf),
    Patch(FilePatchParams),
    /// Restore the content saved in a backup file.
    Backup(PathBuf),
    Delete,
}

struct ChangeRequest {
    host: String,
    path: String,
    operation: FileOperation,
    expected_sha256: Option<String>,
    overwrite: bool,
    must_exist: bool,
    must_not_exist: bool,
    new_mode: u32,
    content: NewContent,
}

fn file_changed(path: &str, current: Option<&str>, expected: Option<&str>) -> RpcError {
    let message = match (current, expected) {
        (None, _) => format!("`{path}` does not exist"),
        (Some(_), Some(_)) => format!("`{path}` has changed since it was read"),
        (Some(_), None) => format!("`{path}` already exists"),
    };
    let hint = match (current, expected) {
        (Some(_), None) => {
            "pass expected_sha256 (from files.read) to replace it, or overwrite: true"
        }
        _ => "read the file again with files.read and redo the edit against the current content",
    };
    RpcError::new(ErrorKind::FileChanged, message)
        .with_hint(hint)
        .with_details(json!({ "current_sha256": current, "expected_sha256": expected }))
}

async fn change(
    daemon: &Daemon,
    client: &ClientInfo,
    request: ChangeRequest,
) -> Result<FileChangeResult, RpcError> {
    let path = sftp_path(&request.path)?;
    let remote = Remote::open(daemon, &request.host).await?;
    let target = remote.target(&path).await?;
    let id = new_id("fc");
    let backup = daemon.backups_dir.join(&id);
    let result = apply(daemon, client, &remote, &target, &id, &backup, request).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&backup).await;
    }
    result
}

async fn apply(
    daemon: &Daemon,
    client: &ClientInfo,
    remote: &Remote,
    target: &Target,
    id: &str,
    backup: &Path,
    request: ChangeRequest,
) -> Result<FileChangeResult, RpcError> {
    let snapshot = match &target.existing {
        Some(attrs) => Some(
            remote
                .snapshot(&target.path, attrs.clone(), Some(backup))
                .await?,
        ),
        None => None,
    };
    let current = snapshot.as_ref().map(|s| s.sha256.as_str());
    let expected = request.expected_sha256.as_deref().map(normalize_sha);
    match (&expected, current) {
        (Some(expected), Some(current)) if expected == current => {}
        (Some(expected), _) => {
            return Err(file_changed(&target.path, current, Some(expected)));
        }
        (None, Some(current))
            if request.must_not_exist || (!request.overwrite && !request.must_exist) =>
        {
            return Err(file_changed(&target.path, Some(current), None));
        }
        (None, None) if request.must_exist => {
            return Err(RpcError::new(
                ErrorKind::FileNotFound,
                format!("`{}`: no such file", target.path),
            )
            .with_hint("create it with files.write"));
        }
        _ => {}
    }

    let old_bytes = snapshot.as_ref().and_then(|s| s.bytes.as_deref());
    let new_content = match request.content {
        NewContent::Bytes(bytes) => Some(Source::Bytes(bytes)),
        NewContent::Local(path) => Some(Source::Local(path)),
        NewContent::Backup(path) => Some(Source::Local(path)),
        NewContent::Delete => None,
        NewContent::Patch(params) => {
            let bytes = old_bytes.ok_or_else(|| {
                invalid(format!(
                    "`{}` is larger than 16 MiB; use files.upload to replace it",
                    target.path
                ))
            })?;
            let text = std::str::from_utf8(bytes)
                .map_err(|_| invalid(format!("`{}` is not UTF-8 text", target.path)))?;
            let patched = match (&params.edits, &params.diff) {
                (Some(edits), None) if !edits.is_empty() => patch::apply_edits(text, edits)?,
                (None, Some(diff)) => patch::apply_unified(text, diff)?,
                _ => {
                    return Err(invalid(
                        "pass either `edits` (non-empty) or `diff`, not both",
                    ));
                }
            };
            Some(Source::Bytes(patched.into_bytes()))
        }
    };

    let mut warnings = Vec::new();
    let (sha256, size) = match &new_content {
        Some(source) => {
            remote
                .replace(target, source, request.new_mode, &mut warnings)
                .await?
        }
        None => {
            remote
                .sftp
                .remove_file(target.path.as_str())
                .await
                .map_err(|err| sftp_error(err, &target.path))?;
            (String::new(), 0)
        }
    };

    let new_bytes = match &new_content {
        Some(Source::Bytes(bytes)) => Some(bytes.clone()),
        Some(Source::Local(path)) if size <= MAX_TEXT_FILE => tokio::fs::read(path).await.ok(),
        Some(Source::Local(_)) => None,
        None => Some(Vec::new()),
    };
    let summary = match (old_bytes, &new_bytes) {
        (old, Some(new)) if old.is_some() || target.existing.is_none() => {
            patch::summarize(&target.path, old.unwrap_or_default(), new)
        }
        _ => patch::Summary::default(),
    };
    let record = FileChange {
        id: id.to_string(),
        host: remote.host.clone(),
        path: target.path.clone(),
        operation: request.operation,
        client: client.clone(),
        created_at: now_ms(),
        created: target.existing.is_none(),
        deleted: new_content.is_none(),
        previous_sha256: snapshot.as_ref().map(|s| s.sha256.clone()),
        sha256: sha256.clone(),
        size,
        added_lines: summary.added,
        removed_lines: summary.removed,
        diff: summary.diff,
        has_backup: snapshot.is_some(),
    };
    if let Err(err) = daemon.store.insert_file_change(record.clone()).await {
        tracing::warn!(%err, "cannot record a file change");
        warnings.push("the change was made but could not be recorded for files.revert".into());
    }
    daemon.events.publish(Event::FileChanged {
        change: FileChange {
            diff: None,
            ..record.clone()
        },
    });
    Ok(FileChangeResult {
        host: record.host,
        path: record.path,
        change_id: record.id,
        created: record.created,
        deleted: record.deleted,
        sha256,
        previous_sha256: record.previous_sha256,
        size,
        added_lines: record.added_lines,
        removed_lines: record.removed_lines,
        warnings,
    })
}

fn new_mode(mode: Option<&str>) -> Result<u32, RpcError> {
    mode.map(parse_mode)
        .transpose()
        .map(|mode| mode.unwrap_or(DEFAULT_NEW_MODE))
}

/// The diff a `files.write` or `files.patch` call would produce, shown when it waits
/// for approval. `None` when it cannot be computed (binary, too large, unreachable).
pub async fn preview(daemon: &Daemon, params: &serde_json::Value) -> Option<String> {
    let host = params.get("host")?.as_str()?;
    let path = sftp_path(params.get("path")?.as_str()?).ok()?;
    let remote = Remote::open(daemon, host).await.ok()?;
    let target = remote.target(&path).await.ok()?;
    let old = match &target.existing {
        Some(attrs) => {
            remote
                .snapshot(&target.path, attrs.clone(), None)
                .await
                .ok()?
                .bytes?
        }
        None => Vec::new(),
    };
    let new = if let Some(content) = params.get("content").and_then(|c| c.as_str()) {
        content.as_bytes().to_vec()
    } else {
        let params: FilePatchParams = serde_json::from_value(params.clone()).ok()?;
        let text = std::str::from_utf8(&old).ok()?;
        let patched = match (&params.edits, &params.diff) {
            (Some(edits), None) => patch::apply_edits(text, edits).ok()?,
            (None, Some(diff)) => patch::apply_unified(text, diff).ok()?,
            _ => return None,
        };
        patched.into_bytes()
    };
    patch::summarize(&target.path, &old, &new).diff
}

pub async fn write(
    daemon: &Daemon,
    client: &ClientInfo,
    params: FileWriteParams,
) -> Result<FileChangeResult, RpcError> {
    let request = ChangeRequest {
        new_mode: new_mode(params.mode.as_deref())?,
        host: params.host,
        path: params.path,
        operation: FileOperation::Write,
        expected_sha256: params.expected_sha256,
        overwrite: params.overwrite.unwrap_or(false),
        must_exist: false,
        must_not_exist: false,
        content: NewContent::Bytes(params.content.into_bytes()),
    };
    change(daemon, client, request).await
}

pub async fn patch(
    daemon: &Daemon,
    client: &ClientInfo,
    params: FilePatchParams,
) -> Result<FileChangeResult, RpcError> {
    let request = ChangeRequest {
        host: params.host.clone(),
        path: params.path.clone(),
        operation: FileOperation::Patch,
        expected_sha256: params.expected_sha256.clone(),
        overwrite: true,
        must_exist: true,
        must_not_exist: false,
        new_mode: DEFAULT_NEW_MODE,
        content: NewContent::Patch(params),
    };
    change(daemon, client, request).await
}

pub async fn upload(
    daemon: &Daemon,
    client: &ClientInfo,
    params: FileUploadParams,
) -> Result<FileChangeResult, RpcError> {
    let local = absolute_local(&params.local_path)?;
    let metadata = tokio::fs::metadata(&local)
        .await
        .map_err(|err| local_error(err, &local))?;
    if !metadata.is_file() {
        return Err(invalid(format!("`{}` is not a file", local.display())));
    }
    let request = ChangeRequest {
        new_mode: new_mode(params.mode.as_deref())?,
        host: params.host,
        path: params.remote_path,
        operation: FileOperation::Upload,
        expected_sha256: params.expected_sha256,
        overwrite: params.overwrite.unwrap_or(false),
        must_exist: false,
        must_not_exist: false,
        content: NewContent::Local(local),
    };
    change(daemon, client, request).await
}

pub async fn download(
    daemon: &Daemon,
    params: FileDownloadParams,
) -> Result<FileDownloadResult, RpcError> {
    let local = absolute_local(&params.local_path)?;
    if !params.overwrite.unwrap_or(false) && tokio::fs::try_exists(&local).await.unwrap_or(false) {
        return Err(invalid(format!("`{}` already exists", local.display()))
            .with_hint("pass overwrite: true to replace it"));
    }
    let path = sftp_path(&params.remote_path)?;
    let remote = Remote::open(daemon, &params.host).await?;
    let (real, attrs) = remote.regular_file(&path).await?;
    let file_name = local
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let partial = local.with_file_name(format!(".{file_name}.hostlane-{}", &new_id("t")[2..]));
    let result = remote.snapshot(&real, attrs, Some(&partial)).await;
    let snapshot = match result {
        Ok(snapshot) => snapshot,
        Err(err) => {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(err);
        }
    };
    if let Err(err) = tokio::fs::rename(&partial, &local).await {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(local_error(err, &local));
    }
    Ok(FileDownloadResult {
        host: remote.host,
        remote_path: real,
        local_path: local.display().to_string(),
        size: snapshot.size,
        sha256: snapshot.sha256,
    })
}

fn change_not_found(id: &str) -> RpcError {
    invalid(format!("no file change with id `{id}`"))
        .with_hint("use the change_id returned by files.write, files.patch or files.upload")
}

fn valid_change_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub async fn get_change(daemon: &Daemon, id: &str) -> Result<FileChange, RpcError> {
    if !valid_change_id(id) {
        return Err(change_not_found(id));
    }
    let mut change = daemon
        .store
        .get_file_change(id.to_string())
        .await?
        .ok_or_else(|| change_not_found(id))?;
    change.has_backup = change.has_backup
        && tokio::fs::try_exists(daemon.backups_dir.join(id))
            .await
            .unwrap_or(false);
    Ok(change)
}

/// Restores the file to how it was before `change_id`: removes a created file,
/// otherwise writes back the saved content. Refuses if the file changed since.
pub async fn revert(
    daemon: &Daemon,
    client: &ClientInfo,
    change_id: &str,
) -> Result<FileChangeResult, RpcError> {
    let recorded = get_change(daemon, change_id).await?;
    let content = if recorded.created {
        NewContent::Delete
    } else if recorded.has_backup {
        NewContent::Backup(daemon.backups_dir.join(&recorded.id))
    } else {
        return Err(RpcError::new(
            ErrorKind::FileError,
            format!(
                "the previous content of `{}` is no longer stored",
                recorded.path
            ),
        )
        .with_hint("backups are kept for 30 days"));
    };
    let request = ChangeRequest {
        host: recorded.host,
        path: recorded.path,
        operation: FileOperation::Revert,
        expected_sha256: (!recorded.deleted).then_some(recorded.sha256),
        overwrite: false,
        must_exist: !recorded.deleted,
        must_not_exist: recorded.deleted,
        new_mode: DEFAULT_NEW_MODE,
        content,
    };
    let path = request.path.clone();
    change(daemon, client, request).await.map_err(|err| {
        if err.kind() != ErrorKind::FileChanged {
            return err;
        }
        let mut conflict = RpcError::new(
            ErrorKind::FileChanged,
            format!("`{path}` has changed since {change_id}"),
        )
        .with_hint("revert the later changes first, newest first, or edit the file directly");
        conflict.data.details = err.data.details;
        conflict
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_against_home() {
        assert_eq!(sftp_path("~").unwrap(), ".");
        assert_eq!(sftp_path("~/app/.env").unwrap(), "app/.env");
        assert_eq!(sftp_path("/etc/hosts").unwrap(), "/etc/hosts");
        assert!(sftp_path("").is_err());
        assert_eq!(split("/etc/hosts"), ("/etc", "hosts"));
        assert_eq!(split("/hosts"), ("/", "hosts"));
        assert_eq!(split("notes.txt"), (".", "notes.txt"));
        assert_eq!(join("/", "a"), "/a");
    }

    #[test]
    fn modes_and_hashes() {
        assert_eq!(parse_mode("0755").unwrap(), 0o755);
        assert_eq!(parse_mode("644").unwrap(), 0o644);
        assert!(parse_mode("0999").is_err());
        assert!(parse_mode("").is_err());
        assert_eq!(normalize_sha("SHA256:ABC"), "abc");
    }

    #[test]
    fn line_selection() {
        let text = "a\nb\nc\nd";
        let lines = select_lines(text, 2, 2, 1024);
        assert_eq!(lines.content, "b\nc\n");
        assert_eq!((lines.start, lines.end, lines.total), (2, 3, 4));
        let lines = select_lines(text, 1, 100, 4);
        assert_eq!(lines.content, "a\nb\n");
        assert_eq!(lines.end, 2);
        let lines = select_lines("", 1, 10, 1024);
        assert_eq!((lines.start, lines.end, lines.total), (1, 0, 0));
        let lines = select_lines("long line\n", 1, 10, 4);
        assert_eq!((lines.content.as_str(), lines.end), ("long", 1));
    }

    #[test]
    fn search_prefers_rg_and_quotes_everything() {
        let command = search_command(&FileSearchParams {
            host: "h".into(),
            pattern: "it's".into(),
            path: Some("~/my app".into()),
            glob: Some("*.conf".into()),
            ignore_case: Some(true),
            ..Default::default()
        });
        assert!(command.starts_with("if command -v rg"));
        assert!(command.contains(r"-e 'it'\''s' -- "));
        assert!(command.contains("\"$HOME\"/'my app'"));
        assert!(command.contains("--include='*.conf'"));
        assert!(command.contains("grep -rnI --null --color=never -E -i"));
    }

    #[test]
    fn matches_are_parsed_from_null_separated_lines() {
        let out = b"rg\n./a b.txt\x0012:hello: world\n./c\x003:x\r\nnoise\n";
        let (tool, matches) = parse_matches(out);
        assert_eq!(tool, "rg");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].path, "./a b.txt");
        assert_eq!(matches[0].line, 12);
        assert_eq!(matches[0].text, "hello: world");
        assert_eq!(matches[1].text, "x");
    }
}
