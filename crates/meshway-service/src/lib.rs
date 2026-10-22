//! Service manager readiness: the systemd `sd_notify` datagram protocol, the
//! one implementation the Node daemon and the Server share. Without
//! `NOTIFY_SOCKET` (foreground, containers, other platforms) this is a no-op;
//! a failed notification is logged and never changes the caller's behavior.
#![forbid(unsafe_code)]

use std::time::Duration;

/// Sends one newline-separated list of `KEY=VALUE` assignments, such as
/// `READY=1` or `STOPPING=1`.
pub fn notify(state: &str) {
    #[cfg(target_os = "linux")]
    if let Some(target) = std::env::var_os("NOTIFY_SOCKET")
        && let Err(error) = send(&target, state)
    {
        tracing::warn!(%error, "Service manager notification failed");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = state;
}

/// Startup is still progressing: the service manager's start timeout runs
/// again from now for `budget`, and `status` is shown as the unit's status.
pub fn extend_startup(budget: Duration, status: &str) {
    notify(&extension(budget, status));
}

fn extension(budget: Duration, status: &str) -> String {
    let status: String = status
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    format!(
        "EXTEND_TIMEOUT_USEC={}\nSTATUS={status}",
        budget.as_micros()
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extension_names_the_budget_and_a_one_line_status() {
        assert_eq!(
            extension(Duration::from_secs(60), "migrating\nschema 8"),
            "EXTEND_TIMEOUT_USEC=60000000\nSTATUS=migrating schema 8"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn delivers_state_to_path_and_abstract_sockets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notify");
        use std::os::unix::net::UnixDatagram;
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
