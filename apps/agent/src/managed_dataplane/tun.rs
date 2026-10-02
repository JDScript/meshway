//! A daemon-lifetime route sink with generation-scoped packet I/O. The interface
//! is never persistent or adopted. A lower-priority unreachable route survives
//! its removal (including process death), preventing mesh-to-underlay fallthrough.

use ipnet::Ipv4Net;
use meshway_dataplane::packet_io::{Device as PacketDevice, PacketBatch};
use std::{io, net::Ipv4Addr, sync::Arc};

use super::{PacketPlan, TunOptions};
use crate::AgentError;

pub(crate) enum Device {
    #[cfg(target_os = "linux")]
    Linux {
        device: tun_rs::AsyncDevice,
        buffer: tokio::sync::Mutex<Vec<u8>>,
    },
    #[cfg(test)]
    Mock(Arc<super::tests::MockDevice>),
}

impl PacketDevice for Device {
    async fn recv_batch(&self) -> io::Result<PacketBatch> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux { device, buffer } => {
                let mut bytes = buffer.lock().await;
                loop {
                    let length = device.recv(&mut bytes).await?;
                    if length == 0 {
                        return Err(io::Error::other("TUN closed"));
                    }
                    // Previously these malformed, over-MTU packets were discarded
                    // by admission. Preserve that disposition, not a device failure.
                    if length > meshway_dataplane::MAX_PACKET_PAYLOAD {
                        tokio::task::yield_now().await;
                        continue;
                    }
                    return PacketBatch::new(vec![bytes[..length].to_vec()]);
                }
            }
            #[cfg(test)]
            Self::Mock(device) => {
                let mut bytes = vec![0; 65_536];
                let length = device.recv(&mut bytes).await?;
                bytes.truncate(length);
                PacketBatch::new(vec![bytes])
            }
            #[cfg(not(any(target_os = "linux", test)))]
            _ => Err(io::Error::other("managed IPv4 TUN requires Linux")),
        }
    }

    async fn send_batch(&self, packets: PacketBatch) -> io::Result<()> {
        #[cfg(not(any(target_os = "linux", test)))]
        {
            let _ = packets;
            Err(io::Error::other("managed IPv4 TUN requires Linux"))
        }
        #[cfg(any(target_os = "linux", test))]
        {
            for bytes in packets.into_packets() {
                let length = match self {
                    #[cfg(target_os = "linux")]
                    Self::Linux { device, .. } => device.send(&bytes).await?,
                    #[cfg(test)]
                    Self::Mock(device) => device.send(&bytes).await?,
                    #[cfg(not(any(target_os = "linux", test)))]
                    _ => return Err(io::Error::other("managed IPv4 TUN requires Linux")),
                };
                if length != bytes.len() {
                    return Err(io::Error::other("short TUN write"));
                }
            }
            Ok(())
        }
    }

    fn drain(&self) -> io::Result<()> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux { device, .. } => linux::drain(device),
            #[cfg(test)]
            Self::Mock(device) => device.drain(),
            #[cfg(not(any(target_os = "linux", test)))]
            _ => Err(io::Error::other("managed IPv4 TUN requires Linux")),
        }
    }
}

pub(crate) struct Tun {
    pub device: Arc<Device>,
    pub name: String,
    local: Ipv4Addr,
    prefix: Ipv4Net,
    mtu: u16,
}

