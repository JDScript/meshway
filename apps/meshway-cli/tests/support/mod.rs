//! Test transport for the real CLI; runtime behavior comes from meshway-node.
pub async fn run_node(
    settings: meshway_node::Settings,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), &'static str> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let node = meshway_node::Node::open(
        settings,
        std::sync::Arc::new(meshway_node::host::ControlOnly),
    )
    .await?;
    let directory = node.directory();
    let uid = std::fs::metadata(directory.path()).unwrap().uid();
    let path = directory.path().join("control.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let ipc = node.ipc();
    let run = node.run(shutdown);
    tokio::pin!(run);
    loop {
        tokio::select! {
            result = &mut run => { std::fs::remove_file(path).unwrap(); return result; },
            accepted = listener.accept() => {
                let (stream, _) = accepted.unwrap();
                if stream.peer_cred().is_ok_and(|peer| peer.uid() == uid) { let _ = ipc.submit(stream); }
            }
        }
    }
}
