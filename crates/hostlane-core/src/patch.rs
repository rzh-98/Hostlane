//! Text edits for `files.patch` (search-and-replace edits or a unified diff) and the
//! diff summary recorded for every file change.

use std::time::Duration;

use hostlane_protocol::{ErrorKind, FileEdit, RpcError};
use serde_json::json;
use similar::{ChangeTag, TextDiff};

/// Larger files get line counts but no stored diff.
const DIFF_INPUT_CAP: usize = 8 * 1024 * 1024;
/// Stored diffs are cut here.
const DIFF_OUTPUT_CAP: usize = 1024 * 1024;

fn failed(message: String, details: serde_json::Value) -> RpcError {
    RpcError::new(ErrorKind::PatchFailed, message)
        .with_hint("nothing was written; read the file again with files.read and retry")
        .with_details(details)
}

/// `\n` → `\r\n`, for edits written against a file with Windows line endings.
fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Applies search-and-replace edits in order.
pub fn apply_edits(text: &str, edits: &[FileEdit]) -> Result<String, RpcError> {
    let crlf = text.contains("\r\n");
    let mut out = text.to_string();
    for (index, edit) in edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                format!("edit {index}: `old_text` must not be empty"),
            ));
        }
        let mut old = edit.old_text.clone();
        let mut new = edit.new_text.clone();
        let mut count = out.matches(&old).count();
        if count == 0 && crlf && old.contains('\n') && !old.contains("\r\n") {
            old = to_crlf(&old);
            new = to_crlf(&new);
            count = out.matches(&old).count();
        }
        let replace_all = edit.replace_all.unwrap_or(false);
        match count {
            0 => {
                return Err(failed(
                    format!("edit {index}: `old_text` was not found in the file"),
                    json!({ "edit": index, "matches": 0 }),
                ));
            }
            1 => {}
            _ if replace_all => {}
            _ => {
                return Err(failed(
                    format!(
                        "edit {index}: `old_text` occurs {count} times; add surrounding lines to make it unique, or set replace_all"
                    ),
                    json!({ "edit": index, "matches": count }),
                ));
            }
        }
        out = if replace_all {
            out.replace(&old, &new)
        } else {
            out.replacen(&old, &new, 1)
        };
    }
    Ok(out)
}

#[derive(Debug, Default)]
struct Hunk {
    /// 1-based first old line from the header; for a pure insertion, the line it follows.
    old_start: usize,
    old_count: Option<usize>,
    new_count: Option<usize>,
    old: Vec<String>,
    new: Vec<String>,
    old_no_eol: bool,
    new_no_eol: bool,
}

/// `-12,3` → (12, Some(3)); `-12` → (12, Some(1)).
fn range(spec: &str) -> Option<(usize, Option<usize>)> {
    let spec = &spec[1..];
    match spec.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok())),
        None => Some((spec.parse().ok()?, Some(1))),
    }
}

fn parse_header(line: &str) -> Option<Hunk> {
    let mut parts = line.strip_prefix("@@")?.split_whitespace();
    let old = parts.next().filter(|p| p.starts_with('-'))?;
    let new = parts.next().filter(|p| p.starts_with('+'))?;
    let (old_start, old_count) = range(old)?;
    let (_, new_count) = range(new)?;
    Some(Hunk {
        old_start,
        old_count,
        new_count,
        ..Hunk::default()
    })
}

/// Drops blank lines that trail a hunk beyond the counts in its header
/// (separators between hunks rather than empty context lines).
fn trim_trailing_blanks(hunk: &mut Hunk) {
    while let (Some(old_count), Some(new_count)) = (hunk.old_count, hunk.new_count) {
        let extra = hunk.old.len() > old_count && hunk.new.len() > new_count;
        let blank = hunk.old.last().is_some_and(String::is_empty)
            && hunk.new.last().is_some_and(String::is_empty);
        if !(extra && blank) {
            break;
        }
        hunk.old.pop();
        hunk.new.pop();
    }
}

