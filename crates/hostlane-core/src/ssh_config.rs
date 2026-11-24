//! Reads `~/.ssh/config` into [`Host`] entries.
//!
//! Follows OpenSSH semantics for the parts Hostlane needs: the first value
//! obtained for an option wins, `Host` patterns (`*`, `?`, `!`) select which
//! blocks apply to an alias, and `Include` is expanded in place. Only concrete
//! aliases (no wildcards) become hosts. `Match` blocks are skipped.

use std::path::{Path, PathBuf};

use hostlane_protocol::{Host, HostSource};

const MAX_INCLUDE_DEPTH: usize = 16;
const DEFAULT_PORT: u16 = 22;

#[derive(Debug, Default)]
pub struct Loaded {
    pub hosts: Vec<Host>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
struct Block {
    patterns: Vec<String>,
    options: Vec<(String, String)>,
    is_match: bool,
}

/// Loads hosts from the given config file. A missing file yields an empty list.
pub fn load(path: &Path) -> Loaded {
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, &base),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Loaded::default(),
        Err(err) => Loaded {
            hosts: Vec::new(),
            warnings: vec![format!("cannot read {}: {err}", path.display())],
        },
    }
}

/// Parses config text; `base` resolves relative `Include` paths.
pub fn parse(text: &str, base: &Path) -> Loaded {
    let mut loaded = Loaded::default();
    let mut blocks = vec![Block {
        patterns: vec!["*".into()],
        options: Vec::new(),
        is_match: false,
    }];
    parse_into(text, base, 0, &mut blocks, &mut loaded.warnings);
    loaded.hosts = resolve(&blocks);
    loaded
}

