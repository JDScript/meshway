//! Unix listener ownership and peer authentication. Typed dispatch is in Node.
mod socket;
pub(crate) use socket::Socket;

pub(crate) async fn serve(
    socket: Socket,
    dispatcher: meshway_node::ipc::Handle,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), &'static str> {
    loop {
        tokio::select! { biased;
            _ = &mut stop => return Ok(()),
            accepted = socket.listener.accept() => {
                let (stream, _) = accepted.map_err(|_| "IPC accept failed.")?;
                if stream.peer_cred().is_ok_and(|peer| peer.uid() == 0 || peer.uid() == rustix::process::geteuid().as_raw()) {
                    let _ = dispatcher.submit(stream);
                }
            }
        }
    }
}
