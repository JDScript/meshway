use crate::{
    status::View,
    store::{Action, SocketIdentity, Store},
};
use meshway_ipc::{Command, ErrorCode, Request, Response, ResultBody, VERSION};
use meshway_model::{RequestId, Version};
use meshway_private_storage::Directory;
use std::{
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{UnixListener, UnixStream},
    sync::{Semaphore, mpsc, oneshot, watch},
    task::JoinSet,
};

pub struct Mutation {
    pub action: Action,
    pub id: RequestId,
    pub expected: Version,
    pub reply: oneshot::Sender<Result<(), ErrorCode>>,
}

pub struct Socket {
    listener: UnixListener,
    path: PathBuf,
    identity: SocketIdentity,
    _directory: Arc<Directory>,
}
impl Socket {
    pub async fn bind(store: &mut Store) -> Result<Self, &'static str> {
        let path = store.directory.path().join("control.sock");
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                let owned = store
                    .record
                    .socket
                    .is_some_and(|id| matches_file(&meta, id));
                if !owned {
                    return Err("Existing IPC path has no matching ownership record.");
                }
                match tokio::time::timeout(Duration::from_millis(200), UnixStream::connect(&path))
                    .await
                {
                    Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
                    _ => return Err("Existing IPC socket is active or cannot be verified."),
                }
                let current = fs::symlink_metadata(&path).map_err(|_| "IPC ownership changed.")?;
                if !matches_file(
                    &current,
                    store.record.socket.ok_or("Missing IPC ownership.")?,
                ) {
                    return Err("IPC ownership changed.");
                }
                fs::remove_file(&path).map_err(|_| "Cannot remove owned stale IPC socket.")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect IPC path."),
        }
        let listener = UnixListener::bind(&path).map_err(|_| "Cannot bind IPC socket.")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|_| "Cannot protect IPC socket.")?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| "Cannot inspect new IPC socket.")?;
        let identity = SocketIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        let socket = Self {
            listener,
            path,
            identity,
            _directory: Arc::clone(&store.directory),
        };
        store.socket(identity)?;
        Ok(socket)
    }
}
fn matches_file(meta: &fs::Metadata, id: SocketIdentity) -> bool {
    meta.file_type().is_socket()
        && meta.uid() == rustix::process::geteuid().as_raw()
        && meta.mode() & 0o7777 == 0o600
        && meta.dev() == id.device
        && meta.ino() == id.inode
}
impl Drop for Socket {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|meta| matches_file(&meta, self.identity)) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn authorized(stream: &UnixStream) -> bool {
    stream
        .peer_cred()
        .is_ok_and(|peer| peer.uid() == 0 || peer.uid() == rustix::process::geteuid().as_raw())
}

async fn handle(mut stream: UnixStream, commands: mpsc::Sender<Mutation>, view: View) {
    if !authorized(&stream) {
        return;
    }
    let Ok(request) = meshway_ipc::read::<Request>(&mut stream).await else {
        return;
    };
    if !authorized(&stream) {
        return;
    }
    let result = if request.version != VERSION {
        Err(ErrorCode::Version)
    } else {
        let action = match request.command {
            Command::Status => None,
            Command::Up { expected_revision } => Some((Action::Up, expected_revision)),
            Command::Down { expected_revision } => Some((Action::Down, expected_revision)),
        };
        if let Some((action, expected)) = action {
            let (reply, result) = oneshot::channel();
            match commands.try_send(Mutation {
                action,
                expected,
                id: request.request_id,
                reply,
            }) {
                Ok(()) => result.await.unwrap_or(Err(ErrorCode::Stopping)),
                Err(mpsc::error::TrySendError::Full(_)) => Err(ErrorCode::Busy),
                Err(mpsc::error::TrySendError::Closed(_)) => Err(ErrorCode::Stopping),
            }
        } else {
            Ok(())
        }
    };
    let response = Response {
        version: VERSION,
        request_id: request.request_id,
        result: match result {
            Ok(()) => ResultBody::Status {
                status: Box::new(view.status()),
            },
            Err(code) => ResultBody::Error { code },
        },
    };
    let _ = meshway_ipc::write(&mut stream, &response).await;
}

pub async fn serve(
    socket: Socket,
    commands: mpsc::Sender<Mutation>,
    view: View,
    mut stop: watch::Receiver<bool>,
) -> Result<(), &'static str> {
    let permits = Arc::new(Semaphore::new(32));
    let mut clients = JoinSet::new();
    let outcome = loop {
        tokio::select! { biased;
            result = stop.changed() => { if result.is_err() || *stop.borrow() { break Ok(()); } },
            Some(_) = clients.join_next(), if !clients.is_empty() => {},
            accepted = socket.listener.accept() => {
                let (stream,_) = match accepted { Ok(value) => value, Err(_) => break Err("IPC accept failed.") };
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue; };
                let commands = commands.clone(); let view = view.clone();
                clients.spawn(async move {
                    let _permit = permit;
                    let _ = tokio::time::timeout(Duration::from_secs(10),handle(stream,commands,view)).await;
                });
            },
        }
    };
    clients.abort_all();
    while clients.join_next().await.is_some() {}
    outcome
}
