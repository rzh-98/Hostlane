use crate::{Daemon, pool::Conn, ssh::Trust};
use hostlane_protocol::{ErrorKind, ReverseForward, RpcError};
use russh::client::{ChannelOpenHandle, Msg};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Semaphore, watch};

const LOOPBACK: &str = "127.0.0.1";
struct Target {
    port: u16,
    stop: watch::Sender<()>,
    capacity: Arc<Semaphore>,
}

#[derive(Clone, Default)]
pub struct Routes(Arc<Mutex<BTreeMap<u16, Target>>>);

impl Routes {
    pub fn incoming(
        &self,
        address: &str,
        port: u32,
        reply: ChannelOpenHandle,
        channel: russh::Channel<Msg>,
    ) {
        if address != LOOPBACK {
            return;
        }
        let Ok(port) = u16::try_from(port) else {
            return;
        };
        let guard = self.0.lock().unwrap();
        let Some(target) = guard.get(&port) else {
            return;
        };
        let Ok(permit) = target.capacity.clone().try_acquire_owned() else {
            return;
        };
        let mut stop = target.stop.subscribe();
        let port = target.port;
        tokio::spawn(async move {
            let _permit = permit;
            tokio::select! {
                _ = stop.changed() => {},
                _ = async {
                    if let Ok(Ok(mut local)) = tokio::time::timeout(Duration::from_secs(5),tokio::net::TcpStream::connect((LOOPBACK,port))).await {
                        reply.accept().await;
                        let _ = tokio::io::copy_bidirectional(&mut channel.into_stream(),&mut local).await;
                    }
                } => {}
            }
        });
    }
}

#[derive(Default)]
pub struct Reverse(tokio::sync::Mutex<Vec<(ReverseForward, Arc<Conn>)>>);

impl Reverse {
    pub async fn list(&self) -> Vec<ReverseForward> {
        let mut entries = self.0.lock().await;
        entries.retain(|(_, conn)| !conn.handle.is_closed());
        entries.iter().map(|(item, _)| item.clone()).collect()
    }

    pub async fn open(
        &self,
        daemon: &Daemon,
        mut item: ReverseForward,
    ) -> Result<ReverseForward, RpcError> {
        if item.local_port == 0 {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                "local port must be nonzero",
            ));
        }
        let (entry, loaded) = daemon.resolve_host(&item.host).await?;
        let (conn, _, _) = daemon.pool.connect(&entry, &loaded, Trust::Strict).await?;
        let mut entries = self.0.lock().await;
        entries.retain(|(_, conn)| !conn.handle.is_closed());
        if entries.len() >= 32 {
            return Err(RpcError::new(
                ErrorKind::InvalidParams,
                "at most 32 reverse forwards",
            ));
        }
        let assigned = conn
            .handle
            .tcpip_forward(LOOPBACK, item.remote_port.into())
            .await
            .map_err(|_| {
                RpcError::new(
                    ErrorKind::ForwardingDenied,
                    "remote listener refused (port in use or forwarding disabled)",
                )
            })?;
        if item.remote_port == 0 {
            item.remote_port = u16::try_from(assigned)
                .ok()
                .filter(|port| *port > 0)
                .ok_or_else(|| {
                    RpcError::new(ErrorKind::ForwardingDenied, "invalid assigned port")
                })?;
        }
        conn.routes.0.lock().unwrap().insert(
            item.remote_port,
            Target {
                port: item.local_port,
                stop: watch::channel(()).0,
                capacity: Arc::new(Semaphore::new(32)),
            },
        );
        entries.push((item.clone(), conn));
        Ok(item)
    }

    pub async fn close(&self, host: &str, port: u16) -> Result<ReverseForward, RpcError> {
        let mut entries = self.0.lock().await;
        let index = entries
            .iter()
            .position(|(item, _)| item.host == host && item.remote_port == port)
            .ok_or_else(|| RpcError::new(ErrorKind::TunnelNotFound, "no such reverse forward"))?;
        let (_, conn) = &entries[index];
        conn.routes.0.lock().unwrap().remove(&port);
        if !conn.handle.is_closed() {
            conn.handle
                .cancel_tcpip_forward(LOOPBACK, port.into())
                .await
                .map_err(|_| {
                    RpcError::new(
                        ErrorKind::ForwardingDenied,
                        "could not remove remote listener; local access disabled",
                    )
                })?;
        }
        Ok(entries.swap_remove(index).0)
    }
}
