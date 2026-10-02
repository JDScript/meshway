//! Namespace RPDB owner. This is an internal preparation API, not a CLI setting.
//! Drop never opens a fall-through window: committed sinks and their journal
//! survive process death. Explicit release requires an intentional relinquish.
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr as UnixAddr, UnixDatagram},
    },
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{io::AsyncReadExt, process::Command};

use super::{
    ControllerChannel, MarkReservation, PreparedGeneration, Reservation, SystemDnsSnapshot,
    generation::{table_identity, validate_rules},
    invalid,
};

/// Supplied from enrolled identity and the selected network, before capture.
/// The directory is existing private Agent state, not a caller-chosen RPDB ID.
pub(crate) struct Preparation<'a> {
    pub(crate) state_directory: &'a Path,
    pub(crate) identity: &'a str,
    pub(crate) network: &'a str,
}

/// Only a canonical prefix and an optional *already owned* TUN. Neither a
/// gateway nor a physical-interface bypass is expressible through this API.
pub(crate) struct CapturePrefix<'a> {
    pub(crate) prefix: IpNet,
    pub(crate) tun: Option<&'a TunBinding>,
}

/// Created from the actual owned TUN device; callers cannot fabricate an
/// interface name/index pair from policy or an external request.
pub(crate) struct TunBinding {
    name: String,
    index: u32,
}