/// Some Linux kernels omit the per-interface IPv6 sysctl entirely when default
/// disable_ipv6=1 was inherited at link creation. Only that verified inheritance
/// (or absence of the entire IPv6 subsystem) permits an absent interface entry.
#[cfg(any(target_os = "linux", test))]
fn disable_ipv6(tree: &std::path::Path, name: &str) -> io::Result<()> {
    let interface = tree.join("conf").join(name).join("disable_ipv6");
    match std::fs::read_to_string(&interface) {
        Ok(value) if value.trim() == "1" => Ok(()),
        Ok(value) if value.trim() == "0" => {
            std::fs::write(&interface, b"1\n")?;
            if std::fs::read_to_string(&interface)?.trim() != "1" {
                return Err(io::Error::other("TUN IPv6 disable did not take effect"));
            }
            Ok(())
        }
        Ok(_) => Err(io::Error::other("invalid TUN IPv6 sysctl value")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if !tree.try_exists()? {
                return Ok(());
            }
            match std::fs::read_to_string(tree.join("conf/default/disable_ipv6")) {
                Ok(value) if value.trim() == "1" => Ok(()),
                Ok(_) => Err(io::Error::other(
                    "missing TUN IPv6 sysctl without verified disabled inheritance",
                )),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

impl Tun {
    /// Normal mainloop seam: controller/carrier protection is installed first.
    /// This path does no ambient DNS or main-table route/pin installation.
    #[cfg(all(target_os = "linux", feature = "managed-connections"))]
    #[allow(dead_code)] // Shared P3a mainloop chooses this instead of legacy open.
    pub(crate) async fn open_with_underlay(
        options: &TunOptions,
        plan: &PacketPlan,
        owner: &mut crate::underlay::LinuxUnderlay,
    ) -> Result<Self, AgentError> {
        options.validate()?;
        let policy = plan.route_policy();
        let prefix = Ipv4Net::new(policy.network, policy.prefix_len)
            .map_err(|_| AgentError::Invalid("invalid Mesh prefix"))?;
        // Install the durable reject-only sink BEFORE enabling any new device.
        owner.retain_mesh_sink(prefix.into()).await?;
        let device = linux::create_device(options, plan).await?;
        device.enabled(true)?;
        disable_ipv6(std::path::Path::new("/proc/sys/net/ipv6"), &options.name)?;
        linux::verify_no_ipv6(&options.name).await?;
        let binding = crate::underlay::TunBinding::from_created_device(&device)?;
        owner
            .install_tun(&[crate::underlay::CapturePrefix {
                prefix: prefix.into(),
                tun: Some(&binding),
            }])
            .await?;
        Ok(Self {
            device: Arc::new(Device::Linux {
                device,
                buffer: tokio::sync::Mutex::new(vec![0; 65_536]),
            }),
            name: options.name.clone(),
            local: plan.local,
            prefix: plan.prefix,
            mtu: plan.mtu,
        })
    }

    #[cfg(all(target_os = "linux", feature = "managed-connections"))]
    #[allow(dead_code)] // P3a mainloop owner installs this binding at the shared seam.
    pub(crate) fn underlay_binding(&self) -> io::Result<crate::underlay::TunBinding> {
        match self.device.as_ref() {
            Device::Linux { device, .. } => {
                crate::underlay::TunBinding::from_created_device(device)
            }
            #[cfg(test)]
            Device::Mock(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "mock TUN has no Linux ownership binding",
            )),
        }
    }

    pub fn matches(&self, plan: &PacketPlan) -> bool {
        let policy = plan.route_policy();
        self.local == policy.local
            && self.prefix.network() == policy.network
            && self.prefix.prefix_len() == policy.prefix_len
            && self.mtu == policy.mtu
    }

    #[cfg(any(not(target_os = "linux"), test))]
    pub async fn open(
        options: &TunOptions,
        plan: &PacketPlan,
        controller: &str,
        may_reuse_guard: bool,
    ) -> Result<Self, AgentError> {
        options.validate()?;
        #[cfg(target_os = "linux")]
        {
            linux::open(options, plan, controller, may_reuse_guard).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (plan, controller, may_reuse_guard);
            Err(AgentError::Invalid("managed IPv4 TUN requires Linux"))
        }
    }

    #[cfg(test)]
    pub fn mock(device: Arc<super::tests::MockDevice>, plan: &PacketPlan) -> Self {
        Self {
            device: Arc::new(Device::Mock(device)),
            name: "mock0".into(),
            local: plan.local,
            prefix: plan.prefix,
            mtu: plan.mtu,
        }
    }
}

/// Local proof only: adjacency reports retain their existing meaning. Ready
/// means a configured owned device and a live packet generation, not delivery.
fn proof_json(
    name: &str,
    plan: &PacketPlan,
    ready: bool,
    reason: &str,
) -> Result<String, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct Proof<'a> {
        event: &'static str,
        node_id: &'a str,
        revision: u32,
        digest: &'a str,
        observed_at: chrono::DateTime<chrono::Utc>,
        tun_name: &'a str,
        ipv4: String,
        ipv4_prefix: String,
        mtu: u16,
        ipv6: bool,
        tun_ready: bool,
        allowed_paths: usize,
        route_sink_retained: bool,
        reason: &'a str,
    }
    let proof = Proof {
        event: "managed_dataplane",
        node_id: &plan.node_id,
        revision: plan.revision,
        digest: &plan.digest,
        observed_at: chrono::Utc::now(),
        tun_name: name,
        ipv4: plan.local.to_string(),
        ipv4_prefix: plan.prefix.to_string(),
        mtu: plan.mtu,
        ipv6: false,
        tun_ready: ready,
        allowed_paths: if ready { plan.paths.len() } else { 0 },
        route_sink_retained: true,
        reason,
    };
    serde_json::to_string(&proof)
}

pub(crate) fn proof(
    name: &str,
    plan: &PacketPlan,
    ready: bool,
    reason: &str,
    events: Option<&tokio::sync::mpsc::Sender<crate::daemon::SessionEvent>>,
) {
    if let Some(events) = events
        && let Ok(json) = proof_json(name, plan, ready, reason)
    {
        // Terminal backpressure never retains packet authorization or a socket.
        let _ = events.try_send(crate::daemon::SessionEvent::Receipt(json));
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    #[cfg(test)]
    use std::net::IpAddr;
    use std::{path::Path, process::Stdio, time::Duration};
    use tokio::{io::AsyncReadExt, process::Command, time};

    #[cfg(test)]
    const GUARD_METRIC: &str = "42760";
    #[cfg(test)]
    const TUN_METRIC: &str = "42759";
    #[cfg(test)]
    const PROTOCOL: &str = "242";

    pub(super) fn drain(device: &tun_rs::AsyncDevice) -> io::Result<()> {
        let mut bytes = [0; 65_536];
        for _ in 0..256 {
            match device.try_recv(&mut bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
                Ok(0) => return Err(io::Error::other("TUN closed")),
                Ok(_) => {}
            }
        }
        Err(io::Error::other(
            "TUN queue did not quiesce before generation activation",
        ))
    }

    async fn ip(args: &[&str]) -> Result<Vec<u8>, AgentError> {
        let binary = ["/usr/sbin/ip", "/sbin/ip", "/usr/bin/ip", "/bin/ip"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
            .ok_or(AgentError::Invalid("managed TUN requires iproute2"))?;
        time::timeout(Duration::from_secs(3), async {
            let mut child = Command::new(binary).args(args)
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
                .kill_on_drop(true).spawn()?;
            let mut stdout = child.stdout.take().expect("piped stdout").take(65_537);
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).await?;
            if bytes.len() > 65_536 { return Err(AgentError::Invalid("iproute2 output exceeds bounds")); }
            if !child.wait().await?.success() {
                return Err(AgentError::Invalid("managed TUN route operation failed (existing route conflict or missing NET_ADMIN)"));
            }
            Ok(bytes)
        }).await.map_err(|_| AgentError::Invalid("managed TUN route operation timed out"))?
    }

    #[cfg(test)]
    async fn underlay(controller: &str, prefix: Ipv4Net) -> Result<(), AgentError> {
        let url = reqwest::Url::parse(controller)
            .map_err(|_| AgentError::Invalid("invalid controller URL"))?;
        let host = url
            .host_str()
            .ok_or(AgentError::Invalid("missing controller hostname"))?;
        let host = host
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(host);
        let resolved = time::timeout(
            Duration::from_secs(3),
            tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(443))),
        )
        .await
        .map_err(|_| AgentError::DataplaneResolveFailed)?
        .map_err(|_| AgentError::DataplaneResolveFailed)?;
        let addresses: Vec<_> = resolved.take(33).collect();
        if addresses.is_empty() {
            return Err(AgentError::DataplaneResolveFailed);
        }
        if addresses.len() > 32
            || addresses
                .iter()
                .any(|address| super::super::in_prefix(address.ip(), prefix))
        {
            return Err(AgentError::Invalid(
                "controller endpoint overlaps mesh IPv4 route",
            ));
        }
        // Reject DNS recursion before adding either route. Resolver metadata is
        // local OS configuration, not an HTTP contract. Bound reads as well.
        let file = std::fs::File::open("/etc/resolv.conf")?;
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::Read::take(file, 65_537), &mut text)?;
        if text.len() > 65_536 {
            return Err(AgentError::Invalid("resolver configuration exceeds bounds"));
        }
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            if fields.next() == Some("nameserver") {
                let address: IpAddr = fields
                    .next()
                    .ok_or(AgentError::Invalid("invalid resolver address"))?
                    .parse()
                    .map_err(|_| AgentError::Invalid("unsupported resolver address"))?;
                if super::super::in_prefix(address, prefix) {
                    return Err(AgentError::Invalid("DNS server overlaps mesh IPv4 route"));
                }
            }
        }
        Ok(())
    }

    #[derive(serde::Deserialize)]
    #[cfg(test)]
    struct Route {
        dst: Option<String>,
        #[serde(rename = "type")]
        kind: Option<String>,
        protocol: Option<String>,
        metric: Option<u32>,
        dev: Option<String>,
        gateway: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct InterfaceAddresses {
        #[serde(default)]
        addr_info: Vec<InterfaceAddress>,
    }

    #[derive(serde::Deserialize)]
    struct InterfaceAddress {
        family: String,
    }

    pub(super) async fn verify_no_ipv6(name: &str) -> Result<(), AgentError> {
        let addresses: Vec<InterfaceAddresses> =
            serde_json::from_slice(&ip(&["-j", "-6", "address", "show", "dev", name]).await?)
                .map_err(|_| AgentError::Invalid("cannot verify TUN IPv6 address absence"))?;
        if addresses
            .iter()
            .flat_map(|interface| &interface.addr_info)
            .any(|address| address.family == "inet6")
        {
            return Err(AgentError::Invalid(
                "managed IPv4 TUN unexpectedly has an IPv6 address",
            ));
        }
        Ok(())
    }

    pub(super) async fn create_device(
        options: &TunOptions,
        plan: &PacketPlan,
    ) -> Result<tun_rs::AsyncDevice, AgentError> {
        if Path::new("/sys/class/net")
            .join(&options.name)
            .try_exists()?
        {
            return Err(AgentError::Invalid(
                "requested TUN name already exists; refusing to adopt",
            ));
        }
        let device = tun_rs::DeviceBuilder::new()
            .enable(false)
            .mtu(options.mtu)
            .offload(false)
            .build_async()?;
        if device.name()? != options.name {
            device.set_name(&options.name)?;
        }
        disable_ipv6(Path::new("/proc/sys/net/ipv6"), &options.name)?;
        device.set_network_address(plan.local, 32, None)?;
        device.set_tx_queue_len(128)?;
        Ok(device)
    }

    #[cfg(test)]
    pub(super) async fn open(
        options: &TunOptions,
        plan: &PacketPlan,
        controller: &str,
        may_reuse_guard: bool,
    ) -> Result<Tun, AgentError> {
        let policy = plan.route_policy();
        debug_assert!(policy.retain_fail_closed_sink);
        let route_prefix = Ipv4Net::new(policy.network, policy.prefix_len)
            .map_err(|_| AgentError::Invalid("invalid mesh IPv4 pool"))?;
        underlay(controller, route_prefix).await?;
        // Preflight rejects existing names. Allocation below is unnamed, never
        // attaches to a named persistent TUN; rename is atomic against races.
        if Path::new("/sys/class/net")
            .join(&options.name)
            .try_exists()?
        {
            return Err(AgentError::Invalid(
                "requested TUN name already exists; refusing to adopt",
            ));
        }
        let prefix = route_prefix.to_string();
        let routes: Vec<Route> = serde_json::from_slice(
            &ip(&[
                "-j", "-4", "route", "show", "table", "main", "root", &prefix,
            ])
            .await?,
        )
        .map_err(|_| AgentError::Invalid("cannot validate existing mesh-prefix routes"))?;
        let existing_guard = routes.len() == 1
            && may_reuse_guard
            && routes.iter().all(|route| {
                route.dst.as_deref() == Some(prefix.as_str())
                    && route.kind.as_deref() == Some("unreachable")
                    && route.metric == Some(42760)
                    && route.protocol.as_deref() == Some(PROTOCOL)
                    && route.dev.is_none()
                    && route.gateway.is_none()
            });
        if !routes.is_empty() && !existing_guard {
            return Err(AgentError::Invalid(
                "existing route conflicts with managed mesh IPv4 pool",
            ));
        }
        // No supplied name: kernel allocation always creates a new interface.
        let device = create_device(options, plan).await?;
        // A 32-packet kernel queue drops TCP bursts before the packet task can
        // read them. Keep a bounded 128-packet cushion, below drain's 256-read
        // retirement budget so an old generation can still quiesce atomically.
        if !existing_guard {
            ip(&[
                "-4",
                "route",
                "add",
                "unreachable",
                &prefix,
                "metric",
                GUARD_METRIC,
                "proto",
                PROTOCOL,
            ])
            .await?;
        }
        // No replace, no default route, no forwarding/NAT. A failed add drops
        // only this owned nonpersistent device; the unreachable guard remains.
        device.enabled(true)?;
        // Check again after link-up, when an IPv6 context could be instantiated.
        disable_ipv6(Path::new("/proc/sys/net/ipv6"), &options.name)?;
        verify_no_ipv6(&options.name).await?;
        ip(&[
            "-4",
            "route",
            "add",
            &prefix,
            "dev",
            &options.name,
            "src",
            &plan.local.to_string(),
            "metric",
            TUN_METRIC,
            "proto",
            PROTOCOL,
        ])
        .await?;
        Ok(Tun {
            device: Arc::new(Device::Linux {
                device,
                buffer: tokio::sync::Mutex::new(vec![0; 65_536]),
            }),
            name: options.name.clone(),
            local: plan.local,
            prefix: plan.prefix,
            mtu: plan.mtu,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_digest_is_the_installed_generation_even_when_saved_revision_advances() {
        #[derive(serde::Deserialize)]
        struct Evidence {
            revision: u32,
            digest: String,
            tun_ready: bool,
        }
        let (payloads, _) = crate::managed_dataplane::tests::fixtures(2);
        let old = &payloads[0];
        let generation = PacketPlan::compile(old, 1000).unwrap();
        let ready: Evidence = serde_json::from_str(
            &proof_json("meshway0", &generation, true, "generation_active").unwrap(),
        )
        .unwrap();
        assert!(ready.tun_ready);
        assert_eq!(
            ready.digest,
            meshway_protocol::dataplane_digest(old).unwrap()
        );
        let mut newer = old.clone();
        newer.revision += 1;
        newer.configuration.revision += 1;
        newer.connections.revision += 1;
        let replacement = PacketPlan::compile(&newer, 1000).unwrap();
        assert_ne!(replacement.digest, generation.digest);
        let withdrawn: Evidence = serde_json::from_str(
            &proof_json("meshway0", &generation, false, "generation_withdrawn").unwrap(),
        )
        .unwrap();
        assert!(!withdrawn.tun_ready);
        assert_eq!(withdrawn.revision, ready.revision);
        assert_eq!(withdrawn.digest, ready.digest);
        let ready: Evidence = serde_json::from_str(
            &proof_json("meshway0", &replacement, true, "generation_active").unwrap(),
        )
        .unwrap();
        assert_eq!(ready.revision, newer.revision);
        assert_eq!(
            ready.digest,
            meshway_protocol::dataplane_digest(&newer).unwrap()
        );
    }

    #[test]
    fn missing_per_interface_ipv6_sysctl_requires_verified_disabled_inheritance() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("ipv6");
        assert!(disable_ipv6(&tree, "meshway0").is_ok());
        std::fs::create_dir_all(tree.join("conf/default")).unwrap();
        assert!(disable_ipv6(&tree, "meshway0").is_err());
        let default = tree.join("conf/default/disable_ipv6");
        std::fs::write(&default, b"0\n").unwrap();
        assert!(disable_ipv6(&tree, "meshway0").is_err());
        std::fs::write(&default, b"1\n").unwrap();
        assert!(disable_ipv6(&tree, "meshway0").is_ok());
        std::fs::create_dir_all(tree.join("conf/meshway0")).unwrap();
        let own = tree.join("conf/meshway0/disable_ipv6");
        std::fs::write(&own, b"0\n").unwrap();
        disable_ipv6(&tree, "meshway0").unwrap();
        assert_eq!(std::fs::read_to_string(&own).unwrap(), "1\n");
        std::fs::write(&own, b"unknown\n").unwrap();
        assert!(
            disable_ipv6(&tree, "meshway0").is_err(),
            "default=1 cannot mask invalid own settings"
        );
        std::fs::remove_file(&own).unwrap();
        std::fs::create_dir(&own).unwrap();
        assert!(
            disable_ipv6(&tree, "meshway0").is_err(),
            "default=1 cannot mask non-ENOENT failures"
        );
    }
}
