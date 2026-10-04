//! Recover and remove only the private socket recorded by this daemon.
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::net::{UnixListener, UnixStream};

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SocketIdentity {
    device: u64,
    inode: u64,
}

pub(crate) struct Socket {
    pub(super) listener: UnixListener,
    path: PathBuf,
    identity: SocketIdentity,
    _directory: Arc<Directory>,
}
impl Socket {
    pub async fn bind(
        directory: Arc<Directory>,
        previous: Option<SocketIdentity>,
    ) -> Result<Self, &'static str> {
        let path = directory.path().join("control.sock");
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                let owned = previous.is_some_and(|id| matches_file(&meta, id));
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
                if !matches_file(&current, previous.ok_or("Missing IPC ownership.")?) {
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
        Ok(Self {
            listener,
            path,
            identity,
            _directory: directory,
        })
    }

    pub fn identity(&self) -> SocketIdentity {
        self.identity
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
