use std::{
    io,
    net::Shutdown,
    sync::{Arc, Mutex, Weak},
};

#[cfg(any(target_os = "linux", test))]
use serde::Deserialize;
use socket2::Socket;
#[cfg(target_os = "linux")]
use socket2::{Domain, Protocol, Type};
use tokio_util::sync::CancellationToken;

use super::{Purpose, SocketFactory, closed, invalid};

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarkReservation {
    value: u32,
    mask: u32,
}

impl MarkReservation {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn new(value: u32, mask: u32) -> io::Result<Self> {
        if mask == 0 || value == 0 || value & !mask != 0 {
            return Err(invalid("nonzero mark must fit its nonzero reserved mask"));
        }
        Ok(Self { value, mask })
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn merge(self, existing: u32) -> io::Result<u32> {
        let owned = existing & self.mask;
        if owned != 0 && owned != self.value {
            return Err(invalid(
                "socket already carries conflicting reserved mark bits",
            ));
        }
        Ok((existing & !self.mask) | self.value)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn protect(self, socket: &Socket) -> io::Result<()> {
        let mark = self.merge(socket.mark()?)?;
        socket.set_mark(mark)?;
        if socket.mark()? != mark {
            return Err(io::Error::other("SO_MARK readback mismatch"));
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(super) fn verify(self, socket: &Socket) -> io::Result<()> {
        if socket.mark()? & self.mask != self.value {
            return Err(io::Error::other("socket lost its underlay mark"));
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn protect(self, _socket: &Socket) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Linux SO_MARK underlay required",
        ))
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn verify(self, _socket: &Socket) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Linux SO_MARK underlay required",
        ))
    }
}

/// Supplied only by the internal routing owner, never by device CLI or IPC.
/// This slice verifies an already installed reservation; it does not install,
/// adopt, persist or remove RPDB rules, and cannot prove crash-recovery ownership.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reservation {
    pub(crate) owner: String,
    pub(crate) generation: u64,
    pub(crate) mark: MarkReservation,
    /// Reviewed system/VPN bits from the routing owner, not a device setting.
    pub(crate) foreign_bits: u32,
    pub(crate) first_priority: u32,
    pub(crate) last_priority: u32,
    pub(crate) capture_tables: Vec<u32>,
    /// Optional owned reject-only Mesh table. Unlike local profile capture, its
    /// lookup also fences forwarded Mesh packets; it must never contain TUN or
    /// default routes. Guard still jumps over the entire owned block.
    pub(crate) mesh_sink_table: Option<u32>,
}

pub(super) struct State {
    pub(super) active: bool,
    sockets: Vec<Registered>,
}

struct Registered {
    socket: Weak<Socket>,
    #[allow(dead_code)] // Kept for typed socket admission/audit, not a runtime DTO.
    purpose: Purpose,
}

pub(super) struct Generation {
    controller_only: bool,
    pub(super) reservation: Reservation,
    pub(super) state: Mutex<State>,
    pub(super) cancelled: CancellationToken,
    #[cfg(target_os = "linux")]
    namespace_lease: Option<Arc<super::owner::Lease>>,
}

impl Generation {
    pub(super) fn admit_purpose(&self, purpose: Purpose) -> io::Result<()> {
        if self.controller_only
            && !matches!(
                purpose,
                Purpose::Controller | Purpose::BootstrapDns(super::BootstrapFor::Controller)
            )
        {
            return Err(invalid(
                "bootstrap capability admits only controller sockets and its DNS",
            ));
        }
        Ok(())
    }

