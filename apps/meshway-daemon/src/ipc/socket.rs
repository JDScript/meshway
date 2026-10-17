//! Recover and remove only a stale socket of this daemon's private directory.
use meshway_node::ipc::SocketIdentity;
use meshway_private_storage::Directory;
use std::{
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::net::{UnixListener, UnixStream};

/// The local IPC socket in the daemon's private state directory.
pub(crate) const FILE: &str = "control.sock";

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
        let path = directory.path().join(FILE);
        let device = fs::metadata(directory.path())
            .map_err(|_| "Cannot inspect IPC directory.")?
            .dev();
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                // Reclaiming needs a gone predecessor (connect refused) and a
                // continuous identity (same inode). The device number is not
                // stable across mounts and boots, so it never vetoes. Without
                // a record, the private 0700 directory under daemon.lock is
                // the ownership proof.
                if !matches_file(&meta, previous, device) {
                    return Err("Existing IPC path has no matching ownership record.");
                }
                match tokio::time::timeout(Duration::from_millis(200), UnixStream::connect(&path))
                    .await
                {
                    Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
                    _ => return Err("Existing IPC socket is active or cannot be verified."),
                }
                let current = fs::symlink_metadata(&path).map_err(|_| "IPC ownership changed.")?;
                if !matches_file(&current, previous, device) || current.ino() != meta.ino() {
                    return Err("IPC ownership changed.");
                }
                if previous.is_none() {
                    tracing::warn!(
                        path = %path.display(),
                        "Reclaiming an unrecorded stale IPC socket in the private state directory"
                    );
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
fn matches_file(meta: &fs::Metadata, previous: Option<SocketIdentity>, device: u64) -> bool {
    meta.file_type().is_socket()
        && meta.uid() == rustix::process::geteuid().as_raw()
        && meta.mode() & 0o7777 == 0o600
        && meta.dev() == device
        && previous.is_none_or(|id| meta.ino() == id.inode)
}
impl Drop for Socket {
    fn drop(&mut self) {
        let device = fs::metadata(self._directory.path()).map(|meta| meta.dev());
        if let Ok(device) = device
            && fs::symlink_metadata(&self.path)
                .is_ok_and(|meta| matches_file(&meta, Some(self.identity), device))
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stale(directory: &Directory) -> fs::Metadata {
        let path = directory.path().join("control.sock");
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::symlink_metadata(&path).unwrap()
    }

    fn directory() -> (tempfile::TempDir, Arc<Directory>) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap().join("state");
        let directory = Directory::open(&path, "daemon.lock").unwrap();
        (root, Arc::new(directory))
    }

    #[tokio::test]
    async fn a_stale_socket_on_a_renumbered_device_is_reclaimed_by_inode() {
        let (_root, directory) = directory();
        let meta = stale(&directory);
        let recorded = SocketIdentity {
            device: meta.dev().wrapping_add(1),
            inode: meta.ino(),
        };
        let socket = Socket::bind(directory, Some(recorded)).await.unwrap();
        assert_ne!(socket.identity().inode, 0);
    }

    #[tokio::test]
    async fn an_unrecorded_stale_socket_in_the_private_directory_is_reclaimed() {
        let (_root, directory) = directory();
        stale(&directory);
        Socket::bind(directory, None).await.unwrap();
    }

    #[tokio::test]
    async fn a_different_inode_or_a_live_listener_is_never_reclaimed() {
        let (_root, directory) = directory();
        let meta = stale(&directory);
        let other = SocketIdentity {
            device: meta.dev(),
            inode: meta.ino().wrapping_add(1),
        };
        assert!(Socket::bind(directory.clone(), Some(other)).await.is_err());
        let live = Socket::bind(directory.clone(), None).await.unwrap();
        let recorded = live.identity();
        assert!(
            Socket::bind(directory.clone(), Some(recorded))
                .await
                .is_err()
        );
        assert!(Socket::bind(directory, None).await.is_err());
        drop(live);
    }
}
