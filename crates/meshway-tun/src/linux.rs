use std::os::unix::fs::FileTypeExt;

use crate::{Error, Options, Tun};

pub(super) fn create(options: Options) -> Result<Tun, Error> {
    tokio::runtime::Handle::try_current().map_err(|_| Error::NoRuntime)?;

    // tun-rs attempts mkdir/mknod when this node is absent. Device provisioning
    // belongs to the host, so fail before invoking its builder in that case.
    if !std::fs::metadata("/dev/net/tun")?
        .file_type()
        .is_char_device()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "/dev/net/tun must be an existing character device",
        )
        .into());
    }

    // Leaving ifr_name empty selects a fresh kernel-allocated tun%d, rather than
    // looking up a user-selected existing interface. Never call name()/persist()
    // on the builder or expose the underlying device to callers.
    let mut builder = tun_rs::DeviceBuilder::new()
        .layer(tun_rs::Layer::L3)
        .mtu(options.mtu)
        .offload(false)
        .multi_queue(false)
        .packet_information(false)
        .enable(true);
    if let Some((address, prefix)) = options.ipv4 {
        builder = builder.ipv4(address, prefix, None);
    }
    if let Some((address, prefix)) = options.ipv6 {
        builder = builder.ipv6(address, prefix);
    }
    let device = builder.build_async()?;
    let name = device.name()?;
    let mtu = usize::from(device.mtu()?);
    Ok(Tun { device, name, mtu })
}
