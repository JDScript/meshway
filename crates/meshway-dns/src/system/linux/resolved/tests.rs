/// A link-settings conflict names only the fields that differ, as
/// resolvectl shows them, with the installation marker shortened.
#[test]
fn a_settings_conflict_names_the_differing_fields() {
    let bind = "127.0.0.1:5354".parse().unwrap();
    let marker = format!("i{}.{}", "ab".repeat(16), super::MARKER_SUFFIX);
    let installed = super::local_settings(
        bind,
        vec![("meshway.test".into(), false), (marker.clone(), true)],
    );
    let mut actual = installed.clone();
    actual.domains = vec![("meshway.test".into(), false)];
    actual.default_route = true;
    let (expected, found) = installed.differences(&actual);
    assert_eq!(
        expected,
        "domains=[meshway.test, ~iabababab….meshway-installation.invalid] defaultRoute=false"
    );
    assert_eq!(found, "domains=[meshway.test] defaultRoute=true");
    assert_eq!(
        installed.differences(&installed),
        (String::new(), String::new())
    );
    let original = super::local_settings(bind, Vec::new());
    let (expected, found) = installed.differences(&original);
    assert!(!expected.contains("dns="), "{expected}");
    let mut cleared = original.clone();
    cleared.dns.clear();
    assert_eq!(
        original.differences(&cleared),
        ("dns=[127.0.0.1:5354]".into(), "dns=[]".into())
    );
    let failure = crate::SystemDnsFailure::new(crate::SystemDnsError::Conflict, "link_settings")
        .expected(expected)
        .actual(found)
        .source("D-Bus :1.1 /org/freedesktop/resolve1/link/_34 org.freedesktop.resolve1.Link");
    assert_eq!(
        failure.to_string(),
        "DNS link_settings check failed (system DNS resource ownership conflict): expected domains=[meshway.test, ~iabababab….meshway-installation.invalid]; found domains=[] [D-Bus :1.1 /org/freedesktop/resolve1/link/_34 org.freedesktop.resolve1.Link]"
    );
}

#[test]
fn the_proxy_stub_needs_its_exact_udp_and_tcp_sockets() {
    let header = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";
    let row = |address: &str, state: &str| {
        format!(
            "   0: {address} 00000000:0000 {state} 00000000:00000000 00:00000000 00000000   991        0 1 2 0000000000000000 0\n"
        )
    };
    let stub = format!("{:08X}:0035", u32::from_ne_bytes([127, 0, 0, 54]));
    let main = format!("{:08X}:0035", u32::from_ne_bytes([127, 0, 0, 53]));
    let udp = format!("{header}{}{}", row(&main, "07"), row(&stub, "07"));
    let tcp = format!("{header}{}{}", row(&main, "0A"), row(&stub, "0A"));
    assert!(super::proxy_stub_in(&udp, &tcp));
    // systemd < 248: only 127.0.0.53.
    let old_udp = format!("{header}{}", row(&main, "07"));
    let old_tcp = format!("{header}{}", row(&main, "0A"));
    assert!(!super::proxy_stub_in(&old_udp, &old_tcp));
    // A connected or non-listening socket at the address is not the stub.
    assert!(!super::proxy_stub_in(
        &udp,
        &format!("{header}{}", row(&stub, "01"))
    ));
    assert!(!super::proxy_stub_in(
        &format!("{header}{}", row(&stub, "01")),
        &tcp
    ));
    assert!(!super::proxy_stub_in("", ""));
}
