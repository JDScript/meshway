use super::{Configuration, ipv6::disable_ipv6};
use std::{io, path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

pub(super) struct Device {
    pub(super) device: tun_rs::AsyncDevice,
    pub(super) buffer: tokio::sync::Mutex<Vec<u8>>,
    pub(super) index: u32,
    name: String,
}

fn index(name: &str) -> io::Result<u32> {
    std::fs::read_to_string(Path::new("/sys/class/net").join(name).join("ifindex"))?
        .trim()
        .parse()
        .map_err(|_| io::Error::other("unknown TUN interface identity"))
}

async fn verify_no_ipv6(name: &str) -> io::Result<()> {
    // Address inspection only; never a route mutation or ambient resolver.
    let binary = ["/usr/sbin/ip", "/sbin/ip", "/usr/bin/ip", "/bin/ip"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .ok_or_else(|| io::Error::other("TUN requires iproute2"))?;
    let bytes = tokio::time::timeout(Duration::from_secs(3), async {
        let mut child = Command::new(binary)
            .args(["-o", "-6", "address", "show", "dev", name])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing ip output"))?
            .take(65_537)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > 65_536 || !child.wait().await?.success() {
            return Err(io::Error::other("cannot inspect TUN IPv6 addresses"));
        }
        Ok::<_, io::Error>(bytes)
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TUN inspection timed out"))??;
    if !bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(io::Error::other("IPv4 TUN unexpectedly has IPv6 addresses"));
    }
    Ok(())
}

impl Device {
    pub(super) async fn open(configuration: &Configuration) -> io::Result<Self> {
        if Path::new("/sys/class/net")
            .join(&configuration.name)
            .try_exists()?
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "TUN name already exists; no adoption",
            ));
        }
        let device = tun_rs::DeviceBuilder::new()
            .enable(false)
            .mtu(configuration.mtu)
            .offload(false)
            .build_async()?;
        if device.name()? != configuration.name {
            device.set_name(&configuration.name)?;
        }
        disable_ipv6(Path::new("/proc/sys/net/ipv6"), &configuration.name)?;
        if let Some(address) = configuration.address {
            device.set_network_address(address, configuration.prefix_len, configuration.peer)?;
        }
        device.set_tx_queue_len(128)?;
        device.enabled(true)?;
        disable_ipv6(Path::new("/proc/sys/net/ipv6"), &configuration.name)?;
        verify_no_ipv6(&configuration.name).await?;
        let index = index(&configuration.name)?;
        let owned = Self {
            device,
            buffer: tokio::sync::Mutex::new(vec![0; 65_536]),
            index,
            name: configuration.name.clone(),
        };
        owned.verify()?;
        Ok(owned)
    }

    pub(super) fn verify(&self) -> io::Result<()> {
        if self.device.name()? != self.name
            || index(&self.name)? != self.index
            || !Path::new("/sys/class/net")
                .join(&self.name)
                .join("tun_flags")
                .is_file()
        {
            return Err(io::Error::other("owned TUN identity changed"));
        }
        Ok(())
    }

    pub(super) fn drain(&self) -> io::Result<()> {
        let mut bytes = [0; 65_536];
        for _ in 0..256 {
            match self.device.try_recv(&mut bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
                Ok(0) => return Err(io::Error::other("TUN closed")),
                Ok(_) => {}
            }
        }
        Err(io::Error::other(
            "TUN queue did not quiesce before activation",
        ))
    }
}
