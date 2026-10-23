//! Service manager readiness: the systemd `sd_notify` datagram protocol, the
//! one implementation the Node daemon and the Server share. Without
//! `NOTIFY_SOCKET` (foreground, containers, other platforms) this is a no-op;
//! a failed notification is logged and never changes the caller's behavior.
//! A notification waits at most [`WRITE_TIMEOUT`] for room in the service
//! manager's queue: a short backlog still delivers `READY=1`, a manager that
//! is not reading loses that one notification and never stalls the caller's
//! lifecycle.
#![forbid(unsafe_code)]

use std::time::Duration;

/// The longest a notification waits for the service manager's queue.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(1);

/// Sends one newline-separated list of `KEY=VALUE` assignments, such as
/// `READY=1` or `STOPPING=1`.
pub fn notify(state: &str) {
    #[cfg(target_os = "linux")]
    if let Some(target) = std::env::var_os("NOTIFY_SOCKET") {
        match send(&target, state) {
            Ok(()) => {}
            Err(error) if timed_out(&error) => {
                tracing::debug!(%error, timeout_ms = WRITE_TIMEOUT.as_millis() as u64,
                    "Service manager notification not taken in time; dropped");
            }
            Err(error) => tracing::warn!(%error, "Service manager notification failed"),
        }
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
    socket.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let sent = socket.send_to_addr(state.as_bytes(), &address)?;
    if sent != state.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "short notification datagram",
        ));
    }
    Ok(())
}

/// A send that ran out of [`WRITE_TIMEOUT`] (`SO_SNDTIMEO`: `EAGAIN`).
#[cfg(target_os = "linux")]
fn timed_out(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
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

    /// The receiver's queue filled from senders of their own, as separate
    /// notifications fill it (not one sender's buffer).
    #[cfg(target_os = "linux")]
    fn fill(path: &std::path::Path) -> Vec<std::os::unix::net::UnixDatagram> {
        let mut fillers = Vec::new();
        loop {
            let filler = std::os::unix::net::UnixDatagram::unbound().unwrap();
            filler.set_nonblocking(true).unwrap();
            if filler.send_to(b"WATCHDOG=1", path).is_err() {
                return fillers;
            }
            fillers.push(filler);
            assert!(fillers.len() < 10_000, "the receiver queue never filled");
        }
    }

    /// F8a F9: a service manager that does not read its full queue loses
    /// the notification after the write timeout; the sender never blocks
    /// longer.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_full_notification_queue_does_not_block_the_sender_past_its_timeout() {
        use std::os::unix::net::UnixDatagram;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notify");
        let receiver = UnixDatagram::bind(&path).unwrap();
        let fillers = fill(&path);
        let (done, finished) = std::sync::mpsc::channel();
        let target = path.clone();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let _ = done.send(send(target.as_os_str(), "STOPPING=1"));
        });
        let result = finished
            .recv_timeout(WRITE_TIMEOUT + Duration::from_secs(4))
            .expect("the notification blocked past its timeout on a full queue");
        assert!(timed_out(&result.unwrap_err()));
        assert!(started.elapsed() >= WRITE_TIMEOUT - Duration::from_millis(100));
        drop((receiver, fillers));
    }

    /// F8a F9: a queue that is full for a moment still takes the
    /// notification once the service manager reads (within the timeout).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_briefly_full_notification_queue_still_delivers() {
        use std::os::unix::net::UnixDatagram;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notify");
        let receiver = UnixDatagram::bind(&path).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let fillers = fill(&path);
        let queued = fillers.len();
        let target = path.clone();
        let sender = std::thread::spawn(move || send(target.as_os_str(), "READY=1"));
        std::thread::sleep(Duration::from_millis(200));
        let mut buffer = [0; 64];
        let mut received = Vec::new();
        for _ in 0..=queued {
            match receiver.recv(&mut buffer) {
                Ok(length) => received.push(buffer[..length].to_vec()),
                Err(_) => break,
            }
        }
        sender.join().unwrap().unwrap();
        assert_eq!(received.last().map(Vec::as_slice), Some(&b"READY=1"[..]));
        drop(fillers);
    }
}
