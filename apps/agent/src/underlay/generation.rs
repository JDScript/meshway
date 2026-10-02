use std::{
    io,
    net::Shutdown,
    sync::{Arc, Mutex, Weak},
};

use serde::Deserialize;
use socket2::{Domain, Protocol, Socket, Type};
use tokio_util::sync::CancellationToken;

use super::{Purpose, SocketFactory, closed, invalid};

#[derive(Clone, Copy, Debug)]
pub(crate) struct MarkReservation {
    value: u32,
    mask: u32,
}

impl MarkReservation {
    pub(crate) fn new(value: u32, mask: u32) -> io::Result<Self> {
        if mask == 0 || value == 0 || value & !mask != 0 {
            return Err(invalid("nonzero mark must fit its nonzero reserved mask"));
        }
        Ok(Self { value, mask })
    }

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
#[derive(Clone, Debug)]
pub(crate) struct Reservation {
    pub(crate) owner: String,
    pub(crate) generation: u64,
    pub(crate) mark: MarkReservation,
    /// Reviewed system/VPN bits from the routing owner, not a device setting.
    pub(crate) foreign_bits: u32,
    pub(crate) first_priority: u32,
    pub(crate) last_priority: u32,
    pub(crate) capture_tables: Vec<u32>,
}

pub(super) struct State {
    pub(super) active: bool,
    sockets: Vec<Registered>,
}

struct Registered {
    socket: Weak<Socket>,
    purpose: Purpose,
}

pub(super) struct Generation {
    pub(super) reservation: Reservation,
    pub(super) state: Mutex<State>,
    pub(super) cancelled: CancellationToken,
}

impl Generation {
    pub(super) fn register(
        &self,
        socket: Arc<Socket>,
        purpose: Purpose,
    ) -> io::Result<Arc<Socket>> {
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
pub(crate) struct PreparedGeneration(Arc<Generation>);

impl Drop for PreparedGeneration {
    fn drop(&mut self) {
        self.0.retire();
    }
}

impl PreparedGeneration {
    pub(crate) async fn prepare(reservation: Reservation) -> io::Result<Self> {
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
        {
            return Err(invalid("invalid internal underlay reservation"));
        }
        for family in ["-4", "-6"] {
            let result = tokio::process::Command::new("ip")
                .args(["-j", family, "rule", "show"])
                .output()
                .await?;
            if !result.status.success() {
                return Err(io::Error::other("cannot inspect RPDB"));
            }
            validate_rules(&reservation, &result.stdout)?;
        }
        // Capability and both address families verified before publishing factory.
        for domain in [Domain::IPV4, Domain::IPV6] {
            let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
            reservation.mark.protect(&socket)?;
        }
        Ok(Self(Arc::new(Generation {
            reservation,
            state: Mutex::new(State {
                active: true,
                sockets: Vec::new(),
            }),
            cancelled: CancellationToken::new(),
        })))
    }

    pub(crate) fn factory(&self) -> SocketFactory {
        SocketFactory(self.0.clone())
    }
    pub(crate) fn retire(&self) {
        self.0.retire();
    }
}

#[derive(Deserialize)]
struct Rule {
    priority: u32,
    #[serde(flatten)]
    attributes: std::collections::BTreeMap<String, serde_json::Value>,
}

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

fn validate_rules(reservation: &Reservation, bytes: &[u8]) -> io::Result<()> {
    let rules: Vec<Rule> = serde_json::from_slice(bytes).map_err(invalid_json)?;
    let mut guard = 0;
    let mut target = 0;
    let mut priorities = std::collections::HashSet::new();
    for rule in rules {
        let in_block =
            (reservation.first_priority..=reservation.last_priority).contains(&rule.priority);
        if !in_block {
            let a = &rule.attributes;
            if a.get("table")
                .and_then(number)
                .is_some_and(|table| reservation.capture_tables.contains(&table))
            {
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
            if rule.priority < reservation.first_priority
                && a.get("table").and_then(serde_json::Value::as_str) != Some("local")
            {
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
            let table = a.remove("table").as_ref().and_then(number);
            if table.is_none_or(|table| !reservation.capture_tables.contains(&table))
                || a.remove("iif").as_ref().and_then(serde_json::Value::as_str) != Some("lo")
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

fn invalid_json(error: serde_json::Error) -> io::Error {
    invalid(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
