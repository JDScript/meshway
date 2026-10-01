//! Outer TCP socket boundary, shared by TLS/TCP and TLS/WebSocket. Platform
//! protection must happen before connect/listen, never as a global IP bypass.
use std::{io, net::SocketAddr};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

pub(crate) enum Purpose<'a> {
    Listener {
        listener_id: &'a str,
    },
    Dial {
        candidate_id: &'a str,
        peer_node_id: &'a str,
    },
}

/// Controller-driven platform adapters can set SO_MARK or an equivalent
/// per-socket exclusion here. No process-wide or destination-wide exemptions.
pub(crate) trait Protection: Send + Sync {
    fn prepare(&self, address: SocketAddr, purpose: Purpose<'_>) -> io::Result<TcpSocket>;
    fn accepted(&self, socket: &TcpStream, listener_id: &str) -> io::Result<()>;
}

/// Current Mesh-prefix-only execution uses the system underlay. Full IP capture
/// must install a platform protection adapter before activating these sockets.
pub(crate) struct SystemUnderlay;
impl Protection for SystemUnderlay {
    fn prepare(&self, address: SocketAddr, purpose: Purpose<'_>) -> io::Result<TcpSocket> {
        // Preserve IDs at this boundary for future platform ownership/logical
        // binding. They are not destination exemptions or business permissions.
        match purpose {
            Purpose::Listener { listener_id } => {
                let _ = listener_id;
            }
            Purpose::Dial {
                candidate_id,
                peer_node_id,
            } => {
                let _ = (candidate_id, peer_node_id);
            }
        }
        if address.is_ipv4() {
            TcpSocket::new_v4()
        } else {
            TcpSocket::new_v6()
        }
    }
    fn accepted(&self, _: &TcpStream, _: &str) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn listen(
    protection: &dyn Protection,
    address: SocketAddr,
    id: &str,
) -> io::Result<TcpListener> {
    let socket = protection.prepare(address, Purpose::Listener { listener_id: id })?;
    // Match Tokio's listener binding. Retired TLS sessions can leave TIME_WAIT;
    // this permits immediate generation rebind, not shared listeners.
    // SO_REUSEPORT remains disabled.
    socket.set_reuseaddr(true)?;
    socket.bind(address)?;
    socket.listen(128)
}

pub(crate) async fn dial(
    protection: &dyn Protection,
    address: SocketAddr,
    id: &str,
    peer: &str,
) -> io::Result<TcpStream> {
    let socket = protection.prepare(
        address,
        Purpose::Dial {
            candidate_id: id,
            peer_node_id: peer,
        },
    )?;
    socket.connect(address).await
}
