#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
use hostlane_core::{
    client::{Client, ClientError},
    paths,
};
use hostlane_protocol::ClientInfo;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::process::Command;

fn client(name: &str) -> Client {
    Client::new(paths::socket_path()).with_info(ClientInfo::new(name))
}
fn success(result: Value) -> Value {
    json!({"ok":true,"result":result})
}
fn fail(kind: &str, message: impl ToString) -> Value {
    json!({"ok":false,"error":{"kind":kind,"message":message.to_string()}})
}
async fn rpc(method: &str, params: Value) -> Value {
    match tokio::time::timeout(
        Duration::from_secs(60),
        client("desktop").call(method, params),
    )
    .await
    {
        Ok(Ok(value)) => success(value),
        Ok(Err(ClientError::Rpc(err))) => {
            json!({"ok":false,"error":{"kind":err.data.kind,"message":err.message,"details":err.data.details,"hint":err.data.hint}})
        }
        Ok(Err(ClientError::NotRunning(_))) => fail("DAEMON_NOT_RUNNING", "守护进程未运行"),
        Ok(Err(err)) => fail("IO", err),
        Err(_) => fail("TIMEOUT", "守护进程响应超时"),
    }
}
fn allowed(method: &str) -> bool {
    static METHODS: std::sync::LazyLock<std::collections::HashMap<String, bool>> =
        std::sync::LazyLock::new(|| {
            serde_json::from_str(include_str!("../../src/shared/renderer-methods.json"))
                .expect("valid renderer allowlist")
        });
    METHODS.get(method) == Some(&true)
}
#[tauri::command]
async fn hostlane_call(method: String, params: Option<Value>) -> Value {
    if !allowed(&method) {
        return fail("INVALID_PARAMS", "此操作不向页面开放");
    }
    rpc(&method, params.unwrap_or(Value::Null)).await
}
fn binary(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("HOSTLANE_BIN").map(PathBuf::from) {
        return if path.is_absolute() {
            Ok(path)
        } else {
            Err("HOSTLANE_BIN 必须为绝对路径".into())
        };
    }
    let name = if cfg!(windows) {
        "hostlane.exe"
    } else {
        "hostlane"
    };
    if cfg!(debug_assertions) {
        return Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../target/debug")
            .join(name));
    }
    Ok(app
        .path()
        .resource_dir()
        .map_err(|err| err.to_string())?
        .join(if cfg!(target_os = "macos") {
            "bin"
        } else {
            "resources/bin"
        })
        .join(name))
}
fn command(path: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(path);
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    cmd
}
async fn cli(app: &AppHandle, args: &[&str]) -> Value {
    let Ok(path) = binary(app) else {
        return fail("DAEMON_BINARY_MISSING", "找不到 Hostlane CLI");
    };
    match tokio::time::timeout(
        Duration::from_secs(30),
        command(path).arg("--json").args(args).output(),
    )
    .await
    {
        Ok(Ok(out)) => {
            let parsed: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
            if out.status.success() {
                success(parsed)
            } else if parsed.get("error").is_some() {
                json!({"ok":false,"error":parsed["error"]})
            } else {
                fail("IO", "Hostlane CLI 操作失败")
            }
        }
        Ok(Err(err)) => fail("IO", err),
        Err(_) => fail("TIMEOUT", "CLI 响应超时"),
    }
}
#[tauri::command]
async fn start_daemon(app: AppHandle) -> Value {
    cli(&app, &["daemon", "start"]).await
}
#[tauri::command]
async fn install_service(app: AppHandle) -> Value {
    cli(&app, &["daemon", "install"]).await
}
#[tauri::command]
async fn uninstall_service(app: AppHandle) -> Value {
    cli(&app, &["daemon", "uninstall"]).await
}
async fn events(app: AppHandle) {
    if rpc("daemon.status", Value::Null).await["error"]["kind"] == "DAEMON_NOT_RUNNING" {
        start_daemon(app.clone()).await;
    }
    let mut delay = 1;
    loop {
        if let Ok(mut sub) = client("desktop").subscribe().await {
            delay = 1;
            if let Some(window) = app.get_webview_window("main") {
                let status = rpc("policy.status", Value::Null).await;
                let _ = window.set_badge_count(
                    status["result"]["pending_approvals"]
                        .as_i64()
                        .filter(|count| *count > 0),
                );
            }
            let _ = app.emit_to("main", "hostlane-event", json!({"type":"subscribed"}));
            while let Ok(Some(event)) = sub.next().await {
                if let Some(window) = app.get_webview_window("main") {
                    match &event {
                        hostlane_protocol::Event::Policy { status } => {
                            let count = status.pending_approvals as i64;
                            let _ = window.set_badge_count((count > 0).then_some(count));
                        }
                        hostlane_protocol::Event::ApprovalRequested { approval }
                            if !window.is_focused().unwrap_or(false) =>
                        {
                            let _ = app
                                .notification()
                                .builder()
                                .title(format!("{} 请求审批", approval.client.name))
                                .body(&approval.summary)
                                .show();
                        }
                        _ => {}
                    }
                }
                let _ = app.emit_to("main", "hostlane-event", &event);
            }
        }
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(10);
    }
}
fn main() {
    let smoke = std::env::args().any(|arg| arg == "--check-renderer");
    let mut builder = tauri::Builder::default();
    if !smoke {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }));
    }
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            hostlane_call,
            start_daemon,
            install_service,
            uninstall_service,
            actions::choose_key,
            actions::reveal_trust_record,
            actions::open_external,
            actions::configure_mcp,
            actions::export_audit,
            actions::prompt_secret,
            actions::setup_status,
            actions::link_cli
        ])
        .setup(move |app| {
            let window =
                tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                    .incognito(smoke)
                    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                    .on_navigation(local_page)
                    .build()?;
            let handle = app.handle().clone();
            app.notification().on_action(move |_| {
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = window.emit("navigate", "approvals");
                }
            })?;
            tauri::async_runtime::spawn(events(app.handle().clone()));
            if smoke {
                std::thread::spawn(move || {
                    for _ in 0..45 {
                        std::thread::sleep(Duration::from_secs(1));
                        let handle = window.app_handle().clone();
                        let _ =
                            window.eval_with_callback(include_str!("smoke.js"), move |mounted| {
                                if mounted == "true" {
                                    handle.exit(0)
                                }
                            });
                    }
                    window.app_handle().exit(1);
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Hostlane desktop");
}
fn local_page(url: &tauri::Url) -> bool {
    matches!(
        (url.scheme(), url.host_str()),
        ("tauri", Some("localhost")) | ("http" | "https", Some("tauri.localhost"))
    ) || (cfg!(debug_assertions)
        && url.scheme() == "http"
        && url.host_str() == Some("localhost")
        && url.port() == Some(1420))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricts_renderer() {
        assert!(allowed("daemon.status"));
        assert!(allowed("policy.rules.delete"));
        for method in ["secrets.set", "daemon.shutdown", "exec.run", ""] {
            assert!(!allowed(method));
        }
    }
}
