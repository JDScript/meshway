use meshway_agent_tun::{Configuration, Device};

#[tokio::test]
async fn invalid_configuration_is_rejected_before_device_allocation() {
    let configuration = Configuration {
        name: "foreign/name".into(),
        mtu: 1000,
        address: Some("100.96.0.1".parse().unwrap()),
        prefix_len: 32,
        peer: None,
    };
    assert!(
        matches!(Device::open(&configuration).await, Err(error) if error.kind() == std::io::ErrorKind::InvalidInput)
    );
}

#[cfg(not(target_os = "linux"))]
#[tokio::test]
async fn unsupported_platform_does_not_publish_fabricated_identity() {
    let configuration = Configuration {
        name: "meshway0".into(),
        mtu: 1000,
        address: Some("100.96.0.1".parse().unwrap()),
        prefix_len: 32,
        peer: None,
    };
    assert!(
        matches!(Device::open(&configuration).await, Err(error) if error.kind() == std::io::ErrorKind::Unsupported)
    );
}
