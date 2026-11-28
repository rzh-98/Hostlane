use super::*;
use rfd::{AsyncFileDialog, AsyncMessageDialog, MessageButtons, MessageDialogResult};
use std::{env, path::Path, process::Stdio};
use tauri_plugin_opener::OpenerExt;
use tokio::{io::AsyncWriteExt, sync::Mutex};

static PROMPT: Mutex<()> = Mutex::const_new(());
static CONFIG: Mutex<()> = Mutex::const_new(());
fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "无法定位用户目录".into())
}
#[tauri::command]
pub async fn choose_key() -> Option<PathBuf> {
    AsyncFileDialog::new()
        .set_title("选择 SSH 私钥")
        .set_directory(home().ok()?.join(".ssh"))
        .pick_file()
        .await
        .map(|file| file.path().to_owned())
}
#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    let url = tauri::Url::parse(&url).map_err(|_| "无效的网址")?;
    if !matches!(url.scheme(), "https" | "http") || url.host().is_none() {
        return Err("只允许 HTTP(S)".into());
    }
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|err| err.to_string())
}
#[tauri::command]
pub async fn reveal_trust_record(app: AppHandle, host: String) -> bool {
    let reply = rpc("connections.list", Value::Null).await;
    let Some(data) = reply["result"]["connections"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["host"] == host))
        .map(|item| &item["error"]["data"])
    else {
        return false;
    };
    if data["kind"] != "HOST_KEY_CHANGED" {
        return false;
    }
    let Some((file, line)) = data["details"]["recorded_at"]
        .as_str()
        .and_then(|loc| loc.rsplit_once(':'))
    else {
        return false;
    };
    line.parse::<usize>().is_ok()
        && Path::new(file).is_absolute()
        && !file.starts_with("//")
        && !file.starts_with("\\\\")
        && app.opener().reveal_item_in_dir(file).is_ok()
}
#[cfg(windows)]
fn powershell() -> PathBuf {
    PathBuf::from(env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32/WindowsPowerShell/v1.0/powershell.exe")
}
async fn password(label: &str) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    let mut prompt = {
        let mut cmd = command("/usr/bin/osascript");
        cmd.args(["-e",r#"on run argv
try
activate
set answer to display dialog (item 1 of argv) with title "Hostlane · SSH" default answer "" with hidden answer buttons {"取消", "保存"} default button "保存" cancel button "取消"
return text returned of answer
on error number -128
return ""
end try
end run"#,label]);
        cmd
    };
    #[cfg(windows)]
    let mut prompt = {
        use base64::Engine;
        let script = include_str!("password.ps1");
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let mut cmd = command(powershell());
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-EncodedCommand",
            &encoded,
        ])
        .env("HOSTLANE_DIALOG_LABEL", label);
        cmd
    };
    #[cfg(target_os = "linux")]
    let mut prompt = {
        if Path::new("/usr/bin/zenity").is_file() {
            let mut cmd = command("/usr/bin/zenity");
            cmd.args(["--password", "--title", label]);
            cmd
        } else {
            let mut cmd = command("/usr/bin/kdialog");
            cmd.args([
                "--title",
                "Hostlane · SSH",
                "--password",
                &label
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
            ]);
            cmd
        }
    };
    let output = tokio::time::timeout(
        Duration::from_secs(180),
        prompt.stderr(Stdio::null()).output(),
    )
    .await
    .map_err(|_| "密码输入已超时")?
    .map_err(|_| "无法打开系统密码窗口")?;
    if !output.status.success() {
        return if output.status.code() == Some(1) {
            Ok(None)
        } else {
            Err("系统密码窗口不可用".into())
        };
    }
    let mut text = String::from_utf8(output.stdout).map_err(|_| "无效的密码输入")?;
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    Ok((!text.is_empty()).then_some(text))
}
#[tauri::command]
pub async fn prompt_secret(host: String, kind: String, key: Option<String>) -> Value {
    let Ok(_guard) = PROMPT.try_lock() else {
        return fail("IO", "已有密码窗口打开");
    };
    if host.chars().any(char::is_control)
        || !matches!(kind.as_str(), "password" | "passphrase")
        || key
            .as_ref()
            .is_some_and(|key| kind != "passphrase" || key.chars().any(char::is_control))
    {
        return fail("INVALID_PARAMS", "无效的主机或密钥");
    }
    let reply = rpc("hosts.list", Value::Null).await;
    if reply["ok"] != true {
        return reply;
    }
    let Some(entry) = reply["result"]["hosts"]
        .as_array()
        .and_then(|all| all.iter().find(|entry| entry["id"] == host))
    else {
        return fail("HOST_NOT_FOUND", "找不到主机");
    };
    let label = format!(
        "{} · {} ({}) {}",
        if kind == "password" {
            "登录密码"
        } else {
            "密钥口令"
        },
        host,
        entry["hostname"]
            .as_str()
            .unwrap_or("")
            .replace(char::is_control, " "),
        key.as_deref().unwrap_or("")
    );
    match password(&label).await {
        Ok(Some(secret)) => {
            let result = rpc(
                "secrets.set",
                json!({"host":host,"kind":kind,"key_path":key,"secret":secret}),
            )
            .await;
            if result["ok"] == true {
                success(json!(true))
            } else {
                fail("IO", "无法保存，请检查系统钥匙串和连接状态")
            }
        }
        Ok(None) => success(json!(false)),
        Err(err) => fail("IO", err),
    }
}
#[tauri::command]
pub async fn configure_mcp(app: AppHandle, client: String, token: Option<String>) -> Value {
    let Ok(_guard) = CONFIG.try_lock() else {
        return fail("IO", "已有配置操作正在进行");
    };
    if token.as_deref().is_some_and(|token| {
        !token
            .strip_prefix("hlt_")
            .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    }) {
        return fail("INVALID_PARAMS", "无效的令牌");
    }
    let Ok(home) = home() else {
        return fail("IO", "找不到用户目录");
    };
    let file = match client.as_str() {
        "codex" => env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("config.toml"),
        "claude" => env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or(home)
            .join(".claude.json"),
        "cursor" => home.join(".cursor/mcp.json"),
        _ => return fail("INVALID_PARAMS", "无效的客户端"),
    };
    if AsyncMessageDialog::new()
        .set_title("写入 MCP 配置？")
        .set_description(format!(
            "请先关闭 {client}。Hostlane 将更新 {} 的 hostlane 条目。所选令牌将保存在本机配置中。",
            file.display()
        ))
        .set_buttons(MessageButtons::YesNo)
        .show()
        .await
        != MessageDialogResult::Yes
    {
        return success(Value::Null);
    }
    let Ok(path) = binary(&app) else {
        return fail("IO", "找不到 CLI");
    };
    let run = async {
        let mut child = command(path)
            .arg("configure-mcp")
            .arg(client)
            .arg(&file)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(token.as_deref().unwrap_or("").as_bytes())
                .await?;
        }
        child.wait().await
    };
    match tokio::time::timeout(Duration::from_secs(20), run).await {
        Ok(Ok(status)) if status.success() => success(json!(file)),
        _ => fail("IO", "写入配置失败，请检查文件权限"),
    }
}
#[tauri::command]
pub async fn export_audit(app: AppHandle, format: String, scope: Value) -> Value {
    if !matches!(format.as_str(), "csv" | "jsonl") {
        return fail("INVALID_PARAMS", "未知的导出格式");
    }
    let Some(filters) = scope.as_object() else {
        return fail("INVALID_PARAMS", "筛选条件无效");
    };
    let mut args = vec!["audit-export".to_owned(), format!("--format={format}")];
    for (key, value) in filters {
        let arg = match key.as_str() {
            "client" | "task" | "session" | "host" => value
                .as_str()
                .filter(|text| text.len() <= 4096 && !text.contains('\0'))
                .map(|text| format!("--{key}={text}")),
            "min_risk" => value
                .as_str()
                .filter(|text| matches!(*text, "L0" | "L1" | "L2" | "L3"))
                .map(|text| format!("--min-risk={}", text.to_lowercase())),
            "since" => value.as_u64().map(|num| format!("--since={num}")),
            _ => None,
        };
        let Some(arg) = arg else {
            return fail("INVALID_PARAMS", "筛选条件无效");
        };
        args.push(arg);
    }
    let Some(dest) = AsyncFileDialog::new()
        .set_file_name(format!("hostlane-audit.{format}"))
        .save_file()
        .await
    else {
        return success(Value::Null);
    };
    let Some(parent) = dest.path().parent() else {
        return fail("IO", "无效路径");
    };
    let Ok(path) = binary(&app) else {
        return fail("IO", "找不到 CLI");
    };
    let run = async {
        let temp = tempfile::NamedTempFile::new_in(parent)?;
        let status = command(path)
            .args(args)
            .env("HOSTLANE_CLIENT", "desktop")
            .env_remove("HOSTLANE_TOKEN")
            .stdout(temp.reopen()?)
            .stderr(Stdio::null())
            .status()
            .await?;
        if !status.success() {
            return Err(std::io::Error::other("导出失败"));
        }
        temp.as_file().sync_all()?;
        temp.persist(dest.path())?;
        Ok::<_, std::io::Error>(())
    };
    match run.await {
        Ok(()) => success(json!(dest.path())),
        Err(err) => fail("IO", err),
    }
}
#[cfg(windows)]
async fn user_path(add: Option<&Path>) -> Result<String, String> {
    let dir = add.and_then(Path::to_str).unwrap_or("");
    if dir.contains(';') {
        return Err("无效目录".into());
    }
    let out = command(powershell())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            include_str!("path.ps1"),
        ])
        .env("HOSTLANE_PATH_ENTRY", dir)
        .output()
        .await
        .map_err(|err| err.to_string())?;
    if !out.status.success() {
        return Err("无法读取或修改用户 PATH".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
#[tauri::command]
pub async fn setup_status(app: AppHandle) -> Result<Value, String> {
    let bin = binary(&app)?;
    let link = if cfg!(windows) {
        bin.clone()
    } else {
        home()?.join(".local/bin/hostlane")
    };
    let target = std::fs::read_link(&link)
        .ok()
        .map(|path| link.parent().unwrap().join(path));
    #[cfg(windows)]
    let path = user_path(None).await.unwrap_or_default();
    #[cfg(not(windows))]
    let path = {
        let mut cmd = command(env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into()));
        match tokio::time::timeout(
            Duration::from_secs(3),
            cmd.args(["-ilc", "printf '__HL__%s__HL__' \"$PATH\""])
                .output(),
        )
        .await
        {
            Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout)
                .split("__HL__")
                .nth(1)
                .unwrap_or("")
                .to_string(),
            _ => env::var("PATH").unwrap_or_default(),
        }
    };
    let on_path = env::split_paths(&path).any(|dir| Some(dir.as_path()) == link.parent());
    let linked = if cfg!(windows) {
        on_path
    } else {
        target.as_ref() == Some(&bin)
    };
    let service = cli(&app, &["daemon", "service"]).await;
    let service = &service["result"];
    let expected = if cfg!(windows) {
        bin.with_file_name("hostlane-background.exe")
    } else {
        bin.clone()
    };
    let mut vars = serde_json::Map::new();
    for name in [
        "HOSTLANE_DATA_DIR",
        "HOSTLANE_SOCKET",
        "HOSTLANE_SSH_CONFIG",
    ] {
        if let Ok(value) = env::var(name) {
            vars.insert(name.to_owned(), json!(value));
        }
    }
    Ok(
        json!({"binary":bin,"binaryExists":bin.is_file(),"serviceOutdated":service["installed"]==true&&service["binary"].as_str().is_some_and(|value|Path::new(value)!=expected),"command":if linked{&link}else{&bin},"env":vars,"cli":{"supported":true,"linkPath":link,"linkTarget":target,"linked":linked,"dirOnPath":on_path},"service":service}),
    )
}
#[tauri::command]
pub async fn link_cli(app: AppHandle) -> Value {
    let run = async {
        let bin = binary(&app)?;
        if !bin.is_file() {
            return Err("找不到 CLI".to_string());
        }
        #[cfg(windows)]
        user_path(bin.parent()).await?;
        #[cfg(unix)]
        {
            let link = home()?.join(".local/bin/hostlane");
            let perform = || {
                std::fs::create_dir_all(link.parent().unwrap())?;
                if let Ok(meta) = link.symlink_metadata() {
                    if !meta.is_symlink() {
                        return Err(std::io::Error::other("已有同名文件，不会覆盖"));
                    }
                    std::fs::remove_file(&link)?;
                }
                std::os::unix::fs::symlink(&bin, &link)
            };
            perform().map_err(|err| err.to_string())?;
        }
        setup_status(app).await
    };
    match run.await {
        Ok(value) => success(value),
        Err(err) => fail("IO", err),
    }
}
