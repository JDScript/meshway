use super::*;

#[test]
fn fixed_windows_normalization_table_pressure_and_ttl_are_bounded() {
    let config = ServiceConfig {
        max_source_ips: 2,
        global_datagrams_per_second: 100,
        per_ip_datagrams_per_second: 2,
        global_output_bytes_per_second: 44,
    };
    let mut limits = Limits::new(config);
    let now = limits.window;
    let first = "127.0.0.1".parse().unwrap();
    assert_eq!(limits.admit(first, now), Admission::Allowed);
    assert_eq!(
        limits.admit("::ffff:127.0.0.1".parse().unwrap(), now),
        Admission::Allowed
    );
    assert_eq!(limits.admit(first, now), Admission::Rate);
    assert_eq!(limits.sources.len(), 1);
    assert_eq!(
        limits.admit("::1".parse().unwrap(), now),
        Admission::Allowed
    );
    for n in 2..200_u8 {
        let ip = IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, n));
        assert_ne!(limits.admit(ip, now), Admission::Allowed);
    }
    assert_eq!(limits.sources.len(), 2);
    assert!(limits.sources.contains_key(&first));
    assert!(limits.reserve_output(44));
    assert!(!limits.reserve_output(1));
    assert_eq!(limits.admit(first, now + WINDOW), Admission::Allowed);
    assert!(limits.reserve_output(44));
    assert_eq!(
        limits.admit("127.0.0.2".parse().unwrap(), now + SOURCE_TTL),
        Admission::Allowed
    );
    assert_eq!(limits.sources.len(), 2); // first was refreshed at t=1.
    assert_eq!(
        limits.admit("127.0.0.3".parse().unwrap(), now + SOURCE_TTL + WINDOW),
        Admission::Allowed
    );
    assert_eq!(limits.sources.len(), 2);
}

#[test]
fn global_budget_includes_new_sources_and_resets() {
    let mut limits = Limits::new(ServiceConfig {
        global_datagrams_per_second: 2,
        per_ip_datagrams_per_second: 2,
        ..ServiceConfig::default()
    });
    let now = limits.window;
    for n in 1..=2 {
        assert_eq!(
            limits.admit(IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, n)), now),
            Admission::Allowed
        );
    }
    assert_eq!(limits.admit("::1".parse().unwrap(), now), Admission::Rate);
    assert_eq!(limits.sources.len(), 2);
    assert_eq!(
        limits.admit("::1".parse().unwrap(), now + WINDOW),
        Admission::Allowed
    );
}
