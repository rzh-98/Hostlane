use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hostlane_protocol::{ErrorKind, HostReachability, ReachabilityStatus, RpcError};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinSet;

use crate::{Daemon, util::now_ms};

const SETTING: &str = "ssh.reachability.hosts";
const PERIOD_MS: u64 = 300_000;
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Reachability {
    hosts: Mutex<BTreeMap<String, Watch>>,
    wake: Notify,
}

#[derive(Default)]
struct Watch {
    generation: Arc<()>,
    last_attempt: u64,
    result: Option<(ReachabilityStatus, u64)>,
}

impl Reachability {
    pub async fn load(&self, daemon: &Daemon) -> Result<(), RpcError> {
        if let Some(raw) = daemon.store.setting(SETTING).await? {
            let ids: Vec<String> = serde_json::from_str(&raw).map_err(|_| {
                RpcError::new(
                    ErrorKind::Internal,
                    "invalid saved SSH reachability monitors",
                )
            })?;
            self.hosts
                .lock()
                .await
                .extend(ids.into_iter().map(|id| (id, Watch::default())));
        }
        Ok(())
    }

    pub async fn list(&self) -> Vec<HostReachability> {
        self.hosts
            .lock()
            .await
            .iter()
            .map(|(host, watch)| {
                let (status, checked_at) = watch.result.unwrap_or((ReachabilityStatus::Pending, 0));
                HostReachability {
                    host: host.clone(),
                    status,
                    checked_at,
                }
            })
            .collect()
    }

    pub async fn set(&self, daemon: &Daemon, host: String, enabled: bool) -> Result<(), RpcError> {
        let mut hosts = self.hosts.lock().await;
        if hosts.contains_key(&host) == enabled {
            return Ok(());
        }
        let mut ids: Vec<_> = hosts.keys().filter(|id| **id != host).cloned().collect();
        if enabled {
            daemon.resolve_host(&host).await?;
            ids.push(host.clone());
        }
        daemon
            .store
            .set_setting(SETTING.into(), Some(serde_json::json!(ids).to_string()))
            .await?;
        if enabled {
            hosts.insert(host, Watch::default());
        } else {
            hosts.remove(&host);
        }
        self.wake.notify_one();
        Ok(())
    }

    pub async fn invalidate(&self, host: &str) {
        if let Some(watch) = self.hosts.lock().await.get_mut(host) {
            *watch = Watch::default();
            self.wake.notify_one();
        }
    }
}

pub async fn monitor(daemon: Arc<Daemon>) {
    monitor_with_period(daemon, PERIOD_MS).await
}

async fn monitor_with_period(daemon: Arc<Daemon>, period_ms: u64) {
    loop {
        let now = now_ms();
        let due: Vec<_> = daemon
            .reachability
            .hosts
            .lock()
            .await
            .iter()
            .filter(|(_, watch)| {
                now < watch.last_attempt || now.saturating_sub(watch.last_attempt) >= period_ms
            })
            .map(|(host, watch)| (host.clone(), watch.generation.clone()))
            .collect();
        if !due.is_empty() {
            let catalog = daemon.load_hosts().await.ok().map(Arc::new);
            let mut tasks = JoinSet::new();
            for (host, generation) in due {
                if tasks.len() >= 8 {
                    tasks.join_next().await;
                }
                let daemon = daemon.clone();
                let catalog = catalog.clone();
                tasks.spawn(async move {
                    let monitors = &daemon.reachability.hosts;
                    if !monitors
                        .lock()
                        .await
                        .get(&host)
                        .is_some_and(|watch| Arc::ptr_eq(&generation, &watch.generation))
                    {
                        return;
                    }
                    let sample = match catalog
                        .as_ref()
                        .and_then(|loaded| Some((loaded.find(&host)?, loaded)))
                    {
                        Some((entry, loaded)) => tokio::time::timeout(
                            TIMEOUT,
                            daemon.pool.sample_reachability(entry, loaded),
                        )
                        .await
                        .unwrap_or_else(|_| (ReachabilityStatus::TimedOut, now_ms())),
                        None => (ReachabilityStatus::HostUnavailable, now_ms()),
                    };
                    if let Some(watch) = monitors.lock().await.get_mut(&host)
                        && Arc::ptr_eq(&generation, &watch.generation)
                    {
                        watch.result = Some(sample);
                        watch.last_attempt = now_ms();
                    }
                });
            }
            while tasks.join_next().await.is_some() {}
        }
        let now = now_ms();
        let delay = daemon
            .reachability
            .hosts
            .lock()
            .await
            .values()
            .map(|watch| {
                if now < watch.last_attempt {
                    0
                } else {
                    watch
                        .last_attempt
                        .saturating_add(period_ms)
                        .saturating_sub(now)
                }
            })
            .min();
        match delay {
            Some(ms) => tokio::select! {
                _ = daemon.reachability.wake.notified() => {},
                _ = tokio::time::sleep(Duration::from_millis(ms.min(30_000))) => {},
            },
            None => daemon.reachability.wake.notified().await,
        }
    }
}