fn parse_into(
    text: &str,
    base: &Path,
    depth: usize,
    blocks: &mut Vec<Block>,
    warnings: &mut Vec<String>,
) {
    for raw in text.lines() {
        let Some((key, value)) = split_line(raw) else {
            continue;
        };
        match key.as_str() {
            "host" => blocks.push(Block {
                patterns: tokenize(&value),
                options: Vec::new(),
                is_match: false,
            }),
            "match" => blocks.push(Block {
                patterns: Vec::new(),
                options: Vec::new(),
                is_match: true,
            }),
            "include" => {
                if depth >= MAX_INCLUDE_DEPTH {
                    warnings.push("Include nested too deeply; ignored".into());
                    continue;
                }
                for pattern in tokenize(&value) {
                    for file in expand_include(&pattern, base) {
                        match std::fs::read_to_string(&file) {
                            Ok(text) => parse_into(&text, base, depth + 1, blocks, warnings),
                            Err(err) => {
                                warnings.push(format!("cannot read {}: {err}", file.display()))
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(block) = blocks.last_mut() {
                    block.options.push((key, value));
                }
            }
        }
    }
}

/// Splits `Keyword value` / `Keyword=value`, lowercasing the keyword.
fn split_line(raw: &str) -> Option<(String, String)> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let split = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let key = line[..split].to_ascii_lowercase();
    let rest = line[split..].trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
    Some((key, rest.to_string()))
}

/// Whitespace-separated arguments, honouring double quotes.
fn tokenize(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in value.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(path),
    }
}

fn expand_include(pattern: &str, base: &Path) -> Vec<PathBuf> {
    let path = expand_tilde(pattern);
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    let Some(pattern) = path.to_str() else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = glob::glob(pattern)
        .map(|paths| {
            paths
                .filter_map(Result::ok)
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn is_concrete(pattern: &str) -> bool {
    !pattern.contains(['*', '?']) && !pattern.starts_with('!')
}

/// OpenSSH-style wildcard match: `*` any run, `?` one character.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

fn block_applies(block: &Block, alias: &str) -> bool {
    if block.is_match {
        return false;
    }
    let mut matched = false;
    for pattern in &block.patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if wildcard_match(negated, alias) {
                return false;
            }
        } else if wildcard_match(pattern, alias) {
            matched = true;
        }
    }
    matched
}

fn resolve(blocks: &[Block]) -> Vec<Host> {
    let mut aliases: Vec<&str> = Vec::new();
    for block in blocks.iter().filter(|b| !b.is_match) {
        for pattern in &block.patterns {
            if is_concrete(pattern) && !aliases.contains(&pattern.as_str()) {
                aliases.push(pattern);
            }
        }
    }
    aliases
        .into_iter()
        .map(|alias| resolve_alias(blocks, alias))
        .collect()
}

fn resolve_alias(blocks: &[Block], alias: &str) -> Host {
    let mut hostname = None;
    let mut user = None;
    let mut port = None;
    let mut proxy_jump = None;
    let mut identity_files = Vec::new();

    for block in blocks.iter().filter(|b| block_applies(b, alias)) {
        for (key, value) in &block.options {
            match key.as_str() {
                "hostname" => {
                    hostname.get_or_insert_with(|| value.replace("%h", alias));
                }
                "user" => {
                    user.get_or_insert_with(|| value.clone());
                }
                "port" => {
                    if port.is_none() {
                        port = value.parse::<u16>().ok();
                    }
                }
                "proxyjump" => {
                    proxy_jump.get_or_insert_with(|| value.clone());
                }
                "identityfile" => {
                    let file = tokenize(value).into_iter().next().unwrap_or_default();
                    let file = expand_tilde(&file).display().to_string();
                    if !identity_files.contains(&file) {
                        identity_files.push(file);
                    }
                }
                _ => {}
            }
        }
    }

    Host {
        id: alias.to_string(),
        alias: alias.to_string(),
        hostname: hostname.unwrap_or_else(|| alias.to_string()),
        user,
        port: port.unwrap_or(DEFAULT_PORT),
        proxy_jump: proxy_jump.filter(|p| !p.eq_ignore_ascii_case("none")),
        identity_files,
        source: HostSource::SshConfig,
        env: None,
        tags: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(text: &str) -> Vec<Host> {
        parse(text, Path::new("/nonexistent")).hosts
    }

    #[test]
    fn first_value_wins_and_wildcards_supply_defaults() {
        let list = hosts(
            "Host web1\n  HostName 10.0.0.1\n  User deploy\n\nHost web2 web3\n  HostName %h.example.com\n\nHost *\n  User root\n  Port 2222\n",
        );
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].alias, "web1");
        assert_eq!(list[0].hostname, "10.0.0.1");
        assert_eq!(list[0].user.as_deref(), Some("deploy"));
        assert_eq!(list[0].port, 2222);
        assert_eq!(list[1].hostname, "web2.example.com");
        assert_eq!(list[2].user.as_deref(), Some("root"));
    }

    #[test]
    fn skips_patterns_negations_and_match_blocks() {
        let list = hosts(
            "Host *.internal !bastion\n  User ops\nHost bastion db.internal\n  Port 22\nMatch host db.internal\n  User ignored\n",
        );
        let aliases: Vec<_> = list.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, ["bastion", "db.internal"]);
        assert_eq!(list[0].user, None);
        assert_eq!(list[1].user.as_deref(), Some("ops"));
    }

    #[test]
    fn handles_equals_quotes_and_case() {
        let list = hosts(
            "HOST box\n  hostname=1.2.3.4\n  IdentityFile \"/keys/my key\"\n  ProxyJump none\n",
        );
        assert_eq!(list[0].hostname, "1.2.3.4");
        assert_eq!(list[0].identity_files, ["/keys/my key"]);
        assert_eq!(list[0].proxy_jump, None);
    }

    #[test]
    fn expands_includes_relative_to_base() {
        let dir = std::env::temp_dir().join(format!("hostlane-include-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("conf.d")).unwrap();
        std::fs::write(
            dir.join("conf.d/a.conf"),
            "Host included\n  HostName 9.9.9.9\n",
        )
        .unwrap();
        let loaded = parse("Include conf.d/*.conf\nHost local\n", &dir);
        std::fs::remove_dir_all(&dir).unwrap();
        let aliases: Vec<_> = loaded.hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, ["included", "local"]);
        assert_eq!(loaded.hosts[0].hostname, "9.9.9.9");
    }

    #[test]
    fn wildcard_matching() {
        assert!(wildcard_match("web-*", "web-1"));
        assert!(wildcard_match("db?", "db1"));
        assert!(!wildcard_match("db?", "db12"));
        assert!(wildcard_match("*", ""));
    }
}
