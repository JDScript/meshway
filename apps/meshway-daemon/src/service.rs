//! Service manager readiness: the systemd `sd_notify` datagram protocol.
//! Without `NOTIFY_SOCKET` (foreground, containers, other platforms) this is a
//! no-op; a failed notification is logged and never changes Node behavior.

pub(crate) fn notify(state: &str) {
    #[cfg(target_os = "linux")]
    if let Some(target) = std::env::var_os("NOTIFY_SOCKET")
        && let Err(error) = send(&target, state)
    {
        tracing::warn!(%error, "Service manager notification failed");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = state;
}

#[cfg(target_os = "linux")]
fn send(target: &std::ffi::OsStr, state: &str) -> std::io::Result<()> {
    use std::os::{linux::net::SocketAddrExt, unix::ffi::OsStrExt, unix::net};
    let bytes = target.as_bytes();
    let address = match bytes.split_first() {
        Some((b'@', name)) => net::SocketAddr::from_abstract_name(name)?,
        Some((b'/', _)) => net::SocketAddr::from_pathname(target)?,
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "NOTIFY_SOCKET is neither absolute nor abstract",
            ));
        }
    };
    let socket = net::UnixDatagram::unbound()?;
    let sent = socket.send_to_addr(state.as_bytes(), &address)?;
    if sent != state.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "short notification datagram",
        ));
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::send;
    use std::os::unix::net::UnixDatagram;

    #[test]
    fn delivers_state_to_path_and_abstract_sockets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notify");
        let receiver = UnixDatagram::bind(&path).unwrap();
        send(path.as_os_str(), "READY=1").unwrap();
        let mut buffer = [0; 64];
        let length = receiver.recv(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"READY=1");

        use std::os::linux::net::SocketAddrExt;
        let name = format!("meshway-notify-test-{}", std::process::id());
        let address = std::os::unix::net::SocketAddr::from_abstract_name(&name).unwrap();
        let receiver = UnixDatagram::bind_addr(&address).unwrap();
        send(format!("@{name}").as_ref(), "STOPPING=1").unwrap();
        let length = receiver.recv(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"STOPPING=1");

        assert!(send("relative".as_ref(), "READY=1").is_err());
    }
}
