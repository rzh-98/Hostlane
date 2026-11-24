use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Unix time in milliseconds.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Sortable unique id such as `ex-019a8c3f2b10-7f3a9c`, safe to use as a file name.
pub fn new_id(prefix: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let mut random = [0u8; 3];
    if getrandom::fill(&mut random).is_err() {
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        random.copy_from_slice(&count.to_be_bytes()[1..]);
    }
    format!(
        "{prefix}-{:012x}-{:02x}{:02x}{:02x}",
        now_ms(),
        random[0],
        random[1],
        random[2]
    )
}

/// Quotes a string for a POSIX shell.
pub fn sh_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:@%+,".contains(&b))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// A remote path for a shell command line; `~` and `~/…` expand to the home directory.
pub fn sh_path(path: &str) -> String {
    if path == "~" {
        "\"$HOME\"".to_string()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("\"$HOME\"/{}", sh_quote(rest))
    } else {
        sh_quote(path)
    }
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(bytes))
}

/// Longest prefix of `bytes` that does not end inside a UTF-8 sequence.
pub fn utf8_prefix_len(bytes: &[u8]) -> usize {
    let len = bytes.len();
    for back in 1..=len.min(3) {
        let byte = bytes[len - back];
        if byte & 0b1100_0000 == 0b1000_0000 {
            continue;
        }
        let needed = if byte >= 0b1111_0000 {
            4
        } else if byte >= 0b1110_0000 {
            3
        } else if byte >= 0b1100_0000 {
            2
        } else {
            1
        };
        return if needed > back { len - back } else { len };
    }
    len
}

/// Number of leading UTF-8 continuation bytes, so a cut can start on a character.
pub fn utf8_skip_continuation(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take(3)
        .take_while(|b| **b & 0b1100_0000 == 0b1000_0000)
        .count()
}

/// Decodes a byte stream that arrives in chunks without splitting characters.
#[derive(Default)]
pub struct Utf8Chunker {
    pending: Vec<u8>,
}

impl Utf8Chunker {
    pub fn push(&mut self, chunk: &[u8]) -> String {
        self.pending.extend_from_slice(chunk);
        let cut = utf8_prefix_len(&self.pending);
        let rest = self.pending.split_off(cut);
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending = rest;
        text
    }

    pub fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("/srv/app"), "/srv/app");
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote(""), "''");
        assert_eq!(sh_path("~"), "\"$HOME\"");
        assert_eq!(sh_path("~/my app"), "\"$HOME\"/'my app'");
        assert_eq!(sh_path("/srv"), "/srv");
    }

    #[test]
    fn utf8_boundaries() {
        let text = "a中b".as_bytes();
        assert_eq!(utf8_prefix_len(&text[..2]), 1);
        assert_eq!(utf8_prefix_len(&text[..4]), 4);
        assert_eq!(utf8_skip_continuation(&text[2..]), 2);

        let mut chunker = Utf8Chunker::default();
        let mut out = chunker.push(&text[..2]);
        out += &chunker.push(&text[2..]);
        out += &chunker.finish();
        assert_eq!(out, "a中b");
    }

    #[test]
    fn ids_are_unique_and_sortable() {
        let a = new_id("ex");
        let b = new_id("ex");
        assert_ne!(a, b);
        assert!(a.starts_with("ex-"));
        assert_eq!(a.len(), b.len());
    }
}
