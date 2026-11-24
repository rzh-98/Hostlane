//! Host key verification against OpenSSH `known_hosts` files.
//!
//! Hostlane reads the user's files (as named by `UserKnownHostsFile`, default
//! `~/.ssh/known_hosts`) and its own `<data dir>/known_hosts`, and only ever
//! writes to its own file. Supported: comma-separated patterns with `*`, `?`
//! and `!`, hashed names (`|1|salt|hash`), `[host]:port`, and `@revoked`.
//! `@cert-authority` lines are ignored. Malformed lines are skipped.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use data_encoding::BASE64;
use hmac::{Hmac, KeyInit, Mac};
use russh::keys::ssh_key::{HashAlg, PublicKey};
use sha1::Sha1;

/// Outcome of looking a server key up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Known,
    /// No key is recorded for this name.
    Unknown,
    /// Other keys are recorded for this name; `at` is the first one.
    Changed {
        at: String,
    },
    /// The key is listed under `@revoked`.
    Revoked {
        at: String,
    },
}

/// The name OpenSSH records a host under: `host`, or `[host]:port` off port 22.
pub fn entry_name(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == 22 {
        host
    } else {
        format!("[{host}]:{port}")
    }
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

struct Line {
    path: PathBuf,
    number: usize,
    revoked: bool,
    key: PublicKey,
}

impl Line {
    fn at(&self) -> String {
        format!("{}:{}", self.path.display(), self.number)
    }
}

fn matching_lines(name: &str, files: &[PathBuf]) -> Vec<Line> {
    let mut out = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split_whitespace();
            let mut first = fields.next().unwrap_or_default();
            let mut revoked = false;
            if let Some(marker) = first.strip_prefix('@') {
                if marker != "revoked" {
                    continue;
                }
                revoked = true;
                first = fields.next().unwrap_or_default();
            }
            let (Some(key_type), Some(key_data)) = (fields.next(), fields.next()) else {
                continue;
            };
            if !patterns_match(first, name) {
                continue;
            }
            let Ok(key) = PublicKey::from_openssh(&format!("{key_type} {key_data}")) else {
                continue;
            };
            out.push(Line {
                path: path.clone(),
                number: index + 1,
                revoked,
                key,
            });
        }
    }
    out
}

/// Checks `key` for the host recorded as `name` (see [`entry_name`]).
pub fn check(name: &str, key: &PublicKey, files: &[PathBuf]) -> Check {
    let lines = matching_lines(name, files);
    if let Some(line) = lines
        .iter()
        .find(|l| l.revoked && l.key.key_data() == key.key_data())
    {
        return Check::Revoked { at: line.at() };
    }
    let recorded: Vec<&Line> = lines.iter().filter(|l| !l.revoked).collect();
    if recorded.iter().any(|l| l.key.key_data() == key.key_data()) {
        Check::Known
    } else if let Some(first) = recorded.first() {
        Check::Changed { at: first.at() }
    } else {
        Check::Unknown
    }
}

/// Key algorithms already recorded for `name`, so they can be preferred during key exchange.
pub fn recorded_algorithms(name: &str, files: &[PathBuf]) -> Vec<russh::keys::Algorithm> {
    let mut out = Vec::new();
    for line in matching_lines(name, files) {
        let algorithm = line.key.algorithm();
        if !line.revoked && !out.contains(&algorithm) {
            out.push(algorithm);
        }
    }
    out
}

/// Appends `name key-type base64` to `path`, creating it with mode 0600.
pub fn append(path: &Path, name: &str, key: &PublicKey) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let openssh = key
        .to_openssh()
        .map_err(|err| io::Error::other(err.to_string()))?;
    let mut parts = openssh.split_whitespace();
    let (key_type, data) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    writeln!(file, "{name} {key_type} {data}")
}

