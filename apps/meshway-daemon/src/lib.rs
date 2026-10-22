//! Unix process host for the shared Node runtime.
#![forbid(unsafe_code)]
#![cfg(unix)]
mod ipc;
pub use meshway_node::Settings;

pub async fn run(
    settings: Settings,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), &'static str> {
    use std::io::Write;
    #[cfg(target_os = "linux")]
    let platform = std::sync::Arc::new(meshway_node::host::Linux);
    #[cfg(not(target_os = "linux"))]
    let platform = std::sync::Arc::new(meshway_node::host::ControlOnly);
    let mut node = meshway_node::Node::open(settings, platform).await?;
    let socket = ipc::Socket::bind(node.directory(), node.socket_identity()).await?;
    node.record_socket(socket.identity())?;
    let path = node.directory().path().join("control.sock");
    let mut lifecycle = node.lifecycle();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let mut listener = tokio::spawn(ipc::serve(socket, node.ipc(), stopped));
    let cancel = tokio_util::sync::CancellationToken::new();
    let run = node.run(async {
        tokio::select! { _ = shutdown => {}, _ = cancel.cancelled() => {} }
    });
    tokio::pin!(run);
    let mut finished = false;
    let mut stopping = false;
    // Closed only as `run` returns; then `run` is the next ready branch.
    let mut watching = true;
    let result = loop {
        tokio::select! { biased;
            changed = lifecycle.changed(), if watching => {
                if changed.is_err() { watching = false; continue; }
                match *lifecycle.borrow_and_update() {
                    meshway_node::Lifecycle::Ready => {
                        tracing::info!(socket = %path.display(), "Local IPC ready");
                        if writeln!(std::io::stdout().lock(), "Local IPC ready: {}", path.display()).is_err() {
                            break Err("Cannot write Node readiness.");
                        }
                        meshway_service::notify("READY=1");
                    }
                    meshway_node::Lifecycle::Stopping => {
                        meshway_service::notify("STOPPING=1");
                        stopping = true;
                    }
                    _ => {}
                }
            }
            result = &mut run => { finished = true; break result; },
            result = &mut listener => break result.unwrap_or(Err("IPC service stopped unexpectedly.")),
        }
    };
    // A watch is level-triggered: an immediate failure can coalesce Stopping
    // and Stopped before this host is polled again. Always notify systemd once.
    if !stopping {
        meshway_service::notify("STOPPING=1");
    }
    cancel.cancel();
    if !finished {
        let _ = run.await;
    }
    let _ = stop.send(());
    if !listener.is_finished() {
        let _ = listener.await;
    }
    tracing::info!(failed = result.is_err(), "Node process stopped");
    writeln!(std::io::stdout().lock(), "Node stopped.")
        .map_err(|_| "Cannot write Node shutdown.")?;
    result
}
