//! Registers the daemon as a per-user service, so it starts at login and is restarted
//! if it crashes: a launchd agent on macOS, a `systemd --user` unit on Linux.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hostlane_core::client::Client;
use hostlane_core::paths;
use hostlane_protocol::method;
use serde::Serialize;
use serde_json::Value;

pub const LABEL: &str = "com.hostlane.daemon";
const UNIT: &str = "hostlane.service";

/// Passed on to the service, so a test setup or custom location keeps working.
const ENV_VARS: &[&str] = &[
    paths::SOCKET_ENV,
    paths::DATA_DIR_ENV,
    paths::SSH_CONFIG_ENV,
    "HOSTLANE_NOTIFY",
];

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Manager {
    Launchd,
    Systemd,
}

impl Manager {
    fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::Launchd)
        } else if cfg!(target_os = "linux") {
            Some(Self::Systemd)
        } else {
            None
        }
    }

    fn file(self) -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        match self {
            Self::Launchd => home
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
            Self::Systemd => std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("systemd/user")
                .join(UNIT),
        }
    }
}

#[derive(Serialize)]
pub struct ServiceStatus {
    pub supported: bool,
    pub manager: Option<Manager>,
    pub installed: bool,
    pub path: Option<PathBuf>,
    /// The `hostlane` binary the installed service runs.
    pub binary: Option<String>,
}

/// What the service runs: this binary, `daemon run`, and the Hostlane environment.
pub struct Definition {
    pub binary: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub log: PathBuf,
}

impl Definition {
    pub fn current(socket: Option<&Path>) -> Result<Self> {
        let binary = std::env::current_exe()
            .context("cannot find the hostlane binary")?
            .canonicalize()
            .context("cannot resolve the hostlane binary")?;
        let mut args = vec!["daemon".to_string(), "run".to_string()];
        if let Some(socket) = socket {
            args.push("--socket".into());
            args.push(socket.display().to_string());
        }
        let env = ENV_VARS
            .iter()
            .filter_map(|name| {
                std::env::var(name)
                    .ok()
                    .filter(|value| !value.is_empty())
                    .map(|value| (name.to_string(), value))
            })
            .collect();
        Ok(Self {
            binary,
            args,
            env,
            log: paths::log_path(),
        })
    }
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn launchd_plist(def: &Definition) -> String {
    let mut args = format!(
        "        <string>{}</string>\n",
        xml(&def.binary.display().to_string())
    );
    for arg in &def.args {
        args.push_str(&format!("        <string>{}</string>\n", xml(arg)));
    }
    let mut env = String::new();
    if !def.env.is_empty() {
        env.push_str("    <key>EnvironmentVariables</key>\n    <dict>\n");
        for (name, value) in &def.env {
            env.push_str(&format!(
                "        <key>{}</key>\n        <string>{}</string>\n",
                xml(name),
                xml(value)
            ));
        }
        env.push_str("    </dict>\n");
    }
    let log = xml(&def.log.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
{args}    </array>
{env}    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Background</string>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#
    )
}

/// systemd splits `ExecStart=` on spaces; quote each word and escape `\`, `"` and `%`.
fn systemd_word(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{escaped}\"")
}

pub fn systemd_unit(def: &Definition) -> String {
    let mut exec = systemd_word(&def.binary.display().to_string());
    for arg in &def.args {
        exec.push(' ');
        exec.push_str(&systemd_word(arg));
    }
    let env: String = def
        .env
        .iter()
        .map(|(name, value)| format!("Environment={}\n", systemd_word(&format!("{name}={value}"))))
        .collect();
    format!(
        "[Unit]\n\
         Description=Hostlane daemon: SSH runtime for your own agents\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         {env}\
         Restart=on-failure\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// The binary an installed service file runs, read back from the file.
fn installed_binary(manager: Manager, content: &str) -> Option<String> {
    match manager {
        Manager::Launchd => {
            let after = content.split("<key>ProgramArguments</key>").nth(1)?;
            let start = after.find("<string>")? + "<string>".len();
            let end = after[start..].find("</string>")?;
            Some(
                after[start..start + end]
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&amp;", "&"),
            )
        }
        Manager::Systemd => {
            let line = content.lines().find_map(|l| l.strip_prefix("ExecStart="))?;
            let rest = line.strip_prefix('"')?;
            let mut binary = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => binary.extend(chars.next()),
                    '"' => break,
                    c => binary.push(c),
                }
            }
            Some(binary.replace("%%", "%"))
        }
    }
}

pub fn status() -> ServiceStatus {
    let Some(manager) = Manager::current() else {
        return ServiceStatus {
            supported: false,
            manager: None,
            installed: false,
            path: None,
            binary: None,
        };
    };
    let path = manager.file();
    let content = std::fs::read_to_string(&path).ok();
    ServiceStatus {
        supported: true,
        manager: Some(manager),
        installed: content.is_some(),
        binary: content.and_then(|c| installed_binary(manager, &c)),
        path: Some(path),
    }
}

fn run(program: &str, args: &[&str]) -> Result<std::process::Output> {
    Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("cannot run `{program}`"))
}

