use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::Path,
};

use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{
    Value, json,
    value::{RawValue, to_raw_value},
};
use toml_edit::{DocumentMut, Item, Table};

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Target {
    Codex,
    Claude,
    Cursor,
}

const ROUTING_ENV: [&str; 3] = [
    "HOSTLANE_SOCKET",
    "HOSTLANE_DATA_DIR",
    "HOSTLANE_SSH_CONFIG",
];

fn table<'a>(
    root: &'a mut dyn toml_edit::TableLike,
    key: &str,
) -> Result<&'a mut dyn toml_edit::TableLike> {
    root.entry(key)
        .or_insert(Item::Table(Table::new()))
        .as_table_like_mut()
        .context("配置条目不是表，未修改")
}

fn known_command(command: Option<&str>) -> bool {
    command
        .and_then(|value| value.rsplit(['/', '\\']).next())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("hostlane") || name.eq_ignore_ascii_case("hostlane.exe")
        })
}

fn merge(
    target: Target,
    original: &str,
    binary: &str,
    env: &BTreeMap<String, String>,
) -> Result<String> {
    if matches!(target, Target::Codex) {
        let mut doc = original
            .parse::<DocumentMut>()
            .map_err(|_| anyhow!("无法解析 TOML，未修改配置"))?;
        let servers = table(doc.as_table_mut(), "mcp_servers")?;
        let exists = servers.contains_key("hostlane");
        let server = table(servers, "hostlane")?;
        ensure!(
            !exists
                || known_command(server.get("command").and_then(Item::as_str))
                    && !server.contains_key("url"),
            "已有其他服务使用 hostlane 名称，未覆盖"
        );
        server.insert("command", toml_edit::value(binary));
        server.insert(
            "args",
            toml_edit::value(["mcp"].into_iter().collect::<toml_edit::Array>()),
        );
        server
            .entry("tool_timeout_sec")
            .or_insert(toml_edit::value(600));
        if server.contains_key("env") || !env.is_empty() {
            let vars = table(server, "env")?;
            for name in ROUTING_ENV {
                if !env.contains_key(name) {
                    vars.remove(name);
                }
            }
            for (name, value) in env {
                vars.insert(name, toml_edit::value(value.as_str()));
            }
        }
        return Ok(doc.to_string());
    }
    type Object = BTreeMap<String, Box<RawValue>>;
    let mut root: Object =
        serde_json::from_str(original).map_err(|_| anyhow!("无法解析 JSON 对象，未修改配置"))?;
    let mut servers: Object = match root.get("mcpServers") {
        Some(raw) => {
            serde_json::from_str(raw.get()).map_err(|_| anyhow!("mcpServers 不是 JSON 对象"))?
        }
        None => Object::new(),
    };
    let mut entry = match servers.get("hostlane") {
        Some(raw) => {
            let existing: Value = serde_json::from_str(raw.get())?;
            ensure!(
                known_command(existing["command"].as_str()) && existing.get("url").is_none(),
                "已有其他服务使用 hostlane 名称，未覆盖"
            );
            existing
        }
        None => json!({}),
    };
    entry["command"] = json!(binary);
    entry["args"] = json!(["mcp"]);
    if entry.get("env").is_some() || !env.is_empty() {
        if entry.get("env").is_none() {
            entry["env"] = json!({});
        }
        let vars = entry["env"].as_object_mut().context("env 不是 JSON 对象")?;
        for name in ROUTING_ENV {
            if !env.contains_key(name) {
                vars.remove(name);
            }
        }
        for (name, value) in env {
            vars.insert(name.clone(), json!(value));
        }
    }
    servers.insert("hostlane".into(), to_raw_value(&entry)?);
    root.insert("mcpServers".into(), to_raw_value(&servers)?);
    Ok(serde_json::to_string_pretty(&root)? + "\n")
}

fn read(path: &Path) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(stat) => {
            ensure!(
                stat.is_file() && stat.len() <= 32 * 1024 * 1024,
                "配置不是普通文件或过大，未修改"
            );
            Ok(Some(fs::read_to_string(path)?))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

pub fn configure(target: Target, path: &Path) -> Result<()> {
    let mut token = zeroize::Zeroizing::new(String::new());
    std::io::stdin().take(4097).read_to_string(&mut token)?;
    ensure!(
        token.len() <= 4096 && !token.contains(char::is_control),
        "令牌无效"
    );
    let mut env: BTreeMap<_, _> = ROUTING_ENV
        .into_iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_owned(), value))
        })
        .collect();
    if !token.is_empty() {
        env.insert("HOSTLANE_TOKEN".into(), token.to_string());
    }
    let parent = path.parent().context("无效的配置路径")?;
    let previous = read(path)?;
    let output = merge(
        target,
        previous
            .as_deref()
            .unwrap_or(if matches!(target, Target::Codex) {
                ""
            } else {
                "{}"
            }),
        std::env::current_exe()?.to_str().context("程序路径无效")?,
        &env,
    )?;
    if previous.as_deref() == Some(&output) {
        return Ok(());
    }
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(output.as_bytes())?;
    temp.as_file().sync_all()?;
    ensure!(
        previous == read(path)?,
        "配置已被其他程序修改，请关闭客户端后重试"
    );
    if previous.is_none() {
        temp.persist_noclobber(path)?;
    } else {
        temp.persist(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merges_without_replacing_unrelated_settings_or_tokens() {
        let src = "title='keep' # comment\n[mcp_servers.hostlane]\ncommand='hostlane'\n[mcp_servers.hostlane.env]\nHOSTLANE_TOKEN='previous'\n";
        let merged = merge(
            Target::Codex,
            src,
            "/with spaces/hostlane",
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(merged.contains("# comment"));
        assert_eq!(merged.parse::<DocumentMut>().unwrap()["mcp_servers"]["hostlane"]["env"]["HOSTLANE_TOKEN"].as_str(),src.parse::<DocumentMut>().unwrap()["mcp_servers"]["hostlane"]["env"]["HOSTLANE_TOKEN"].as_str());
        assert_eq!(
            merge(
                Target::Codex,
                &merged,
                "/with spaces/hostlane",
                &BTreeMap::new()
            )
            .unwrap(),
            merged
        );
        let source = r#"{"other":{"id":123456789123456789123456789},"mcpServers":{"sibling":{"command":"other"}}}"#;
        let merged = merge(Target::Claude, source, "hostlane.exe", &BTreeMap::new()).unwrap();
        assert!(merged.contains("123456789123456789123456789") && merged.contains("sibling"));
        assert!(
            merge(
                Target::Cursor,
                r#"{"mcpServers":{"hostlane":{"url":"other"}}}"#,
                "hostlane",
                &BTreeMap::new()
            )
            .is_err()
        );
        assert!(merge(Target::Codex, "invalid!!", "hostlane", &BTreeMap::new()).is_err());
    }
}
