//! `read`, `ls`, `search`, `write`, `edit`, `patch`, `upload`, `download`, `diff`, `revert`.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use hostlane_core::client::Client;
use hostlane_protocol::{
    FileChange, FileChangeParams, FileChangeResult, FileContent, FileDownloadParams,
    FileDownloadResult, FileEdit, FileKind, FileList, FileListParams, FilePatchParams,
    FileReadParams, FileSearchParams, FileSearchResult, FileUploadParams, FileWriteParams, method,
};

use crate::{ago, print_json, print_table, read_stdin};

#[derive(Subcommand)]
pub enum FileCommand {
    /// Print a remote text file. Its sha256 and line range go to stderr.
    Read {
        host: String,
        /// Absolute, or relative to the remote home directory.
        path: String,
        /// First line to print (1-based).
        #[arg(long)]
        offset: Option<u64>,
        /// Lines to print (default 2000; output is capped at 256 KiB).
        #[arg(long)]
        limit: Option<u64>,
    },
    /// List a remote directory (default: the home directory).
    Ls {
        host: String,
        path: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Search remote files with rg, or grep -r when rg is missing.
    Search {
        host: String,
        /// Regular expression, or a literal with --fixed.
        pattern: String,
        /// File or directory to search (default: the home directory).
        path: Option<String>,
        /// Only files matching this glob, e.g. '*.conf'.
        #[arg(long)]
        glob: Option<String>,
        #[arg(short = 'F', long)]
        fixed: bool,
        #[arg(short = 'i', long)]
        ignore_case: bool,
        /// Matches to return (default 200, at most 5000).
        #[arg(long)]
        max: Option<u32>,
    },
    /// Write standard input to a remote file, atomically, keeping owner and mode.
    ///
    /// Replacing an existing file needs --expect <sha256 from `read`> or --overwrite.
    Write {
        host: String,
        path: String,
        /// Fail unless the file currently has this sha256.
        #[arg(long, value_name = "SHA256")]
        expect: Option<String>,
        #[arg(long)]
        overwrite: bool,
        /// Octal permissions for a new file (default 0644).
        #[arg(long)]
        mode: Option<String>,
    },
    /// Replace exact text in a remote file. Each --old must match exactly once.
    Edit {
        host: String,
        path: String,
        /// Text to find. Repeat --old/--new for several edits, applied in order.
        #[arg(long, required = true, allow_hyphen_values = true)]
        old: Vec<String>,
        #[arg(long, required = true, allow_hyphen_values = true)]
        new: Vec<String>,
        /// Replace every occurrence instead of requiring exactly one.
        #[arg(long)]
        all: bool,
        #[arg(long, value_name = "SHA256")]
        expect: Option<String>,
    },
    /// Apply a unified diff from standard input to a remote file.
    Patch {
        host: String,
        path: String,
        #[arg(long, value_name = "SHA256")]
        expect: Option<String>,
    },
    /// Copy a local file to the server, atomically.
    Upload {
        host: String,
        local: String,
        remote: String,
        #[arg(long, value_name = "SHA256")]
        expect: Option<String>,
        #[arg(long)]
        overwrite: bool,
        #[arg(long)]
        mode: Option<String>,
    },
    /// Copy a remote file to this machine.
    Download {
        host: String,
        remote: String,
        local: String,
        #[arg(long)]
        overwrite: bool,
    },
    /// Show a recorded file change and its diff.
    Diff { change_id: String },
    /// Undo a file change, if the file has not changed since.
    Revert { change_id: String },
}

fn absolute(path: &str) -> Result<String> {
    let path = Path::new(path);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("cannot read the current directory")?
            .join(path)
    };
    Ok(absolute.display().to_string())
}

