use super::{
    command::Commands,
    host::{self, Failure, ReversePath},
};
use crate::{Error, Result};

/// Installation requirements, judged again on the live host at install time.
/// This backend cannot preserve opaque foreign mark writes or flow offload.
/// Refusing installation preserves those rules rather than rewriting them.
pub(super) async fn check(commands: &mut Commands, owned: &[(String, String)]) -> Result<()> {
    let read = |path: &std::path::Path| {
        host::sysctl(path).map_err(|failure| match failure {
            Failure::Unavailable => Error::Io(std::io::ErrorKind::NotFound.into()),
            Failure::Malformed => Error::Target,
        })
    };
    if read("/proc/sys/net/ipv4/ip_forward".as_ref())? != 1 {
        return Err(Error::Target);
    }
    let all = read("/proc/sys/net/ipv4/conf/all/rp_filter".as_ref())?;
    let all_mark = read("/proc/sys/net/ipv4/conf/all/src_valid_mark".as_ref())?;
    if all > 2 || all_mark > 1 {
        return Err(Error::Target);
    }
    // Every interface, present and future routes alike, not only the probed ones.
    for (index, entry) in std::fs::read_dir("/proc/sys/net/ipv4/conf")?.enumerate() {
        if index >= 256 {
            return Err(Error::Capacity);
        }
        let entry = entry?;
        if entry.file_name() == "all" {
            continue;
        }
        let rpf = read(&entry.path().join("rp_filter"))?;
        let mark = read(&entry.path().join("src_valid_mark"))?;
        if host::reverse_path(all, rpf, all_mark, mark) != ReversePath::Usable {
            return Err(Error::Conflict);
        }
    }
    let foreign =
        host::foreign_ruleset(commands, owned)
            .await
            .map_err(|failure| match failure {
                Failure::Unavailable => Error::Netlink,
                Failure::Malformed => Error::Conflict,
            })?;
    if foreign.iter().any(|object| {
        matches!(
            object.kind.as_str(),
            "flowtable" | "flow" | "mangle" | "ct helper"
        ) || host::contains(&object.data, &["flowtable", "flow", "mangle", "ct helper"])
            || host::uses_mark(&object.data)
    }) {
        return Err(Error::Conflict);
    }
    // Legacy iptables by content, as the probe judges it: loaded tables that
    // cannot be read, or that touch marks or conntrack handling, conflict.
    for family in [
        meshway_model::exit::ExitFamily::Ipv4,
        meshway_model::exit::ExitFamily::Ipv6,
    ] {
        match super::firewall::legacy(commands, family).await {
            Ok(None) => {}
            Ok(Some(legacy)) if !legacy.marks => {}
            Ok(Some(_)) => return Err(Error::Conflict),
            // An unreadable IPv6 table set blocks only the IPv6 family,
            // which the probe reports; installation needs IPv4.
            Err(_) if family == meshway_model::exit::ExitFamily::Ipv6 => {}
            // A legacy table that could not be read proves nothing; one
            // read back unparseable is not one this owner can judge.
            Err(Failure::Unavailable) => return Err(Error::Netlink),
            Err(Failure::Malformed) => return Err(Error::Conflict),
        }
    }
    Ok(())
}