pub async fn read_banner(stream: impl AsyncRead + Unpin) -> ReachabilityStatus {
    let mut reader = BufReader::new(stream.take(8192));
    let mut line = Vec::new();
    while let Ok(count) = reader.read_until(b'\n', &mut line).await {
        if count == 0 || !line.ends_with(b"\n") {
            break;
        }
        if line.starts_with(b"SSH-") {
            let text = line.strip_suffix(b"\n").unwrap_or(&line);
            let text = text.strip_suffix(b"\r").unwrap_or(text);
            let version = text
                .strip_prefix(b"SSH-2.0-")
                .or_else(|| text.strip_prefix(b"SSH-1.99-"));
            if count <= 255
                && version.is_some_and(|rest| {
                    !rest.is_empty()
                        && rest[0].is_ascii_graphic()
                        && rest
                            .iter()
                            .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
                })
            {
                return ReachabilityStatus::SshBanner;
            }
            break;
        }
        line.clear();
    }
    ReachabilityStatus::NoSshBanner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resamples_without_logging_in_and_stops_when_disabled() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = std::env::temp_dir().join(crate::util::new_id("reachability"));
        let config = root.join("config");
        let daemon = Arc::new(
            Daemon::open(crate::DaemonOptions {
                socket_path: root.join("socket"),
                data_dir: root.clone(),
                ssh_config_path: config.clone(),
            })
            .unwrap(),
        );
        std::fs::write(
            config,
            format!(
                "Host demo\n  HostName 127.0.0.1\n  Port {}\n",
                listener.local_addr().unwrap().port()
            ),
        )
        .unwrap();
        let worker = tokio::spawn(monitor_with_period(daemon.clone(), 80));
        daemon
            .reachability
            .set(&daemon, "demo".into(), true)
            .await
            .unwrap();
        for banner in [b"SSH-2.0-fixture\r\n".as_slice(), b"HTTP/1.0 200 OK\r\n"] {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            stream.write_all(banner).await.unwrap();
            stream.shutdown().await.unwrap();
            let mut received = Vec::new();
            tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut received))
                .await
                .unwrap()
                .unwrap();
            assert!(received.is_empty());
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while daemon.reachability.list().await[0].status != ReachabilityStatus::NoSshBanner {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        daemon
            .reachability
            .set(&daemon, "demo".into(), false)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await
                .is_err()
        );
        worker.abort();
        let _ = worker.await;
        drop(daemon);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn accepts_only_a_complete_ssh_identification() {
        assert_eq!(
            read_banner(&b"notice\r\nSSH-2.0-OpenSSH_9.6\r\n"[..]).await,
            ReachabilityStatus::SshBanner
        );
        for bytes in [
            b"".as_slice(),
            b"HTTP/1.1 200 OK\r\n",
            b"SSH-error\n",
            b"SSH-2.0-x",
            b"SSH-2.0-\r\n",
        ] {
            assert_eq!(read_banner(bytes).await, ReachabilityStatus::NoSshBanner);
        }
        let mut overlong = vec![b'x'; 8192];
        overlong.extend_from_slice(b"\nSSH-2.0-valid\n");
        assert_eq!(
            read_banner(overlong.as_slice()).await,
            ReachabilityStatus::NoSshBanner
        );
    }
}