/// OpenSSH `match_hostname`: any negated match rejects, otherwise any positive match accepts.
fn patterns_match(patterns: &str, name: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.split(',') {
        if let Some(negated) = pattern.strip_prefix('!') {
            if pattern_matches(negated, name) {
                return false;
            }
        } else if pattern_matches(pattern, name) {
            matched = true;
        }
    }
    matched
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    if let Some(hashed) = pattern.strip_prefix("|1|") {
        return hashed_matches(hashed, name);
    }
    crate::ssh_config::wildcard_match(&pattern.to_ascii_lowercase(), name)
}

fn hashed_matches(hashed: &str, name: &str) -> bool {
    let Some((salt, hash)) = hashed.split_once('|') else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (
        BASE64.decode(salt.as_bytes()),
        BASE64.decode(hash.as_bytes()),
    ) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(&salt) else {
        return false;
    };
    mac.update(name.as_bytes());
    mac.verify_slice(&hash).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBLlbfoTK2uRwhVWju822MY0RIqiaKI9JTaU1oS3qzTU";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEYngSx15jFqZ9waICF4vtGHDO5N1YqrhMU8dCoISvDy";

    fn key(text: &str) -> PublicKey {
        PublicKey::from_openssh(text).unwrap()
    }

    fn file(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hostlane-kh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn known_unknown_and_changed() {
        let path = file(
            "basic",
            &format!(
                "# comment\n\nweb.example.com,10.0.0.1  {KEY_A}  laptop\ngarbage line\n[db]:2222 {KEY_B}\n"
            ),
        );
        let files = [path.clone()];
        assert_eq!(check("web.example.com", &key(KEY_A), &files), Check::Known);
        assert_eq!(check("10.0.0.1", &key(KEY_A), &files), Check::Known);
        assert_eq!(
            check(&entry_name("db", 2222), &key(KEY_B), &files),
            Check::Known
        );
        assert_eq!(check("db", &key(KEY_B), &files), Check::Unknown);
        assert_eq!(
            check("web.example.com", &key(KEY_B), &files),
            Check::Changed {
                at: format!("{}:3", path.display())
            }
        );
    }

    fn hashed(name: &str) -> String {
        let salt = b"0123456789abcdefghij";
        let mut mac = Hmac::<Sha1>::new_from_slice(salt).unwrap();
        mac.update(name.as_bytes());
        let hash = mac.finalize().into_bytes();
        format!("|1|{}|{}", BASE64.encode(salt), BASE64.encode(&hash))
    }

    #[test]
    fn patterns_negation_hashes_and_revocation() {
        let path = file(
            "patterns",
            &format!(
                "*.internal,!secret.internal {KEY_A}\n{} {KEY_B}\n@revoked bad.internal {KEY_A}\n@cert-authority *.corp {KEY_B}\n",
                hashed("[hidden]:2222")
            ),
        );
        let files = [path];
        assert_eq!(check("app.internal", &key(KEY_A), &files), Check::Known);
        assert_eq!(
            check("secret.internal", &key(KEY_A), &files),
            Check::Unknown
        );
        assert_eq!(check("[hidden]:2222", &key(KEY_B), &files), Check::Known);
        assert_eq!(check("hidden", &key(KEY_B), &files), Check::Unknown);
        assert!(matches!(
            check("bad.internal", &key(KEY_A), &files),
            Check::Revoked { .. }
        ));
        assert_eq!(check("x.corp", &key(KEY_B), &files), Check::Unknown);
    }

    #[test]
    fn append_then_known() {
        let path = file("append", "");
        append(&path, &entry_name("Box", 2200), &key(KEY_A)).unwrap();
        let files = [path.clone()];
        assert_eq!(check("[box]:2200", &key(KEY_A), &files), Check::Known);
        assert_eq!(
            recorded_algorithms("[box]:2200", &files),
            [russh::keys::Algorithm::Ed25519]
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("[box]:2200 ssh-ed25519 ")
        );
    }
}