    pub(super) fn admit_listener(&self) -> io::Result<()> {
        if self.controller_only {
            return Err(invalid("bootstrap capability cannot listen"));
        }
        Ok(())
    }
    pub(super) fn check_namespace(&self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(lease) = &self.namespace_lease {
            lease.check_current()?;
        }
        Ok(())
    }
    pub(super) fn register(
        &self,
        socket: Arc<Socket>,
        purpose: Purpose,
    ) -> io::Result<Arc<Socket>> {
        self.admit_purpose(purpose)?;
        let mut state = self.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Err(closed());
        }
        state
            .sockets
            .retain(|entry| entry.socket.strong_count() != 0);
        state.sockets.push(Registered {
            socket: Arc::downgrade(&socket),
            purpose,
        });
        Ok(socket)
    }

    pub(super) fn retire(&self) {
        // Same lock as every application I/O poll/try_send: no new application
        // bytes enter the kernel after the fence. Already sent bytes stay sent.
        if let Ok(mut state) = self.state.lock() {
            state.active = false;
            for socket in state
                .sockets
                .drain(..)
                .filter_map(|entry| entry.socket.upgrade())
            {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
        self.cancelled.cancel();
    }
}

/// Unique retirement owner; clones of factory/resolver/HTTP cannot keep an old
/// generation active when this owner is dropped.
pub struct PreparedGeneration(Arc<Generation>);

impl Drop for PreparedGeneration {
    fn drop(&mut self) {
        self.0.retire();
    }
}

impl PreparedGeneration {
    #[cfg(target_os = "linux")]
    pub(crate) async fn prepare(reservation: Reservation) -> io::Result<Self> {
        let _ = MarkReservation::new(reservation.mark.value, reservation.mark.mask)?;
        if reservation.owner.is_empty()
            || reservation.first_priority == 0
            || reservation.last_priority >= 32766
            || reservation
                .first_priority
                .checked_add(2)
                .is_none_or(|p| p > reservation.last_priority)
            || reservation.capture_tables.is_empty()
            || reservation.capture_tables.iter().any(|table| *table <= 255)
            || reservation
                .capture_tables
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != reservation.capture_tables.len()
            || reservation.foreign_bits & reservation.mark.mask != 0
            || reservation
                .mesh_sink_table
                .is_some_and(|table| table <= 255 || reservation.capture_tables.contains(&table))
        {
            return Err(invalid("invalid internal underlay reservation"));
        }
        for family in ["-4", "-6"] {
            let result = tokio::process::Command::new("ip")
                // rt_tables aliases are display names, not table identities.
                // Ask the kernel/iproute2 boundary for numeric IDs before any
                // ownership/reference decision (including inactive reserves).
                .args(["-N", "-j", family, "rule", "show"])
                .output()
                .await?;
            if !result.status.success() {
                return Err(io::Error::other("cannot inspect RPDB"));
            }
            validate_rules(&reservation, &result.stdout)?;
            if let Some(table) = reservation.mesh_sink_table {
                let result = tokio::process::Command::new("ip")
                    .args(["-N", "-j", family, "route", "show", "table", "all"])
                    .output()
                    .await?;
                if !result.status.success() {
                    return Err(io::Error::other("cannot inspect reject-only Mesh table"));
                }
                let routes: Vec<serde_json::Value> =
                    serde_json::from_slice(&result.stdout).map_err(invalid_json)?;
                let mut sinks = Vec::new();
                for route in routes {
                    if route
                        .get("table")
                        .map(table_identity)
                        .transpose()?
                        .unwrap_or(254)
                        == table
                    {
                        sinks.push(route);
                    }
                }
                // An unused reserved table may not yet exist in the FIB; a
                // successful all-table dump proves emptiness without treating
                // iproute2's missing-table error as permission to skip checks.
                validate_mesh_sinks(table, &serde_json::to_vec(&sinks).map_err(invalid_json)?)?;
            }
        }
        // Capability and both address families verified before publishing factory.
        for domain in [Domain::IPV4, Domain::IPV6] {
            let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
            reservation.mark.protect(&socket)?;
        }
        Ok(Self(Arc::new(Generation {
            controller_only: false,
            reservation,
            state: Mutex::new(State {
                active: true,
                sockets: Vec::new(),
            }),
            cancelled: CancellationToken::new(),
            #[cfg(target_os = "linux")]
            namespace_lease: None,
        })))
    }

    pub fn factory(&self) -> SocketFactory {
        SocketFactory(self.0.clone())
    }

    #[cfg(target_os = "linux")]
    pub(super) fn restrict_to_controller(&mut self) {
        Arc::get_mut(&mut self.0)
            .expect("unpublished bootstrap generation")
            .controller_only = true;
    }
    pub fn retire(&self) {
        self.0.retire();
    }

    pub fn active(&self) -> bool {
        self.0.state.lock().is_ok_and(|state| state.active)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn retain_namespace(&mut self, lease: Arc<super::owner::Lease>) {
        // Only before handing out any factory. Retired wrappers may still hold
        // FDs, so keep reservation locked until the last such wrapper is gone.
        Arc::get_mut(&mut self.0)
            .expect("unpublished generation")
            .namespace_lease = Some(lease);
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn validate_mesh_sinks(table: u32, bytes: &[u8]) -> io::Result<()> {
    let routes: Vec<serde_json::Value> = serde_json::from_slice(bytes).map_err(invalid_json)?;
    for route in routes {
        let attributes = route
            .as_object()
            .ok_or_else(|| invalid("unknown Mesh sink route"))?;
        let destination = route
            .get("dst")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid("missing Mesh sink prefix"))?;
        let prefix = destination
            .parse::<ipnet::IpNet>()
            .or_else(|_| {
                destination
                    .parse::<std::net::IpAddr>()
                    .map(ipnet::IpNet::from)
            })
            .map_err(|_| invalid("invalid Mesh sink prefix"))?;
        if prefix.prefix_len() == 0
            || prefix != prefix.trunc()
            || !route
                .get("type")
                .is_some_and(|kind| kind.as_str() == Some("unreachable") || number(kind) == Some(7))
            || route
                .get("table")
                .map(table_identity)
                .transpose()?
                .is_some_and(|id| id != table)
            || route
                .get("dev")
                .is_some_and(|dev| dev.as_str() != Some("lo") || prefix.addr().is_ipv4())
            || attributes.keys().any(|key| {
                ![
                    "type", "dst", "table", "protocol", "metric", "flags", "pref", "dev", "scope",
                ]
                .contains(&key.as_str())
            })
        {
            return Err(invalid(
                "Mesh sink table must contain only non-default unreachable prefixes",
            ));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[cfg(any(target_os = "linux", test))]
struct Rule {
    priority: u32,
    #[serde(flatten)]
    attributes: std::collections::BTreeMap<String, serde_json::Value>,
}

#[cfg(any(target_os = "linux", test))]
fn number(value: &serde_json::Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .or_else(|| {
            let text = value.as_str()?;
            text.strip_prefix("0x").map_or_else(
                || text.parse().ok(),
                |hex| u32::from_str_radix(hex, 16).ok(),
            )
        })
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn validate_rules(reservation: &Reservation, bytes: &[u8]) -> io::Result<()> {
    let rules: Vec<Rule> = serde_json::from_slice(bytes).map_err(invalid_json)?;
    let mut guard = 0;
    let mut target = 0;
    let mut priorities = std::collections::HashSet::new();
    for rule in rules {
        let in_block =
            (reservation.first_priority..=reservation.last_priority).contains(&rule.priority);
        if !in_block {
            let a = &rule.attributes;
            let table = a.get("table").map(table_identity).transpose()?;
            if table.is_some_and(|table| {
                reservation.capture_tables.contains(&table)
                    || reservation.mesh_sink_table == Some(table)
            }) {
                return Err(invalid(
                    "owned capture table referenced outside guarded block",
                ));
            }
            if let Some(value) = a.get("fwmark") {
                let _ = number(value).ok_or_else(|| invalid("unknown external fwmark format"))?;
                let mask = a
                    .get("fwmask")
                    .map(number)
                    .unwrap_or(Some(u32::MAX))
                    .ok_or_else(|| invalid("unknown external fwmask format"))?;
                if mask & reservation.mark.mask != 0 {
                    return Err(invalid("external rule owns reserved mark bits"));
                }
            }
            if a.keys().any(|key| {
                !["src", "dst", "table", "protocol", "fwmark", "fwmask"].contains(&key.as_str())
            }) {
                return Err(invalid("unreviewed external RPDB selector"));
            }
            if rule.priority < reservation.first_priority && table != Some(255) {
                return Err(invalid(
                    "preceding system policy needs explicit capture-order integration",
                ));
            }
            continue;
        }
        if !priorities.insert(rule.priority) {
            return Err(invalid("duplicate priority in reserved block"));
        }
        let mut a = rule.attributes;
        if a.remove("src").as_ref().and_then(serde_json::Value::as_str) != Some("all") {
            return Err(invalid("non-default source in reserved rule block"));
        }
        if rule.priority == reservation.first_priority {
            let mark = a.remove("fwmark").as_ref().and_then(number);
            let mask = a.remove("fwmask").as_ref().and_then(number);
            let goto = a.remove("goto").as_ref().and_then(number);
            let interface = a.remove("iif");
            if mark != Some(reservation.mark.value)
                || mask != Some(reservation.mark.mask)
                || goto != Some(reservation.last_priority)
                || interface.as_ref().and_then(serde_json::Value::as_str) != Some("lo")
            {
                return Err(invalid("guard does not match reservation"));
            }
            guard += 1;
        } else if rule.priority == reservation.last_priority {
            if a.remove("nop") != Some(serde_json::Value::Null) {
                return Err(invalid("missing stable NOP target"));
            }
            target += 1;
        } else {
            let table = a.remove("table").as_ref().map(table_identity).transpose()?;
            let is_sink = table.is_some()
                && table == reservation.mesh_sink_table
                && rule.priority == reservation.last_priority - 1;
            if !is_sink
                && (table.is_none_or(|table| !reservation.capture_tables.contains(&table))
                    || a.remove("iif").as_ref().and_then(serde_json::Value::as_str) != Some("lo"))
            {
                return Err(invalid("foreign rule in reserved capture block"));
            }
        }
        // Unknown selectors (not, dst, uid, unresolved, etc.) are fail closed.
        if !a.is_empty() {
            return Err(invalid("unknown attributes in reserved rule block"));
        }
    }
    if guard != 1 || target != 1 {
        return Err(invalid("guard/NOP absent or duplicated"));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn invalid_json(error: serde_json::Error) -> io::Error {
    invalid(error.to_string())
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn table_identity(value: &serde_json::Value) -> io::Result<u32> {
    if let Some(id) = number(value) {
        return Ok(id);
    }
    // Canonical builtin forms are also accepted by the validator for legacy
    // snapshots. prepare always requests numeric output, so custom rt_tables
    // aliases (even aliases shadowing builtin names) cannot change its identity.
    match value.as_str() {
        Some("local") => Ok(255),
        Some("main") => Ok(254),
        Some("default") => Ok(253),
        Some("unspec") => Ok(0),
        _ => Err(invalid(
            "unresolved RPDB table identity; numeric output required",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_mesh_sink_is_reject_only_and_guarded() {
        let reservation = Reservation {
            owner: "sink".into(),
            generation: 1,
            mark: MarkReservation::new(0x80000000, 0x80000000).unwrap(),
            foreign_bits: 0,
            first_priority: 1,
            last_priority: 5,
            capture_tables: vec![10000, 10001],
            mesh_sink_table: Some(10002),
        };
        let rules = serde_json::json!([
            {"priority":0,"src":"all","table":255},
            {"priority":1,"src":"all","fwmark":"0x80000000","fwmask":"0x80000000","iif":"lo","goto":5},
            {"priority":2,"src":"all","table":10000,"iif":"lo"},
            {"priority":4,"src":"all","table":10002},
            {"priority":5,"src":"all","nop":null}
        ]);
        assert!(validate_rules(&reservation, &serde_json::to_vec(&rules).unwrap()).is_ok());
        assert!(validate_mesh_sinks(10002, br#"[{"type":"unreachable","dst":"100.64.0.0/24","protocol":242,"metric":42760,"flags":[]}]"#).is_ok());
        for invalid in [
            br#"[{"type":"unreachable","dst":"default"}]"#.as_slice(),
            br#"[{"dst":"100.64.0.0/24","dev":"tun0"}]"#,
            br#"[{"type":"unreachable","dst":"100.64.0.0/24","encap":{}}]"#,
        ] {
            assert!(validate_mesh_sinks(10002, invalid).is_err());
        }
    }

    #[test]
    fn foreign_bits_survive_and_conflicts_reject() {
        let mark = MarkReservation::new(0x4d570000, 0xffff0000).unwrap();
        assert_eq!(mark.merge(0x20).unwrap(), 0x4d570020);
        assert_eq!(mark.merge(0x4d570020).unwrap(), 0x4d570020);
        assert!(mark.merge(0x11110020).is_err());
        assert!(MarkReservation::new(0, u32::MAX).is_err());
        assert!(MarkReservation::new(0x100, 0xff).is_err());
    }

    #[test]
    fn stable_exact_target_and_no_foreign_selector() {
        let reservation = Reservation {
            owner: "test".into(),
            generation: 1,
            mark: MarkReservation::new(0x4d570000, 0xffff0000).unwrap(),
            foreign_bits: 0,
            first_priority: 18000,
            last_priority: 18003,
            capture_tables: vec![18401],
            mesh_sink_table: None,
        };
        let valid = r#"[{"priority":18000,"src":"all","fwmark":"0x4d570000","fwmask":"0xffff0000","iif":"lo","goto":18003},{"priority":18001,"src":"all","iif":"lo","table":18401},{"priority":18003,"src":"all","nop":null}]"#;
        assert!(validate_rules(&reservation, valid.as_bytes()).is_ok());
        assert!(
            validate_rules(
                &reservation,
                valid.replace("\"goto\":18003", "\"goto\":18004").as_bytes()
            )
            .is_err()
        );
        assert!(
            validate_rules(
                &reservation,
                valid
                    .replace("\"iif\":\"lo\"", "\"iif\":\"lo\",\"not\":true")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(validate_rules(&reservation, valid.replace("18401", "18402").as_bytes()).is_err());
    }

    #[test]
    fn numeric_and_builtin_forms_preserve_staging_and_external_checks() {
        let reservation = Reservation {
            owner: "ur1".into(),
            generation: 1,
            mark: MarkReservation::new(0x4d570000, 0xffff0000).unwrap(),
            foreign_bits: 0,
            first_priority: 18000,
            last_priority: 18003,
            capture_tables: vec![18401, 18402],
            mesh_sink_table: None,
        };
        let guard = serde_json::json!({"priority":18000,"src":"all","fwmark":"0x4d570000","fwmask":"0xffff0000","iif":"lo","goto":18003});
        let nop = serde_json::json!({"priority":18003,"src":"all","nop":null});
        for (local, main, default) in [
            (
                serde_json::json!(255),
                serde_json::json!(254),
                serde_json::json!(253),
            ),
            (
                serde_json::json!("255"),
                serde_json::json!("254"),
                serde_json::json!("253"),
            ),
            (
                serde_json::json!("local"),
                serde_json::json!("main"),
                serde_json::json!("default"),
            ),
        ] {
            let mut rules = vec![
                serde_json::json!({"priority":0,"src":"all","table":local}),
                guard.clone(),
                nop.clone(),
                serde_json::json!({"priority":32766,"src":"all","table":main}),
                serde_json::json!({"priority":32767,"src":"all","table":default}),
                serde_json::json!({"priority":19000,"src":"all","table":"18550","fwmark":"0x20","fwmask":"0xff"}),
            ];
            // Guard-only is legitimate preparation, not capture-applied.
            assert!(validate_rules(&reservation, &serde_json::to_vec(&rules).unwrap()).is_ok());
            rules.push(serde_json::json!({"priority":18001,"src":"all","iif":"lo","table":18401}));
            assert!(validate_rules(&reservation, &serde_json::to_vec(&rules).unwrap()).is_ok());
            // Reserve 18402 has no in-block rule, but every outside reference
            // must reject, regardless of its display form.
            for table in [
                serde_json::json!(18402),
                serde_json::json!("18402"),
                serde_json::json!("mw_capture_two"),
                serde_json::json!("unknown_vpn"),
            ] {
                let mut conflicting = rules.clone();
                conflicting.push(serde_json::json!({"priority":19001,"src":"all","table":table}));
                assert!(
                    validate_rules(&reservation, &serde_json::to_vec(&conflicting).unwrap())
                        .is_err()
                );
            }
            let mut foreign = rules.clone();
            foreign.push(
                serde_json::json!({"priority":19002,"src":"all","table":18551,"uidrange":"1-2"}),
            );
            assert!(validate_rules(&reservation, &serde_json::to_vec(&foreign).unwrap()).is_err());
            foreign = rules.clone();
            foreign.push(serde_json::json!({"priority":19002,"src":"all","table":18551,"fwmark":"0x10000","fwmask":"0xffff0000"}));
            assert!(validate_rules(&reservation, &serde_json::to_vec(&foreign).unwrap()).is_err());
        }
        assert_eq!(table_identity(&serde_json::json!("unspec")).unwrap(), 0);
        assert!(table_identity(&serde_json::json!(true)).is_err());
    }
}
