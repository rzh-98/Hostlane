#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use clap::Parser;
    use hostlane_core::{Daemon, DaemonOptions, server};
    use std::{fs::OpenOptions, path::PathBuf, sync::Arc};

    #[derive(Parser)]
    struct Options {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long)]
        data_dir: PathBuf,
        #[arg(long)]
        ssh_config: PathBuf,
    }

    let options = Options::parse();
    std::fs::create_dir_all(&options.data_dir)?;
    let log = OpenOptions::new()
        .append(true)
        .create(true)
        .open(options.data_dir.join("daemon.log"))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,russh=warn".into()),
        )
        .with_ansi(false)
        .with_writer(log)
        .init();
    let daemon = Daemon::open(DaemonOptions {
        socket_path: options.socket,
        data_dir: options.data_dir,
        ssh_config_path: options.ssh_config,
    })?;
    server::serve(Arc::new(daemon)).await?;
    Ok(())
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("The login task host is only used on Windows.");
    std::process::ExitCode::FAILURE
}