fn parse_diff(diff: &str) -> Result<Vec<Hunk>, RpcError> {
    let invalid = |message: String| RpcError::new(ErrorKind::InvalidParams, message);
    let lines: Vec<&str> = diff.lines().collect();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut files = 0;
    let mut last: Option<char> = None;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].strip_suffix('\r').unwrap_or(lines[index]);
        index += 1;
        if line.starts_with("--- ")
            && lines
                .get(index)
                .is_some_and(|next| next.starts_with("+++ "))
        {
            files += 1;
            if files > 1 {
                return Err(invalid(
                    "the diff changes more than one file; send one diff per file".into(),
                ));
            }
            index += 1;
            continue;
        }
        if line.starts_with("@@") {
            if let Some(hunk) = hunks.last_mut() {
                trim_trailing_blanks(hunk);
            }
            let hunk = parse_header(line)
                .ok_or_else(|| invalid(format!("invalid hunk header `{line}`")))?;
            hunks.push(hunk);
            last = None;
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            if line.starts_with("diff ")
                || line.starts_with("index ")
                || line.starts_with("new file")
                || line.starts_with("deleted file")
                || line.starts_with("similarity")
                || line.starts_with("rename ")
                || line.starts_with("old mode")
                || line.starts_with("new mode")
                || line.trim().is_empty()
            {
                continue;
            }
            return Err(invalid(format!(
                "line {index} of the diff is not part of a hunk; a hunk starts with `@@ -line,count +line,count @@`"
            )));
        };
        let (tag, text) = match line.chars().next() {
            None => (' ', ""),
            Some(tag) => (tag, &line[tag.len_utf8()..]),
        };
        match tag {
            ' ' => {
                hunk.old.push(text.into());
                hunk.new.push(text.into());
            }
            '-' => hunk.old.push(text.into()),
            '+' => hunk.new.push(text.into()),
            '\\' => match last {
                Some('-') => hunk.old_no_eol = true,
                Some('+') => hunk.new_no_eol = true,
                Some(' ') => {
                    hunk.old_no_eol = true;
                    hunk.new_no_eol = true;
                }
                _ => {}
            },
            _ => {
                return Err(invalid(format!(
                    "line {index} of the diff starts with `{tag}`; hunk lines start with ' ', '-' or '+'"
                )));
            }
        }
        if tag != '\\' {
            last = Some(tag);
        }
    }
    if let Some(hunk) = hunks.last_mut() {
        trim_trailing_blanks(hunk);
    }
    if hunks.is_empty() {
        return Err(invalid("the diff has no hunks".into()));
    }
    Ok(hunks)
}

fn block_matches(lines: &[&str], at: usize, block: &[String], loose: bool) -> bool {
    block.iter().enumerate().all(|(offset, want)| {
        let have = lines[at + offset].trim_end_matches('\r');
        if loose {
            have.trim_end() == want.trim_end()
        } else {
            have == want
        }
    })
}

/// Nearest position at or after `from` where `block` matches, searching outward from `near`.
fn locate(lines: &[&str], block: &[String], from: usize, near: usize) -> Option<usize> {
    if block.len() > lines.len() {
        return None;
    }
    let last = lines.len() - block.len();
    if from > last {
        return None;
    }
    let near = near.clamp(from, last);
    for loose in [false, true] {
        for distance in 0..=(last - from) {
            let candidates = [near.checked_add(distance), near.checked_sub(distance)];
            for at in candidates.into_iter().flatten() {
                if at >= from && at <= last && block_matches(lines, at, block, loose) {
                    return Some(at);
                }
            }
        }
    }
    None
}

/// Applies a unified diff. Hunks are found by their content, nearest to the line in
/// their header, so slightly wrong line numbers still apply.
pub fn apply_unified(text: &str, diff: &str) -> Result<String, RpcError> {
    let hunks = parse_diff(diff)?;
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut ends_with_newline = text.ends_with('\n');
    let lines: Vec<&str> = if text.is_empty() {
        Vec::new()
    } else {
        let body = text.strip_suffix('\n').unwrap_or(text);
        body.split('\n').collect()
    };

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut cursor = 0;
    for (index, hunk) in hunks.iter().enumerate() {
        let at = if hunk.old.is_empty() {
            let after = if hunk.old_count == Some(0) {
                hunk.old_start
            } else {
                hunk.old_start.saturating_sub(1)
            };
            after.clamp(cursor, lines.len())
        } else {
            locate(&lines, &hunk.old, cursor, hunk.old_start.saturating_sub(1)).ok_or_else(
                || {
                    failed(
                        format!(
                            "hunk {} (@@ -{} @@) does not match the file",
                            index + 1,
                            hunk.old_start
                        ),
                        json!({
                            "hunk": index + 1,
                            "line": hunk.old_start,
                            "first_line": hunk.old.first(),
                        }),
                    )
                },
            )?
        };
        out.extend(lines[cursor..at].iter().map(|line| line.to_string()));
        for line in &hunk.new {
            if eol == "\r\n" {
                out.push(format!("{line}\r"));
            } else {
                out.push(line.clone());
            }
        }
        cursor = at + hunk.old.len();
        if cursor == lines.len() {
            if hunk.new_no_eol {
                ends_with_newline = false;
            } else if hunk.old_no_eol || lines.is_empty() {
                ends_with_newline = true;
            }
        }
    }
    out.extend(lines[cursor..].iter().map(|line| line.to_string()));

    let mut result = out.join("\n");
    if ends_with_newline && !out.is_empty() {
        result.push('\n');
    }
    if eol == "\r\n" && !ends_with_newline && result.ends_with('\r') {
        result.pop();
    }
    Ok(result)
}

/// Line counts and a unified diff between two versions, when both are text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub added: u64,
    pub removed: u64,
    pub diff: Option<String>,
}

