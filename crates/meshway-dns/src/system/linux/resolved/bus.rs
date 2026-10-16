//! resolve1 over D-Bus: the owner proof and the bounded calls and property reads.
use super::{Checked, IFACE, LINK, LinkSettings, PATH, SERVICE, TIMEOUT};
use crate::system::linux::{Namespace, SystemDnsError, SystemDnsFailure};
use serde::Serialize;
use zbus::zvariant::{DynamicDeserialize, DynamicType, OwnedObjectPath, OwnedValue};

/// The current resolve1 owner, proven to run in this namespace as the OS
/// resolver, and the object path of the owned link on it.
pub(super) async fn attach(
    connection: &zbus::Connection,
    namespace: &Namespace,
    index: u32,
) -> Checked<(String, String)> {
    let owner = name_owner(connection).await?;
    verify_owner(connection, &owner, namespace).await?;
    verify_os_resolver(connection, &owner).await?;
    let path: OwnedObjectPath = raw(
        connection,
        &owner,
        PATH,
        IFACE,
        "GetLink",
        &(index as i32,),
        "resolved_link",
    )
    .await
    .map_err(|failure| failure.expected(format!("resolved knows link index {index}")))?;
    Ok((owner, path.to_string()))
}
/// One resolve1 call or property read; `check` names it in a failure, whose
/// `actual` is the D-Bus error (for example `ServiceUnknown` while resolved
/// restarts) or the 5 s deadline.
pub(super) async fn raw<B, R>(
    connection: &zbus::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
    check: &'static str,
) -> Checked<R>
where
    B: Serialize + DynamicType,
    R: for<'d> DynamicDeserialize<'d>,
{
    let source = format!("D-Bus {destination} {path} {interface}.{method}");
    let failure = |actual: String| {
        SystemDnsFailure::new(SystemDnsError::Backend, check)
            .actual(actual)
            .source(&source)
    };
    tokio::time::timeout(TIMEOUT, async {
        let reply = connection
            .call_method(Some(destination), path, Some(interface), method, body)
            .await
            .map_err(|error| failure(error.to_string()))?;
        reply
            .body()
            .deserialize()
            .map_err(|error| failure(format!("unexpected reply: {error}")))
    })
    .await
    .map_err(|_| failure("no reply within 5 s".into()))?
}
pub(super) async fn property<T>(
    connection: &zbus::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
    check: &'static str,
) -> Checked<T>
where
    T: TryFrom<OwnedValue>,
{
    let source = format!("D-Bus {destination} {path} {interface}.{name}");
    let value: OwnedValue = raw(
        connection,
        destination,
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(interface, name),
        check,
    )
    .await
    .map_err(|failure| failure.source(&source))?;
    value.try_into().map_err(|_| {
        SystemDnsFailure::new(SystemDnsError::Backend, check)
            .actual("unexpected property type")
            .source(source)
    })
}
pub(super) async fn name_owner(connection: &zbus::Connection) -> Checked<String> {
    raw(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetNameOwner",
        &(SERVICE,),
        "resolved_owner",
    )
    .await
    .map_err(|failure| failure.expected(format!("{SERVICE} has a bus owner")))
}
pub(super) async fn verify_owner(
    connection: &zbus::Connection,
    expected: &str,
    namespace: &Namespace,
) -> Checked<()> {
    let current = name_owner(connection).await?;
    if current != expected {
        return Err(
            SystemDnsFailure::new(SystemDnsError::Conflict, "resolved_owner")
                .expected(expected)
                .actual(current)
                .source(format!("D-Bus GetNameOwner({SERVICE})")),
        );
    }
    let pid: u32 = raw(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetConnectionUnixProcessID",
        &(expected,),
        "resolved_process",
    )
    .await?;
    // resolved commonly runs as systemd-resolve, not root. The system bus owns
    // service-name policy; PID/netns is checked regardless of service UID.
    namespace.verify_pid(pid)
}

pub(super) async fn verify_os_resolver(connection: &zbus::Connection, owner: &str) -> Checked<()> {
    let mode: String = property(
        connection,
        owner,
        PATH,
        IFACE,
        "ResolvConfMode",
        "resolv_conf_mode",
    )
    .await?;
    let listener: String = property(
        connection,
        owner,
        PATH,
        IFACE,
        "DNSStubListener",
        "stub_listener",
    )
    .await?;
    // An active resolve1 bus is insufficient: uplink/foreign resolv.conf lets
    // libc bypass per-link routing. Initial OS support requires the actual
    // dynamically maintained resolved stub file and both UDP/TCP listeners.
    // Static mode does not publish installed search domains to libc. NSS overrides are separate
    // host integration, never inferred from D-Bus method success.
    if mode != "stub" {
        return Err(
            SystemDnsFailure::new(SystemDnsError::Unsupported, "resolv_conf_mode")
                .expected("stub")
                .actual(mode)
                .source(format!("D-Bus {owner} {PATH} {IFACE}.ResolvConfMode")),
        );
    }
    if listener != "yes" {
        return Err(
            SystemDnsFailure::new(SystemDnsError::Unsupported, "stub_listener")
                .expected("yes")
                .actual(listener)
                .source(format!("D-Bus {owner} {PATH} {IFACE}.DNSStubListener")),
        );
    }
    Ok(())
}
pub(super) async fn read_settings(
    connection: &zbus::Connection,
    owner: &str,
    path: &str,
) -> Checked<LinkSettings> {
    const CHECK: &str = "link_settings";
    let settings = LinkSettings {
        dns: property(connection, owner, path, LINK, "DNSEx", CHECK).await?,
        domains: property(connection, owner, path, LINK, "Domains", CHECK).await?,
        default_route: property(connection, owner, path, LINK, "DefaultRoute", CHECK).await?,
        llmnr: property(connection, owner, path, LINK, "LLMNR", CHECK).await?,
        mdns: property(connection, owner, path, LINK, "MulticastDNS", CHECK).await?,
        tls: property(connection, owner, path, LINK, "DNSOverTLS", CHECK).await?,
        dnssec: property(connection, owner, path, LINK, "DNSSEC", CHECK).await?,
    };
    if settings.dns.len() > 8 || settings.domains.len() > 256 {
        return Err(SystemDnsFailure::new(SystemDnsError::Capacity, CHECK)
            .expected("at most 8 servers and 256 domains")
            .actual(format!(
                "{} servers, {} domains",
                settings.dns.len(),
                settings.domains.len()
            ))
            .source(format!("D-Bus {owner} {path} {LINK}")));
    }
    Ok(settings)
}