pub async fn run(client: &Client, command: FileCommand, json: bool) -> Result<()> {
    match command {
        FileCommand::Read {
            host,
            path,
            offset,
            limit,
        } => {
            let params = FileReadParams {
                host,
                path,
                offset,
                limit,
            };
            let file: FileContent = client
                .call_as(method::FILES_READ, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&file);
                return Ok(());
            }
            if file.binary {
                eprintln!(
                    "hostlane: {} is binary ({} bytes, sha256 {}); use `hostlane download`",
                    file.path, file.size, file.sha256
                );
                return Ok(());
            }
            let mut stdout = std::io::stdout();
            stdout.write_all(file.content.as_bytes())?;
            stdout.flush()?;
            eprintln!(
                "hostlane: {} lines {}-{} of {} · sha256 {}",
                file.path, file.start_line, file.end_line, file.total_lines, file.sha256
            );
            if file.truncated {
                eprintln!(
                    "hostlane: more lines follow; continue with --offset {}",
                    file.end_line + 1
                );
            }
        }
        FileCommand::Ls { host, path, limit } => {
            let params = FileListParams { host, path, limit };
            let list: FileList = client
                .call_as(method::FILES_LIST, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&list);
                return Ok(());
            }
            let rows: Vec<[String; 4]> = list
                .entries
                .iter()
                .map(|entry| {
                    let name = match entry.kind {
                        FileKind::Dir => format!("{}/", entry.name),
                        FileKind::Symlink => format!("{}@", entry.name),
                        _ => entry.name.clone(),
                    };
                    [
                        entry.mode.clone(),
                        entry.size.to_string(),
                        entry.modified_at.map(ago).unwrap_or_default(),
                        name,
                    ]
                })
                .collect();
            println!("{}", list.path);
            print_table(["MODE", "SIZE", "MODIFIED", "NAME"], &rows);
            if list.truncated {
                eprintln!("hostlane: the listing was cut; raise --limit");
            }
        }
        FileCommand::Search {
            host,
            pattern,
            path,
            glob,
            fixed,
            ignore_case,
            max,
        } => {
            let params = FileSearchParams {
                host,
                pattern,
                path,
                glob,
                fixed: fixed.then_some(true),
                ignore_case: ignore_case.then_some(true),
                max_results: max,
            };
            let result: FileSearchResult = client
                .call_as(method::FILES_SEARCH, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&result);
                return Ok(());
            }
            for found in &result.matches {
                println!("{}:{}:{}", found.path, found.line, found.text);
            }
            for warning in &result.warnings {
                eprintln!("{}: {warning}", result.tool);
            }
            if result.truncated {
                eprintln!("hostlane: more matches exist; raise --max or narrow the search");
            }
        }
        FileCommand::Write {
            host,
            path,
            expect,
            overwrite,
            mode,
        } => {
            let params = FileWriteParams {
                host,
                path,
                content: read_stdin()?,
                expected_sha256: expect,
                overwrite: overwrite.then_some(true),
                mode,
            };
            let result = client
                .call_as(method::FILES_WRITE, serde_json::to_value(&params)?)
                .await?;
            print_change(&result, "Wrote", json);
        }
        FileCommand::Edit {
            host,
            path,
            old,
            new,
            all,
            expect,
        } => {
            if old.len() != new.len() {
                bail!("give one --new for each --old");
            }
            let edits = old
                .into_iter()
                .zip(new)
                .map(|(old_text, new_text)| FileEdit {
                    old_text,
                    new_text,
                    replace_all: all.then_some(true),
                })
                .collect();
            let params = FilePatchParams {
                host,
                path,
                expected_sha256: expect,
                edits: Some(edits),
                diff: None,
            };
            let result = client
                .call_as(method::FILES_PATCH, serde_json::to_value(&params)?)
                .await?;
            print_change(&result, "Edited", json);
        }
        FileCommand::Patch { host, path, expect } => {
            let params = FilePatchParams {
                host,
                path,
                expected_sha256: expect,
                edits: None,
                diff: Some(read_stdin()?),
            };
            let result = client
                .call_as(method::FILES_PATCH, serde_json::to_value(&params)?)
                .await?;
            print_change(&result, "Patched", json);
        }
        FileCommand::Upload {
            host,
            local,
            remote,
            expect,
            overwrite,
            mode,
        } => {
            let params = FileUploadParams {
                host,
                local_path: absolute(&local)?,
                remote_path: remote,
                expected_sha256: expect,
                overwrite: overwrite.then_some(true),
                mode,
            };
            let result = client
                .call_as(method::FILES_UPLOAD, serde_json::to_value(&params)?)
                .await?;
            print_change(&result, "Uploaded", json);
        }
        FileCommand::Download {
            host,
            remote,
            local,
            overwrite,
        } => {
            let params = FileDownloadParams {
                host,
                remote_path: remote,
                local_path: absolute(&local)?,
                overwrite: overwrite.then_some(true),
            };
            let result: FileDownloadResult = client
                .call_as(method::FILES_DOWNLOAD, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&result);
            } else {
                println!(
                    "Downloaded {} to {} ({} bytes) · sha256 {}",
                    result.remote_path, result.local_path, result.size, result.sha256
                );
            }
        }
        FileCommand::Diff { change_id } => {
            let params = FileChangeParams { change_id };
            let change: FileChange = client
                .call_as(method::FILES_CHANGE, serde_json::to_value(&params)?)
                .await?;
            if json {
                print_json(&change);
                return Ok(());
            }
            eprintln!(
                "hostlane: {} {:?} {}:{} by {} {} (+{} -{})",
                change.id,
                change.operation,
                change.host,
                change.path,
                change.client.name,
                ago(change.created_at),
                change.added_lines,
                change.removed_lines
            );
            match &change.diff {
                Some(diff) => print!("{diff}"),
                None => eprintln!("hostlane: no diff stored (binary or very large file)"),
            }
        }
        FileCommand::Revert { change_id } => {
            let params = FileChangeParams { change_id };
            let result = client
                .call_as(method::FILES_REVERT, serde_json::to_value(&params)?)
                .await?;
            print_change(&result, "Reverted", json);
        }
    }
    Ok(())
}

fn print_change(result: &FileChangeResult, verb: &str, json: bool) {
    if json {
        print_json(result);
        return;
    }
    if result.deleted {
        println!("Removed {} · change {}", result.path, result.change_id);
    } else {
        let created = if result.created { " (new file)" } else { "" };
        println!(
            "{verb} {}{created}: +{} -{} lines, {} bytes · sha256 {} · change {}",
            result.path,
            result.added_lines,
            result.removed_lines,
            result.size,
            result.sha256,
            result.change_id
        );
    }
    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }
}
