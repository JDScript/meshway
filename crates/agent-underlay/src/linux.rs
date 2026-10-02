//! Opaque Linux owners. Journal machinery and its test doubles stay private.
use crate::{
    BootstrapPreparation, CapturePrefix, Preparation, PreparedGeneration, SystemDnsSnapshot, owner,
};
use std::io;

pub struct LinuxUnderlay(owner::LinuxUnderlay);

impl LinuxUnderlay {
    pub async fn prepare(input: Preparation<'_>) -> io::Result<Self> {
        owner::LinuxUnderlay::prepare(input).await.map(Self)
    }
    pub async fn socket_generation(&self) -> io::Result<PreparedGeneration> {
        self.0.socket_generation().await
    }
    pub fn system_dns(&self) -> io::Result<SystemDnsSnapshot> {
        self.0.system_dns()
    }
    pub async fn capture(&mut self, prefixes: &[CapturePrefix<'_>]) -> io::Result<()> {
        self.0.capture(prefixes).await
    }
    pub async fn retain_mesh_sink(&mut self, prefix: ipnet::IpNet) -> io::Result<()> {
        self.0.retain_mesh_sink(prefix).await
    }
    pub async fn install_tun(&mut self, prefixes: &[CapturePrefix<'_>]) -> io::Result<()> {
        self.0.install_tun(prefixes).await
    }
    pub async fn activate_tun(&mut self, prefixes: &[CapturePrefix<'_>]) -> io::Result<()> {
        self.0.activate_tun(prefixes).await
    }
    pub async fn withdraw_tun(&mut self) -> io::Result<()> {
        self.0.withdraw_tun().await
    }
    pub async fn relinquish(self) -> io::Result<()> {
        self.0.relinquish().await
    }
}

pub struct BootstrapUnderlay(owner::BootstrapUnderlay);

impl BootstrapUnderlay {
    pub async fn prepare(input: BootstrapPreparation<'_>) -> io::Result<Self> {
        owner::BootstrapUnderlay::prepare(input).await.map(Self)
    }
    pub async fn controller_generation(&self) -> io::Result<PreparedGeneration> {
        self.0.controller_generation().await
    }
    pub async fn bind(self, input: Preparation<'_>) -> io::Result<LinuxUnderlay> {
        self.0.bind(input).await.map(LinuxUnderlay)
    }
}
