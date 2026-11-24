//! Where Hostlane keeps its socket and data. The desktop app mirrors these rules
//! in `apps/desktop/src/main/paths.ts`; change both together.

use std::path::PathBuf;

pub const SOCKET_ENV: &str = "HOSTLANE_SOCKET";
pub const DATA_DIR_ENV: &str = "HOSTLANE_DATA_DIR";
pub const SSH_CONFIG_ENV: &str = "HOSTLANE_SSH_CONFIG";
const SOCKET_NAME: &str = "hostlane.sock";

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Logs, database and backups.
///
/// - macOS: `~/Library/Application Support/Hostlane`
/// - Linux: `$XDG_STATE_HOME/hostlane` or `~/.local/state/hostlane`
/// - Windows: `%APPDATA%\Hostlane`
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV) {
        return PathBuf::from(dir);
    }
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Hostlane")
    } else if cfg!(windows) {
        dirs::config_dir().unwrap_or_else(home).join("Hostlane")
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/state"))
            .join("hostlane")
    }
}

/// Local socket the daemon listens on.
///
/// - macOS: inside [`data_dir`]
/// - Linux: `$XDG_RUNTIME_DIR/hostlane/hostlane.sock`, falling back to [`data_dir`]
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os(SOCKET_ENV) {
        return PathBuf::from(path);
    }
    if cfg!(target_os = "linux")
        && let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
    {
        return PathBuf::from(runtime).join("hostlane").join(SOCKET_NAME);
    }
    data_dir().join(SOCKET_NAME)
}

pub fn log_path() -> PathBuf {
    data_dir().join("daemon.log")
}

pub fn ssh_config_path() -> PathBuf {
    std::env::var_os(SSH_CONFIG_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".ssh/config"))
}