impl TunBinding {
    pub(crate) fn from_created_device(device: &tun_rs::AsyncDevice) -> io::Result<Self> {
        let name = device.name()?;
        verify_tun(&name)?;
        let index =
            std::fs::read_to_string(Path::new("/sys/class/net").join(&name).join("ifindex"))?
                .trim()
                .parse()
                .map_err(|_| invalid("unknown owned TUN identity"))?;
        Ok(Self { name, index })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Namespace {
    boot: String,
    /// None only permits decoding a legacy journal for explicit refusal.
    #[serde(default)]
    cookie: Option<u64>,
    device: u64,
    inode: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
enum Resource {
    Nop {
        family: u8,
        priority: u32,
    },
    Guard {
        family: u8,
        priority: u32,
        mark: u32,
        target: u32,
    },
    Capture {
        family: u8,
        priority: u32,
        table: u32,
    },
    MeshLookup {
        family: u8,
        priority: u32,
        table: u32,
    },
    Sink {
        family: u8,
        table: u32,
        prefix: String,
    },
    Tun {
        family: u8,
        table: u32,
        prefix: String,
        name: String,
        index: u32,
    },
}

impl Resource {
    fn occupies_slot(&self, object: &Value) -> io::Result<bool> {
        let expected = self.expected();
        if self.is_rule() {
            return Ok(object.get("priority") == expected.get("priority"));
        }
        let table = object
            .get("table")
            .map(table_identity)
            .transpose()?
            .unwrap_or(254);
        let expected_table = expected
            .get("table")
            .map(table_identity)
            .transpose()?
            .ok_or_else(|| invalid("missing expected table"))?;
        Ok(table == expected_table
            && object.get("dst") == expected.get("dst")
            && object.get("metric") == expected.get("metric"))
    }
    fn family(&self) -> u8 {
        match self {
            Self::Nop { family, .. }
            | Self::Guard { family, .. }
            | Self::Capture { family, .. }
            | Self::MeshLookup { family, .. }
            | Self::Sink { family, .. }
            | Self::Tun { family, .. } => *family,
        }
    }

    fn args(&self, action: &str) -> Vec<String> {
        let mut args = vec![format!("-{}", self.family())];
        match self {
            Self::Nop { priority, .. } => args.extend([
                "rule".into(),
                action.into(),
                "priority".into(),
                priority.to_string(),
                "nop".into(),
            ]),
            Self::Guard {
                priority,
                mark,
                target,
                ..
            } => args.extend([
                "rule".into(),
                action.into(),
                "priority".into(),
                priority.to_string(),
                "fwmark".into(),
                format!("{mark:#x}/{mark:#x}"),
                "iif".into(),
                "lo".into(),
                "goto".into(),
                target.to_string(),
            ]),
            Self::Capture {
                priority, table, ..
            } => args.extend([
                "rule".into(),
                action.into(),
                "priority".into(),
                priority.to_string(),
                "iif".into(),
                "lo".into(),
                "lookup".into(),
                table.to_string(),
            ]),
            Self::MeshLookup {
                priority, table, ..
            } => args.extend([
                "rule".into(),
                action.into(),
                "priority".into(),
                priority.to_string(),
                "lookup".into(),
                table.to_string(),
            ]),
            Self::Sink { table, prefix, .. } => args.extend([
                "route".into(),
                action.into(),
                "unreachable".into(),
                prefix.clone(),
                "table".into(),
                table.to_string(),
                "metric".into(),
                "42760".into(),
                "proto".into(),
                "242".into(),
            ]),
            Self::Tun {
                table,
                prefix,
                name,
                ..
            } => args.extend([
                "route".into(),
                action.into(),
                prefix.clone(),
                "table".into(),
                table.to_string(),
                "dev".into(),
                name.clone(),
                "metric".into(),
                "42759".into(),
                "proto".into(),
                "242".into(),
            ]),
        }
        args
    }

    fn is_rule(&self) -> bool {
        matches!(
            self,
            Self::Nop { .. } | Self::Guard { .. } | Self::Capture { .. } | Self::MeshLookup { .. }
        )
    }

    fn expected(&self) -> Value {
        match self {
            Self::Nop { priority, .. } => {
                serde_json::json!({"priority":priority,"src":"all","nop":null})
            }
            Self::Guard {
                priority,
                mark,
                target,
                ..
            } => {
                serde_json::json!({"priority":priority,"src":"all","fwmark":format!("{mark:#x}"),"fwmask":format!("{mark:#x}"),"iif":"lo","goto":target})
            }
            Self::Capture {
                priority, table, ..
            } => serde_json::json!({"priority":priority,"src":"all","iif":"lo","table":table}),
            Self::MeshLookup {
                priority, table, ..
            } => serde_json::json!({"priority":priority,"src":"all","table":table}),
            Self::Sink { table, prefix, .. } => {
                let mut value = serde_json::json!({"type":"unreachable","dst":route_destination(prefix),"table":table,"protocol":242,"metric":42760,"flags":[]});
                if self.family() == 6 {
                    value["pref"] = "medium".into();
                    // IPv6 FIB reject routes are backed by the namespace's lo
                    // device; this is rejection, never an underlay interface pin.
                    value["dev"] = "lo".into();
                }
                value
            }
            Self::Tun {
                table,
                prefix,
                name,
                ..
            } => {
                let mut value = serde_json::json!({"dst":route_destination(prefix),"table":table,"dev":name,"protocol":242,"metric":42759,"flags":[]});
                if self.family() == 4 {
                    value["scope"] = "link".into();
                } else {
                    value["pref"] = "medium".into();
                }
                value
            }
        }
    }
}

fn route_destination(prefix: &str) -> String {
    match prefix.parse::<IpNet>() {
        Ok(net) if net.prefix_len() == 0 => "default".into(),
        Ok(net) if net.prefix_len() == if net.addr().is_ipv4() { 32 } else { 128 } => {
            net.addr().to_string()
        }
        _ => prefix.into(),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Installed {
    resource: Resource,
    // Exact numeric kernel evidence; unknown attributes are never discarded.
    observed: Value,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u8,
    namespace: Namespace,
    identity: String,
    network: String,
    reservation: Reservation,
    installed: Vec<Installed>,
    // A crash between a kernel mutation and durable evidence is ambiguous.
    // Recovery reports it; it does NOT adopt/delete a matching foreign object.
    pending: Option<Resource>,
    #[serde(default)]
    operation: Option<Operation>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    start: usize,
    resources: Vec<Resource>,
    phase: OperationPhase,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum OperationPhase {
    Install,
    Rollback,
    /// Original nonpersistent bindings cannot return. Keep rejecting resources
    /// and complete their lookups, but retire every runtime Tun in this intent.
    Converge,
}

pub(super) struct Lease {
    scope: Namespace,
    _namespace: File,
    _singleton: UnixDatagram,
    _state_lock: File,
}

impl Namespace {
    fn same_lifetime(&self, other: &Self) -> io::Result<bool> {
        let left = checked_cookie(
            self.cookie
                .ok_or_else(|| invalid("legacy scope has no netns lifetime cookie"))?,
        )?;
        let right = checked_cookie(
            other
                .cookie
                .ok_or_else(|| invalid("current scope has no netns lifetime cookie"))?,
        )?;
        Ok(self.boot == other.boot && left == right)
    }

    fn matching_locator(&self, other: &Self) -> bool {
        self.device == other.device && self.inode == other.inode
    }
}

fn checked_cookie(cookie: u64) -> io::Result<u64> {
    if cookie == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zero SO_NETNS_COOKIE; lifetime identity unavailable",
        ));
    }
    Ok(cookie)
}

fn checked_cookie_result(result: io::Result<u64>) -> io::Result<u64> {
    checked_cookie(result?)
}

fn socket_cookie(socket: &UnixDatagram) -> io::Result<u64> {
    // Fixed dependency's safe Linux wrapper, never SO_COOKIE or inode fallback.
    checked_cookie_result(socket2::SockRef::from(socket).netns_cookie())
}

fn current_namespace(socket: &UnixDatagram) -> io::Result<(Namespace, File)> {
    // /proc/self is the process leader. All normal Agent workers must share its
    // namespace; a socket created by another thread scope cannot authorize it.
    let file = File::open("/proc/thread-self/ns/net")?;
    let metadata = file.metadata()?;
    let leader = std::fs::metadata("/proc/self/ns/net")?;
    if (metadata.dev(), metadata.ino()) != (leader.dev(), leader.ino()) {
        return Err(invalid(
            "Agent thread and process-leader network namespaces differ",
        ));
    }
    let cookie = socket_cookie(socket)?;
    let witness = UnixDatagram::unbound()?;
    if socket_cookie(&witness)? != cookie {
        return Err(invalid(
            "socket does not belong to current thread netns lifetime",
        ));
    }
    let after = std::fs::metadata("/proc/thread-self/ns/net")?;
    if (after.dev(), after.ino()) != (metadata.dev(), metadata.ino()) {
        return Err(invalid("namespace changed during lifetime preparation"));
    }
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_owned();
    if boot.is_empty() {
        return Err(invalid("missing boot binding for netns cookie"));
    }
    Ok((
        Namespace {
            boot,
            cookie: Some(cookie),
            device: metadata.dev(),
            inode: metadata.ino(),
        },
        file,
    ))
}

impl Lease {
    pub(super) fn check_current(&self) -> io::Result<()> {
        let (current, _) = current_namespace(&self._singleton)?;
        if !self.scope.same_lifetime(&current)? || !self.scope.matching_locator(&current) {
            return Err(invalid(
                "namespace lease does not match current Agent thread scope",
            ));
        }
        let pinned = self._namespace.metadata()?;
        if (pinned.dev(), pinned.ino()) != (current.device, current.inode) {
            return Err(invalid("pinned namespace locator changed"));
        }
        Ok(())
    }
}

pub(crate) struct LinuxUnderlay {
    lease: Arc<Lease>,
    directory: PathBuf,
    journal: Journal,
    #[cfg(test)]
    checkpoint: Option<Arc<InstallCheckpoint>>,
}

#[cfg(test)]
struct InstallCheckpoint {
    after: usize,
    crash: bool,
    reached: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[cfg(test)]
fn crash_checkpoint(kind: &str) -> Option<Arc<InstallCheckpoint>> {
    if std::env::var("MESHWAY_UNDERLAY_TEST_OPERATION")
        .ok()
        .as_deref()
        != Some(kind)
    {
        return None;
    }
    let after = std::env::var("MESHWAY_UNDERLAY_TEST_COMMIT")
        .ok()?
        .parse()
        .ok()?;
    Some(Arc::new(InstallCheckpoint {
        after,
        crash: true,
        reached: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    }))
}

#[cfg(test)]
fn scope_crash_checkpoint(directory: &Path, boundary: &str) {
    if directory.join("scope-exit-armed").exists()
        && std::env::var("MESHWAY_UNDERLAY_TEST_SCOPE_EXIT")
            .ok()
            .as_deref()
            == Some(boundary)
    {
        println!("scope rollover process exit at {boundary}");
        std::process::exit(0);
    }
}

async fn command(binary: &str, args: &[String]) -> io::Result<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let thread = std::fs::metadata("/proc/thread-self/ns/net")?;
        let leader = std::fs::metadata("/proc/self/ns/net")?;
        if (thread.dev(), thread.ino()) != (leader.dev(), leader.ino()) {
            return Err(invalid(
                "routing command thread scope differs from Agent leader",
            ));
        }
        let mut child = Command::new(binary)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| invalid("missing command stdout"))?
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > 1_048_576 {
            return Err(invalid("routing inspection exceeds bounds"));
        }
        if !child.wait().await?.success() {
            return Err(io::Error::other(format!("{binary} operation failed")));
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "routing operation timed out"))?
}

async fn ip(args: Vec<String>) -> io::Result<Vec<u8>> {
    command("ip", &args).await
}

async fn objects(family: u8, rules: bool) -> io::Result<Vec<Value>> {
    let mut args = vec![
        "-N".into(),
        "-j".into(),
        format!("-{family}"),
        if rules { "rule" } else { "route" }.into(),
        "show".into(),
    ];
    if !rules {
        args.extend(["table".into(), "all".into()]);
    }
    serde_json::from_slice(&ip(args).await?).map_err(|error| invalid(error.to_string()))
}

fn private_directory(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir()
        || meta.permissions().mode() & 0o077 != 0
        || meta.uid() != rustix::process::getuid().as_raw()
    {
        return Err(invalid("underlay state must be an owned private directory"));
    }
    Ok(())
}

fn private_file(path: &Path, create: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.permissions().mode() & 0o077 != 0
        || meta.uid() != rustix::process::getuid().as_raw()
    {
        return Err(invalid(
            "underlay journal/lock must be private and unaliased",
        ));
    }
    Ok(file)
}

fn write_journal(directory: &Path, journal: &Journal) -> io::Result<()> {
    let bytes = serde_json::to_vec(journal).map_err(|error| invalid(error.to_string()))?;
    if bytes.len() > 1_048_576 {
        return Err(invalid("underlay journal exceeds durable recovery budget"));
    }
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let temporary = directory.join(format!(".underlay-{suffix}"));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, directory.join("underlay.json"))?;
        File::open(directory)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn archive_scope(directory: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let archives = directory.join("underlay-scopes");
    match std::fs::DirBuilder::new().mode(0o700).create(&archives) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    private_directory(&archives)?;
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
    let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let destination = archives.join(format!("scope-{name}.json"));
    if destination.try_exists()? {
        return Err(invalid("archive collision; no replacement"));
    }
    std::fs::rename(directory.join("underlay.json"), destination)?;
    File::open(&archives)?.sync_all()?;
    File::open(directory)?.sync_all()
}

fn same_object(actual: &Value, expected: &Value) -> bool {
    // ip -N renders some numeric attributes as numbers and others as hex/text.
    // Only numeric representation differences are normalized, never selectors.
    let mut actual = actual.clone();
    let mut expected = expected.clone();
    for value in [&mut actual, &mut expected] {
        for key in [
            "table", "protocol", "fwmark", "fwmask", "type", "scope", "pref",
        ] {
            if let Some(field) = value.get_mut(key)
                && let Some(number) = numeric_attribute(key, field)
            {
                *field = number.into();
            }
        }
    }
    actual == expected
}

fn numeric_attribute(key: &str, value: &Value) -> Option<u32> {
    match (key, value.as_str()) {
        ("type", Some("unreachable")) => Some(7),
        ("scope", Some("link")) => Some(253),
        ("pref", Some("medium")) => Some(0),
        _ => table_identity(value).ok(),
    }
}

fn reserved_aliases() -> io::Result<BTreeSet<u32>> {
    let mut ids = BTreeSet::new();
    for path in ["/etc/iproute2/rt_tables", "/usr/lib/iproute2/rt_tables"] {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                for line in text.lines() {
                    let mut words = line
                        .split('#')
                        .next()
                        .unwrap_or_default()
                        .split_whitespace();
                    if let Some(id) = words.next() {
                        ids.insert(
                            id.parse()
                                .map_err(|_| invalid("unknown rt_tables reservation"))?,
                        );
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    // Drop-in aliases can reserve empty tables too. Conservative rather than
    // overlooking an external reservation that has no kernel routes yet.
    for path in ["/etc/iproute2/rt_tables.d", "/usr/lib/iproute2/rt_tables.d"] {
        if Path::new(path).try_exists()? && std::fs::read_dir(path)?.next().is_some() {
            return Err(invalid(
                "rt_tables drop-in reservations require explicit integration",
            ));
        }
    }
    Ok(ids)
}

async fn audit_marks() -> io::Result<()> {
    // Includes native nft and iptables-nft rules. Refuse unknown/legacy MARK
    // state instead of pretending RPDB inspection proves mark-bit ownership.
    let nft = command("nft", &["-j".into(), "list".into(), "ruleset".into()]).await?;
    let value: Value = serde_json::from_slice(&nft).map_err(|error| invalid(error.to_string()))?;
    fn mark_key(value: &Value) -> bool {
        match value {
            Value::Object(map) => map
                .iter()
                .any(|(key, value)| key.contains("mark") || value == "mark" || mark_key(value)),
            Value::Array(values) => values.iter().any(mark_key),
            _ => false,
        }
    }
    if mark_key(&value) {
        return Err(invalid("netfilter mark use requires explicit integration"));
    }
    for binary in ["iptables-legacy-save", "ip6tables-legacy-save"] {
        let bytes = command(binary, &[]).await?;
        if bytes.iter().any(|byte| !byte.is_ascii_whitespace()) {
            return Err(invalid(
                "legacy netfilter state requires explicit integration",
            ));
        }
    }
    Ok(())
}

fn select_reservation(
    rules: &[Value],
    routes: &[Value],
    aliases: &BTreeSet<u32>,
    owner: String,
) -> io::Result<Reservation> {
    let mut used_bits = 0u32;
    let mut used_tables = aliases.clone();
    let mut priorities = BTreeSet::new();
    for rule in rules {
        let priority = rule
            .get("priority")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| invalid("unknown RPDB priority"))?;
        priorities.insert(priority);
        if let Some(table) = rule.get("table") {
            used_tables.insert(table_identity(table)?);
        }
        if rule.get("fwmark").is_some() {
            let mask = rule
                .get("fwmask")
                .map(table_identity)
                .transpose()?
                .unwrap_or(u32::MAX);
            used_bits |= mask;
        }
    }
    for route in routes {
        used_tables.insert(
            route
                .get("table")
                .map(table_identity)
                .transpose()?
                .unwrap_or(254),
        );
    }
    let mark = (0..32)
        .rev()
        .map(|bit| 1u32 << bit)
        .find(|bit| bit & used_bits == 0)
        .ok_or_else(|| invalid("no unreserved mark bit"))?;
    let first = (1..32762)
        .find(|first| (*first..=*first + 4).all(|p| !priorities.contains(&p)))
        .ok_or_else(|| invalid("no empty contiguous RPDB block"))?;
    let tables: Vec<_> = (10000..60000)
        .filter(|id| !used_tables.contains(id))
        .take(3)
        .collect();
    if tables.len() != 3 {
        return Err(invalid("no unreserved capture tables"));
    }
    let reservation = Reservation {
        owner,
        generation: 1,
        mark: MarkReservation::new(mark, mark)?,
        foreign_bits: 0,
        first_priority: first,
        last_priority: first + 4,
        capture_tables: tables[..2].to_vec(),
        mesh_sink_table: Some(tables[2]),
    };
    // Validate all original foreign selectors/order with the existing reviewed
    // validator, using synthetic guard-only staging. No kernel changes yet.
    let mut staged = rules.to_vec();
    staged.extend([
        Resource::Nop {
            family: 4,
            priority: first + 4,
        }
        .expected(),
        Resource::Guard {
            family: 4,
            priority: first,
            mark,
            target: first + 4,
        }
        .expected(),
    ]);
    // Combined IPv4/IPv6 contains duplicate standard priorities, harmless outside
    // the block; selected block is empty in BOTH families.
    validate_rules(
        &reservation,
        &serde_json::to_vec(&staged).map_err(|error| invalid(error.to_string()))?,
    )?;
    Ok(reservation)
}

impl LinuxUnderlay {
    async fn owned_ip(&self, args: Vec<String>) -> io::Result<Vec<u8>> {
        self.lease.check_current()?;
        ip(args).await
    }

    pub(crate) async fn prepare(input: Preparation<'_>) -> io::Result<Self> {
        if input.identity.is_empty()
            || input.network.is_empty()
            || input.identity.len() > 256
            || input.network.len() > 256
        {
            return Err(invalid("missing or oversized enrolled identity binding"));
        }
        private_directory(input.state_directory)?;
        // Linux abstract AF_UNIX names belong to the network namespace, not a
        // mount view. The fixed singleton cannot be bypassed with private /run,
        // a different node ID or another state directory. Safe std APIs only.
        let singleton = UnixDatagram::bind_addr(&UnixAddr::from_abstract_name(
            b"meshway.underlay.rpdb-owner.v1",
        )?)?;
        let (namespace, namespace_file) = current_namespace(&singleton)?;
        let state_lock = private_file(&input.state_directory.join("underlay.lock"), true)?;
        state_lock
            .try_lock()
            .map_err(|error| io::Error::other(format!("underlay state busy: {error}")))?;
        let lease = Arc::new(Lease {
            scope: namespace.clone(),
            _namespace: namespace_file,
            _singleton: singleton,
            _state_lock: state_lock,
        });
        audit_marks().await?;
        let path = input.state_directory.join("underlay.json");
        let previous = match private_file(&path, false) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(1_048_577).read_to_end(&mut bytes)?;
                if bytes.len() > 1_048_576 {
                    return Err(invalid("underlay journal exceeds bounds"));
                }
                Some(
                    serde_json::from_slice::<Journal>(&bytes)
                        .map_err(|error| invalid(error.to_string()))?,
                )
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if let Some(journal) = previous {
            if journal.version != 3
                || journal.namespace.cookie.is_none()
                || journal.namespace.cookie == Some(0)
                || journal.namespace.boot.is_empty()
            {
                return Err(invalid(
                    "unverified legacy/invalid journal lifetime; preserve journal and resources for explicit migration",
                ));
            }
            if journal.identity != input.identity || journal.network != input.network {
                return Err(invalid(
                    "underlay journal namespace/identity mismatch; no adoption",
                ));
            }
            if !journal.namespace.same_lifetime(&namespace)? {
                // Fresh scope is never recovery of the old permissions. Audit
                // and allocation preflight precede durable archival; no old
                // resource shape is adopted or removed in this namespace.
                let mut rules = objects(4, true).await?;
                rules.extend(objects(6, true).await?);
                let mut routes = objects(4, false).await?;
                routes.extend(objects(6, false).await?);
                let candidate = select_reservation(
                    &rules,
                    &routes,
                    &reserved_aliases()?,
                    format!("{}/{}", input.network, input.identity),
                )?;
                for domain in [socket2::Domain::IPV4, socket2::Domain::IPV6] {
                    candidate.mark.protect(&socket2::Socket::new(
                        domain,
                        socket2::Type::STREAM,
                        Some(socket2::Protocol::TCP),
                    )?)?;
                }
                lease.check_current()?;
                archive_scope(input.state_directory)?;
                #[cfg(test)]
                scope_crash_checkpoint(input.state_directory, "archived");
            } else {
                if !journal.namespace.matching_locator(&namespace) {
                    return Err(invalid(
                        "same cookie but inconsistent namespace locator evidence",
                    ));
                }
                if journal.pending.is_some() {
                    return Err(invalid(
                        "ambiguous interrupted RPDB mutation; manual ownership reconciliation required",
                    ));
                }
                let mut owner = Self {
                    lease,
                    directory: input.state_directory.into(),
                    journal,
                    #[cfg(test)]
                    checkpoint: None,
                };
                owner.recover_operation().await?;
                if owner.journal.installed.len() < 4
                    && owner.journal.installed.iter().all(|item| {
                        matches!(item.resource, Resource::Nop { .. } | Resource::Guard { .. })
                    })
                {
                    owner.rollback_partial_reservation().await?;
                    // Known committed partial prepare is cleaned exactly. The caller
                    // retries preparation, never receives a half-ready capability.
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "interrupted reservation rolled back exactly; retry preparation",
                    ));
                }
                owner.verify().await?;
                // Recovery only restores the protected capability. Old packet/TUN
                // generations are never revived; their active routes are withdrawn.
                owner.withdraw_tun().await?;
                return Ok(owner);
            }
        }
        let mut rules = objects(4, true).await?;
        rules.extend(objects(6, true).await?);
        let mut routes = objects(4, false).await?;
        routes.extend(objects(6, false).await?);
        let reservation = select_reservation(
            &rules,
            &routes,
            &reserved_aliases()?,
            format!("{}/{}", input.network, input.identity),
        )?;
        let journal = Journal {
            version: 3,
            namespace,
            identity: input.identity.into(),
            network: input.network.into(),
            reservation,
            installed: Vec::new(),
            pending: None,
            operation: None,
        };
        lease.check_current()?;
        write_journal(input.state_directory, &journal)?;
        #[cfg(test)]
        scope_crash_checkpoint(input.state_directory, "fresh-journal");
        let mut owner = Self {
            lease,
            directory: input.state_directory.into(),
            journal,
            #[cfg(test)]
            checkpoint: crash_checkpoint("reservation"),
        };
        let first = owner.journal.reservation.first_priority;
        let last = owner.journal.reservation.last_priority;
        let mark = owner.journal.reservation.mark.merge(0)?;
        // Install stable target in both families before either guard. A failure
        // rolls back exactly the mutations evidenced by this live transaction.
        let resources = vec![
            Resource::Nop {
                family: 4,
                priority: last,
            },
            Resource::Nop {
                family: 6,
                priority: last,
            },
            Resource::Guard {
                family: 4,
                priority: first,
                mark,
                target: last,
            },
            Resource::Guard {
                family: 6,
                priority: first,
                mark,
                target: last,
            },
        ];
        if let Err(error) = owner.install_transaction(resources).await {
            if owner.journal.installed.is_empty() && owner.journal.pending.is_none() {
                std::fs::remove_file(owner.directory.join("underlay.json"))?;
                File::open(&owner.directory)?.sync_all()?;
            }
            return Err(error);
        }
        owner.verify().await?;
        Ok(owner)
    }

    pub(crate) async fn socket_generation(&self) -> io::Result<PreparedGeneration> {
        self.verify().await?;
        let mut reservation = self.journal.reservation.clone();
        let mut identity = [0; 8];
        getrandom::fill(&mut identity).map_err(|error| io::Error::other(error.to_string()))?;
        reservation.generation = u64::from_ne_bytes(identity);
        let mut generation = PreparedGeneration::prepare(reservation).await?;
        generation.retain_namespace(self.lease.clone());
        Ok(generation)
    }

    pub(crate) async fn controller_channel(
        &self,
        origin: reqwest::Url,
    ) -> io::Result<ControllerChannel> {
        let generation = self.socket_generation().await?;
        ControllerChannel::from_prepared(generation, origin, SystemDnsSnapshot::from_system()?)
    }

    pub(crate) fn system_dns(&self) -> io::Result<SystemDnsSnapshot> {
        SystemDnsSnapshot::from_system()
    }

    async fn rollback_partial_reservation(&mut self) -> io::Result<()> {
        self.validate_journal_resources()?;
        let reservation = &self.journal.reservation;
        for family in [4, 6] {
            let mut rules = objects(family, true).await?;
            for installed in self
                .journal
                .installed
                .iter()
                .filter(|item| item.resource.family() == family)
            {
                if rules
                    .iter()
                    .filter(|rule| same_object(rule, &installed.observed))
                    .count()
                    != 1
                {
                    return Err(invalid(
                        "partial reservation evidence changed; no foreign rollback",
                    ));
                }
            }
            rules.retain(|rule| {
                !self.journal.installed.iter().any(|item| {
                    item.resource.family() == family && same_object(rule, &item.observed)
                })
            });
            rules.extend([
                Resource::Nop {
                    family,
                    priority: reservation.last_priority,
                }
                .expected(),
                Resource::Guard {
                    family,
                    priority: reservation.first_priority,
                    mark: reservation.mark.merge(0)?,
                    target: reservation.last_priority,
                }
                .expected(),
            ]);
            validate_rules(
                reservation,
                &serde_json::to_vec(&rules).map_err(|error| invalid(error.to_string()))?,
            )?;
            if objects(family, false).await?.iter().any(|route| {
                route.get("table").is_some_and(|table| {
                    table_identity(table).is_ok_and(|id| reservation.capture_tables.contains(&id))
                })
            }) {
                return Err(invalid("foreign route in partial reservation; no rollback"));
            }
        }
        while !self.journal.installed.is_empty() {
            self.remove_one(self.journal.installed.len() - 1).await?;
        }
        std::fs::remove_file(self.directory.join("underlay.json"))?;
        File::open(&self.directory)?.sync_all()
    }

    async fn verify(&self) -> io::Result<()> {
        if self.journal.pending.is_some() || self.journal.operation.is_some() {
            return Err(invalid(
                "pending ownership mutation; capability unavailable until reconciliation",
            ));
        }
        self.verify_topology()?;
        self.verify_resources(false).await
    }

    async fn verify_resources(&self, partial_reservation: bool) -> io::Result<()> {
        self.lease.check_current()?;
        self.validate_journal_resources()?;
        let aliases = reserved_aliases()?;
        if self
            .journal
            .reservation
            .capture_tables
            .iter()
            .copied()
            .chain(self.journal.reservation.mesh_sink_table)
            .any(|table| aliases.contains(&table))
        {
            return Err(invalid(
                "external alias reservation overlaps owned numeric table",
            ));
        }
        audit_marks().await?;
        for family in [4, 6] {
            self.lease.check_current()?;
            let rules = objects(family, true).await?;
            let mut checked_rules = rules.clone();
            if partial_reservation {
                for resource in [
                    Resource::Nop {
                        family,
                        priority: self.journal.reservation.last_priority,
                    },
                    Resource::Guard {
                        family,
                        priority: self.journal.reservation.first_priority,
                        mark: self.journal.reservation.mark.merge(0)?,
                        target: self.journal.reservation.last_priority,
                    },
                ] {
                    if !checked_rules
                        .iter()
                        .any(|rule| same_object(rule, &resource.expected()))
                    {
                        checked_rules.push(resource.expected());
                    }
                }
            }
            validate_rules(
                &self.journal.reservation,
                &serde_json::to_vec(&checked_rules).map_err(|error| invalid(error.to_string()))?,
            )?;
            for rule in &rules {
                if rule
                    .get("priority")
                    .and_then(Value::as_u64)
                    .is_some_and(|priority| {
                        (u64::from(self.journal.reservation.first_priority)
                            ..=u64::from(self.journal.reservation.last_priority))
                            .contains(&priority)
                    })
                    && !self.journal.installed.iter().any(|item| {
                        item.resource.family() == family
                            && item.resource.is_rule()
                            && same_object(rule, &item.observed)
                    })
                {
                    return Err(invalid("foreign rule in reserved block; no adoption"));
                }
            }
            let routes = objects(family, false).await?;
            for installed in self
                .journal
                .installed
                .iter()
                .filter(|item| item.resource.family() == family)
            {
                let objects = if installed.resource.is_rule() {
                    &rules
                } else {
                    &routes
                };
                let count = objects
                    .iter()
                    .filter(|object| same_object(object, &installed.observed))
                    .count();
                // Nonpersistent TUN disappears on process death; a missing route
                // is acceptable ONLY for the journal's active TUN, not a sink.
                if count == 0 && matches!(installed.resource, Resource::Tun { .. }) {
                    continue;
                }
                if count != 1 {
                    return Err(invalid(
                        "owned RPDB resource changed/missing/duplicated; no foreign cleanup",
                    ));
                }
            }
            for route in routes {
                let table = route
                    .get("table")
                    .map(table_identity)
                    .transpose()?
                    .unwrap_or(254);
                if (self.journal.reservation.capture_tables.contains(&table)
                    || self.journal.reservation.mesh_sink_table == Some(table))
                    && !self
                        .journal
                        .installed
                        .iter()
                        .any(|item| same_object(&route, &item.observed))
                {
                    return Err(invalid("foreign route in reserved capture table"));
                }
            }
        }
        Ok(())
    }

    async fn install_one(&mut self, resource: Resource) -> io::Result<()> {
        self.lease.check_current()?;
        if let Resource::Tun { name, index, .. } = &resource {
            verify_tun(name)?;
            if link_index(name).await? != *index {
                return Err(invalid(
                    "original TUN ownership no longer exists; no resurrection",
                ));
            }
        }
        for object in objects(resource.family(), resource.is_rule()).await? {
            if resource.occupies_slot(&object)? {
                return Err(invalid(
                    "resource slot already occupied; no foreign replacement",
                ));
            }
        }
        self.journal.pending = Some(resource.clone());
        write_journal(&self.directory, &self.journal)?;
        if let Err(error) = self.owned_ip(resource.args("add")).await {
            // The command may have committed before timeout/exit failure. Only
            // a proven absence clears pending; otherwise preserve ambiguity.
            if error.kind() == io::ErrorKind::TimedOut {
                return Err(error);
            }
            for object in objects(resource.family(), resource.is_rule()).await? {
                if resource.occupies_slot(&object)? {
                    return Err(error);
                }
            }
            self.journal.pending = None;
            write_journal(&self.directory, &self.journal)?;
            return Err(error);
        }
        let objects = objects(resource.family(), resource.is_rule()).await?;
        let matches: Vec<_> = objects
            .iter()
            .filter(|object| same_object(object, &resource.expected()))
            .collect();
        let mut slot_count = 0;
        for object in &objects {
            slot_count += usize::from(resource.occupies_slot(object)?);
        }
        if matches.len() != 1 || slot_count != 1 {
            return Err(invalid(format!(
                "installed resource did not match exact expected attributes: expected {}, inspected {:?}",
                resource.expected(),
                objects
            )));
        }
        let observed = matches[0].clone();
        self.journal
            .installed
            .push(Installed { resource, observed });
        self.journal.pending = None;
        write_journal(&self.directory, &self.journal)
    }

    async fn remove_one(&mut self, index: usize) -> io::Result<()> {
        self.lease.check_current()?;
        self.validate_journal_resources()?;
        let installed = self.journal.installed[index].clone();
        let objects = objects(installed.resource.family(), installed.resource.is_rule()).await?;
        let count = objects
            .iter()
            .filter(|object| same_object(object, &installed.observed))
            .count();
        if count == 0 && matches!(installed.resource, Resource::Tun { .. }) {
            self.journal.installed.remove(index);
            return write_journal(&self.directory, &self.journal);
        }
        if count != 1 {
            return Err(invalid("refusing to remove changed/foreign RPDB resource"));
        }
        if let Resource::Tun {
            ref name, index, ..
        } = installed.resource
            && link_index(name).await? != index
        {
            return Err(invalid("TUN identity changed; no foreign cleanup"));
        }
        self.journal.pending = Some(installed.resource.clone());
        write_journal(&self.directory, &self.journal)?;
        self.owned_ip(installed.resource.args("del")).await?;
        self.journal.installed.remove(index);
        self.journal.pending = None;
        write_journal(&self.directory, &self.journal)
    }

    fn validate_journal_resources(&self) -> io::Result<()> {
        let reservation = &self.journal.reservation;
        if reservation.first_priority == 0
            || reservation.last_priority >= 32766
            || reservation
                .first_priority
                .checked_add(2)
                .is_none_or(|priority| priority > reservation.last_priority)
        {
            return Err(invalid("invalid journal priority reservation"));
        }
        let intent: Vec<_> = self
            .journal
            .operation
            .as_ref()
            .map(|operation| {
                operation
                    .resources
                    .iter()
                    .map(|resource| Installed {
                        resource: resource.clone(),
                        observed: resource.expected(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        for installed in self.journal.installed.iter().chain(&intent) {
            let resource = &installed.resource;
            let bound = match resource {
                Resource::Nop { priority, .. } => *priority == reservation.last_priority,
                Resource::Guard {
                    priority,
                    mark,
                    target,
                    ..
                } => {
                    *priority == reservation.first_priority
                        && *mark == reservation.mark.merge(0)?
                        && *target == reservation.last_priority
                }
                Resource::Capture {
                    priority, table, ..
                } => {
                    *priority > reservation.first_priority
                        && *priority < reservation.last_priority
                        && reservation.capture_tables.contains(table)
                }
                Resource::MeshLookup {
                    priority, table, ..
                } => {
                    Some(*table) == reservation.mesh_sink_table
                        && *priority == reservation.last_priority - 1
                }
                Resource::Sink { table, prefix, .. } => {
                    (reservation.capture_tables.contains(table)
                        || Some(*table) == reservation.mesh_sink_table)
                        && prefix.parse::<IpNet>().is_ok_and(|prefix| {
                            prefix == prefix.trunc()
                                && (Some(*table) != reservation.mesh_sink_table
                                    || prefix.prefix_len() != 0)
                        })
                }
                Resource::Tun { table, prefix, .. } => {
                    reservation.capture_tables.contains(table)
                        && prefix
                            .parse::<IpNet>()
                            .is_ok_and(|prefix| prefix == prefix.trunc())
                }
            };
            if !bound
                || ![4, 6].contains(&resource.family())
                || !same_object(&installed.observed, &resource.expected())
            {
                return Err(invalid(
                    "journal resource is not bound to reservation and exact kernel evidence",
                ));
            }
        }
        Ok(())
    }

    async fn install_transaction(&mut self, resources: Vec<Resource>) -> io::Result<()> {
        if self.journal.pending.is_some() || self.journal.operation.is_some() {
            return Err(invalid(
                "cannot continue an ambiguous ownership transaction",
            ));
        }
        // Check the complete durable projection BEFORE any kernel mutation.
        // Reserve space for numeric/string iproute2 representation differences
        // and the write-ahead pending row; never write a journal we cannot read.
        let mut projected = self.journal.clone();
        projected.operation = Some(Operation {
            start: self.journal.installed.len(),
            resources: resources.clone(),
            phase: OperationPhase::Install,
        });
        projected.pending = resources.last().cloned();
        projected
            .installed
            .extend(resources.iter().map(|resource| Installed {
                resource: resource.clone(),
                observed: resource.expected(),
            }));
        if serde_json::to_vec(&projected)
            .map_err(|error| invalid(error.to_string()))?
            .len()
            > 1_048_576 - 65_536
        {
            return Err(invalid(
                "resource transaction exceeds durable recovery budget",
            ));
        }
        let start = self.journal.installed.len();
        self.journal.operation = projected.operation;
        if let Err(error) = self.validate_operation_intent(
            self.journal
                .operation
                .as_ref()
                .ok_or_else(|| invalid("missing projected intent"))?,
        ) {
            self.journal.operation = None;
            return Err(error);
        }
        write_journal(&self.directory, &self.journal)?;
        #[cfg(test)]
        if let Some(checkpoint) = &self.checkpoint
            && checkpoint.after == 0
        {
            if checkpoint.crash {
                std::process::exit(0);
            }
            checkpoint.reached.notify_one();
            checkpoint.release.notified().await;
        }
        for resource in resources {
            if let Err(error) = self.install_one(resource).await {
                if self.journal.pending.is_some() {
                    return Err(io::Error::other(format!(
                        "{error}; ambiguous mutation journal retained; capability unavailable"
                    )));
                }
                self.journal
                    .operation
                    .as_mut()
                    .ok_or_else(|| invalid("missing operation intent"))?
                    .phase = OperationPhase::Rollback;
                write_journal(&self.directory, &self.journal)?;
                while self.journal.installed.len() > start {
                    let last = self.journal.installed.len() - 1;
                    self.remove_one(last).await.map_err(|rollback| {
                        io::Error::other(format!("{error}; rollback failed: {rollback}"))
                    })?;
                }
                self.finish_operation().await?;
                return Err(error);
            }
            #[cfg(test)]
            if let Some(checkpoint) = &self.checkpoint
                && self.journal.installed.len() - start == checkpoint.after
            {
                if checkpoint.crash {
                    std::process::exit(0);
                }
                checkpoint.reached.notify_one();
                checkpoint.release.notified().await;
            }
        }
        self.finish_operation().await
    }

    async fn finish_operation(&mut self) -> io::Result<()> {
        let intent = self
            .journal
            .operation
            .as_ref()
            .ok_or_else(|| invalid("missing full operation intent"))?;
        let expected: Vec<_> = intent
            .resources
            .iter()
            .filter(|resource| {
                intent.phase != OperationPhase::Converge
                    || !matches!(resource, Resource::Tun { .. })
            })
            .collect();
        if self.journal.pending.is_some()
            || intent.start > self.journal.installed.len()
            || (intent.phase != OperationPhase::Rollback
                && (self.journal.installed.len() - intent.start != expected.len()
                    || self.journal.installed[intent.start..]
                        .iter()
                        .zip(&expected)
                        .any(|(installed, expected)| &installed.resource != *expected)))
            || (intent.phase == OperationPhase::Rollback
                && self.journal.installed.len() != intent.start)
        {
            return Err(invalid("whole operation resource set is not committed"));
        }
        let empty_rollback = self.journal.installed.is_empty()
            && self.journal.operation.as_ref().is_some_and(|operation| {
                operation.start == 0 && operation.phase == OperationPhase::Rollback
            });
        if !empty_rollback {
            self.verify_topology()?;
        }
        self.verify_resources(empty_rollback).await?;
        let operation = self.journal.operation.take();
        if let Err(error) = write_journal(&self.directory, &self.journal) {
            self.journal.operation = operation;
            return Err(error);
        }
        Ok(())
    }

    /// Resolve only a fully evidenced resource-level prefix of the durable
    /// operation. Ambiguous single mutations remain terminal. This never
    /// reconstructs a vanished TUN, old socket or PacketGeneration.
    pub(crate) async fn recover_operation(&mut self) -> io::Result<()> {
        if self.journal.pending.is_some() {
            return Err(invalid(
                "ambiguous mutation requires ownership reconciliation",
            ));
        }
        let Some(mut operation) = self.journal.operation.clone() else {
            return Ok(());
        };
        self.validate_operation_intent(&operation)?;
        if operation.start > self.journal.installed.len() {
            return Err(invalid("invalid operation baseline"));
        }
        let completed = self.journal.installed.len() - operation.start;
        if operation.phase != OperationPhase::Converge
            && (completed > operation.resources.len()
                || self.journal.installed[operation.start..]
                    .iter()
                    .zip(&operation.resources)
                    .any(|(installed, resource)| &installed.resource != resource))
        {
            return Err(invalid(
                "operation does not match its exact owned resource prefix",
            ));
        }
        self.verify_resources(true).await?;
        if operation.phase == OperationPhase::Install {
            for resource in &operation.resources {
                if let Resource::Tun { name, index, .. } = resource
                    && binding_unavailable(name, *index).await?
                {
                    operation.phase = OperationPhase::Converge;
                    self.journal.operation = Some(operation.clone());
                    write_journal(&self.directory, &self.journal)?;
                    break;
                }
            }
        }
        match operation.phase {
            OperationPhase::Install => {
                for resource in operation.resources.into_iter().skip(completed) {
                    self.install_one(resource).await?;
                }
                self.finish_operation().await
            }
            OperationPhase::Rollback => {
                while self.journal.installed.len() > operation.start {
                    self.remove_one(self.journal.installed.len() - 1).await?;
                }
                self.finish_operation().await
            }
            OperationPhase::Converge => {
                let expected: Vec<_> = operation
                    .resources
                    .iter()
                    .filter(|resource| !matches!(resource, Resource::Tun { .. }))
                    .cloned()
                    .collect();
                let kept: Vec<_> = self.journal.installed[operation.start..]
                    .iter()
                    .filter(|installed| !matches!(installed.resource, Resource::Tun { .. }))
                    .map(|installed| &installed.resource)
                    .collect();
                if kept.len() > expected.len()
                    || kept
                        .iter()
                        .zip(&expected)
                        .any(|(actual, expected)| *actual != expected)
                    || self.journal.installed[operation.start..]
                        .iter()
                        .any(|installed| !operation.resources.contains(&installed.resource))
                {
                    return Err(invalid("convergence does not match original owned intent"));
                }
                for index in (operation.start..self.journal.installed.len()).rev() {
                    if matches!(self.journal.installed[index].resource, Resource::Tun { .. }) {
                        self.remove_one(index).await?;
                    }
                }
                let completed = self.journal.installed.len() - operation.start;
                for resource in expected.into_iter().skip(completed) {
                    self.install_one(resource).await?;
                }
                self.finish_operation().await
            }
        }
    }

    fn validate_operation_intent(&self, operation: &Operation) -> io::Result<()> {
        if operation.start > self.journal.installed.len() || operation.resources.is_empty() {
            return Err(invalid("invalid operation baseline/empty intent"));
        }
        if operation.start == 0 {
            let first = self.journal.reservation.first_priority;
            let last = self.journal.reservation.last_priority;
            let mark = self.journal.reservation.mark.merge(0)?;
            let reservation = vec![
                Resource::Nop {
                    family: 4,
                    priority: last,
                },
                Resource::Nop {
                    family: 6,
                    priority: last,
                },
                Resource::Guard {
                    family: 4,
                    priority: first,
                    mark,
                    target: last,
                },
                Resource::Guard {
                    family: 6,
                    priority: first,
                    mark,
                    target: last,
                },
            ];
            if operation.resources != reservation || operation.phase == OperationPhase::Converge {
                return Err(invalid("start=0 is restricted to initial reservation"));
            }
        } else {
            self.verify_topology_of(&self.journal.installed[..operation.start])?;
        }
        let mut projected = self.journal.clone();
        projected.installed.truncate(operation.start);
        projected
            .installed
            .extend(operation.resources.iter().map(|resource| Installed {
                resource: resource.clone(),
                observed: resource.expected(),
            }));
        projected.pending = operation.resources.last().cloned();
        if serde_json::to_vec(&projected)
            .map_err(|error| invalid(error.to_string()))?
            .len()
            > 1_048_576 - 65_536
        {
            return Err(invalid("stored intent exceeds durable recovery budget"));
        }
        // Validate full original Install topology too, even while rolling back.
        self.verify_topology_of(&projected.installed)
    }

    fn verify_topology(&self) -> io::Result<()> {
        self.verify_topology_of(&self.journal.installed)
    }

    fn verify_topology_of(&self, installed: &[Installed]) -> io::Result<()> {
        let resources: Vec<_> = installed.iter().map(|item| &item.resource).collect();
        for family in [4, 6] {
            if resources.iter().filter(|resource| matches!(resource, Resource::Nop { family: f, .. } if *f == family)).count() != 1
                || resources.iter().filter(|resource| matches!(resource, Resource::Guard { family: f, .. } if *f == family)).count() != 1 {
                return Err(invalid("incomplete dual-stack guard reservation"));
            }
        }
        for resource in &resources {
            match resource {
                Resource::Sink { family, table, .. } if Some(*table) == self.journal.reservation.mesh_sink_table => {
                    if resources.iter().filter(|resource| matches!(resource, Resource::MeshLookup { family: f, table: t, .. } if f == family && t == table)).count() != 1 { return Err(invalid("Mesh sink lacks its effective lookup")); }
                },
                Resource::Sink { table, .. } | Resource::Tun { table, .. } | Resource::Capture { table, .. } => {
                    for family in [4, 6] {
                        if resources.iter().filter(|resource| matches!(resource, Resource::Capture { family: f, table: t, .. } if *f == family && t == table)).count() != 1 { return Err(invalid("capture manifest requires both address-family lookups")); }
                    }
                    if !resources.iter().any(|resource| matches!(resource, Resource::Sink { table: t, .. } if t == table)) { return Err(invalid("capture lookup lacks owned sinks")); }
                    if let Resource::Tun { family, prefix, .. } = resource
                        && !resources.iter().any(|resource| matches!(resource, Resource::Sink { family: f, table: t, prefix: p } if f == family && t == table && p == prefix)) { return Err(invalid("active TUN lacks its exact persistent sink")); }
                },
                Resource::MeshLookup { family, table, .. }
                    if !resources.iter().any(|resource| matches!(resource, Resource::Sink { family: f, table: t, .. } if f == family && t == table)) => { return Err(invalid("Mesh lookup lacks owned rejecting prefixes")); },
                _ => {},
            }
        }
        Ok(())
    }

    /// Publish only after normal mainloop installed controller/carrier owners.
    /// Every prefix receives a persistent unreachable sink before lookup. Drop,
    /// lease withdrawal and process death retain that sink. No legacy route is
    /// adopted or removed here; migration must present its real ownership proof.
    pub(crate) async fn capture(&mut self, prefixes: &[CapturePrefix<'_>]) -> io::Result<()> {
        self.recover_operation().await?;
        self.verify().await?;
        if self
            .journal
            .installed
            .iter()
            .any(|item| matches!(item.resource, Resource::Capture { .. }))
        {
            let table = self.journal.reservation.capture_tables[0];
            let sinks: BTreeSet<_> = self
                .journal
                .installed
                .iter()
                .filter_map(|item| match &item.resource {
                    Resource::Sink {
                        family,
                        table: t,
                        prefix,
                    } if *t == table => Some((*family, prefix.clone())),
                    _ => None,
                })
                .collect();
            let requested: BTreeSet<_> = prefixes
                .iter()
                .map(|prefix| {
                    (
                        if prefix.prefix.addr().is_ipv4() { 4 } else { 6 },
                        prefix.prefix.to_string(),
                    )
                })
                .collect();
            let active: BTreeSet<_> = self
                .journal
                .installed
                .iter()
                .filter_map(|item| match &item.resource {
                    Resource::Tun {
                        family,
                        table: t,
                        prefix,
                        name,
                        index,
                    } if *t == table => Some((*family, prefix.clone(), name.clone(), *index)),
                    _ => None,
                })
                .collect();
            let requested_active: BTreeSet<_> = prefixes
                .iter()
                .filter_map(|prefix| {
                    prefix.tun.map(|tun| {
                        (
                            if prefix.prefix.addr().is_ipv4() { 4 } else { 6 },
                            prefix.prefix.to_string(),
                            tun.name.clone(),
                            tun.index,
                        )
                    })
                })
                .collect();
            if !requested.is_empty()
                && requested.len() == prefixes.len()
                && sinks == requested
                && active == requested_active
            {
                for prefix in prefixes {
                    if let Some(binding) = prefix.tun {
                        verify_tun(&binding.name)?;
                        if link_index(&binding.name).await? != binding.index {
                            return Err(invalid("idempotent TUN binding changed"));
                        }
                    }
                }
                return Ok(());
            }
            return Err(invalid(
                "existing capture differs; withdraw then activate exact owned prefix set",
            ));
        }
        // Do not publish capture without verified dual-stack socket privileges
        // and a supported real bootstrap topology, even if a caller skipped
        // the intended mainloop capability-installation sequence.
        let preflight = self.socket_generation().await?;
        drop(preflight);
        let _ = self.system_dns()?;
        for family in [4, 6] {
            for route in objects(family, false).await? {
                let table = route
                    .get("table")
                    .map(table_identity)
                    .transpose()?
                    .unwrap_or(254);
                if table == 254
                    && route.get("protocol").is_some_and(|value| {
                        table_identity(value).is_ok_and(|protocol| protocol == 242)
                    })
                    && !self.journal.installed.iter().any(|installed| {
                        installed.resource.family() == family
                            && same_object(&route, &installed.observed)
                    })
                {
                    return Err(invalid(
                        "legacy main-table Meshway resources require reliable ownership migration before capture",
                    ));
                }
            }
        }
        if prefixes.is_empty()
            || prefixes.len() > 4096
            || self
                .journal
                .installed
                .iter()
                .any(|item| matches!(item.resource, Resource::Capture { .. }))
        {
            return Err(invalid("capture requires a bounded fresh prefix set"));
        }
        let table = self.journal.reservation.capture_tables[0];
        let priority = self.journal.reservation.first_priority + 1;
        let mut resources = Vec::new();
        let mut seen = BTreeSet::new();
        for prefix in prefixes {
            if prefix.prefix != prefix.prefix.trunc() || !seen.insert(prefix.prefix) {
                return Err(invalid(
                    "capture prefixes must be unique canonical networks",
                ));
            }
            let family = if prefix.prefix.addr().is_ipv4() { 4 } else { 6 };
            resources.push(Resource::Sink {
                family,
                table,
                prefix: prefix.prefix.to_string(),
            });
        }
        for prefix in prefixes {
            if let Some(binding) = prefix.tun {
                verify_tun(&binding.name)?;
                if link_index(&binding.name).await? != binding.index {
                    return Err(invalid("owned TUN binding changed before capture"));
                }
                resources.push(Resource::Tun {
                    family: if prefix.prefix.addr().is_ipv4() { 4 } else { 6 },
                    table,
                    prefix: prefix.prefix.to_string(),
                    name: binding.name.clone(),
                    index: binding.index,
                });
            }
        }
        for family in [4, 6] {
            resources.push(Resource::Capture {
                family,
                priority,
                table,
            });
        }
        self.install_transaction(resources).await?;
        self.verify().await
    }

    /// Mesh-only rejection in the guarded block also fences forwarded
    /// packets (which deliberately do not match local iif-lo profile capture).
    /// This never routes an internal socket into TUN; overlapping underlay and
    /// Mesh addresses remain unreachable. It is not a destination bypass.
    pub(crate) async fn retain_mesh_sink(&mut self, prefix: IpNet) -> io::Result<()> {
        self.recover_operation().await?;
        self.verify().await?;
        if prefix.prefix_len() == 0 || prefix != prefix.trunc() {
            return Err(invalid(
                "Mesh sink requires a canonical non-default network",
            ));
        }
        let family = if prefix.addr().is_ipv4() { 4 } else { 6 };
        let table = self
            .journal
            .reservation
            .mesh_sink_table
            .ok_or_else(|| invalid("missing reject-only Mesh reservation"))?;
        if self.journal.installed.iter().any(|installed| matches!(&installed.resource, Resource::Sink { family: f, table: t, prefix: p } if *f == family && *t == table && p == &prefix.to_string())) { return Ok(()); }
        let mut resources = vec![Resource::Sink {
            family,
            table,
            prefix: prefix.to_string(),
        }];
        if !self.journal.installed.iter().any(|installed| matches!(installed.resource, Resource::MeshLookup { family: f, .. } if f == family)) {
            resources.push(Resource::MeshLookup { family, priority: self.journal.reservation.last_priority - 1, table });
        }
        self.install_transaction(resources).await?;
        self.verify().await
    }

    pub(crate) async fn install_tun(&mut self, prefixes: &[CapturePrefix<'_>]) -> io::Result<()> {
        self.recover_operation().await?;
        if self
            .journal
            .installed
            .iter()
            .any(|installed| matches!(installed.resource, Resource::Capture { .. }))
        {
            self.activate_tun(prefixes).await
        } else {
            self.capture(prefixes).await
        }
    }

    /// Packet owner must fence/join before calling this. It never retires the
    /// independent controller recovery owner or removes Mesh unreachable sinks.
    pub(crate) async fn withdraw_tun(&mut self) -> io::Result<()> {
        self.verify().await?;
        for index in (0..self.journal.installed.len()).rev() {
            if matches!(self.journal.installed[index].resource, Resource::Tun { .. }) {
                self.remove_one(index).await?;
            }
        }
        Ok(())
    }

    /// Reattach a newly owned TUN after packet/carrier withdrawal. Prefixes and
    /// existing capture rules cannot change through this seam: all old sinks
    /// remain in force while preparing, installing, or rolling back new routes.
    pub(crate) async fn activate_tun(&mut self, prefixes: &[CapturePrefix<'_>]) -> io::Result<()> {
        self.recover_operation().await?;
        self.verify().await?;
        if self
            .journal
            .installed
            .iter()
            .any(|item| matches!(item.resource, Resource::Tun { .. }))
        {
            return Err(invalid(
                "old active TUN routes must be withdrawn before replacement",
            ));
        }
        let expected: BTreeSet<_> = self
            .journal
            .installed
            .iter()
            .filter_map(|item| match &item.resource {
                Resource::Sink { prefix, table, .. }
                    if *table == self.journal.reservation.capture_tables[0] =>
                {
                    Some(prefix.clone())
                }
                _ => None,
            })
            .collect();
        let actual: BTreeSet<_> = prefixes
            .iter()
            .map(|prefix| prefix.prefix.to_string())
            .collect();
        if expected.is_empty() || actual != expected || actual.len() != prefixes.len() {
            return Err(invalid(
                "TUN activation must exactly retain the owned sink prefix set",
            ));
        }
        let preflight = self.socket_generation().await?;
        drop(preflight);
        let _ = self.system_dns()?;
        let mut resources = Vec::new();
        for prefix in prefixes {
            let binding = prefix
                .tun
                .ok_or_else(|| invalid("TUN activation requires actual owned device binding"))?;
            verify_tun(&binding.name)?;
            if link_index(&binding.name).await? != binding.index {
                return Err(invalid("new TUN binding changed before activation"));
            }
            resources.push(Resource::Tun {
                family: if prefix.prefix.addr().is_ipv4() { 4 } else { 6 },
                table: self.journal.reservation.capture_tables[0],
                prefix: prefix.prefix.to_string(),
                name: binding.name.clone(),
                index: binding.index,
            });
        }
        self.install_transaction(resources).await?;
        self.verify().await
    }

    /// Explicit cancellation of ALL capture, not lease withdrawal. Caller must
    /// first drop controller and carrier retirement owners (not just factories).
    pub(crate) async fn relinquish(mut self) -> io::Result<()> {
        if Arc::strong_count(&self.lease) != 1 {
            return Err(invalid(
                "live socket owners still hold namespace reservation",
            ));
        }
        self.verify().await?;
        // Capture lookups before guard before stable targets; routes last. There
        // is no business bypass during normal withdraw; this is explicit opt-out.
        for kind in 0..4 {
            for index in (0..self.journal.installed.len()).rev() {
                let selected = match self.journal.installed[index].resource {
                    Resource::Capture { .. } | Resource::MeshLookup { .. } => kind == 0,
                    Resource::Guard { .. } => kind == 1,
                    Resource::Nop { .. } => kind == 2,
                    Resource::Sink { .. } | Resource::Tun { .. } => kind == 3,
                };
                if selected {
                    self.remove_one(index).await?;
                }
            }
        }
        std::fs::remove_file(self.directory.join("underlay.json"))?;
        File::open(&self.directory)?.sync_all()
    }
}

fn verify_tun(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name.len() > 15
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(invalid("invalid owned TUN name"));
    }
    let flags = std::fs::read_to_string(Path::new("/sys/class/net").join(name).join("tun_flags"))?;
    if u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16)
        .map_err(|_| invalid("unknown TUN flags"))?
        & 1
        == 0
    {
        return Err(invalid("capture target must be a TUN"));
    }
    Ok(())
}

async fn binding_unavailable(name: &str, index: u32) -> io::Result<bool> {
    match std::fs::read_to_string(Path::new("/sys/class/net").join(name).join("ifindex")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
        Ok(value) => {
            let actual: u32 = value
                .trim()
                .parse()
                .map_err(|_| invalid("unknown original TUN identity"))?;
            if actual != index {
                return Ok(true);
            }
            verify_tun(name)?;
            Ok(false)
        }
    }
}

#[cfg(test)]
#[path = "owner_review_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "owner_tun_recovery_tests.rs"]
mod tun_recovery_tests;

#[cfg(test)]
#[path = "owner_cookie_tests.rs"]
mod cookie_tests;

async fn link_index(name: &str) -> io::Result<u32> {
    let links: Vec<Value> = serde_json::from_slice(
        &ip(vec![
            "-j".into(),
            "link".into(),
            "show".into(),
            "dev".into(),
            name.into(),
        ])
        .await?,
    )
    .map_err(|error| invalid(error.to_string()))?;
    if links.len() != 1 {
        return Err(invalid("missing unique TUN binding"));
    }
    links[0]
        .get("ifindex")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| invalid("unknown TUN identity"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_preserves_aliases_masks_order_and_unknown_failures() {
        let rules = serde_json::json!([
            {"priority":0,"src":"all","table":255},
            {"priority":19000,"src":"all","table":18550,"fwmark":"0x20","fwmask":"0xff"},
            {"priority":32766,"src":"all","table":254},
            {"priority":32767,"src":"all","table":253}
        ]);
        let rules = rules.as_array().unwrap();
        let aliases = BTreeSet::from([10000]);
        let reservation = select_reservation(rules, &[], &aliases, "owner".into()).unwrap();
        assert_eq!(reservation.first_priority, 1);
        assert_eq!(reservation.capture_tables, [10001, 10002]);
        assert_eq!(reservation.mark.merge(0x20).unwrap(), 0x80000020);
        let mut unknown = rules.clone();
        unknown.push(serde_json::json!({"priority":20000,"src":"all","table":"alias"}));
        assert!(select_reservation(&unknown, &[], &aliases, "owner".into()).is_err());
        unknown = rules.clone();
        unknown
            .push(serde_json::json!({"priority":20000,"src":"all","table":18551,"uidrange":"1-2"}));
        assert!(select_reservation(&unknown, &[], &aliases, "owner".into()).is_err());
        unknown = rules.clone();
        unknown.push(serde_json::json!({"priority":1,"src":"all","table":18551}));
        assert!(select_reservation(&unknown, &[], &aliases, "owner".into()).is_err());
        unknown = rules.clone();
        unknown.push(serde_json::json!({"priority":20000,"src":"all","table":18551,"fwmark":1}));
        assert!(select_reservation(&unknown, &[], &aliases, "owner".into()).is_err());
    }

    #[tokio::test]
    #[ignore = "requires isolated Docker namespace with NET_ADMIN and actual TUNs"]
    async fn namespace_ownership_and_default_docker_dns() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
            Ok("1")
        );
        let state = tempfile::Builder::new()
            .prefix("underlay-owner-")
            .tempdir_in("/run")
            .unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let input = || Preparation {
            state_directory: state.path(),
            identity: "fixture-node",
            network: "fixture-network",
        };
        let original4 = objects(4, true).await.unwrap();
        let original6 = objects(6, true).await.unwrap();
        // Empty alias is externally reserved even with no routes/references.
        std::fs::write(
            "/etc/iproute2/rt_tables",
            "255 local\n254 main\n253 default\n0 unspec\n10000 external_empty\n",
        )
        .unwrap();
        let mut owner = LinuxUnderlay::prepare(input()).await.unwrap();
        assert_eq!(owner.journal.reservation.capture_tables, [10001, 10002]);
        assert!(
            LinuxUnderlay::prepare(input()).await.is_err(),
            "concurrent namespace reservation must reject"
        );
        let other = tempfile::Builder::new().tempdir_in("/run").unwrap();
        std::fs::set_permissions(other.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            LinuxUnderlay::prepare(Preparation {
                state_directory: other.path(),
                identity: "other",
                network: "other"
            })
            .await
            .is_err()
        );
        // Live failed transaction rolls back only its completed route mutation.
        let table = owner.journal.reservation.capture_tables[1];
        let count = owner.journal.installed.len();
        let rollback_error = owner
            .install_transaction(vec![
                Resource::Sink {
                    family: 4,
                    table,
                    prefix: "198.18.71.0/24".into(),
                },
                Resource::Tun {
                    family: 6,
                    table,
                    prefix: "fd71::/64".into(),
                    name: "missing-tun".into(),
                    index: 0,
                },
            ])
            .await
            .unwrap_err();
        assert_eq!(owner.journal.installed.len(), count);
        assert!(owner.journal.pending.is_none(), "{rollback_error}");
        owner.verify().await.unwrap();
        let installed_count = owner.journal.installed.len();
        let budget_tun = TunBinding {
            name: "mwprod6".into(),
            index: link_index("mwprod6").await.unwrap(),
        };
        let oversized: Vec<_> = (0..4096)
            .map(|index| CapturePrefix {
                prefix: format!("fd72:{index:x}::/64").parse().unwrap(),
                tun: Some(&budget_tun),
            })
            .collect();
        assert!(owner.capture(&oversized).await.is_err());
        assert_eq!(owner.journal.installed.len(), installed_count);
        assert!(
            owner.journal.pending.is_none(),
            "budget failure must precede all mutations"
        );
        let aliases = std::fs::read_to_string("/etc/iproute2/rt_tables").unwrap();
        std::fs::write(
            "/etc/iproute2/rt_tables",
            format!("{aliases}10001 foreign_claim\n"),
        )
        .unwrap();
        assert!(owner.socket_generation().await.is_err());
        std::fs::write("/etc/iproute2/rt_tables", aliases).unwrap();
        let controller = owner.socket_generation().await.unwrap();
        let channel = owner
            .controller_channel(reqwest::Url::parse("http://underlay-owner-server:41861/").unwrap())
            .await
            .unwrap();
        let carrier = owner.socket_generation().await.unwrap();
        let controller_factory = controller.factory();
        let old_factory = carrier.factory();
        // Real default Docker DNS, and /etc/hosts before DNS, not manual fixture
        // nameservers. Network is --internal; these names never leave it.
        let resolver = super::super::BootstrapResolver::new(
            controller_factory.clone(),
            owner.system_dns().unwrap(),
        );
        let hosts = resolver
            .resolve(super::super::Purpose::Controller, "OWNER-HOST")
            .await
            .unwrap();
        assert_eq!(
            hosts,
            ["172.30.247.20".parse::<std::net::IpAddr>().unwrap()]
        );
        let docker = resolver
            .resolve(super::super::Purpose::Carrier, "underlay-owner-server")
            .await
            .unwrap();
        assert!(docker.contains(&"172.30.247.20".parse().unwrap()));
        assert!(docker.contains(&"fd74:6d77:247::20".parse().unwrap()));
        let stun_dns = resolver
            .resolve(super::super::Purpose::Stun, "underlay-owner-server")
            .await
            .unwrap();
        assert_eq!(stun_dns, docker);
        owner
            .retain_mesh_sink("198.18.71.0/24".parse().unwrap())
            .await
            .unwrap();
        owner
            .retain_mesh_sink("fd71::/64".parse().unwrap())
            .await
            .unwrap();
        let tun4 = TunBinding {
            name: "mwprod4".into(),
            index: link_index("mwprod4").await.unwrap(),
        };
        let tun6 = TunBinding {
            name: "mwprod6".into(),
            index: link_index("mwprod6").await.unwrap(),
        };
        owner
            .capture(&[
                CapturePrefix {
                    prefix: "172.30.247.20/32".parse().unwrap(),
                    tun: Some(&tun4),
                },
                CapturePrefix {
                    prefix: "fd74:6d77:247::20/128".parse().unwrap(),
                    tun: Some(&tun6),
                },
            ])
            .await
            .unwrap();
        async fn control_request(channel: &ControllerChannel) {
            let error = channel
                .client()
                .with_api_key("fixture-only")
                .enroll_agent(crate::generated::types::EnrollAgentRequest {
                    agent_version: "namespace-owner-fixture".into(),
                    name: "isolated".into(),
                    node_credential: "a".repeat(64),
                })
                .await
                .unwrap_err();
            match error {
                crate::generated::client::ApiOpError::Api(error) => {
                    assert_eq!(error.status, 401);
                    assert!(error.raw_body.is_empty());
                }
                error => panic!("prepared controller channel failed: {error:?}"),
            }
        }
        control_request(&channel).await;
        for address in ["172.30.247.20:41861", "[fd74:6d77:247::20]:41861"] {
            let remote = address.parse().unwrap();
            let socket = controller_factory
                .tcp(super::super::Purpose::Controller, remote)
                .await
                .unwrap();
            drop(socket);
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(150),
                    tokio::net::TcpStream::connect(remote)
                )
                .await
                .is_err()
            );
        }
        for (bind, remote) in [
            ("0.0.0.0:0", "172.30.247.20:41865"),
            ("[::]:0", "[fd74:6d77:247::20]:41865"),
        ] {
            let remote = remote.parse().unwrap();
            let probe = controller_factory
                .udp(super::super::Purpose::Stun, bind.parse().unwrap())
                .unwrap();
            probe.send_to(b"owned-stun-purpose", remote).await.unwrap();
            let mut bytes = [0; 128];
            let (length, source) =
                tokio::time::timeout(Duration::from_secs(2), probe.recv_from(&mut bytes))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(source, remote);
            assert_eq!(&bytes[..length], b"owned-stun-purpose");
            // A separate probe purpose is not published as a listener mapping,
            // peer endpoint, NAT traversal success, or device region.
        }
        // Prepare replacement BEFORE old carrier retirement. PacketGeneration is
        // deliberately absent here: this API cannot revive it or confer access.
        let replacement = owner.socket_generation().await.unwrap();
        carrier.retire();
        control_request(&channel).await;
        assert!(
            old_factory
                .tcp(
                    super::super::Purpose::Carrier,
                    "172.30.247.20:41861".parse().unwrap()
                )
                .await
                .is_err()
        );
        assert!(
            replacement
                .factory()
                .tcp(
                    super::super::Purpose::Carrier,
                    "172.30.247.20:41861".parse().unwrap()
                )
                .await
                .is_ok()
        );
        owner.withdraw_tun().await.unwrap();
        // Protected controller still works, ordinary Mesh route now unreachable.
        assert!(
            controller_factory
                .tcp(
                    super::super::Purpose::Controller,
                    "172.30.247.20:41861".parse().unwrap()
                )
                .await
                .is_ok()
        );
        let lookup = ip(vec![
            "-4".into(),
            "route".into(),
            "get".into(),
            "172.30.247.20".into(),
        ])
        .await;
        assert!(lookup.is_err());
        for (family, destination, source) in [
            (4, "198.18.71.2", "172.30.247.20"),
            (6, "fd71::2", "fd74:6d77:247::20"),
        ] {
            assert!(
                ip(vec![
                    format!("-{family}"),
                    "route".into(),
                    "get".into(),
                    destination.into(),
                    "from".into(),
                    source.into(),
                    "iif".into(),
                    "eth0".into()
                ])
                .await
                .is_err()
            );
        }
        let invalid_tun = TunBinding {
            name: tun4.name.clone(),
            index: tun4.index + 1,
        };
        assert!(
            owner
                .activate_tun(&[
                    CapturePrefix {
                        prefix: "172.30.247.20/32".parse().unwrap(),
                        tun: Some(&invalid_tun)
                    },
                    CapturePrefix {
                        prefix: "fd74:6d77:247::20/128".parse().unwrap(),
                        tun: Some(&tun6)
                    },
                ])
                .await
                .is_err()
        );
        owner.verify().await.unwrap();
        owner
            .activate_tun(&[
                CapturePrefix {
                    prefix: "172.30.247.20/32".parse().unwrap(),
                    tun: Some(&tun4),
                },
                CapturePrefix {
                    prefix: "fd74:6d77:247::20/128".parse().unwrap(),
                    tun: Some(&tun6),
                },
            ])
            .await
            .unwrap();
        owner.withdraw_tun().await.unwrap();
        channel.retire();
        drop(channel);
        drop((
            replacement,
            carrier,
            controller,
            resolver,
            controller_factory,
            old_factory,
        ));
        drop(owner); // simulates restart: all committed sinks/guards survive
        let mut recovered = LinuxUnderlay::prepare(input()).await.unwrap();
        assert!(
            recovered
                .journal
                .installed
                .iter()
                .any(|item| matches!(item.resource, Resource::Sink { .. }))
        );
        recovered.withdraw_tun().await.unwrap();
        drop(recovered);
        assert!(
            LinuxUnderlay::prepare(Preparation {
                state_directory: state.path(),
                identity: "wrong-identity",
                network: "fixture-network"
            })
            .await
            .is_err()
        );
        let mut recovered = LinuxUnderlay::prepare(input()).await.unwrap();
        // Unknown outside selector cannot be silently ignored or removed.
        ip(vec![
            "-4".into(),
            "rule".into(),
            "add".into(),
            "priority".into(),
            "21000".into(),
            "uidrange".into(),
            "1-2".into(),
            "lookup".into(),
            "main".into(),
        ])
        .await
        .unwrap();
        let foreign = objects(4, true)
            .await
            .unwrap()
            .into_iter()
            .find(|rule| rule.get("priority").and_then(Value::as_u64) == Some(21000))
            .unwrap();
        assert!(recovered.socket_generation().await.is_err());
        assert!(objects(4, true).await.unwrap().contains(&foreign));
        ip(vec![
            "-4".into(),
            "rule".into(),
            "del".into(),
            "priority".into(),
            "21000".into(),
            "uidrange".into(),
            "1-2".into(),
            "lookup".into(),
            "main".into(),
        ])
        .await
        .unwrap();
        // Persisted half-commit is refused without shape-based adoption/deletion.
        recovered.journal.pending = Some(Resource::Nop {
            family: 4,
            priority: 22000,
        });
        write_journal(state.path(), &recovered.journal).unwrap();
        drop(recovered);
        assert!(LinuxUnderlay::prepare(input()).await.is_err());
        // Test-only reconciliation: we KNOW no mutation was issued for pending.
        let mut journal: Journal =
            serde_json::from_slice(&std::fs::read(state.path().join("underlay.json")).unwrap())
                .unwrap();
        journal.pending = None;
        write_journal(state.path(), &journal).unwrap();
        let recovered = LinuxUnderlay::prepare(input()).await.unwrap();
        recovered.relinquish().await.unwrap();
        assert_eq!(objects(4, true).await.unwrap(), original4);
        assert_eq!(objects(6, true).await.unwrap(), original6);
        // A crash after journal-committed partial guard installation is not
        // ambiguous: exact evidence permits rollback, then clean retry.
        let mut partial = LinuxUnderlay::prepare(input()).await.unwrap();
        for index in [3, 2, 1] {
            partial.remove_one(index).await.unwrap();
        }
        drop(partial);
        assert!(LinuxUnderlay::prepare(input()).await.is_err());
        assert!(!state.path().join("underlay.json").exists());
        let retried = LinuxUnderlay::prepare(input()).await.unwrap();
        retried.relinquish().await.unwrap();
        assert_eq!(objects(4, true).await.unwrap(), original4);
        assert_eq!(objects(6, true).await.unwrap(), original6);
        println!(
            "namespace reservation, rollback, recovery, foreign retention, Docker DNS/hosts, dual-stack capture, independent retirement: ok"
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated Docker namespace with NET_ADMIN and TUN"]
    async fn namespace_normal_tun_preparation() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
            Ok("1")
        );
        // This separate kernel-resource case uses the existing typed PacketPlan
        // fixture's loopback availability allocator; it is not part of the outer
        // socket syscall audit and never sends ordinary egress traffic.
        let (payloads, _) = crate::managed_dataplane::tests::fixtures(2);
        let plan = crate::managed_dataplane::PacketPlan::compile(&payloads[0], 1000).unwrap();
        let state = tempfile::Builder::new().tempdir_in("/run").unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let original4 = objects(4, true).await.unwrap();
        let original6 = objects(6, true).await.unwrap();
        let mut owner = LinuxUnderlay::prepare(Preparation {
            state_directory: state.path(),
            identity: "tun-fixture",
            network: "tun-network",
        })
        .await
        .unwrap();
        let control = owner.socket_generation().await.unwrap();
        let options = crate::managed_dataplane::TunOptions {
            name: "mwowner0".into(),
            mtu: 1000,
        };
        let tun =
            crate::managed_dataplane::tun::Tun::open_with_underlay(&options, &plan, &mut owner)
                .await
                .unwrap();
        let _binding = tun.underlay_binding().unwrap();
        let lookup = ip(vec![
            "-j".into(),
            "-4".into(),
            "route".into(),
            "get".into(),
            "100.64.0.2".into(),
        ])
        .await
        .unwrap();
        println!(
            "active actual TUN lookup: {}",
            String::from_utf8_lossy(&lookup)
        );
        assert!(String::from_utf8(lookup).unwrap().contains("mwowner0"));
        owner.withdraw_tun().await.unwrap();
        drop(tun);
        assert!(!Path::new("/sys/class/net/mwowner0").exists());
        assert!(
            ip(vec![
                "-4".into(),
                "route".into(),
                "get".into(),
                "100.64.0.2".into()
            ])
            .await
            .is_err()
        );
        assert!(
            ip(vec![
                "-4".into(),
                "route".into(),
                "get".into(),
                "100.64.0.2".into(),
                "from".into(),
                "172.30.247.20".into(),
                "iif".into(),
                "eth0".into()
            ])
            .await
            .is_err()
        );
        for address in ["172.30.247.20:41861", "[fd74:6d77:247::20]:41861"] {
            let socket = control
                .factory()
                .tcp(super::super::Purpose::Controller, address.parse().unwrap())
                .await
                .unwrap();
            let expected: std::net::IpAddr = if address.starts_with('[') {
                "fd74:6d77:247::12"
            } else {
                "172.30.247.12"
            }
            .parse()
            .unwrap();
            assert_eq!(socket.local_addr().unwrap().ip(), expected);
        }
        let replacement = crate::managed_dataplane::tun::Tun::open_with_underlay(
            &crate::managed_dataplane::TunOptions {
                name: "mwowner1".into(),
                mtu: 1000,
            },
            &plan,
            &mut owner,
        )
        .await
        .unwrap();
        owner.withdraw_tun().await.unwrap();
        drop(replacement);
        drop(control);
        owner.relinquish().await.unwrap();
        assert_eq!(objects(4, true).await.unwrap(), original4);
        assert_eq!(objects(6, true).await.unwrap(), original6);
    }

    #[tokio::test]
    #[ignore = "requires isolated Docker namespace; deliberately exits without Drop"]
    async fn namespace_crash_prepare() {
        use std::os::unix::fs::DirBuilderExt;
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
            Ok("1")
        );
        let directory = Path::new("/run/underlay-crash-state");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(directory)
            .unwrap();
        let mut owner = LinuxUnderlay::prepare(Preparation {
            state_directory: directory,
            identity: "crash-node",
            network: "crash-network",
        })
        .await
        .unwrap();
        owner
            .retain_mesh_sink("198.18.73.0/24".parse().unwrap())
            .await
            .unwrap();
        let generation = owner.socket_generation().await.unwrap();
        let _socket = generation
            .factory()
            .udp(super::super::Purpose::Stun, "0.0.0.0:0".parse().unwrap())
            .unwrap();
        println!(
            "committed actual namespace resources; exit bypasses all Rust Drop implementations"
        );
        std::io::stdout().flush().unwrap();
        std::process::exit(0);
    }

    #[tokio::test]
    #[ignore = "must run in a new process after namespace_crash_prepare"]
    async fn namespace_crash_recover() {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
            Ok("1")
        );
        let owner = LinuxUnderlay::prepare(Preparation {
            state_directory: Path::new("/run/underlay-crash-state"),
            identity: "crash-node",
            network: "crash-network",
        })
        .await
        .unwrap();
        assert!(
            owner
                .journal
                .installed
                .iter()
                .any(|item| matches!(item.resource, Resource::Sink { .. }))
        );
        assert!(
            ip(vec![
                "-4".into(),
                "route".into(),
                "get".into(),
                "198.18.73.2".into()
            ])
            .await
            .is_err()
        );
        let generation = owner.socket_generation().await.unwrap();
        assert!(
            generation
                .factory()
                .tcp(
                    super::super::Purpose::Controller,
                    "172.30.247.20:41861".parse().unwrap()
                )
                .await
                .is_ok()
        );
        drop(generation);
        owner.relinquish().await.unwrap();
        println!(
            "separate-process recovery verified durable journal, actual resources, sink and new protected capability"
        );
    }
}
