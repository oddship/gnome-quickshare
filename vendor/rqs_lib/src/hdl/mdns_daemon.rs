use std::ops::Deref;
use std::time::Duration;

use mdns_sd::{DaemonStatus, ServiceDaemon};

/// mdns-sd's handle is cloneable, but dropping its last handle does not stop
/// the OS thread. Own its lifetime explicitly, including task abortion/errors.
pub(super) struct OwnedDaemon {
    daemon: ServiceDaemon,
    shutdown_sent: bool,
}

impl OwnedDaemon {
    pub fn new() -> Result<Self, anyhow::Error> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
            shutdown_sent: false,
        })
    }

    pub async fn shutdown(&mut self) -> Result<(), anyhow::Error> {
        let status = self.daemon.shutdown()?;
        self.shutdown_sent = true;
        let status = tokio::time::timeout(Duration::from_secs(2), status.recv_async()).await??;
        anyhow::ensure!(status == DaemonStatus::Shutdown, "mDNS daemon did not stop");
        Ok(())
    }
}

impl Deref for OwnedDaemon {
    type Target = ServiceDaemon;
    fn deref(&self) -> &Self::Target {
        &self.daemon
    }
}

impl Drop for OwnedDaemon {
    fn drop(&mut self) {
        if !self.shutdown_sent {
            // No await in Drop: normal exits use shutdown() and await its ack.
            // Aborted tasks must at least request exit instead of leaking the
            // thread, sockets and its repeated multicast searches indefinitely.
            let _ = self.daemon.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dropped_owner_stops_daemon_even_when_observer_survives() {
        let daemon = OwnedDaemon::new().unwrap();
        let observer = ServiceDaemon::clone(&daemon);
        drop(daemon);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(status) = observer.status()
                    && matches!(
                        tokio::time::timeout(Duration::from_millis(50), status.recv_async()).await,
                        Ok(Ok(DaemonStatus::Shutdown))
                    )
                {
                    break;
                }
                // A status request queued just behind Exit may never receive
                // a reply. Retry it after the command receiver has closed.
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
