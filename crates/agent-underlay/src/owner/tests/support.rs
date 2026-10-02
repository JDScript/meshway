//! Test-only orchestration of public device/capture capabilities, no App import.
use super::*;
use crate::Purpose;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(super) struct Plan {
    pub prefix: ipnet::Ipv4Net,
    pub local: std::net::Ipv4Addr,
}
pub(super) struct TunOptions {
    pub name: String,
    pub mtu: u16,
}
pub(super) struct Tun {
    device: meshway_agent_tun::Device,
}
impl Tun {
    pub(super) async fn open_with_underlay(
        options: &TunOptions,
        plan: &Plan,
        owner: &mut LinuxUnderlay,
    ) -> io::Result<Self> {
        owner.retain_mesh_sink(plan.prefix.into()).await?;
        let device = meshway_agent_tun::Device::open(&meshway_agent_tun::Configuration {
            name: options.name.clone(),
            mtu: options.mtu,
            address: Some(plan.local),
            prefix_len: 32,
            peer: None,
        })
        .await?;
        let binding = TunBinding::from_created_device(&device)?;
        owner
            .install_tun(&[CapturePrefix {
                prefix: plan.prefix.into(),
                tun: Some(&binding),
            }])
            .await?;
        Ok(Self { device })
    }
    pub(super) fn underlay_binding(&self) -> io::Result<TunBinding> {
        TunBinding::from_created_device(&self.device)
    }
}

pub(super) struct ControllerTestChannel {
    owner: PreparedGeneration,
}
impl ControllerTestChannel {
    pub(super) async fn prepare(owner: &LinuxUnderlay) -> io::Result<Self> {
        let generation = owner.socket_generation().await?;
        Ok(Self { owner: generation })
    }
    pub(super) async fn request(&self) -> io::Result<()> {
        let mut socket = self
            .owner
            .factory()
            .tcp(Purpose::Controller, "172.30.247.20:41861".parse().unwrap())
            .await?;
        // A bounded test HTTP request checks reachability after recovery. Normal
        // generated operations and TLS remain covered by the App acceptance.
        socket.write_all(b"GET /api/v1/overview HTTP/1.1\r\nHost: underlay-owner-server\r\nAuthorization: Bearer fixture-only\r\nConnection: close\r\n\r\n").await?;
        let mut bytes = [0; 4096];
        let count = tokio::time::timeout(Duration::from_secs(3), socket.read(&mut bytes))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "fixture HTTP response timeout")
            })??;
        if !String::from_utf8_lossy(&bytes[..count]).starts_with("HTTP/1.1 401") {
            return Err(invalid("fixture protected HTTP did not return 401"));
        }
        Ok(())
    }
    pub(super) fn retire(&self) {
        self.owner.retire();
    }
}
