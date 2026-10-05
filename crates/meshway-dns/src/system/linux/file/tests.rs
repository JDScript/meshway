use super::*;
use meshway_configuration::{DnsConfiguration, execution::DnsEgress};

struct Fixture {
    _root: tempfile::TempDir,
    directory: Arc<Directory>,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        // Workspace ancestors satisfy production ownership checks; /tmp's
        // world-writable ancestor deliberately does not.
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let directory =
            Arc::new(Directory::open(&root.path().join("private"), "fixture.lock").unwrap());
        let path = root.path().join("resolv.conf");
        std::fs::write(
            &path,
            b"# fixture\nnameserver 192.0.2.53\nsearch original.example\noptions ndots:2\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        Self {
            _root: root,
            directory,
            path,
        }
    }
    fn backend(&self) -> Result<Backend, SystemDnsError> {
        Backend::prepare(
            self.directory.clone(),
            self.path.clone(),
            "127.0.0.1:53".parse().unwrap(),
        )
    }
}
fn plan() -> SystemDnsPlan {
    let cfg: DnsConfiguration =
        serde_json::from_str(r#"{"domain":"mesh.example","searchDomains":["mesh.example"]}"#)
            .unwrap();
    SystemDnsPlan::new(&cfg, &DnsEgress::NoExit).unwrap()
}

#[tokio::test]
async fn tcp_only_system_policy_is_preserved_and_local_stub_cycles_are_rejected() {
    let fixture = Fixture::new();
    std::fs::write(&fixture.path, b"nameserver 192.0.2.53\noptions use-vc\n").unwrap();
    let mut backend = fixture.backend().unwrap();
    assert_eq!(
        backend.transport().unwrap(),
        meshway_configuration::DnsTransport::Tcp
    );
    backend.install(&plan()).await.unwrap();
    assert!(
        resolv_conf::Config::parse(std::fs::read(&fixture.path).unwrap())
            .unwrap()
            .use_vc
    );
    backend.down().await.unwrap();
    drop(backend);
    for bytes in [
        b"nameserver 127.0.0.53\n".as_slice(),
        b"nameserver fe80::53%eth0\n",
        b"nameserver fe80::53\n",
    ] {
        std::fs::write(&fixture.path, bytes).unwrap();
        assert!(fixture.backend().is_err());
    }
}

#[tokio::test]
async fn atomic_install_recovers_exact_identity_and_restores_original_public_file() {
    let fixture = Fixture::new();
    let original = std::fs::read(&fixture.path).unwrap();
    let original_inode = std::fs::metadata(&fixture.path).unwrap().ino();
    let mut backend = fixture.backend().unwrap();
    backend.install(&plan()).await.unwrap();
    assert_ne!(
        std::fs::metadata(&fixture.path).unwrap().ino(),
        original_inode
    );
    assert_eq!(
        std::fs::metadata(&fixture.path).unwrap().mode() & 0o777,
        0o644
    );
    let installed = std::fs::read(&fixture.path).unwrap();
    let config = resolv_conf::Config::parse(&installed).unwrap();
    assert_eq!(
        config.nameservers,
        vec![resolv_conf::ScopedIp::V4("127.0.0.1".parse().unwrap())]
    );
    assert_eq!(config.ndots, 2);
    assert_eq!(
        backend.upstreams().unwrap(),
        vec!["192.0.2.53:53".parse::<SocketAddr>().unwrap()]
    );
    let id = backend.snapshot_id().unwrap();
    drop(backend);
    let mut recovered = fixture.backend().unwrap();
    assert_eq!(recovered.snapshot_id().unwrap(), id);
    recovered.observe().await.unwrap();
    recovered.down().await.unwrap();
    assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
}

#[tokio::test]
async fn same_bytes_foreign_inode_and_foreign_content_both_prevent_restoration() {
    for replace_inode in [false, true] {
        let fixture = Fixture::new();
        let mut backend = fixture.backend().unwrap();
        backend.install(&plan()).await.unwrap();
        if replace_inode {
            let bytes = std::fs::read(&fixture.path).unwrap();
            let other = fixture.path.with_extension("replacement");
            std::fs::write(&other, bytes).unwrap();
            std::fs::rename(other, &fixture.path).unwrap();
        } else {
            std::fs::write(&fixture.path, b"nameserver 192.0.2.54\n").unwrap();
        }
        let foreign = std::fs::read(&fixture.path).unwrap();
        assert!(backend.observe().await.is_err());
        assert!(backend.down().await.is_err());
        assert_eq!(std::fs::read(&fixture.path).unwrap(), foreign);
        drop(backend);
        assert!(fixture.backend().is_err());
    }
}

#[test]
fn symlink_hardlink_and_unrecoverable_pending_write_are_not_adopted() {
    let fixture = Fixture::new();
    let other = fixture.path.with_extension("other");
    std::fs::hard_link(&fixture.path, &other).unwrap();
    assert!(fixture.backend().is_err());
    std::fs::remove_file(&other).unwrap();
    std::fs::rename(&fixture.path, &other).unwrap();
    std::os::unix::fs::symlink(&other, &fixture.path).unwrap();
    assert!(fixture.backend().is_err());
    std::fs::remove_file(&fixture.path).unwrap();
    std::fs::rename(other, &fixture.path).unwrap();
    let backend = fixture.backend().unwrap();
    {
        let mut state = backend.state.lock().unwrap();
        state.record.pending = true;
        storage::save(&state.directory, "resolv-conf.json", &Some(&state.record)).unwrap();
    }
    drop(backend);
    assert!(fixture.backend().is_err());
}

#[test]
#[ignore = "fixture must supply an isolated Linux bind-mounted resolver file"]
fn file_mount_is_rejected_without_writing_it() {
    assert_eq!(std::env::var("MESHWAY_DNS_ISOLATED").as_deref(), Ok("1"));
    let path = PathBuf::from(std::env::var_os("MESHWAY_DNS_BIND_MOUNT_FILE").unwrap());
    assert!(matches!(open(&path), Err(SystemDnsError::Conflict)));
}