fn check(program: &str, args: &[&str]) -> Result<()> {
    let output = run(program, args)?;
    if !output.status.success() {
        bail!(
            "`{program} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn gui_domain() -> Result<String> {
    let output = run("id", &["-u"])?;
    Ok(format!(
        "gui/{}",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

async fn running(client: &Client) -> bool {
    client
        .call(method::DAEMON_STATUS, Value::Null)
        .await
        .is_ok()
}

/// Stops a daemon started outside the service, so the service can own the socket.
async fn stop_running(client: &Client) -> Result<bool> {
    if !running(client).await {
        return Ok(false);
    }
    client.call(method::DAEMON_SHUTDOWN, Value::Null).await?;
    for _ in 0..50 {
        if !running(client).await {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("the running daemon did not stop within 5s")
}

pub struct Installed {
    pub path: PathBuf,
    pub restarted: bool,
}

pub async fn install(client: &Client, def: &Definition) -> Result<Installed> {
    let Some(manager) = Manager::current() else {
        bail!("a background service is only supported on macOS (launchd) and Linux (systemd)")
    };
    let path = manager.file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    if let Some(dir) = def.log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let content = match manager {
        Manager::Launchd => launchd_plist(def),
        Manager::Systemd => systemd_unit(def),
    };
    if manager == Manager::Launchd {
        let domain = gui_domain()?;
        let _ = run("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
    }
    std::fs::write(&path, content).with_context(|| format!("cannot write {}", path.display()))?;
    let restarted = stop_running(client).await?;
    match manager {
        Manager::Launchd => {
            let domain = gui_domain()?;
            check(
                "launchctl",
                &["bootstrap", &domain, &path.display().to_string()],
            )?;
        }
        Manager::Systemd => {
            check("systemctl", &["--user", "daemon-reload"])?;
            check("systemctl", &["--user", "enable", UNIT])?;
            check("systemctl", &["--user", "restart", UNIT])?;
        }
    }
    for _ in 0..50 {
        if running(client).await {
            return Ok(Installed { path, restarted });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!(
        "the service is installed but the daemon did not come up within 5s; see {}",
        def.log.display()
    )
}

/// Starts the installed service; `false` if none is installed.
pub fn start() -> Result<bool> {
    let Some(manager) = Manager::current() else {
        return Ok(false);
    };
    if !manager.file().exists() {
        return Ok(false);
    }
    match manager {
        Manager::Launchd => {
            let target = format!("{}/{LABEL}", gui_domain()?);
            if check("launchctl", &["kickstart", &target]).is_err() {
                check(
                    "launchctl",
                    &[
                        "bootstrap",
                        &gui_domain()?,
                        &manager.file().display().to_string(),
                    ],
                )?;
            }
        }
        Manager::Systemd => check("systemctl", &["--user", "start", UNIT])?,
    }
    Ok(true)
}

/// Removes the service and stops the daemon it runs. `false` if none was installed.
pub fn uninstall() -> Result<bool> {
    let Some(manager) = Manager::current() else {
        return Ok(false);
    };
    let path = manager.file();
    if !path.exists() {
        return Ok(false);
    }
    match manager {
        Manager::Launchd => {
            let domain = gui_domain()?;
            let _ = run("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
        }
        Manager::Systemd => {
            let _ = run("systemctl", &["--user", "disable", "--now", UNIT]);
        }
    }
    std::fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
    if manager == Manager::Systemd {
        let _ = run("systemctl", &["--user", "daemon-reload"]);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> Definition {
        Definition {
            binary: PathBuf::from("/Applications/Hostlane & Co.app/bin/hostlane"),
            args: vec!["daemon".into(), "run".into()],
            env: vec![("HOSTLANE_DATA_DIR".into(), "/tmp/a b%".into())],
            log: PathBuf::from("/tmp/hl/daemon.log"),
        }
    }

    #[test]
    fn plist_escapes_and_round_trips_the_binary() {
        let plist = launchd_plist(&definition());
        assert!(
            plist.contains("<string>/Applications/Hostlane &amp; Co.app/bin/hostlane</string>")
        );
        assert!(plist.contains("<key>HOSTLANE_DATA_DIR</key>"));
        assert!(plist.contains("<string>com.hostlane.daemon</string>"));
        assert_eq!(
            installed_binary(Manager::Launchd, &plist).as_deref(),
            Some("/Applications/Hostlane & Co.app/bin/hostlane")
        );
    }

    #[test]
    fn unit_quotes_words_and_round_trips_the_binary() {
        let unit = systemd_unit(&definition());
        assert!(unit.contains(
            "ExecStart=\"/Applications/Hostlane & Co.app/bin/hostlane\" \"daemon\" \"run\"\n"
        ));
        assert!(unit.contains("Environment=\"HOSTLANE_DATA_DIR=/tmp/a b%%\"\n"));
        assert_eq!(
            installed_binary(Manager::Systemd, &unit).as_deref(),
            Some("/Applications/Hostlane & Co.app/bin/hostlane")
        );
    }
}
