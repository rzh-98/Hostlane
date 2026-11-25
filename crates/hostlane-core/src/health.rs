use crate::{Daemon, policy::system_notice, ports::probe, ssh::Trust, util::now_ms};
use hostlane_protocol::{ErrorKind, HealthCheck, HealthParams, RpcError};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

const SETTING: &str = "health.checks";

#[derive(Default)]
pub struct Health {
    checks: Mutex<Vec<HealthCheck>>,
}

impl Health {
    pub async fn list(&self) -> Vec<HealthCheck> {
        self.checks.lock().await.clone()
    }

    pub async fn load(&self, daemon: &Daemon) -> Result<(), RpcError> {
        if let Some(raw) = daemon.store.setting(SETTING).await? {
            *self.checks.lock().await = serde_json::from_str(&raw)
                .map_err(|_| RpcError::new(ErrorKind::Internal, "invalid saved health checks"))?;
        }
        Ok(())
    }

    pub async fn set(
        &self,
        daemon: &Daemon,
        params: HealthParams,
    ) -> Result<Vec<HealthCheck>, RpcError> {
        if params.port == 0
            || (params.interval_secs != 0 && !(10..=86400).contains(&params.interval_secs))
        {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                "choose a valid port and 10–86400 seconds, or 0 to remove",
            ));
        }
        if params.interval_secs != 0 {
            daemon.resolve_host(&params.host).await?;
        }
        let mut guard = self.checks.lock().await;
        let mut updated = guard.clone();
        updated.retain(|check| check.host != params.host || check.port != params.port);
        if params.interval_secs != 0 {
            if updated.len() >= 64 {
                return Err(RpcError::new(
                    ErrorKind::InvalidParams,
                    "at most 64 health checks",
                ));
            }
            updated.push(HealthCheck {
                host: params.host,
                port: params.port,
                interval_secs: params.interval_secs,
                healthy: None,
                detail: "pending".into(),
                checked_at: 0,
            });
        }
        let mut config = updated.clone();
        for check in &mut config {
            check.checked_at = 0;
            check.detail = "pending".into();
            check.healthy = None;
        }
        daemon
            .store
            .set_setting(
                SETTING.into(),
                Some(
                    serde_json::to_string(&config)
                        .map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))?,
                ),
            )
            .await?;
        *guard = updated.clone();
        Ok(updated)
    }
}

async fn sample(daemon: &Daemon, check: &HealthCheck) -> Result<(bool, String), RpcError> {
    let (entry, loaded) = daemon.resolve_host(&check.host).await?;
    let (conn, _, _) = daemon.pool.connect(&entry, &loaded, Trust::Strict).await?;
    let outcome = probe::probe(&conn.handle, "127.0.0.1", check.port, None, now_ms()).await;
    Ok(match outcome.http {
        Some(http) => (
            (200..400).contains(&http.status),
            format!("HTTP {}", http.status),
        ),
        None => (
            false,
            outcome.error.unwrap_or_else(|| "no HTTP response".into()),
        ),
    })
}

pub async fn monitor(daemon: Arc<Daemon>) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let now = now_ms();
        let checks = daemon.health.list().await;
        let mut tasks = tokio::task::JoinSet::new();
        for check in checks.into_iter().filter(|check| {
            now.saturating_sub(check.checked_at) >= u64::from(check.interval_secs) * 1000
        }) {
            let daemon = daemon.clone();
            tasks.spawn(async move {
                let (ok, detail) =
                    match tokio::time::timeout(Duration::from_secs(25), sample(&daemon, &check))
                        .await
                    {
                        Ok(Ok(result)) => result,
                        Ok(Err(err)) => (false, err.message),
                        Err(_) => (false, "timed out".into()),
                    };
                let mut checks = daemon.health.checks.lock().await;
                if let Some(current) = checks.iter_mut().find(|current| {
                    current.host == check.host
                        && current.port == check.port
                        && current.interval_secs == check.interval_secs
                        && current.checked_at == check.checked_at
                }) {
                    if !ok
                        && current.healthy != Some(false)
                        && std::env::var("HOSTLANE_NOTIFY").as_deref() != Ok("0")
                    {
                        system_notice(
                            "Hostlane: health check failed",
                            &format!("{}:{} — {}", check.host, check.port, detail),
                        );
                    }
                    current.checked_at = now_ms();
                    current.healthy = Some(ok);
                    current.detail = detail;
                }
            });
        }
        while tasks.join_next().await.is_some() {}
    }
}
