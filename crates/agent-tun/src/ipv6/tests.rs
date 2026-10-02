use super::*;

#[test]
fn missing_per_interface_ipv6_sysctl_requires_verified_disabled_inheritance() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("ipv6");
    assert!(disable_ipv6(&tree, "meshway0").is_ok());
    std::fs::create_dir_all(tree.join("conf/default")).unwrap();
    assert!(disable_ipv6(&tree, "meshway0").is_err());
    let default = tree.join("conf/default/disable_ipv6");
    std::fs::write(&default, b"0\n").unwrap();
    assert!(disable_ipv6(&tree, "meshway0").is_err());
    std::fs::write(&default, b"1\n").unwrap();
    assert!(disable_ipv6(&tree, "meshway0").is_ok());
    std::fs::create_dir_all(tree.join("conf/meshway0")).unwrap();
    let own = tree.join("conf/meshway0/disable_ipv6");
    std::fs::write(&own, b"0\n").unwrap();
    disable_ipv6(&tree, "meshway0").unwrap();
    assert_eq!(std::fs::read_to_string(&own).unwrap(), "1\n");
    std::fs::write(&own, b"unknown\n").unwrap();
    assert!(
        disable_ipv6(&tree, "meshway0").is_err(),
        "default=1 cannot mask invalid own settings"
    );
    std::fs::remove_file(&own).unwrap();
    std::fs::create_dir(&own).unwrap();
    assert!(
        disable_ipv6(&tree, "meshway0").is_err(),
        "default=1 cannot mask non-ENOENT failures"
    );
}
