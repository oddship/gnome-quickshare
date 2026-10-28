use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::mdns_daemon::OwnedDaemon;
use mdns_sd::{AddrType, ServiceInfo};
use tokio::sync::broadcast::Receiver;
use tokio::sync::watch;
use tokio::time::{Instant, interval_at};
use tokio_util::sync::CancellationToken;

use crate::DEVICE_NAME;
use crate::utils::{DeviceType, gen_mdns_endpoint_info, gen_mdns_name};

const INNER_NAME: &str = "MDnsServer";
const TICK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Visibility {
    Visible = 0,
    Invisible = 1,
    Temporarily = 2,
}

#[allow(dead_code)]
impl Visibility {
    pub fn from_raw_value(value: u64) -> Self {
        match value {
            0 => Visibility::Visible,
            1 => Visibility::Invisible,
            2 => Visibility::Temporarily,
            _ => unreachable!(),
        }
    }
}

pub struct MDnsServer {
    daemon: OwnedDaemon,
    service_info: ServiceInfo,
    registered: bool,
    ble_receiver: Receiver<()>,
    visibility_sender: Arc<Mutex<watch::Sender<Visibility>>>,
    visibility_receiver: watch::Receiver<Visibility>,
}

impl MDnsServer {
    pub fn new(
        endpoint_id: [u8; 4],
        service_port: u16,
        ble_receiver: Receiver<()>,
        visibility_sender: Arc<Mutex<watch::Sender<Visibility>>>,
        visibility_receiver: watch::Receiver<Visibility>,
    ) -> Result<Self, anyhow::Error> {
        let service_info = Self::build_service(endpoint_id, service_port, DeviceType::Laptop)?;

        Ok(Self {
            daemon: OwnedDaemon::new()?,
            service_info,
            registered: false,
            ble_receiver,
            visibility_sender,
            visibility_receiver,
        })
    }

    pub async fn run(&mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        info!("{INNER_NAME}: service starting");
        let monitor = self.daemon.monitor()?;
        let mut visibility = *self.visibility_receiver.borrow();
        let mut interval = interval_at(Instant::now() + TICK_INTERVAL, TICK_INTERVAL);

        let result: Result<(), anyhow::Error> = async {
            loop {
                tokio::select! {
                    _ = ctk.cancelled() => {
                        info!("{INNER_NAME}: tracker cancelled, breaking");
                        break;
                    }
                    r = monitor.recv_async() => {
                        match r {
                            Ok(_) => continue,
                            Err(err) => return Err(err.into()),
                        }
                    },
                    _ = self.visibility_receiver.changed() => {
                        visibility = *self.visibility_receiver.borrow_and_update();

                        debug!("{INNER_NAME}: visibility changed: {visibility:?}");
                        if visibility == Visibility::Visible {
                            self.daemon.register(self.service_info.clone())?;
                            self.registered = true;
                        } else if visibility == Visibility::Invisible {
                            self.unregister().await?;
                        } else if visibility == Visibility::Temporarily {
                            self.daemon.register(self.service_info.clone())?;
                            self.registered = true;
                            interval.reset();
                        }
                    }
                    _ = self.ble_receiver.recv() => {
                        if visibility == Visibility::Invisible {
                            continue;
                        }

                        debug!("{INNER_NAME}: ble_receiver: got event");
                        if visibility == Visibility::Visible || visibility == Visibility::Temporarily {
                            // Android can sometime not see the mDNS service if the service
                            // was running BEFORE Android started the Discovery phase for QuickShare.
                            // So resend a broadcast if there's a android device sending.
                            self.daemon.register_resend(self.service_info.get_fullname())?;
                        } else {
                            self.daemon.register(self.service_info.clone())?;
                            self.registered = true;
                        }
                    },
                    _ = interval.tick() => {
                        if visibility != Visibility::Temporarily {
                            continue;
                        }

                        self.unregister().await?;
                        let _ = self.visibility_sender.lock().unwrap().send(Visibility::Invisible);
                    }
                }
            }

            Ok(())
        }.await;

        // Keep the monitor alive through the daemon's acknowledgement.
        let unregistered = self.unregister().await;
        let shutdown = self.daemon.shutdown().await;
        result.and(unregistered).and(shutdown)
    }

    async fn unregister(&mut self) -> Result<(), anyhow::Error> {
        if !self.registered {
            return Ok(());
        }
        let receiver = self.daemon.unregister(self.service_info.get_fullname())?;
        let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv_async()).await??;
        self.registered = false;
        debug!("MDnsServer: service unregistered: {event:?}");
        Ok(())
    }

    fn build_service(
        endpoint_id: [u8; 4],
        service_port: u16,
        device_type: DeviceType,
    ) -> Result<ServiceInfo, anyhow::Error> {
        // This `name` is going to be random every time RQS service restarts.
        // If that is not desired, derive host_name, etc. via some other means
        let name = gen_mdns_name(endpoint_id);
        let device_name = DEVICE_NAME.read().unwrap().clone();
        info!("Broadcasting with: device_name={device_name}, host_name={name}");
        let endpoint_info = gen_mdns_endpoint_info(device_type as u8, &device_name);

        let properties = [("n", endpoint_info)];
        let si = ServiceInfo::new(
            "_FC9F5ED42C8A._tcp.local.",
            &name,
            &name, // Needs to be ASCII?
            "",
            service_port,
            &properties[..],
        )?
        .enable_addr_auto(AddrType::V4);

        Ok(si)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdns_sd::{DaemonStatus, ServiceDaemon};
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn stopping_advertising_stops_its_daemon_across_restarts() {
        for _ in 0..3 {
            let (_ble, receiver) = broadcast::channel(8);
            let (visibility, changes) = watch::channel(Visibility::Invisible);
            let mut server = MDnsServer::new(
                *b"test",
                54321,
                receiver,
                Arc::new(Mutex::new(visibility)),
                changes,
            )
            .unwrap();
            let observer = ServiceDaemon::clone(&server.daemon);
            let ctk = CancellationToken::new();
            ctk.cancel();
            tokio::time::timeout(Duration::from_secs(4), server.run(ctk))
                .await
                .unwrap()
                .unwrap();
            let status = tokio::time::timeout(
                Duration::from_secs(1),
                observer.status().unwrap().recv_async(),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(status, DaemonStatus::Shutdown);
        }
    }
}