pub fn summarize(path: &str, old: &[u8], new: &[u8]) -> Summary {
    if old.len() > DIFF_INPUT_CAP || new.len() > DIFF_INPUT_CAP {
        return Summary::default();
    }
    let (Ok(old), Ok(new)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
        return Summary::default();
    };
    let diff = TextDiff::configure()
        .timeout(Duration::from_secs(2))
        .diff_lines(old, new);
    let mut summary = Summary::default();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => summary.added += 1,
            ChangeTag::Delete => summary.removed += 1,
            ChangeTag::Equal => {}
        }
    }
    let path = path.trim_start_matches('/');
    let mut text = diff
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string();
    if text.len() > DIFF_OUTPUT_CAP {
        let cut = crate::util::utf8_prefix_len(&text.as_bytes()[..DIFF_OUTPUT_CAP]);
        text.truncate(cut);
        text.push_str("\n…[diff cut at 1 MiB]\n");
    }
    summary.diff = Some(text);
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(old: &str, new: &str) -> FileEdit {
        FileEdit {
            old_text: old.into(),
            new_text: new.into(),
            replace_all: None,
        }
    }

    #[test]
    fn edits_replace_unique_text() {
        let text = "server {\n  listen 80;\n}\n";
        let out = apply_edits(text, &[edit("listen 80;", "listen 8080;")]).unwrap();
        assert_eq!(out, "server {\n  listen 8080;\n}\n");
    }

    #[test]
    fn edits_reject_missing_and_ambiguous_text() {
        let err = apply_edits("a\na\n", &[edit("b", "c")]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PatchFailed);
        let err = apply_edits("a\na\n", &[edit("a", "c")]).unwrap_err();
        assert_eq!(err.data.details.unwrap()["matches"], 2);
        let mut all = edit("a", "c");
        all.replace_all = Some(true);
        assert_eq!(apply_edits("a\na\n", &[all]).unwrap(), "c\nc\n");
    }

    #[test]
    fn edits_follow_crlf_files() {
        let out = apply_edits("one\r\ntwo\r\n", &[edit("one\ntwo", "1\n2")]).unwrap();
        assert_eq!(out, "1\r\n2\r\n");
    }

    #[test]
    fn unified_diff_applies_with_offset_line_numbers() {
        let text = "a\nb\nc\nd\ne\nf\n";
        let diff = "--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n b\n-c\n+C\n d\n";
        assert_eq!(apply_unified(text, diff).unwrap(), "a\nb\nC\nd\ne\nf\n");
    }

    #[test]
    fn unified_diff_handles_multiple_hunks_and_insertions() {
        let text = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let diff = "@@ -0,0 +1 @@\n+0\n@@ -2,2 +3,2 @@\n 2\n-3\n+three\n@@ -8 +9,2 @@\n 8\n+9\n";
        assert_eq!(
            apply_unified(text, diff).unwrap(),
            "0\n1\n2\nthree\n4\n5\n6\n7\n8\n9\n"
        );
    }

    #[test]
    fn unified_diff_tracks_the_final_newline() {
        let diff = "@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b\n";
        assert_eq!(apply_unified("a", diff).unwrap(), "b\n");
        let diff = "@@ -1 +1 @@\n-a\n+b\n\\ No newline at end of file\n";
        assert_eq!(apply_unified("a\n", diff).unwrap(), "b");
    }

    #[test]
    fn unified_diff_creates_content_in_an_empty_file() {
        let diff = "@@ -0,0 +1,2 @@\n+hello\n+world\n";
        assert_eq!(apply_unified("", diff).unwrap(), "hello\nworld\n");
    }

    #[test]
    fn unified_diff_rejects_mismatched_hunks() {
        let err = apply_unified("a\nb\n", "@@ -1 +1 @@\n-x\n+y\n").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PatchFailed);
        assert_eq!(err.data.details.unwrap()["hunk"], 1);
        let err = apply_unified("a\n", "just text").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidParams);
    }

    #[test]
    fn unified_diff_keeps_crlf_and_tolerates_blank_context() {
        let text = "a\r\n\r\nb\r\n";
        let diff = "@@ -1,3 +1,3 @@\n a\n\n-b\n+B\n";
        assert_eq!(apply_unified(text, diff).unwrap(), "a\r\n\r\nB\r\n");
    }

    #[test]
    fn summary_counts_lines_and_renders_a_diff() {
        let summary = summarize("/etc/app.conf", b"a\nb\n", b"a\nc\nd\n");
        assert_eq!((summary.added, summary.removed), (2, 1));
        let diff = summary.diff.unwrap();
        assert!(diff.starts_with("--- a/etc/app.conf\n+++ b/etc/app.conf\n"));
        assert!(diff.contains("+d\n"));
        assert_eq!(summarize("x", b"\xff", b"a"), Summary::default());
    }
}
