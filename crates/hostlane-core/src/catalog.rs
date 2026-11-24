//! All hosts Hostlane knows: `~/.ssh/config` first, then hosts added in Hostlane
//! (`manual_hosts`), with labels set in Hostlane (`host_labels`) applied over both.

use std::path::PathBuf;

use hostlane_protocol::{ErrorKind, RpcError};

use crate::ssh_config::{self, Entry, Loaded};
use crate::store::Store;

#[derive(Clone)]
pub struct Catalog {
    ssh_config_path: PathBuf,
    store: Store,
}

impl Catalog {
    pub fn new(ssh_config_path: PathBuf, store: Store) -> Self {
        Self {
            ssh_config_path,
            store,
        }
    }

    pub async fn load(&self) -> Result<Loaded, RpcError> {
        let path = self.ssh_config_path.clone();
        let mut loaded = tokio::task::spawn_blocking(move || ssh_config::load(&path))
            .await
            .map_err(|err| RpcError::new(ErrorKind::Internal, err.to_string()))?;
        match self.store.manual_hosts().await {
            Ok(hosts) => {
                for host in &hosts {
                    loaded.add_manual(host);
                }
            }
            Err(err) => tracing::warn!(%err, "cannot load hosts added in Hostlane"),
        }
        match self.store.host_labels().await {
            Ok(labels) => {
                for entry in &mut loaded.entries {
                    if let Some((env, tags)) = labels.get(&entry.host.id) {
                        entry.host.env = *env;
                        entry.host.tags = tags.clone();
                    }
                }
            }
            Err(err) => tracing::warn!(%err, "cannot load host labels"),
        }
        Ok(loaded)
    }

    /// Finds a host by id, returning it with the catalog it came from.
    pub async fn resolve(&self, id: &str) -> Result<(Entry, Loaded), RpcError> {
        let loaded = self.load().await?;
        match loaded.find(id) {
            Some(entry) => Ok((entry.clone(), loaded)),
            None => Err(not_found(id, &loaded)),
        }
    }
}

pub fn not_found(id: &str, loaded: &Loaded) -> RpcError {
    let known: Vec<String> = loaded
        .entries
        .iter()
        .take(20)
        .map(|e| e.host.id.clone())
        .collect();
    let hint = if known.is_empty() {
        "no hosts are configured; add a Host block to ~/.ssh/config or run `hostlane host add`"
            .to_string()
    } else {
        format!("known hosts: {}", known.join(", "))
    };
    RpcError::new(ErrorKind::HostNotFound, format!("unknown host `{id}`")).with_hint(hint)
}
